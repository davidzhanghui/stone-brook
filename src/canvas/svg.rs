//! 场景 → SVG 字符串；SVG → PNG（resvg）。

use super::geom::*;
use super::paint::{line_cmds, rough_cmds};
use super::*;
use std::fmt::Write;
use std::sync::{Arc, OnceLock};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn d_attr(cmds: &[PathCmd]) -> String {
    let mut d = String::new();
    for c in cmds {
        match *c {
            PathCmd::Move(p) => write!(d, "M{:.1} {:.1} ", p.x, p.y).ok(),
            PathCmd::Line(p) => write!(d, "L{:.1} {:.1} ", p.x, p.y).ok(),
            PathCmd::Cubic(a, b, p) => write!(d, "C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1} ", a.x, a.y, b.x, b.y, p.x, p.y).ok(),
            PathCmd::Close => write!(d, "Z ").ok(),
        };
    }
    d
}

fn color_attr(name: &str, c: Option<Col>) -> String {
    match c {
        Some(c) if c.alpha() > 0. => {
            if c.alpha() < 1. {
                format!(" {name}=\"{}\" {name}-opacity=\"{:.2}\"", c.hex(), c.alpha())
            } else {
                format!(" {name}=\"{}\"", c.hex())
            }
        }
        _ => format!(" {name}=\"none\""),
    }
}

fn path_el(out: &mut String, cmds: &[PathCmd], fill: Option<Col>, stroke: Option<Col>, w: f32, dash: Option<f32>) {
    if cmds.is_empty() {
        return;
    }
    let dash = dash.map(|d| format!(" stroke-dasharray=\"{d} {:.1}\"", d * 0.7)).unwrap_or_default();
    writeln!(
        out,
        "<path d=\"{}\"{}{} stroke-width=\"{w}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}/>",
        d_attr(cmds),
        color_attr("fill", fill),
        color_attr("stroke", stroke)
    )
    .ok();
}

/// 按估算宽度换行。
fn wrap(text: &str, size: f32, bold: bool, max_w: f32) -> Vec<String> {
    wrap_ranges(text, size, bold, max_w).into_iter().map(|r| text[r].to_string()).collect()
}

/// 自动换行，返回每个视觉行在全文中的字节区间（不含换行符）。
fn wrap_ranges(text: &str, size: f32, bold: bool, max_w: f32) -> Vec<std::ops::Range<usize>> {
    let mut out = vec![];
    let mut pos = 0;
    for para in text.split('\n') {
        let mut start = pos;
        for (i, ch) in para.char_indices() {
            let end = pos + i + ch.len_utf8();
            if estimate_width(&text[start..end], size, bold) > max_w && end - start > ch.len_utf8() {
                out.push(start..pos + i);
                start = pos + i;
            }
        }
        out.push(start..pos + para.len());
        pos += para.len() + 1;
    }
    out
}

/// 带样式分段的多行文字：每行一个 <text>，各分段为 <tspan>。
#[allow(clippy::too_many_arguments)]
fn rich_text_el(
    out: &mut String,
    text: &str,
    lines: &[std::ops::Range<usize>],
    segs: &[(usize, super::Fmt)],
    x: f32,
    y_center: f32,
    size: f32,
    color: Col,
    anchor: Anchor,
) {
    let lh = size * 1.38;
    let total = lh * lines.len() as f32;
    let anchor = match anchor {
        Anchor::Start => "start",
        Anchor::Middle => "middle",
        Anchor::End => "end",
    };
    for (i, line) in lines.iter().enumerate() {
        let y = y_center - total / 2. + lh * i as f32 + lh * 0.72;
        let mut body = String::new();
        let mut at = 0;
        for (l, f) in segs {
            let (a, b) = (at.max(line.start), (at + l).min(line.end));
            at += l;
            if b <= a {
                continue;
            }
            let mut deco = vec![];
            if f.deco & super::DECO_UNDERLINE != 0 {
                deco.push("underline");
            }
            if f.deco & super::DECO_STRIKE != 0 {
                deco.push("line-through");
            }
            write!(
                body,
                "<tspan{}{}{}{}>{}</tspan>",
                color_attr("fill", Some(f.color.unwrap_or(color))),
                if f.bold { " font-weight=\"600\"" } else { "" },
                if f.deco & super::DECO_ITALIC != 0 { " font-style=\"italic\"" } else { "" },
                if deco.is_empty() { String::new() } else { format!(" text-decoration=\"{}\"", deco.join(" ")) },
                esc(&text[a..b])
            )
            .ok();
        }
        writeln!(out, "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size}\" text-anchor=\"{anchor}\" xml:space=\"preserve\">{body}</text>").ok();
    }
}

fn text_el(out: &mut String, lines: &[String], x: f32, y_center: f32, size: f32, color: Col, bold: bool, anchor: Anchor) {
    let lh = size * 1.38;
    let total = lh * lines.len() as f32;
    let anchor = match anchor {
        Anchor::Start => "start",
        Anchor::Middle => "middle",
        Anchor::End => "end",
    };
    for (i, l) in lines.iter().enumerate() {
        let y = y_center - total / 2. + lh * i as f32 + lh * 0.72;
        writeln!(
            out,
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size}\" text-anchor=\"{anchor}\"{}{}>{}</text>",
            color_attr("fill", Some(color)),
            if bold { " font-weight=\"600\"" } else { "" },
            esc(l)
        )
        .ok();
    }
}

pub fn to_svg(scene: &Scene) -> String {
    let mut out = String::new();
    let font = scene.font.clone().unwrap_or_else(|| "PingFang SC, -apple-system, sans-serif".into());
    writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\" font-family=\"{}\">",
        scene.width,
        scene.height,
        scene.width,
        scene.height,
        esc(&font)
    )
    .ok();
    if let Some(bg) = scene.bg {
        writeln!(out, "<rect width=\"100%\" height=\"100%\"{}/>", color_attr("fill", Some(bg))).ok();
    }
    let mut i = 0;
    while i < scene.items.len() {
        if matches!(scene.items[i], Item::Line(_)) {
            let start = i;
            while i < scene.items.len() && matches!(scene.items[i], Item::Line(_)) {
                if let Item::Line(l) = &scene.items[i] {
                    line_svg_stroke(&mut out, l, scene.rough);
                }
                i += 1;
            }
            for item in &scene.items[start..i] {
                if let Item::Line(l) = item {
                    line_svg_labels(&mut out, l);
                }
            }
            continue;
        }
        let item = &scene.items[i];
        i += 1;
        match item {
            Item::Shape(s) => shape_svg(&mut out, s, scene.rough),
            Item::Line(_) => {}
            Item::Text(t) => text_el(&mut out, std::slice::from_ref(&t.text), t.pos.x, t.pos.y - t.size * 0.35, t.size, t.color, t.bold, t.anchor),
            Item::Path(p) => path_el(&mut out, &p.cmds, p.fill, p.stroke, p.width, p.dash.then_some(5.)),
            Item::Icon(i) => {
                let c = i.color;
                let (x, y, w, h) = (i.rect.x, i.rect.y, i.rect.w, i.rect.h);
                if crate::board::iconlib::is_color(&i.name) {
                    // 彩色图标作为内嵌 SVG 图片，各自的渐变 / 蒙版 id 互不干扰
                    if let Some(svg) = crate::board::iconlib::svg(&i.name, None) {
                        use base64::Engine;
                        let data = base64::engine::general_purpose::STANDARD.encode(svg);
                        writeln!(out, "<image x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" href=\"data:image/svg+xml;base64,{data}\"/>").ok();
                    }
                } else if let Some(svg) = crate::board::iconlib::svg_with(
                    &i.name,
                    Some(&format!("#{:06x}", c.0 >> 8)),
                    &format!("x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" opacity=\"{:.3}\"", c.alpha()),
                ) {
                    writeln!(out, "{svg}").ok();
                }
            }
            Item::Image(i) => {
                if let Ok(bytes) = std::fs::read(&i.path) {
                    use base64::Engine;
                    let mime = if i.path.ends_with(".png") { "image/png" } else { "image/jpeg" };
                    writeln!(
                        out,
                        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"data:{mime};base64,{}\"/>",
                        i.rect.x,
                        i.rect.y,
                        i.rect.w,
                        i.rect.h,
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    )
                    .ok();
                }
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

fn shape_svg(out: &mut String, s: &ShapeItem, rough: bool) {
    let r = s.rect;
    let (body, deco) = shape_cmds(s.kind, r, s.radius);
    let dash = if s.dotted { Some(2.) } else { s.dash.then_some(5.) };
    match s.kind {
        ShapeKind::StartDot | ShapeKind::Bar => path_el(out, &body, s.stroke.or(s.fill), None, 0., None),
        ShapeKind::EndDot => {
            path_el(out, &body, None, s.stroke, s.stroke_w, None);
            path_el(out, &deco, s.stroke, None, 0., None);
        }
        _ if rough && !(s.kind == ShapeKind::Sticky || s.id.starts_with("sec:")) => {
            let (stroke, hatch) = rough_cmds(&body, s.seed, s.fill.is_some(), s.stroke_w);
            path_el(out, &hatch, None, s.fill, s.stroke_w * 0.8, None);
            path_el(out, &stroke, None, s.stroke, s.stroke_w, None);
            let (d, _) = rough_cmds(&deco, s.seed + 7, false, s.stroke_w);
            path_el(out, &d, None, s.stroke, s.stroke_w, None);
        }
        _ => {
            path_el(out, &body, s.fill, s.stroke, s.stroke_w, dash);
            path_el(out, &deco, None, s.stroke, s.stroke_w, None);
        }
    }
    if matches!(s.kind, ShapeKind::Frame | ShapeKind::FrameAlt) {
        if !s.text.is_empty() {
            let w = estimate_width(&s.text, s.font_size, true) + 16.;
            let h = s.font_size * 1.7;
            let tab = polygon(&[pt(r.x, r.y), pt(r.x + w, r.y), pt(r.x + w, r.y + h - 5.), pt(r.x + w - 6., r.y + h), pt(r.x, r.y + h)]);
            path_el(out, &tab, s.stroke.map(|c| c.with_alpha(0.08)), s.stroke, s.stroke_w * 0.8, None);
            text_el(out, std::slice::from_ref(&s.text), r.x + w / 2., r.y + h / 2., s.font_size, s.text_color, true, Anchor::Middle);
        }
        return;
    }
    if s.kind == ShapeKind::Actor {
        text_el(out, std::slice::from_ref(&s.text), r.center().x, r.bottom() + s.font_size * 0.9, s.font_size, s.text_color, false, Anchor::Middle);
        return;
    }
    if !s.sections.is_empty() {
        let lh = s.font_size * 1.45;
        let mut y = r.y + 6.;
        for (i, sec) in s.sections.iter().enumerate() {
            if i > 0 {
                path_el(out, &[PathCmd::Move(pt(r.x, y)), PathCmd::Line(pt(r.right(), y))], None, s.stroke, s.stroke_w * 0.8, None);
                y += 4.;
            }
            for line in &sec.lines {
                let (x, anchor) = if sec.left { (r.x + 10., Anchor::Start) } else { (r.center().x, Anchor::Middle) };
                text_el(out, std::slice::from_ref(line), x, y + lh / 2., s.font_size, s.text_color, sec.bold, anchor);
                y += lh;
            }
            y += 4.;
        }
        return;
    }
    if !s.text.is_empty() {
        // 与画布绘制共用同一个文字区域
        let a = super::paint::text_area(s.kind, r, s.align);
        let lines = wrap_ranges(&s.text, s.font_size, s.bold, a.w.max(10.));
        let (x, anchor) = match s.align.h() {
            0 => (a.x, Anchor::Start),
            2 => (a.right(), Anchor::End),
            _ => (a.center().x, Anchor::Middle),
        };
        let half = s.font_size * 1.38 * lines.len() as f32 / 2.;
        let cy = if s.align.is_top() {
            a.y + half
        } else if s.align.is_bottom() {
            a.bottom() - half
        } else {
            a.center().y
        };
        let segs = super::rich::resolve(&s.spans, s.text.len(), super::Fmt { bold: s.bold, deco: s.deco, color: None });
        rich_text_el(out, &s.text, &lines, &segs, x, cy, s.font_size, s.text_color, anchor);
    }
    for (c, t) in super::geom::compartments(s.kind, r).into_iter().zip(&s.rows) {
        if t.is_empty() {
            continue;
        }
        let a = super::geom::compartment_text_area(c);
        let lines = wrap(t, s.font_size, false, a.w);
        let half = s.font_size * 1.38 * lines.len() as f32 / 2.;
        text_el(out, &lines, a.x, a.y + half, s.font_size, s.text_color, false, Anchor::Start);
    }
}

fn line_svg_stroke(out: &mut String, l: &LineItem, rough: bool) {
    if l.points.len() < 2 {
        return;
    }
    let cmds = line_cmds(l);
    let dash = if l.dotted {
        Some(2.)
    } else if l.dash {
        Some(5.)
    } else {
        None
    };
    if rough && dash.is_none() {
        let (s, _) = rough_cmds(&cmds, l.seed, false, l.width);
        path_el(out, &s, None, Some(l.stroke), l.width, None);
    } else {
        path_el(out, &cmds, None, Some(l.stroke), l.width, dash);
    }
    let size = (l.width * 4.).max(9.);
    let n = l.points.len();
    let (end_from, start_from) = l.arrow_from();
    for (kind, tip, from) in [(l.end, l.points[n - 1], end_from), (l.start, l.points[0], start_from)] {
        for (cmds, filled) in arrow_cmds(kind, tip, from, size) {
            if filled {
                path_el(out, &cmds, Some(l.stroke), None, 0., None);
            } else {
                let bg = matches!(kind, Arrow::Hollow | Arrow::DiamondHollow | Arrow::Circle).then(|| l.label_bg.unwrap_or(Col::rgb(0xffffff)));
                path_el(out, &cmds, bg, Some(l.stroke), l.width, None);
            }
        }
    }
}

fn line_svg_labels(out: &mut String, l: &LineItem) {
    if l.points.len() < 2 {
        return;
    }
    if !l.label.is_empty() {
        let m = l.label_pos.unwrap_or_else(|| if l.curved || l.corner_radius > 0. { cmds_point_at(&line_cmds(l), 0.5) } else { polyline_mid(&l.points) });
        let w = estimate_width(&l.label, l.font_size, false);
        let lines: Vec<String> = l.label.split('\n').map(|s| s.to_string()).collect();
        if let Some(bg) = l.label_bg {
            let h = l.font_size * 1.3 * lines.len() as f32;
            writeln!(
                out,
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"4\"{}/>",
                m.x - w / 2. - 4.,
                m.y - h / 2. - 1.,
                w + 8.,
                h + 2.,
                color_attr("fill", Some(bg))
            )
            .ok();
        }
        text_el(out, &lines, m.x, m.y, l.font_size, l.label_color, false, Anchor::Middle);
    }
}

pub(crate) fn fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        // 草图手写字体（内嵌）
        db.load_font_data(crate::sketch::fonts::hand_ttf().to_vec());
        db.set_sans_serif_family("PingFang SC");
        Arc::new(db)
    })
    .clone()
}

/// SVG → PNG（2 倍分辨率）。
pub fn svg_to_png(svg: &str, scale: f32) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options { fontdb: fontdb(), ..Default::default() };
    let tree = resvg::usvg::Tree::from_str(svg, &opts).ok()?;
    let size = tree.size();
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_roundtrip_to_png() {
        let mut s = Scene::default();
        let mut a = ShapeItem::new(ShapeKind::RoundRect, rect(10., 10., 120., 50.));
        a.text = "开始".into();
        a.fill = Some(Col::rgb(0xe1eaff));
        s.items.push(Item::Shape(a));
        let mut l = LineItem::new(vec![pt(70., 60.), pt(70., 120.)]);
        l.label = "是".into();
        s.items.push(Item::Line(l));
        s.fit(10.);
        let svg = to_svg(&s);
        assert!(svg.contains("开始"));
        let png = svg_to_png(&svg, 2.).expect("png");
        assert!(png.len() > 100);
    }
}
