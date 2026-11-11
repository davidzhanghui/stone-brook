//! 元素 → 绘制图元（Prim）。
//!
//! 图元在「元素自身坐标系」里生成（相对 (x, y)、未旋转），绘制 / 导出时再统一平移和旋转，
//! 所以拖动元素不必重跑手绘算法。按「影响外观的字段」做哈希缓存。
//! 手绘参数对齐官方 `generateRoughOptions`（excalidraw `packages/element/src/shape.ts`）。

use super::geom::{self, corner_radius, curve_points, v, V};
use super::model::*;
use crate::canvas::{pt, Col, PathCmd, Pt};
use roughr::core::{FillStyle, OpSet, OpSetType, OpType, Options};
use roughr::generator::Generator;
use roughr::Point2D;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Prim {
    Fill {
        cmds: Vec<PathCmd>,
        color: Col,
    },
    Stroke {
        cmds: Vec<PathCmd>,
        color: Col,
        width: f32,
        /// 虚线：(实, 空)
        dash: Option<(f32, f32)>,
        /// 圆头圆角（自由画）
        round: bool,
    },
    Text {
        lines: Vec<String>,
        /// 文本框左上角（元素坐标系）
        x: f32,
        y: f32,
        w: f32,
        size: f32,
        line_h: f32,
        color: Col,
        family: i64,
        /// 0 左、1 中、2 右
        align: u8,
        /// 文字背后垫一块底色（箭头标签）
        bg: Option<Col>,
    },
    Image {
        w: f32,
        h: f32,
        path: Option<String>,
        /// 圆角半径
        radius: f32,
        /// 裁剪：原图像素里的 (x, y, w, h) 与原图尺寸 (nw, nh)
        crop: Option<[f32; 6]>,
    },
}

/// 元素的坐标变换：局部 → 世界。
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub ox: f32,
    pub oy: f32,
    pub angle: f32,
    /// 旋转中心（世界坐标）
    pub cx: f32,
    pub cy: f32,
}

impl Xf {
    pub fn of(e: &Element) -> Xf {
        let c = geom::rot_center(e);
        Xf { ox: e.x as f32, oy: e.y as f32, angle: e.angle as f32, cx: c.x as f32, cy: c.y as f32 }
    }
    pub fn apply(&self, p: Pt) -> Pt {
        let q = pt(p.x + self.ox, p.y + self.oy);
        if self.angle == 0. {
            return q;
        }
        let (s, c) = self.angle.sin_cos();
        let (dx, dy) = (q.x - self.cx, q.y - self.cy);
        pt(self.cx + dx * c - dy * s, self.cy + dx * s + dy * c)
    }
    pub fn cmds(&self, cmds: &[PathCmd]) -> Vec<PathCmd> {
        cmds.iter()
            .map(|c| match *c {
                PathCmd::Move(p) => PathCmd::Move(self.apply(p)),
                PathCmd::Line(p) => PathCmd::Line(self.apply(p)),
                PathCmd::Cubic(a, b, p) => PathCmd::Cubic(self.apply(a), self.apply(b), self.apply(p)),
                PathCmd::Close => PathCmd::Close,
            })
            .collect()
    }
}

// ───────────── 颜色 ─────────────

/// 解析官方颜色串；dark 时套用官方暗色滤镜 `invert(93%) hue-rotate(180deg)`。
pub fn color(s: &str, opacity: f64, dark: bool) -> Option<Col> {
    if s == TRANSPARENT || s.is_empty() {
        return None;
    }
    let c = Col::parse(s).or_else(|| named(s)).unwrap_or(Col::rgb(0x1e1e1e));
    let c = if dark { dark_filter(c) } else { c };
    let a = c.alpha() * (opacity / 100.).clamp(0., 1.) as f32;
    Some(c.with_alpha(a))
}

fn named(s: &str) -> Option<Col> {
    Some(Col::rgb(match s.to_lowercase().as_str() {
        "cyan" => 0x00ffff,
        "magenta" => 0xff00ff,
        "brown" => 0xa52a2a,
        "navy" => 0x000080,
        "teal" => 0x008080,
        "olive" => 0x808000,
        "maroon" => 0x800000,
        "silver" => 0xc0c0c0,
        "lime" => 0x00ff00,
        _ => return None,
    }))
}

pub fn dark_filter(c: Col) -> Col {
    let r = ((c.0 >> 24) & 0xff) as f32 / 255.;
    let g = ((c.0 >> 16) & 0xff) as f32 / 255.;
    let b = ((c.0 >> 8) & 0xff) as f32 / 255.;
    let inv = |x: f32| x + 0.93 * (1. - 2. * x);
    let (r, g, b) = (inv(r), inv(g), inv(b));
    let r2 = (-0.574 * r + 1.430 * g + 0.144 * b).clamp(0., 1.);
    let g2 = (0.426 * r + 0.430 * g + 0.144 * b).clamp(0., 1.);
    let b2 = (0.426 * r + 1.430 * g - 0.856 * b).clamp(0., 1.);
    let to = |x: f32| (x * 255.).round() as u32;
    Col((to(r2) << 24) | (to(g2) << 16) | (to(b2) << 8) | (c.0 & 0xff))
}

// ───────────── 手绘 ─────────────

fn rough_opts(e: &Element, with_fill: bool) -> Options {
    let sw = e.stroke_width as f32;
    let solid = e.stroke_style == "solid";
    let max_side = e.width.abs().max(e.height.abs()) as f32;
    // 官方 adjustRoughness：小图形降低粗糙度，避免糊成一团
    let mut rough = e.roughness as f32;
    if max_side < 10. {
        rough = rough.min(0.5);
    } else if max_side < 30. && rough > 1. {
        rough = 1.;
    }
    let mut o = Options {
        seed: Some(e.seed.unsigned_abs().max(1)),
        roughness: Some(rough),
        stroke_width: Some(if solid { sw } else { sw + 0.5 }),
        fill_weight: Some(sw / 2.),
        hachure_gap: Some(sw * 4.),
        disable_multi_stroke: Some(!solid),
        preserve_vertices: Some(e.roughness < 2.),
        ..Default::default()
    };
    if e.kind == "ellipse" {
        o.curve_fitting = Some(1.);
    }
    if with_fill && e.has_fill() {
        o.fill = Some(roughr::Srgba::new(0., 0., 0., 1.));
        o.fill_style = Some(match e.fill_style.as_str() {
            "hachure" => FillStyle::Hachure,
            "cross-hatch" => FillStyle::CrossHatch,
            "zigzag" => FillStyle::ZigZag,
            "dots" => FillStyle::Dots,
            "dashed" => FillStyle::Dashed,
            "zigzag-line" => FillStyle::ZigZagLine,
            _ => FillStyle::Solid,
        });
    }
    o
}

fn opset_cmds(set: &OpSet<f32>) -> Vec<PathCmd> {
    let mut out = vec![];
    for op in &set.ops {
        match op.op {
            OpType::Move => out.push(PathCmd::Move(pt(op.data[0], op.data[1]))),
            OpType::LineTo => out.push(PathCmd::Line(pt(op.data[0], op.data[1]))),
            OpType::BCurveTo => out.push(PathCmd::Cubic(pt(op.data[0], op.data[1]), pt(op.data[2], op.data[3]), pt(op.data[4], op.data[5]))),
        }
    }
    out
}

struct Style {
    stroke: Option<Col>,
    fill: Option<Col>,
    width: f32,
    dash: Option<(f32, f32)>,
}

fn style(e: &Element, dark: bool) -> Style {
    let sw = e.stroke_width as f32;
    let dash = match e.stroke_style.as_str() {
        "dashed" => Some((8., 8. + sw)),
        "dotted" => Some((1.5, 6. + sw)),
        _ => None,
    };
    Style {
        stroke: color(&e.stroke_color, e.opacity, dark),
        fill: color(&e.background_color, e.opacity, dark),
        width: if dash.is_some() { sw + 0.5 } else { sw },
        dash,
    }
}

fn push_drawable(out: &mut Vec<Prim>, d: roughr::core::Drawable<f32>, st: &Style, fill_weight: f32) {
    for set in &d.sets {
        let cmds = opset_cmds(set);
        if cmds.is_empty() {
            continue;
        }
        match set.op_set_type {
            OpSetType::FillPath => {
                if let Some(c) = st.fill {
                    out.push(Prim::Fill { cmds, color: c });
                }
            }
            OpSetType::FillSketch => {
                if let Some(c) = st.fill {
                    out.push(Prim::Stroke { cmds, color: c, width: fill_weight.max(0.5), dash: None, round: false });
                }
            }
            OpSetType::Path => {
                if let Some(c) = st.stroke {
                    out.push(Prim::Stroke { cmds, color: c, width: st.width, dash: st.dash, round: false });
                }
            }
        }
    }
}

fn p2(x: f64, y: f64) -> Point2D<f32> {
    Point2D::new(x as f32, y as f32)
}

/// 圆角矩形路径（官方用二次曲线）
fn round_rect_d(w: f64, h: f64, r: f64) -> String {
    format!("M {r} 0 L {} 0 Q {w} 0, {w} {r} L {w} {} Q {w} {h}, {} {h} L {r} {h} Q 0 {h}, 0 {} L 0 {r} Q 0 0, {r} 0", w - r, h - r, w - r, h - r)
}

fn diamond_pts(w: f64, h: f64) -> [V; 4] {
    [v(w / 2., 0.), v(w, h / 2.), v(w / 2., h), v(0., h / 2.)]
}

fn round_poly_d(pts: &[V], r: f64) -> String {
    let n = pts.len();
    let mut d = String::new();
    for i in 0..n {
        let (prev, cur, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let a = cur.add(prev.sub(cur).scale(r / prev.dist(cur).max(1e-6)));
        let b = cur.add(next.sub(cur).scale(r / next.dist(cur).max(1e-6)));
        if i == 0 {
            d.push_str(&format!("M {} {} ", a.x, a.y));
        } else {
            d.push_str(&format!("L {} {} ", a.x, a.y));
        }
        d.push_str(&format!("Q {} {}, {} {} ", cur.x, cur.y, b.x, b.y));
    }
    let a0 = {
        let (prev, cur) = (pts[n - 1], pts[0]);
        cur.add(prev.sub(cur).scale(r / prev.dist(cur).max(1e-6)))
    };
    d.push_str(&format!("L {} {} Z", a0.x, a0.y));
    d
}

/// 箭头头图元（局部坐标）。tip 为端点，from 为指向端点的前一点。
fn arrowhead(out: &mut Vec<Prim>, gen: &Generator, e: &Element, kind: &str, tip: V, from: V, st: &Style, bg: Option<Col>) {
    let seg = tip.dist(from);
    if seg < 1e-6 {
        return;
    }
    let base: f64 = match kind {
        "arrow" => 25.,
        "diamond" | "diamond_outline" => 12.,
        "bar" => 15.,
        _ => 15.,
    };
    let mult = if kind.starts_with("diamond") { 0.25 } else { 0.5 };
    let size = base.min(seg * mult).max(4.);
    let dir = tip.sub(from).scale(1. / seg);
    let back = tip.sub(dir.scale(size));
    let mut opts = rough_opts(e, false);
    opts.fill = None;
    let side = |deg: f64| back.rot(deg.to_radians(), tip);
    let solid_style = Style { stroke: st.stroke, fill: st.stroke, width: st.width, dash: None };
    let outline_style = Style { stroke: st.stroke, fill: bg, width: st.width, dash: None };
    let fw = (e.stroke_width / 2.) as f32;
    match kind {
        "arrow" => {
            for deg in [20., -20.] {
                let p = side(deg);
                push_drawable(out, gen.linear_path(&[p2(p.x, p.y), p2(tip.x, tip.y)], false, &Some(opts.clone())), &solid_style, fw);
            }
        }
        "bar" => {
            let perp = v(-dir.y, dir.x).scale(size / 2.);
            let (a, b) = (tip.add(perp), tip.sub(perp));
            push_drawable(out, gen.linear_path(&[p2(a.x, a.y), p2(b.x, b.y)], false, &Some(opts)), &solid_style, fw);
        }
        "dot" | "circle" | "circle_outline" => {
            let d = size.max(e.stroke_width * 4.);
            let c = tip.sub(dir.scale(d / 2.));
            let outline = kind == "circle_outline";
            let mut o = opts.clone();
            o.fill = Some(roughr::Srgba::new(0., 0., 0., 1.));
            o.fill_style = Some(FillStyle::Solid);
            let stl = if outline { &outline_style } else { &solid_style };
            if outline && bg.is_none() {
                o.fill = None;
            }
            push_drawable(out, gen.ellipse(c.x as f32, c.y as f32, d as f32, d as f32, &Some(o)), stl, fw);
        }
        "triangle" | "triangle_outline" => {
            let (a, b) = (side(25.), side(-25.));
            let outline = kind == "triangle_outline";
            let mut o = opts.clone();
            o.fill = Some(roughr::Srgba::new(0., 0., 0., 1.));
            o.fill_style = Some(FillStyle::Solid);
            if outline && bg.is_none() {
                o.fill = None;
            }
            let stl = if outline { &outline_style } else { &solid_style };
            push_drawable(out, gen.polygon(&[p2(tip.x, tip.y), p2(a.x, a.y), p2(b.x, b.y)], &Some(o)), stl, fw);
        }
        "diamond" | "diamond_outline" => {
            let mid = tip.sub(dir.scale(size));
            let back2 = tip.sub(dir.scale(size * 2.));
            let perp = v(-dir.y, dir.x).scale(size / 2.);
            let (a, b) = (mid.add(perp), mid.sub(perp));
            let outline = kind == "diamond_outline";
            let mut o = opts.clone();
            o.fill = Some(roughr::Srgba::new(0., 0., 0., 1.));
            o.fill_style = Some(FillStyle::Solid);
            if outline && bg.is_none() {
                o.fill = None;
            }
            let stl = if outline { &outline_style } else { &solid_style };
            push_drawable(out, gen.polygon(&[p2(tip.x, tip.y), p2(a.x, a.y), p2(back2.x, back2.y), p2(b.x, b.y)], &Some(o)), stl, fw);
        }
        // 官方的 crowfoot 等少见类型：退化成普通箭头
        _ => arrowhead(out, gen, e, "arrow", tip, from, st, bg),
    }
}

/// 在 CPU 上把路径切成虚线段（实线部分输出为独立子路径）。
///
/// 不用 GPUI 的 `dash_array`：它借助 lyon 的路径测量，遇到手绘描边这种多子路径会触发
/// lyon `measure.rs` 的 `unreachable!`。自己切还能保证屏幕与 SVG 导出的虚线一致。
pub fn dash_cmds(cmds: &[PathCmd], on: f32, off: f32) -> Vec<PathCmd> {
    let mut out = vec![];
    let (on, off) = (on.max(0.5), off.max(0.5));
    for (mut pts, closed) in crate::canvas::paint::flatten(cmds) {
        if closed {
            if let Some(f) = pts.first().copied() {
                pts.push(f);
            }
        }
        let mut drawing = true;
        let mut left = on;
        let mut pen_down = false;
        for w in pts.windows(2) {
            let (mut a, b) = (w[0], w[1]);
            let mut seg = a.dist(b);
            while seg > 1e-4 {
                let step = left.min(seg);
                let t = step / seg;
                let m = pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                if drawing {
                    if !pen_down {
                        out.push(PathCmd::Move(a));
                        pen_down = true;
                    }
                    out.push(PathCmd::Line(m));
                }
                seg -= step;
                left -= step;
                a = m;
                if left <= 1e-4 {
                    drawing = !drawing;
                    left = if drawing { on } else { off };
                    pen_down = false;
                }
            }
        }
    }
    out
}

/// 生成一个元素的图元（局部坐标）。虚线一律在这里切好。
pub fn prims(doc: &SketchDoc, e: &Element, dark: bool) -> Vec<Prim> {
    prims_raw(doc, e, dark)
        .into_iter()
        .map(|p| match p {
            Prim::Stroke { cmds, color, width, dash: Some((on, off)), round } => {
                Prim::Stroke { cmds: dash_cmds(&cmds, on, off), color, width, dash: None, round }
            }
            other => other,
        })
        .collect()
}

fn prims_raw(doc: &SketchDoc, e: &Element, dark: bool) -> Vec<Prim> {
    let gen = Generator::default();
    let st = style(e, dark);
    let fw = (e.stroke_width / 2.) as f32;
    let (w, h) = (e.width, e.height);
    let mut out = vec![];
    match e.kind.as_str() {
        "rectangle" => {
            let o = rough_opts(e, true);
            let r = corner_radius(e);
            let d = if r > 0. { gen.path::<f32>(round_rect_d(w, h, r), &Some(o)) } else { gen.rectangle::<f32>(0., 0., w as f32, h as f32, &Some(o)) };
            push_drawable(&mut out, d, &st, fw);
        }
        "diamond" => {
            let o = rough_opts(e, true);
            let pts = diamond_pts(w, h);
            let r = if e.roundness().is_some() { (w.abs().min(h.abs()) * 0.25 * 0.5).min(20.) } else { 0. };
            let d = if r > 1. { gen.path::<f32>(round_poly_d(&pts, r), &Some(o)) } else { gen.polygon::<f32>(&pts.map(|p| p2(p.x, p.y)), &Some(o)) };
            push_drawable(&mut out, d, &st, fw);
        }
        "ellipse" => {
            let o = rough_opts(e, true);
            push_drawable(&mut out, gen.ellipse::<f32>((w / 2.) as f32, (h / 2.) as f32, w as f32, h as f32, &Some(o)), &st, fw);
        }
        "line" | "arrow" => {
            let pts: Vec<V> = e.pts().iter().map(|p| v(p[0], p[1])).collect();
            if pts.len() >= 2 {
                let closed = pts.len() > 2 && pts[0].dist(*pts.last().unwrap()) < 1.;
                let round = e.roundness().is_some() && !e.elbowed();
                let mut o = rough_opts(e, closed);
                if !closed {
                    o.fill = None;
                }
                let ps: Vec<Point2D<f32>> = pts.iter().map(|p| p2(p.x, p.y)).collect();
                let d = if closed && e.has_fill() {
                    gen.polygon::<f32>(&ps, &Some(o))
                } else if round && pts.len() > 2 {
                    gen.curve::<f32>(&ps, &Some(o))
                } else {
                    gen.linear_path::<f32>(&ps, false, &Some(o))
                };
                push_drawable(&mut out, d, &st, fw);
                if e.kind == "arrow" {
                    let flat = curve_points(&pts, round);
                    let bgc = color(&e.background_color, e.opacity, dark);
                    if let Some(k) = e.arrowhead(false) {
                        let n = flat.len();
                        arrowhead(&mut out, &gen, e, k, flat[n - 1], back_point(&flat, true), &st, bgc);
                    }
                    if let Some(k) = e.arrowhead(true) {
                        arrowhead(&mut out, &gen, e, k, flat[0], back_point(&flat, false), &st, bgc);
                    }
                }
            } else if let Some(c) = st.stroke {
                // 刚落笔的单点
                let r = e.stroke_width.max(1.) as f32;
                out.push(Prim::Fill { cmds: crate::canvas::geom::ellipse_cmds(crate::canvas::rect(-r, -r, 2. * r, 2. * r)), color: c });
            }
        }
        "freedraw" => {
            let pts: Vec<Pt> = e.pts().iter().map(|p| pt(p[0] as f32, p[1] as f32)).collect();
            if let Some(c) = st.stroke {
                let wdt = (e.stroke_width * 2.25) as f32;
                if pts.len() == 1 || pts.iter().all(|p| p.dist(pts[0]) < 0.5) {
                    let r = wdt / 2.;
                    let p = pts[0];
                    out.push(Prim::Fill { cmds: crate::canvas::geom::ellipse_cmds(crate::canvas::rect(p.x - r, p.y - r, 2. * r, 2. * r)), color: c });
                } else {
                    // 闭合且有底色：先填充
                    if let Some(f) = st.fill.filter(|_| pts.len() > 2 && pts[0].dist(*pts.last().unwrap()) < 8.) {
                        out.push(Prim::Fill { cmds: crate::canvas::geom::polygon(&pts), color: f });
                    }
                    let smooth = crate::canvas::geom::smooth_cmds(&simplify(&pts, 0.6));
                    out.push(Prim::Stroke { cmds: smooth, color: c, width: wdt, dash: None, round: true });
                }
            }
        }
        "text" => {
            let text = e.text.clone().unwrap_or_default();
            let size = e.font_size_or() as f32;
            let lh = (e.font_size_or() * e.line_height()) as f32;
            let align = match e.text_align() {
                "center" => 1,
                "right" => 2,
                _ => 0,
            };
            // 箭头标签：背后垫画布底色，挡住箭头线
            let label_bg =
                e.container_id().and_then(|c| doc.get(c)).filter(|c| c.is_linear()).map(|_| color(&doc.bg_color(), 100., dark).unwrap_or(Col::rgb(0xffffff)));
            if let Some(c) = st.stroke {
                out.push(Prim::Text {
                    lines: text.split('\n').map(|s| s.to_string()).collect(),
                    x: 0.,
                    y: 0.,
                    w: w as f32,
                    size,
                    line_h: lh,
                    color: c,
                    family: e.font_family(),
                    align,
                    bg: label_bg,
                });
            }
        }
        "image" => {
            let radius = if e.roundness().is_some() { corner_radius(e) as f32 } else { 0. };
            out.push(Prim::Image { w: w as f32, h: h as f32, path: e.file_id().and_then(|f| doc.file_path(f)), radius, crop: image_crop(e) });
        }
        "frame" | "magicframe" => {
            let c = color(if e.stroke_color == DEFAULT_STROKE { "#bbb" } else { &e.stroke_color }, e.opacity, dark).unwrap_or(Col::rgb(0xbbbbbb));
            out.push(Prim::Stroke {
                cmds: crate::canvas::geom::round_rect_cmds(crate::canvas::rect(0., 0., w as f32, h as f32), 8.),
                color: c,
                width: 1.,
                dash: None,
                round: false,
            });
            let name = e.name().map(|s| s.to_string()).unwrap_or_else(|| if e.kind == "magicframe" { "AI Frame".into() } else { "Frame".into() });
            out.push(Prim::Text {
                lines: vec![name],
                x: 0.,
                y: -22.,
                w: w as f32,
                size: 14.,
                line_h: 18.,
                color: color("#868e96", 100., dark).unwrap(),
                family: FONT_NORMAL,
                align: 0,
                bg: None,
            });
        }
        "stickynote" => sticky_prims(&mut out, e, dark),
        "embeddable" | "iframe" if e.link().is_some() => {
            let link = e.link().unwrap_or_default().to_string();
            let fillc = color(if dark { "#1e1e1e" } else { "#f8f9fa" }, e.opacity, false).unwrap();
            let edge = color("#ced4da", e.opacity, dark).unwrap();
            let r = corner_radius(e).min(16.) as f32;
            let rect = crate::canvas::rect(0., 0., w as f32, h as f32);
            out.push(Prim::Fill { cmds: crate::canvas::geom::round_rect_cmds(rect, r), color: fillc });
            out.push(Prim::Stroke { cmds: crate::canvas::geom::round_rect_cmds(rect, r), color: edge, width: 1.5, dash: None, round: false });
            let host = link.split("://").nth(1).unwrap_or(&link).split('/').next().unwrap_or("").to_string();
            let short: String = if link.chars().count() > 60 { link.chars().take(57).chain("…".chars()).collect() } else { link.clone() };
            let ink = color(DEFAULT_STROKE, e.opacity, dark).unwrap();
            let muted = color("#868e96", e.opacity, dark).unwrap();
            out.push(Prim::Text {
                lines: vec![format!("🌐 {host}")],
                x: 0.,
                y: (h / 2. - 24.) as f32,
                w: w as f32,
                size: 18.,
                line_h: 24.,
                color: ink,
                family: FONT_NORMAL,
                align: 1,
                bg: None,
            });
            out.push(Prim::Text {
                lines: vec![short, "双击在浏览器中打开".into()],
                x: 0.,
                y: (h / 2. + 4.) as f32,
                w: w as f32,
                size: 12.,
                line_h: 17.,
                color: muted,
                family: FONT_NORMAL,
                align: 1,
                bg: None,
            });
        }
        // iframe / embeddable 等：虚线框 + 说明，数据原样保留
        _ => {
            let c = color("#868e96", 100., dark).unwrap();
            out.push(Prim::Stroke {
                cmds: crate::canvas::geom::round_rect_cmds(crate::canvas::rect(0., 0., w as f32, h as f32), 6.),
                color: c,
                width: 1.5,
                dash: Some((6., 4.)),
                round: false,
            });
            let label = match e.link() {
                Some(l) => format!("嵌入内容（{}）\n{}", e.kind, l),
                None => format!("不支持的元素：{}", e.kind),
            };
            out.push(Prim::Text {
                lines: label.split('\n').map(|s| s.to_string()).collect(),
                x: 0.,
                y: (h / 2. - 20.) as f32,
                w: w as f32,
                size: 14.,
                line_h: 20.,
                color: c,
                family: FONT_NORMAL,
                align: 1,
                bg: None,
            });
        }
    }
    out
}

/// 图片裁剪（官方 `crop`：原图像素坐标）
fn image_crop(e: &Element) -> Option<[f32; 6]> {
    let c = e.get("crop")?;
    let g = |k: &str| c.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    let (x, y, w, h, nw, nh) = (g("x")?, g("y")?, g("width")?, g("height")?, g("naturalWidth")?, g("naturalHeight")?);
    (w > 0. && h > 0. && nw > 0. && nh > 0.).then_some([x, y, w, h, nw, nh])
}

/// 便签纸面四角（官方 `getStickyNoteRenderPoints`：按粗糙度给角点一点抖动）
fn sticky_points(e: &Element, off: f32, seed_off: i64) -> [Pt; 4] {
    let (w, h) = (e.width as f32, e.height as f32);
    let level = e.roughness.round().clamp(0., 2.) as usize;
    let amount = [0f32, 1.5, 8.][level].min(w.min(h) * 0.012);
    let mut st = (e.seed + seed_off).unsigned_abs() as u32 | 1;
    let mut rnd = || {
        st ^= st << 13;
        st ^= st >> 17;
        st ^= st << 5;
        (st as f32 / u32::MAX as f32 - 0.5) * 2. * amount
    };
    [pt(off + rnd(), off + rnd()), pt(off + w + rnd(), off + rnd()), pt(off + w + rnd(), off + h + rnd()), pt(off + rnd(), off + h + rnd())]
}

fn toward(a: Pt, b: Pt, d: f32) -> Pt {
    let l = a.dist(b).max(1e-6);
    pt(a.x + (b.x - a.x) * d / l, a.y + (b.y - a.y) * d / l)
}

/// 二次贝塞尔转三次
fn quad(from: Pt, ctrl: Pt, to: Pt) -> PathCmd {
    let c1 = pt(from.x + (ctrl.x - from.x) * 2. / 3., from.y + (ctrl.y - from.y) * 2. / 3.);
    let c2 = pt(to.x + (ctrl.x - to.x) * 2. / 3., to.y + (ctrl.y - to.y) * 2. / 3.);
    PathCmd::Cubic(c1, c2, to)
}

/// 便签轮廓（官方 `getStickyNotePathCommands`）：圆角；「潦草」时随机一角微微翘起。
fn sticky_path(e: &Element, shadow: bool) -> Vec<PathCmd> {
    let off = if shadow { 3. } else { 0. };
    let pts = sticky_points(e, off, if shadow { 1 } else { 0 });
    let (w, h) = (e.width as f32, e.height as f32);
    let radius = if e.roundness().is_some() { (w.min(h) * 0.04).min(16.) } else { 0. };
    let lifted = if e.roughness >= 1.5 { (e.seed.unsigned_abs() % 4) as usize } else { 9 };
    let mut segs: Vec<Vec<(Pt, Option<Pt>)>> = vec![]; // (终点, 控制点)
    for i in 0..4 {
        let (prev, p, next) = (pts[(i + 3) % 4], pts[i], pts[(i + 1) % 4]);
        let r = radius.min(p.dist(prev) / 2.).min(p.dist(next) / 2.);
        if i == lifted {
            let size = w.min(h);
            let reach = (size * 0.18).min(40.);
            let lift = (size * 0.02).min(5.) * if shadow { 0.5 } else { 1. };
            let tip = pt(p.x + if i == 1 || i == 2 { -lift } else { lift }, p.y + if i >= 2 { -lift } else { lift });
            let start = toward(p, prev, reach);
            let end = toward(p, next, reach);
            segs.push(vec![
                (start, None),
                (toward(tip, start, r), Some(toward(p, prev, reach / 2.))),
                (toward(tip, end, r), Some(tip)),
                (end, Some(toward(p, next, reach / 2.))),
            ]);
        } else {
            segs.push(vec![(toward(p, prev, r), None), (toward(p, next, r), Some(p))]);
        }
    }
    let start = segs[0].last().unwrap().0;
    let mut cmds = vec![PathCmd::Move(start)];
    let mut cur = start;
    for k in 1..=4 {
        for (to, ctrl) in &segs[k % 4] {
            cmds.push(match ctrl {
                Some(c) => quad(cur, *c, *to),
                None => PathCmd::Line(*to),
            });
            cur = *to;
        }
    }
    cmds.push(PathCmd::Close);
    cmds
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// 便签日期（官方 `getStickyNoteDateLabel`）：「5 Nov」，不是今年时带年份
pub fn sticky_date(created: i64, short: bool) -> Option<String> {
    let secs = created.div_euclid(1000);
    let local = chrono::DateTime::from_timestamp(secs, 0)?.with_timezone(&chrono::Local);
    use chrono::Datelike;
    let label = format!("{} {}", local.day(), MONTHS[local.month0() as usize]);
    let this_year = chrono::Local::now().year();
    Some(if short || local.year() == this_year { label } else { format!("{label} {}", local.year()) })
}

fn sticky_prims(out: &mut Vec<Prim>, e: &Element, dark: bool) {
    let bg = color(if e.has_fill() { &e.background_color } else { STICKY_BG }, e.opacity, dark).unwrap_or(Col::rgb(0xffdf6b));
    let shadow = Col::rgb(0x000000).with_alpha(0.16 * e.opacity as f32 / 100.);
    out.push(Prim::Fill { cmds: sticky_path(e, true), color: shadow });
    let paper = sticky_path(e, false);
    out.push(Prim::Fill { cmds: paper.clone(), color: bg });
    out.push(Prim::Stroke { cmds: paper, color: Col::rgb(0x000000).with_alpha(0.08), width: 0.5, dash: None, round: false });
    if e.width >= STICKY_MIN && e.height >= STICKY_MIN {
        let created = e.get("created").and_then(|v| v.as_i64());
        let short = e.width - STICKY_PADDING * 2. < 80.;
        if let Some(label) = created.and_then(|c| sticky_date(c, short)) {
            let ink = color(&e.stroke_color, e.opacity, dark).unwrap_or(Col::rgb(0x1e1e1e));
            out.push(Prim::Text {
                lines: vec![label],
                x: 0.,
                y: (e.height - 14. - 11.) as f32,
                w: (e.width - STICKY_PADDING) as f32,
                size: 12.,
                line_h: 14.,
                color: ink,
                family: 2,
                align: 2,
                bg: None,
            });
        }
    }
}

fn back_point(flat: &[V], end: bool) -> V {
    // 往回找一个离端点足够远的点，曲线箭头的方向更稳定
    let n = flat.len();
    let tip = if end { flat[n - 1] } else { flat[0] };
    let it: Box<dyn Iterator<Item = &V>> = if end { Box::new(flat.iter().rev().skip(1)) } else { Box::new(flat.iter().skip(1)) };
    let mut last = if end { flat[n - 2] } else { flat[1] };
    for p in it {
        last = *p;
        if p.dist(tip) > 12. {
            break;
        }
    }
    last
}

/// Ramer–Douglas–Peucker 抽稀（自由画点太密时画出来发抖）
fn simplify(pts: &[Pt], eps: f32) -> Vec<Pt> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (mut idx, mut dmax) = (0, 0.);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = seg_dist(*p, a, b);
        if d > dmax {
            idx = i;
            dmax = d;
        }
    }
    if dmax > eps {
        let mut l = simplify(&pts[..=idx], eps);
        let r = simplify(&pts[idx..], eps);
        l.pop();
        l.extend(r);
        l
    } else {
        vec![a, b]
    }
}

use crate::canvas::math::dist_seg as seg_dist;

// ───────────── 缓存 ─────────────

fn shape_hash(doc: &SketchDoc, e: &Element, dark: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    e.kind.hash(&mut h);
    for f in [e.width, e.height, e.stroke_width, e.roughness, e.opacity] {
        f.to_bits().hash(&mut h);
    }
    e.stroke_color.hash(&mut h);
    e.background_color.hash(&mut h);
    e.fill_style.hash(&mut h);
    e.stroke_style.hash(&mut h);
    e.seed.hash(&mut h);
    dark.hash(&mut h);
    for p in e.pts() {
        p[0].to_bits().hash(&mut h);
        p[1].to_bits().hash(&mut h);
    }
    e.text.hash(&mut h);
    e.font_size.map(|f| f.to_bits()).hash(&mut h);
    for k in [
        "roundness",
        "fontFamily",
        "textAlign",
        "startArrowhead",
        "endArrowhead",
        "elbowed",
        "fileId",
        "name",
        "lineHeight",
        "containerId",
        "link",
        "crop",
        "created",
    ] {
        if let Some(v) = e.extra.get(k) {
            k.hash(&mut h);
            v.to_string().hash(&mut h);
        }
    }
    if let Some(f) = e.file_id() {
        doc.file_path(f).hash(&mut h);
    }
    if e.container_id().is_some() {
        doc.bg_color().hash(&mut h);
    }
    h.finish()
}

thread_local! {
    static CACHE: RefCell<crate::lru::Lru<String, (u64, Arc<Vec<Prim>>)>> = RefCell::new(crate::lru::Lru::new(20000));
}

/// 带缓存的图元。
pub fn cached(doc: &SketchDoc, e: &Element, dark: bool) -> Arc<Vec<Prim>> {
    let key = shape_hash(doc, e, dark);
    let ck = format!("{}{}", e.id, if dark { "#d" } else { "" });
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&ck).filter(|(k, _)| *k == key).map(|(_, p)| p.clone())) {
        return hit;
    }
    let p = Arc::new(prims(doc, e, dark));
    CACHE.with(|c| {
        c.borrow_mut().insert(ck, (key, p.clone()));
    });
    p
}

/// 字体族 → 系统字体（屏幕绘制）与 SVG font-family。
pub fn font_family(f: i64) -> (&'static str, &'static str) {
    match f {
        // Helvetica / Nunito / Lilita One / Liberation Sans
        2 | 6 | 7 | 9 => ("PingFang SC", "Nunito, Helvetica, PingFang SC, sans-serif"),
        // Cascadia / Comic Shanns
        3 | 8 => ("Menlo", "Comic Shanns, Cascadia, Menlo, monospace"),
        // Virgil / Excalifont（手写）
        _ => (super::fonts::HAND_FAMILY, "Stonebrook Sketch, Excalifont, Virgil, Xiaolai, Chalkboard SE, Kaiti SC, cursive"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashed_multi_subpath_is_split_on_cpu() {
        // 两个子路径（手绘描边的典型形状）
        let cmds = vec![PathCmd::Move(pt(0., 0.)), PathCmd::Line(pt(100., 0.)), PathCmd::Move(pt(0., 2.)), PathCmd::Line(pt(100., 2.))];
        let d = dash_cmds(&cmds, 8., 10.);
        let moves = d.iter().filter(|c| matches!(c, PathCmd::Move(_))).count();
        assert!(moves >= 10, "{moves}");
        let mut doc = SketchDoc::new();
        let mut r = Element::new("diamond", 0., 0.);
        r.width = 100.;
        r.height = 80.;
        r.stroke_style = "dashed".into();
        doc.elements.push(r.clone());
        assert!(prims(&doc, &r, false).iter().all(|p| !matches!(p, Prim::Stroke { dash: Some(_), .. })));
    }
}
