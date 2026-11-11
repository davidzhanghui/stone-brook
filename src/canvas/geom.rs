//! 几何：形状轮廓、边界交点、曲线平滑、箭头标记、文字宽度估算。

use super::*;

/// 不依赖字体系统的文字宽度估算（中文按 1 个字号，英文约 0.56 个字号）。
pub fn estimate_width(text: &str, size: f32, bold: bool) -> f32 {
    let k = if bold { 1.06 } else { 1.0 };
    text.lines()
        .map(|l| {
            l.chars()
                .map(|c| {
                    if c.is_ascii() {
                        if c == ' ' {
                            0.3
                        } else if c.is_ascii_uppercase() {
                            0.66
                        } else {
                            0.56
                        }
                    } else {
                        1.0
                    }
                })
                .sum::<f32>()
        })
        .fold(0., f32::max)
        * size
        * k
}

pub fn points_bounds(pts: &[Pt]) -> Rect {
    let mut it = pts.iter();
    let Some(first) = it.next() else { return Rect::default() };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in it {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    rect(x0, y0, x1 - x0, y1 - y0)
}

pub fn path_bounds(cmds: &[PathCmd]) -> Rect {
    let mut pts = vec![];
    for c in cmds {
        match *c {
            PathCmd::Move(p) | PathCmd::Line(p) => pts.push(p),
            PathCmd::Cubic(a, b, p) => pts.extend([a, b, p]),
            PathCmd::Close => {}
        }
    }
    points_bounds(&pts)
}

pub fn translate(it: &mut Item, dx: f32, dy: f32) {
    let d = pt(dx, dy);
    match it {
        Item::Shape(s) => {
            s.rect.x += dx;
            s.rect.y += dy;
        }
        Item::Line(l) => l.points.iter_mut().for_each(|p| *p = p.add(d)),
        Item::Text(t) => t.pos = t.pos.add(d),
        Item::Path(p) => {
            for c in &mut p.cmds {
                match c {
                    PathCmd::Move(a) | PathCmd::Line(a) => *a = a.add(d),
                    PathCmd::Cubic(a, b, e) => {
                        *a = a.add(d);
                        *b = b.add(d);
                        *e = e.add(d);
                    }
                    PathCmd::Close => {}
                }
            }
        }
        Item::Image(i) => {
            i.rect.x += dx;
            i.rect.y += dy;
        }
        Item::Icon(i) => {
            i.rect.x += dx;
            i.rect.y += dy;
        }
    }
}

const K: f32 = 0.552_284_8; // 贝塞尔近似圆的控制点系数

pub fn ellipse_cmds(r: Rect) -> Vec<PathCmd> {
    let (cx, cy, rx, ry) = (r.x + r.w / 2., r.y + r.h / 2., r.w / 2., r.h / 2.);
    vec![
        PathCmd::Move(pt(cx + rx, cy)),
        PathCmd::Cubic(pt(cx + rx, cy + ry * K), pt(cx + rx * K, cy + ry), pt(cx, cy + ry)),
        PathCmd::Cubic(pt(cx - rx * K, cy + ry), pt(cx - rx, cy + ry * K), pt(cx - rx, cy)),
        PathCmd::Cubic(pt(cx - rx, cy - ry * K), pt(cx - rx * K, cy - ry), pt(cx, cy - ry)),
        PathCmd::Cubic(pt(cx + rx * K, cy - ry), pt(cx + rx, cy - ry * K), pt(cx + rx, cy)),
        PathCmd::Close,
    ]
}

pub fn round_rect_cmds(r: Rect, rad: f32) -> Vec<PathCmd> {
    let rad = rad.min(r.w / 2.).min(r.h / 2.).max(0.);
    if rad <= 0.01 {
        return polygon(&[pt(r.x, r.y), pt(r.right(), r.y), pt(r.right(), r.bottom()), pt(r.x, r.bottom())]);
    }
    let k = rad * (1. - K);
    vec![
        PathCmd::Move(pt(r.x + rad, r.y)),
        PathCmd::Line(pt(r.right() - rad, r.y)),
        PathCmd::Cubic(pt(r.right() - k, r.y), pt(r.right(), r.y + k), pt(r.right(), r.y + rad)),
        PathCmd::Line(pt(r.right(), r.bottom() - rad)),
        PathCmd::Cubic(pt(r.right(), r.bottom() - k), pt(r.right() - k, r.bottom()), pt(r.right() - rad, r.bottom())),
        PathCmd::Line(pt(r.x + rad, r.bottom())),
        PathCmd::Cubic(pt(r.x + k, r.bottom()), pt(r.x, r.bottom() - k), pt(r.x, r.bottom() - rad)),
        PathCmd::Line(pt(r.x, r.y + rad)),
        PathCmd::Cubic(pt(r.x, r.y + k), pt(r.x + k, r.y), pt(r.x + rad, r.y)),
        PathCmd::Close,
    ]
}

pub fn polygon(pts: &[Pt]) -> Vec<PathCmd> {
    let mut v = Vec::with_capacity(pts.len() + 1);
    for (i, p) in pts.iter().enumerate() {
        v.push(if i == 0 { PathCmd::Move(*p) } else { PathCmd::Line(*p) });
    }
    v.push(PathCmd::Close);
    v
}

/// 外接于矩形的正多边形（第一个顶点朝上，可额外旋转 rot 弧度）。
fn regular(r: Rect, n: usize, rot: f32) -> Vec<Pt> {
    let c = r.center();
    (0..n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + rot + i as f32 * std::f32::consts::TAU / n as f32;
            pt(c.x + a.cos() * r.w / 2., c.y + a.sin() * r.h / 2.)
        })
        .collect()
}

/// 椭圆上角度 a（弧度，y 轴向下，0 为正右方、π/2 为正下方）处的点。
fn arc_pt(c: Pt, rx: f32, ry: f32, a: f32) -> Pt {
    pt(c.x + rx * a.cos(), c.y + ry * a.sin())
}

/// 从角度 a0 画到 a1 的椭圆弧（每段不超过 90°，用三次贝塞尔近似），追加到 cmds；起点需已在 a0 处。
fn arc_to(cmds: &mut Vec<PathCmd>, c: Pt, rx: f32, ry: f32, a0: f32, a1: f32) {
    let n = ((a1 - a0).abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.) as usize;
    let step = (a1 - a0) / n as f32;
    for i in 0..n {
        let t0 = a0 + step * i as f32;
        let t1 = t0 + step;
        let k = 4. / 3. * ((t1 - t0) / 4.).tan();
        let p0 = arc_pt(c, rx, ry, t0);
        let p3 = arc_pt(c, rx, ry, t1);
        let c1 = pt(p0.x - k * rx * t0.sin(), p0.y + k * ry * t0.cos());
        let c2 = pt(p3.x + k * rx * t1.sin(), p3.y - k * ry * t1.cos());
        cmds.push(PathCmd::Cubic(c1, c2, p3));
    }
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

fn seg(a: Pt, b: Pt) -> [PathCmd; 2] {
    [PathCmd::Move(a), PathCmd::Line(b)]
}

/// 竖直虚线（生命线）：拆成一段段短线，GPUI 与 SVG 导出都能原样画出。
fn dashed_v(x: f32, y0: f32, y1: f32) -> Vec<PathCmd> {
    let mut v = vec![];
    let mut y = y0;
    while y < y1 {
        v.extend(seg(pt(x, y), pt(x, (y + 5.).min(y1))));
        y += 9.;
    }
    v
}

fn dashed_h(x0: f32, x1: f32, y: f32) -> Vec<PathCmd> {
    let mut v = vec![];
    let mut x = x0;
    while x < x1 {
        v.extend(seg(pt(x, y), pt((x + 5.).min(x1), y)));
        x += 9.;
    }
    v
}

/// 类图 / 实体 / 数据流加工的标题栏高度。
pub fn header_h(r: Rect) -> f32 {
    32f32.min(r.h * 0.3)
}

/// 多格图形标题栏以外的各格（类图：属性 / 方法；实体：字段列……），与 `shape_cmds` 画的分隔线一致。
pub fn compartments(kind: ShapeKind, r: Rect) -> Vec<Rect> {
    let hd = header_h(r);
    let (x, y, w, h) = (r.x, r.y + hd, r.w, r.h - hd);
    let cols = |ws: &[f32]| {
        let mut out = vec![];
        let mut at = x;
        for f in ws {
            out.push(rect(at, y, w * f, h));
            at += w * f;
        }
        out
    };
    match kind {
        ShapeKind::ClassBox => vec![rect(x, y, w, h * 0.45), rect(x, y + h * 0.45, w, h * 0.55)],
        ShapeKind::InterfaceBox | ShapeKind::Entity | ShapeKind::DfdProcess => vec![rect(x, y, w, h)],
        ShapeKind::Entity2Key => cols(&[0.3, 0.7]),
        ShapeKind::Entity2 => cols(&[0.5, 0.5]),
        ShapeKind::Entity3 => cols(&[1. / 3., 1. / 3., 1. / 3.]),
        _ => vec![],
    }
}

/// 某一格里文字的排版区域（靠左上类图的属性 / 方法）。
pub fn compartment_text_area(c: Rect) -> Rect {
    rect(c.x + 8., c.y + 6., (c.w - 16.).max(4.), (c.h - 8.).max(4.))
}

/// 时序图生命线头部（方框类）的高度。
pub fn life_head(r: Rect) -> f32 {
    40f32.min(r.h * 0.4)
}

/// 时序图生命线图标（小人、边界、控制、实体）的高度；名字写在图标下方。
pub fn life_icon(r: Rect) -> f32 {
    44f32.min(r.h * 0.3)
}

/// 是否为时序图生命线（头部 + 竖直虚线）。
pub fn is_lifeline(kind: ShapeKind) -> bool {
    matches!(
        kind,
        ShapeKind::LifeActor
            | ShapeKind::LifeBox
            | ShapeKind::LifeDb
            | ShapeKind::LifeQueue
            | ShapeKind::LifeBoundary
            | ShapeKind::LifeControl
            | ShapeKind::LifeEntity
            | ShapeKind::LifeCollections
    )
}

/// 生命线虚线的起点 y（头部 / 图标与名字下方），与 `shape_cmds` 画的虚线一致。
pub fn life_dash_top(kind: ShapeKind, r: Rect) -> f32 {
    match kind {
        ShapeKind::LifeActor | ShapeKind::LifeBoundary | ShapeKind::LifeControl | ShapeKind::LifeEntity => r.y + life_icon(r) + 22.,
        _ => r.y + life_head(r),
    }
}

fn brace(x_arm: f32, x_tip: f32, y: f32, h: f32) -> Vec<PathCmd> {
    let xm = (x_arm + x_tip) / 2.;
    let cy = y + h / 2.;
    let r = ((x_arm - x_tip).abs() / 2.).min(h / 4.);
    vec![
        PathCmd::Move(pt(x_arm, y)),
        PathCmd::Cubic(pt(xm, y), pt(xm, y), pt(xm, y + r)),
        PathCmd::Line(pt(xm, cy - r)),
        PathCmd::Cubic(pt(xm, cy), pt(xm, cy), pt(x_tip, cy)),
        PathCmd::Cubic(pt(xm, cy), pt(xm, cy), pt(xm, cy + r)),
        PathCmd::Line(pt(xm, y + h - r)),
        PathCmd::Cubic(pt(xm, y + h), pt(xm, y + h), pt(x_arm, y + h)),
    ]
}

/// 横放圆柱：主体轮廓 + 右端内侧的半个椭圆。
fn h_cylinder(r: Rect) -> (Vec<PathCmd>, Vec<PathCmd>) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let rx = (w * 0.12).clamp(1., 12.);
    let cy = y + h / 2.;
    let mut body = vec![PathCmd::Move(pt(x + rx, y)), PathCmd::Line(pt(x + w - rx, y))];
    arc_to(&mut body, pt(x + w - rx, cy), rx, h / 2., deg(-90.), deg(90.));
    body.push(PathCmd::Line(pt(x + rx, y + h)));
    arc_to(&mut body, pt(x + rx, cy), rx, h / 2., deg(90.), deg(270.));
    body.push(PathCmd::Close);
    let mut deco = vec![PathCmd::Move(pt(x + w - rx, y))];
    arc_to(&mut deco, pt(x + w - rx, cy), rx, h / 2., deg(-90.), deg(-270.));
    (body, deco)
}

/// 正放圆柱（数据库）。
fn cylinder(r: Rect) -> (Vec<PathCmd>, Vec<PathCmd>) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let ry = (h * 0.12).min(12.);
    let top = rect(x, y, w, ry * 2.);
    let mut body = vec![PathCmd::Move(pt(x, y + ry)), PathCmd::Line(pt(x, y + h - ry))];
    body.push(PathCmd::Cubic(pt(x, y + h - ry + ry * K), pt(x + w / 2. - w / 2. * K, y + h), pt(x + w / 2., y + h)));
    body.push(PathCmd::Cubic(pt(x + w / 2. + w / 2. * K, y + h), pt(x + w, y + h - ry + ry * K), pt(x + w, y + h - ry)));
    body.push(PathCmd::Line(pt(x + w, y + ry)));
    body.push(PathCmd::Cubic(pt(x + w, y + ry - ry * K), pt(x + w / 2. + w / 2. * K, y), pt(x + w / 2., y)));
    body.push(PathCmd::Cubic(pt(x + w / 2. - w / 2. * K, y), pt(x, y + ry - ry * K), pt(x, y + ry)));
    body.push(PathCmd::Close);
    let mut deco = ellipse_cmds(top);
    deco.truncate(3); // 只画上沿的下半圈
    (body, deco)
}

/// 简笔小人（头 + 身体 + 手 + 腿），画在矩形 r 里。
fn stick_figure(r: Rect) -> Vec<PathCmd> {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let cx = x + w / 2.;
    let head = (w.min(h) * 0.28).max(5.);
    let mut deco = ellipse_cmds(rect(cx - head / 2., y, head, head));
    let neck = y + head;
    let hip = y + h * 0.66;
    deco.extend(seg(pt(cx, neck), pt(cx, hip)));
    deco.extend(seg(pt(x + w * 0.15, y + h * 0.42), pt(x + w * 0.85, y + h * 0.42)));
    deco.extend(seg(pt(cx, hip), pt(x + w * 0.2, y + h)));
    deco.extend(seg(pt(cx, hip), pt(x + w * 0.8, y + h)));
    deco
}

/// 形状轮廓；第二个返回值是额外的装饰线（圆柱的上沿、子程序的竖线等），只描边不填充。
pub fn shape_cmds(kind: ShapeKind, r: Rect, radius: f32) -> (Vec<PathCmd>, Vec<PathCmd>) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let none = vec![];
    match kind {
        ShapeKind::Rect | ShapeKind::Frame | ShapeKind::Bar => (round_rect_cmds(r, if kind == ShapeKind::Bar { 2. } else { 0. }), none),
        ShapeKind::RoundRect | ShapeKind::Sticky => (round_rect_cmds(r, radius), none),
        ShapeKind::Stadium => (round_rect_cmds(r, h / 2.), none),
        ShapeKind::Ellipse | ShapeKind::StartDot => (ellipse_cmds(r), none),
        ShapeKind::Circle => {
            let d = w.max(h);
            (ellipse_cmds(rect(r.center().x - d / 2., r.center().y - d / 2., d, d)), none)
        }
        ShapeKind::DoubleCircle | ShapeKind::EndDot => {
            let d = w.max(h);
            let outer = rect(r.center().x - d / 2., r.center().y - d / 2., d, d);
            let inner = outer.inflate(-d * 0.18);
            (ellipse_cmds(outer), ellipse_cmds(inner))
        }
        ShapeKind::Diamond => (polygon(&[pt(x + w / 2., y), pt(x + w, y + h / 2.), pt(x + w / 2., y + h), pt(x, y + h / 2.)]), none),
        ShapeKind::Hexagon => {
            let d = (h / 2.).min(w / 4.);
            (polygon(&[pt(x + d, y), pt(x + w - d, y), pt(x + w, y + h / 2.), pt(x + w - d, y + h), pt(x + d, y + h), pt(x, y + h / 2.)]), none)
        }
        ShapeKind::Parallelogram => {
            let d = (h * 0.4).min(w / 4.);
            (polygon(&[pt(x + d, y), pt(x + w, y), pt(x + w - d, y + h), pt(x, y + h)]), none)
        }
        ShapeKind::ParallelogramAlt => {
            let d = (h * 0.4).min(w / 4.);
            (polygon(&[pt(x, y), pt(x + w - d, y), pt(x + w, y + h), pt(x + d, y + h)]), none)
        }
        ShapeKind::Trapezoid => {
            let d = (h * 0.4).min(w / 4.);
            (polygon(&[pt(x + d, y), pt(x + w - d, y), pt(x + w, y + h), pt(x, y + h)]), none)
        }
        ShapeKind::TrapezoidAlt => {
            let d = (h * 0.4).min(w / 4.);
            (polygon(&[pt(x, y), pt(x + w, y), pt(x + w - d, y + h), pt(x + d, y + h)]), none)
        }
        ShapeKind::Cylinder => cylinder(r),
        ShapeKind::Document => {
            let wave = h * 0.12;
            (
                vec![
                    PathCmd::Move(pt(x, y)),
                    PathCmd::Line(pt(x + w, y)),
                    PathCmd::Line(pt(x + w, y + h - wave)),
                    PathCmd::Cubic(pt(x + w * 0.75, y + h - wave * 2.5), pt(x + w * 0.25, y + h + wave * 0.6), pt(x, y + h - wave)),
                    PathCmd::Close,
                ],
                none,
            )
        }
        ShapeKind::Subroutine => {
            let d = 8.0f32.min(w / 6.);
            (
                polygon(&[pt(x, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h)]),
                vec![PathCmd::Move(pt(x + d, y)), PathCmd::Line(pt(x + d, y + h)), PathCmd::Move(pt(x + w - d, y)), PathCmd::Line(pt(x + w - d, y + h))],
            )
        }
        ShapeKind::Flag => {
            let d = (h / 2.).min(w / 5.);
            (polygon(&[pt(x, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h), pt(x + d, y + h / 2.)]), none)
        }
        ShapeKind::Note => {
            let f = 10.0f32.min(w / 4.).min(h / 3.);
            (
                polygon(&[pt(x, y), pt(x + w - f, y), pt(x + w, y + f), pt(x + w, y + h), pt(x, y + h)]),
                vec![PathCmd::Move(pt(x + w - f, y)), PathCmd::Line(pt(x + w - f, y + f)), PathCmd::Line(pt(x + w, y + f))],
            )
        }
        ShapeKind::Triangle => (polygon(&[pt(x + w / 2., y), pt(x + w, y + h), pt(x, y + h)]), none),
        ShapeKind::RightTriangle => (polygon(&[pt(x, y), pt(x + w, y + h), pt(x, y + h)]), none),
        ShapeKind::Pentagon => (polygon(&regular(r, 5, 0.)), none),
        ShapeKind::Octagon => (polygon(&regular(r, 8, std::f32::consts::PI / 8.)), none),
        ShapeKind::Star => {
            let c = r.center();
            let pts: Vec<Pt> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.;
                    let k = if i % 2 == 0 { 1. } else { 0.42 };
                    pt(c.x + a.cos() * w / 2. * k, c.y + a.sin() * h / 2. * k + h * 0.05)
                })
                .collect();
            (polygon(&pts), none)
        }
        ShapeKind::Cross => {
            let (a, b) = (w / 3., h / 3.);
            (
                polygon(&[
                    pt(x + a, y),
                    pt(x + 2. * a, y),
                    pt(x + 2. * a, y + b),
                    pt(x + w, y + b),
                    pt(x + w, y + 2. * b),
                    pt(x + 2. * a, y + 2. * b),
                    pt(x + 2. * a, y + h),
                    pt(x + a, y + h),
                    pt(x + a, y + 2. * b),
                    pt(x, y + 2. * b),
                    pt(x, y + b),
                    pt(x + a, y + b),
                ]),
                none,
            )
        }
        ShapeKind::BlockArrow => {
            let head = (w * 0.35).min(h);
            let (t, b) = (y + h * 0.25, y + h * 0.75);
            (
                polygon(&[pt(x, t), pt(x + w - head, t), pt(x + w - head, y), pt(x + w, y + h / 2.), pt(x + w - head, y + h), pt(x + w - head, b), pt(x, b)]),
                none,
            )
        }
        ShapeKind::DoubleArrow => {
            let head = (w * 0.25).min(h);
            let (t, b) = (y + h * 0.25, y + h * 0.75);
            (
                polygon(&[
                    pt(x, y + h / 2.),
                    pt(x + head, y),
                    pt(x + head, t),
                    pt(x + w - head, t),
                    pt(x + w - head, y),
                    pt(x + w, y + h / 2.),
                    pt(x + w - head, y + h),
                    pt(x + w - head, b),
                    pt(x + head, b),
                    pt(x + head, y + h),
                ]),
                none,
            )
        }
        ShapeKind::Bubble => {
            let tail = (h * 0.22).min(18.);
            let body = rect(x, y, w, h - tail);
            let rad = 8.0f32.min(body.h / 3.);
            let mut cmds = round_rect_cmds(body, rad);
            // 尾巴：从下沿左侧伸出一个三角
            cmds.extend(polygon(&[pt(x + w * 0.18, y + h - tail - 1.), pt(x + w * 0.34, y + h - tail - 1.), pt(x + w * 0.16, y + h)]));
            (cmds, none)
        }
        ShapeKind::Actor => (vec![], stick_figure(r)),
        _ => more_shape_cmds(kind, r),
    }
}

/// 「更多图形」面板里的图形（流程图 / UML / 数据流 / ER / 组件图……）。
fn more_shape_cmds(kind: ShapeKind, r: Rect) -> (Vec<PathCmd>, Vec<PathCmd>) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let (cx, cy) = (x + w / 2., y + h / 2.);
    let c = pt(cx, cy);
    let none = vec![];
    let rect_body = || polygon(&[pt(x, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h)]);
    match kind {
        ShapeKind::Chevron => {
            let d = (h / 2.).min(w / 3.);
            (polygon(&[pt(x, y), pt(x + w - d, y), pt(x + w, cy), pt(x + w - d, y + h), pt(x, y + h), pt(x + d, cy)]), none)
        }
        ShapeKind::HomePlate => {
            let d = (h / 2.).min(w / 3.);
            (polygon(&[pt(x, y), pt(x + w - d, y), pt(x + w, cy), pt(x + w - d, y + h), pt(x, y + h)]), none)
        }
        ShapeKind::LeftArrow => {
            let head = (w * 0.35).min(h);
            let (t, b) = (y + h * 0.25, y + h * 0.75);
            (polygon(&[pt(x, cy), pt(x + head, y), pt(x + head, t), pt(x + w, t), pt(x + w, b), pt(x + head, b), pt(x + head, y + h)]), none)
        }
        ShapeKind::BubbleRound => {
            // 椭圆主体 + 左下角的尾巴，连成一条轮廓（避免尾巴和椭圆之间多出一条线）
            let ec = pt(cx, y + h * 0.42);
            let (rx, ry) = (w / 2., h * 0.42);
            let mut v = vec![PathCmd::Move(arc_pt(ec, rx, ry, deg(125.)))];
            arc_to(&mut v, ec, rx, ry, deg(125.), deg(460.));
            v.push(PathCmd::Line(pt(x + w * 0.16, y + h)));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::Cloud => {
            let n = 9;
            let step = std::f32::consts::TAU / n as f32;
            let at = |a: f32, k: f32| pt(cx + a.cos() * w / 2. * k, cy + a.sin() * h / 2. * k);
            let a0 = -std::f32::consts::FRAC_PI_2;
            let mut v = vec![PathCmd::Move(at(a0, 0.8))];
            for i in 0..n {
                let (t0, t1) = (a0 + step * i as f32, a0 + step * (i + 1) as f32);
                // 大小交替的「云朵」
                let k = if i % 2 == 0 { 1.2 } else { 1.12 };
                v.push(PathCmd::Cubic(at(t0 + step * 0.1, k), at(t1 - step * 0.1, k), at(t1, 0.8)));
            }
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::BraceLeft => {
            let bw = (w * 0.18).min(24.);
            (vec![], brace(x + w, x + w - bw, y, h))
        }
        ShapeKind::BraceRight => {
            let bw = (w * 0.18).min(24.);
            (vec![], brace(x, x + bw, y, h))
        }
        ShapeKind::BlockArc => {
            let (rx, ry) = (w / 2., h / 2.);
            let k = 0.55;
            let mut v = vec![PathCmd::Move(arc_pt(c, rx, ry, deg(45.)))];
            arc_to(&mut v, c, rx, ry, deg(45.), deg(315.));
            v.push(PathCmd::Line(arc_pt(c, rx * k, ry * k, deg(315.))));
            arc_to(&mut v, c, rx * k, ry * k, deg(315.), deg(45.));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::Pie => {
            let mut v = vec![PathCmd::Move(c), PathCmd::Line(arc_pt(c, w / 2., h / 2., 0.))];
            arc_to(&mut v, c, w / 2., h / 2., 0., deg(270.));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::Cube => {
            let d = w.min(h) * 0.22;
            let body = polygon(&[pt(x, y + d), pt(x + d, y), pt(x + w, y), pt(x + w, y + h - d), pt(x + w - d, y + h), pt(x, y + h)]);
            let mut deco = vec![PathCmd::Move(pt(x, y + d)), PathCmd::Line(pt(x + w - d, y + d)), PathCmd::Line(pt(x + w, y))];
            deco.extend(seg(pt(x + w - d, y + d), pt(x + w - d, y + h)));
            (body, deco)
        }
        ShapeKind::HCylinder => h_cylinder(r),
        ShapeKind::ManualInput => (polygon(&[pt(x, y + h * 0.3), pt(x + w, y), pt(x + w, y + h), pt(x, y + h)]), none),
        ShapeKind::Delay => {
            let rx = (h / 2.).min(w / 2.);
            let mut v = vec![PathCmd::Move(pt(x, y)), PathCmd::Line(pt(x + w - rx, y))];
            arc_to(&mut v, pt(x + w - rx, cy), rx, h / 2., deg(-90.), deg(90.));
            v.push(PathCmd::Line(pt(x, y + h)));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::OffPage => (polygon(&[pt(x, y), pt(x + w, y), pt(x + w, y + h * 0.65), pt(cx, y + h), pt(x, y + h * 0.65)]), none),
        ShapeKind::Tape => {
            let d = w.min(h);
            let rr = d / 2.;
            (ellipse_cmds(rect(cx - rr, cy - rr, d, d)), seg(pt(cx, cy + rr), pt((cx + rr * 1.1).min(x + w), cy + rr)).to_vec())
        }
        ShapeKind::Card => {
            let d = w.min(h) * 0.25;
            (polygon(&[pt(x + d, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h), pt(x, y + d)]), none)
        }
        ShapeKind::PunchedTape => {
            let a = h * 0.1;
            (
                vec![
                    PathCmd::Move(pt(x, y + a)),
                    PathCmd::Cubic(pt(x + w * 0.35, y + a * 3.), pt(x + w * 0.65, y - a), pt(x + w, y + a)),
                    PathCmd::Line(pt(x + w, y + h - a)),
                    PathCmd::Cubic(pt(x + w * 0.65, y + h - a * 3.), pt(x + w * 0.35, y + h + a), pt(x, y + h - a)),
                    PathCmd::Close,
                ],
                none,
            )
        }
        ShapeKind::Display => {
            let rr = (w * 0.2).min(h / 2.);
            let mut v = vec![PathCmd::Move(pt(x, cy)), PathCmd::Line(pt(x + w * 0.2, y)), PathCmd::Line(pt(x + w - rr, y))];
            arc_to(&mut v, pt(x + w - rr, cy), rr, h / 2., deg(-90.), deg(90.));
            v.push(PathCmd::Line(pt(x + w * 0.2, y + h)));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::InternalStorage => {
            let d = w.min(h) * 0.2;
            let mut deco = seg(pt(x + d, y), pt(x + d, y + h)).to_vec();
            deco.extend(seg(pt(x, y + d), pt(x + w, y + d)));
            (rect_body(), deco)
        }
        ShapeKind::LoopLimit => {
            let d = w.min(h) * 0.25;
            (polygon(&[pt(x + d, y), pt(x + w - d, y), pt(x + w, y + d), pt(x + w, y + h), pt(x, y + h), pt(x, y + d)]), none)
        }
        ShapeKind::OrJunction | ShapeKind::SumJunction => {
            let d = w.min(h);
            let rr = d / 2.;
            let body = ellipse_cmds(rect(cx - rr, cy - rr, d, d));
            let mut deco = vec![];
            if kind == ShapeKind::OrJunction {
                deco.extend(seg(pt(cx - rr, cy), pt(cx + rr, cy)));
                deco.extend(seg(pt(cx, cy - rr), pt(cx, cy + rr)));
            } else {
                let k = rr * std::f32::consts::FRAC_1_SQRT_2;
                deco.extend(seg(pt(cx - k, cy - k), pt(cx + k, cy + k)));
                deco.extend(seg(pt(cx + k, cy - k), pt(cx - k, cy + k)));
            }
            (body, deco)
        }
        ShapeKind::Collate => {
            let mut v = polygon(&[pt(x, y), pt(x + w, y), pt(cx, cy)]);
            v.extend(polygon(&[pt(cx, cy), pt(x + w, y + h), pt(x, y + h)]));
            (v, none)
        }
        ShapeKind::Sort => (polygon(&[pt(cx, y), pt(x + w, cy), pt(cx, y + h), pt(x, cy)]), seg(pt(x, cy), pt(x + w, cy)).to_vec()),
        ShapeKind::Merge => (polygon(&[pt(x, y), pt(x + w, y), pt(cx, y + h)]), none),
        ShapeKind::StoredData => {
            let rr = (w * 0.15).min(h / 2.);
            let mut v = vec![PathCmd::Move(pt(x + rr, y)), PathCmd::Line(pt(x + w, y))];
            arc_to(&mut v, pt(x + w, cy), rr, h / 2., deg(-90.), deg(-270.));
            v.push(PathCmd::Line(pt(x + rr, y + h)));
            arc_to(&mut v, pt(x + rr, cy), rr, h / 2., deg(90.), deg(270.));
            v.push(PathCmd::Close);
            (v, none)
        }
        ShapeKind::MultiDoc => {
            let o = (w.min(h) * 0.08).max(3.);
            let f = rect(x, y + 2. * o, w - 2. * o, h - 2. * o);
            let wave = f.h * 0.12;
            let body = vec![
                PathCmd::Move(pt(f.x, f.y)),
                PathCmd::Line(pt(f.right(), f.y)),
                PathCmd::Line(pt(f.right(), f.bottom() - wave)),
                PathCmd::Cubic(pt(f.x + f.w * 0.75, f.bottom() - wave * 2.5), pt(f.x + f.w * 0.25, f.bottom() + wave * 0.6), pt(f.x, f.bottom() - wave)),
                PathCmd::Close,
            ];
            let deco = vec![
                PathCmd::Move(pt(x + o, y + 2. * o)),
                PathCmd::Line(pt(x + o, y + o)),
                PathCmd::Line(pt(x + w - o, y + o)),
                PathCmd::Line(pt(x + w - o, y + h - 2. * o - wave)),
                PathCmd::Line(pt(x + w - 2. * o, y + h - 2. * o - wave)),
                PathCmd::Move(pt(x + 2. * o, y + o)),
                PathCmd::Line(pt(x + 2. * o, y)),
                PathCmd::Line(pt(x + w, y)),
                PathCmd::Line(pt(x + w, y + h - 3. * o - wave)),
                PathCmd::Line(pt(x + w - o, y + h - 3. * o - wave)),
            ];
            (body, deco)
        }
        ShapeKind::ClassBox | ShapeKind::InterfaceBox | ShapeKind::Entity | ShapeKind::Entity2Key | ShapeKind::Entity2 | ShapeKind::Entity3 => {
            let hd = header_h(r);
            let mut deco = seg(pt(x, y + hd), pt(x + w, y + hd)).to_vec();
            match kind {
                ShapeKind::ClassBox => deco.extend(seg(pt(x, y + hd + (h - hd) * 0.45), pt(x + w, y + hd + (h - hd) * 0.45))),
                ShapeKind::Entity2Key => deco.extend(seg(pt(x + w * 0.3, y + hd), pt(x + w * 0.3, y + h))),
                ShapeKind::Entity2 => deco.extend(seg(pt(cx, y + hd), pt(cx, y + h))),
                ShapeKind::Entity3 => {
                    deco.extend(seg(pt(x + w / 3., y + hd), pt(x + w / 3., y + h)));
                    deco.extend(seg(pt(x + w * 2. / 3., y + hd), pt(x + w * 2. / 3., y + h)));
                }
                _ => {}
            }
            (rect_body(), deco)
        }
        ShapeKind::DfdProcess => {
            let hd = header_h(r);
            (round_rect_cmds(r, 8f32.min(h / 4.)), seg(pt(x, y + hd), pt(x + w, y + hd)).to_vec())
        }
        ShapeKind::DataStore => {
            let mut deco = seg(pt(x, y), pt(x + w, y)).to_vec();
            deco.extend(seg(pt(x, y + h), pt(x + w, y + h)));
            (vec![], deco)
        }
        ShapeKind::DataStoreId => {
            let d = (w * 0.2).min(36.);
            let mut deco = vec![PathCmd::Move(pt(x + w, y)), PathCmd::Line(pt(x, y)), PathCmd::Line(pt(x, y + h)), PathCmd::Line(pt(x + w, y + h))];
            deco.extend(seg(pt(x + d, y), pt(x + d, y + h)));
            (vec![], deco)
        }
        ShapeKind::OpenRect => {
            (vec![], vec![PathCmd::Move(pt(x + w, y)), PathCmd::Line(pt(x, y)), PathCmd::Line(pt(x, y + h)), PathCmd::Line(pt(x + w, y + h))])
        }
        ShapeKind::LifeBox | ShapeKind::LifeDb | ShapeKind::LifeQueue | ShapeKind::LifeCollections => {
            let hh = life_head(r);
            let head = rect(x, y, w, hh);
            let (body, mut deco) = match kind {
                ShapeKind::LifeDb => cylinder(head),
                ShapeKind::LifeQueue => h_cylinder(head),
                ShapeKind::LifeCollections => {
                    let o = 5f32.min(w / 8.);
                    let front = rect(x, y + o, w - o, hh - o);
                    (
                        round_rect_cmds(front, 3.),
                        vec![
                            PathCmd::Move(pt(x + o, y + o)),
                            PathCmd::Line(pt(x + o, y)),
                            PathCmd::Line(pt(x + w, y)),
                            PathCmd::Line(pt(x + w, y + hh - o)),
                            PathCmd::Line(pt(x + w - o, y + hh - o)),
                        ],
                    )
                }
                _ => (round_rect_cmds(head, 3.), vec![]),
            };
            deco.extend(dashed_v(cx, y + hh, y + h));
            (body, deco)
        }
        ShapeKind::LifeActor | ShapeKind::LifeBoundary | ShapeKind::LifeControl | ShapeKind::LifeEntity => more_shape_cmds_life_actor(kind, r, cx, h, y),
        ShapeKind::Activation => (round_rect_cmds(r, 2f32.min(w / 3.)), none),
        ShapeKind::FrameAlt => (rect_body(), dashed_h(x, x + w, cy)),
        ShapeKind::Component => {
            let t = (w * 0.08).clamp(5., 9.);
            let mut body = polygon(&[pt(x + t, y), pt(x + w, y), pt(x + w, y + h), pt(x + t, y + h)]);
            for ty in [y + h * 0.22, y + h * 0.58] {
                body.extend(polygon(&[pt(x, ty), pt(x + 2. * t, ty), pt(x + 2. * t, ty + t * 1.5), pt(x, ty + t * 1.5)]));
            }
            (body, none)
        }
        ShapeKind::Component2 => {
            let s = (w.min(h) * 0.22).min(16.);
            let ic = rect(x + w - s * 1.6, y + s * 0.5, s, s * 1.1);
            let mut deco = polygon(&[pt(ic.x + s * 0.2, ic.y), pt(ic.right(), ic.y), pt(ic.right(), ic.bottom()), pt(ic.x + s * 0.2, ic.bottom())]);
            for ty in [ic.y + s * 0.2, ic.y + s * 0.6] {
                deco.extend(polygon(&[pt(ic.x - s * 0.1, ty), pt(ic.x + s * 0.5, ty), pt(ic.x + s * 0.5, ty + s * 0.25), pt(ic.x - s * 0.1, ty + s * 0.25)]));
            }
            (rect_body(), deco)
        }
        ShapeKind::ProvidedIface => {
            let rr = (h / 2.).min(w / 3.);
            (ellipse_cmds(rect(x + w - 2. * rr, cy - rr, 2. * rr, 2. * rr)), seg(pt(x, cy), pt(x + w - 2. * rr, cy)).to_vec())
        }
        ShapeKind::AssemblyIface => {
            let rr = (h * 0.3).min(w / 6.);
            let big = rr * 1.6;
            let cc = pt(cx, cy);
            let mut deco = seg(pt(x, cy), pt(cx - rr, cy)).to_vec();
            deco.push(PathCmd::Move(arc_pt(cc, big, big, deg(-70.))));
            arc_to(&mut deco, cc, big, big, deg(-70.), deg(70.));
            deco.extend(seg(pt(cx + big, cy), pt(x + w, cy)));
            (ellipse_cmds(rect(cx - rr, cy - rr, 2. * rr, 2. * rr)), deco)
        }
        ShapeKind::RequiredIface => {
            let rr = (h / 2.).min(w / 3.);
            let cc = pt(x, cy);
            let mut deco = vec![PathCmd::Move(arc_pt(cc, rr, rr, deg(-90.)))];
            arc_to(&mut deco, cc, rr, rr, deg(-90.), deg(90.));
            deco.extend(seg(pt(x + rr, cy), pt(x + w, cy)));
            (vec![], deco)
        }
        _ => (rect_body(), none),
    }
}
/// 从 `more_shape_cmds` 拆出。
fn more_shape_cmds_life_actor(kind: ShapeKind, r: Rect, cx: f32, h: f32, y: f32) -> (Vec<PathCmd>, Vec<PathCmd>) {
    let ih = life_icon(r);
    let s = ih * 0.8;
    let (body, mut deco) = match kind {
        ShapeKind::LifeActor => (vec![], stick_figure(rect(cx - s * 0.4, y, s * 0.8, ih))),
        ShapeKind::LifeBoundary => {
            let rr = ih * 0.36;
            let cc = pt(cx + rr * 0.35, y + ih / 2.);
            let lx = cc.x - rr * 1.9;
            let mut d = seg(pt(lx, cc.y - rr), pt(lx, cc.y + rr)).to_vec();
            d.extend(seg(pt(lx, cc.y), pt(cc.x - rr, cc.y)));
            (ellipse_cmds(rect(cc.x - rr, cc.y - rr, 2. * rr, 2. * rr)), d)
        }
        ShapeKind::LifeControl => {
            let rr = ih * 0.38;
            let cc = pt(cx, y + ih / 2. + 2.);
            let top = pt(cx, cc.y - rr);
            // 圆顶上的小箭头
            let a = rr * 0.45;
            (
                ellipse_cmds(rect(cc.x - rr, cc.y - rr, 2. * rr, 2. * rr)),
                vec![
                    PathCmd::Move(pt(top.x + a * 0.2, top.y - a * 0.7)),
                    PathCmd::Line(pt(top.x - a * 0.6, top.y)),
                    PathCmd::Line(pt(top.x + a * 0.2, top.y + a * 0.7)),
                ],
            )
        }
        _ => {
            let rr = ih * 0.36;
            let cc = pt(cx, y + ih / 2. - 2.);
            (ellipse_cmds(rect(cc.x - rr, cc.y - rr, 2. * rr, 2. * rr)), seg(pt(cc.x - rr, cc.y + rr + 3.), pt(cc.x + rr, cc.y + rr + 3.)).to_vec())
        }
    };
    // 图标下方是名字，名字下面才是生命线
    deco.extend(dashed_v(cx, y + ih + 22., y + h));
    (body, deco)
}

/// 从形状中心指向 toward 的射线与形状边界的交点（连线端点贴边用）。
pub fn boundary_point(kind: ShapeKind, r: Rect, toward: Pt) -> Pt {
    let c = r.center();
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return c;
    }
    let (hw, hh) = (r.w / 2., r.h / 2.);
    let t = match kind {
        ShapeKind::Ellipse | ShapeKind::Cloud | ShapeKind::BubbleRound => 1. / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt(),
        ShapeKind::Circle | ShapeKind::DoubleCircle | ShapeKind::StartDot | ShapeKind::EndDot => {
            let (a, b) = (hw.max(hh), hw.max(hh));
            1. / ((dx / a).powi(2) + (dy / b).powi(2)).sqrt()
        }
        ShapeKind::OrJunction | ShapeKind::SumJunction | ShapeKind::Tape => 1. / ((dx / hw.min(hh)).powi(2) + (dy / hw.min(hh)).powi(2)).sqrt(),
        ShapeKind::Diamond | ShapeKind::Sort => 1. / (dx.abs() / hw + dy.abs() / hh),
        _ => {
            let tx = if dx.abs() > 1e-6 { hw / dx.abs() } else { f32::INFINITY };
            let ty = if dy.abs() > 1e-6 { hh / dy.abs() } else { f32::INFINITY };
            tx.min(ty)
        }
    };
    pt(c.x + dx * t, c.y + dy * t)
}

/// 经过所有点的平滑曲线（Catmull-Rom → 三次贝塞尔）。
pub fn smooth_cmds(pts: &[Pt]) -> Vec<PathCmd> {
    let mut out = vec![];
    if pts.is_empty() {
        return out;
    }
    out.push(PathCmd::Move(pts[0]));
    if pts.len() == 2 {
        out.push(PathCmd::Line(pts[1]));
        return out;
    }
    for i in 0..pts.len() - 1 {
        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { pts[i + 1] };
        // 向心参数化（α = 0.5）：相邻点距差别很大时（如端头短直段接远处拐点）不会过冲打钩；点距相等时与均匀参数化相同
        let (d1, d2, d3) = (p0.dist(p1).sqrt(), p1.dist(p2).sqrt(), p2.dist(p3).sqrt());
        if d2 < 1e-3 {
            continue;
        }
        let c1 = if d1 < 1e-3 {
            p1.lerp(p2, 1. / 3.)
        } else {
            let k = 3. * d1 * (d1 + d2);
            let (a, b, c) = (d1 * d1, d2 * d2, 2. * d1 * d1 + 3. * d1 * d2 + d2 * d2);
            pt((a * p2.x - b * p0.x + c * p1.x) / k, (a * p2.y - b * p0.y + c * p1.y) / k)
        };
        let c2 = if d3 < 1e-3 {
            p2.lerp(p1, 1. / 3.)
        } else {
            let k = 3. * d3 * (d3 + d2);
            let (a, b, c) = (d3 * d3, d2 * d2, 2. * d3 * d3 + 3. * d3 * d2 + d2 * d2);
            pt((a * p1.x - b * p3.x + c * p2.x) / k, (a * p1.y - b * p3.y + c * p2.y) / k)
        };
        out.push(PathCmd::Cubic(c1, c2, p2));
    }
    out
}

/// 经过所有点的平滑曲线，两端沿给定方向伸出（da 为起点朝外方向，db 为终点朝外方向）。
pub fn smooth_cmds_dir(pts: &[Pt], da: Pt, db: Pt) -> Vec<PathCmd> {
    let n = pts.len();
    if n < 2 {
        return polyline_cmds(pts);
    }
    let (a, b) = (pts[0], pts[n - 1]);
    if n == 2 {
        let k = a.dist(b) * 0.45;
        return vec![PathCmd::Move(a), PathCmd::Cubic(a.add(da.scale(k)), b.add(db.scale(k)), b)];
    }
    let mut cmds = smooth_cmds(pts);
    if let Some(PathCmd::Cubic(c1, _, _)) = cmds.get_mut(1) {
        *c1 = a.add(da.scale(a.dist(pts[1]) * 0.45));
    }
    if let Some(PathCmd::Cubic(_, c2, _)) = cmds.last_mut() {
        *c2 = b.add(db.scale(b.dist(pts[n - 2]) * 0.45));
    }
    cmds
}

pub fn polyline_cmds(pts: &[Pt]) -> Vec<PathCmd> {
    pts.iter().enumerate().map(|(i, p)| if i == 0 { PathCmd::Move(*p) } else { PathCmd::Line(*p) }).collect()
}

/// 折线按长度的中点。
pub fn polyline_mid(pts: &[Pt]) -> Pt {
    super::math::polyline_mid(pts).unwrap_or_default()
}

/// 箭头标记：返回 (路径, 是否填充)。tip 为线端点，from 为线上靠近端点的前一个点。
pub fn arrow_cmds(kind: Arrow, tip: Pt, from: Pt, size: f32) -> Vec<(Vec<PathCmd>, bool)> {
    let d = tip.sub(from);
    let len = (d.x * d.x + d.y * d.y).sqrt().max(1e-3);
    let u = pt(d.x / len, d.y / len); // 指向端点的单位向量
    let n = pt(-u.y, u.x);
    let at = |back: f32, side: f32| tip.sub(u.scale(back)).add(n.scale(side));
    let s = size;
    match kind {
        Arrow::None => vec![],
        Arrow::Filled => vec![(polygon(&[tip, at(s, s * 0.45), at(s, -s * 0.45)]), true)],
        Arrow::Open => vec![(vec![PathCmd::Move(at(s, s * 0.5)), PathCmd::Line(tip), PathCmd::Line(at(s, -s * 0.5))], false)],
        Arrow::Hollow => vec![(polygon(&[tip, at(s * 1.2, s * 0.6), at(s * 1.2, -s * 0.6)]), false)],
        Arrow::Circle => {
            let c = tip.sub(u.scale(s * 0.45));
            vec![(ellipse_cmds(rect(c.x - s * 0.4, c.y - s * 0.4, s * 0.8, s * 0.8)), false)]
        }
        Arrow::Cross => vec![(
            vec![
                PathCmd::Move(at(s * 0.9, s * 0.45)),
                PathCmd::Line(at(0., -s * 0.45)),
                PathCmd::Move(at(s * 0.9, -s * 0.45)),
                PathCmd::Line(at(0., s * 0.45)),
            ],
            false,
        )],
        Arrow::DiamondFilled | Arrow::DiamondHollow => {
            vec![(polygon(&[tip, at(s * 0.8, s * 0.45), at(s * 1.6, 0.), at(s * 0.8, -s * 0.45)]), kind == Arrow::DiamondFilled)]
        }
        Arrow::One | Arrow::ZeroOrOne | Arrow::OneOrMany | Arrow::ZeroOrMany => {
            let mut v = vec![];
            // 「多」：鸟爪
            if matches!(kind, Arrow::OneOrMany | Arrow::ZeroOrMany) {
                v.push((
                    vec![
                        PathCmd::Move(at(0., s * 0.55)),
                        PathCmd::Line(at(s, 0.)),
                        PathCmd::Line(at(0., -s * 0.55)),
                        PathCmd::Move(at(0., 0.)),
                        PathCmd::Line(at(s, 0.)),
                    ],
                    false,
                ));
            } else {
                v.push((vec![PathCmd::Move(at(s * 0.6, s * 0.5)), PathCmd::Line(at(s * 0.6, -s * 0.5))], false));
            }
            // 「一」：竖线；「零」：小圆
            match kind {
                Arrow::One | Arrow::OneOrMany => {
                    v.push((vec![PathCmd::Move(at(s * 1.25, s * 0.5)), PathCmd::Line(at(s * 1.25, -s * 0.5))], false));
                }
                _ => {
                    let c = tip.sub(u.scale(s * 1.6));
                    v.push((ellipse_cmds(rect(c.x - s * 0.32, c.y - s * 0.32, s * 0.64, s * 0.64)), false));
                }
            }
            v
        }
    }
}

/// 箭头占用的长度（线段需要缩短这么多，避免线头戳出箭头）。
pub fn arrow_inset(kind: Arrow, size: f32) -> f32 {
    match kind {
        Arrow::Filled => size * 0.9,
        Arrow::Hollow => size * 1.2,
        Arrow::DiamondFilled | Arrow::DiamondHollow => size * 1.6,
        Arrow::Circle => size * 0.85,
        _ => 0.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_hits_rect_edge() {
        let r = rect(0., 0., 100., 50.);
        let p = boundary_point(ShapeKind::Rect, r, pt(200., 25.));
        assert!((p.x - 100.).abs() < 0.01 && (p.y - 25.).abs() < 0.01);
        let d = boundary_point(ShapeKind::Diamond, r, pt(50., 200.));
        assert!((d.y - 50.).abs() < 0.01);
    }

    #[test]
    fn midpoint_of_polyline() {
        let m = polyline_mid(&[pt(0., 0.), pt(10., 0.), pt(10., 10.)]);
        assert!((m.x - 10.).abs() < 0.01 && m.y.abs() < 0.01);
    }
}
