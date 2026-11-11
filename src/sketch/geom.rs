//! 几何：旋转、包围盒、命中测试、箭头绑定、文字换行与容器排版。
//!
//! 绑定点算法（射线与矩形 / 菱形 / 椭圆轮廓求交）参考 gpui-excalidraw `core/binding.rs`
//! （MIT，Copyright (c) LittleGuest），并按官方语义改为「点相对 (x, y)」。

use super::model::*;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct V {
    pub x: f64,
    pub y: f64,
}

pub fn v(x: f64, y: f64) -> V {
    V { x, y }
}

impl V {
    pub fn add(self, o: V) -> V {
        v(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: V) -> V {
        v(self.x - o.x, self.y - o.y)
    }
    pub fn scale(self, s: f64) -> V {
        v(self.x * s, self.y * s)
    }
    pub fn len(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn dist(self, o: V) -> f64 {
        self.sub(o).len()
    }
    /// 绕 c 顺时针旋转 a 弧度（屏幕坐标系，y 向下）
    pub fn rot(self, a: f64, c: V) -> V {
        if a == 0. {
            return self;
        }
        let (s, co) = a.sin_cos();
        let d = self.sub(c);
        v(c.x + d.x * co - d.y * s, c.y + d.x * s + d.y * co)
    }
}

impl crate::canvas::math::P2 for V {
    type S = f64;
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn mk(x: f64, y: f64) -> Self {
        v(x, y)
    }
}

/// 轴对齐矩形
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct R {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl R {
    pub fn from_pts(pts: impl IntoIterator<Item = V>) -> Option<R> {
        let mut it = pts.into_iter();
        let f = it.next()?;
        let mut r = R { x0: f.x, y0: f.y, x1: f.x, y1: f.y };
        for p in it {
            r.x0 = r.x0.min(p.x);
            r.y0 = r.y0.min(p.y);
            r.x1 = r.x1.max(p.x);
            r.y1 = r.y1.max(p.y);
        }
        Some(r)
    }
    pub fn xywh(x: f64, y: f64, w: f64, h: f64) -> R {
        R { x0: x.min(x + w), y0: y.min(y + h), x1: x.max(x + w), y1: y.max(y + h) }
    }
    pub fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn center(&self) -> V {
        v((self.x0 + self.x1) / 2., (self.y0 + self.y1) / 2.)
    }
    pub fn union(&self, o: &R) -> R {
        R { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn inflate(&self, d: f64) -> R {
        R { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }
    pub fn contains(&self, p: V) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
    pub fn contains_r(&self, o: &R) -> bool {
        o.x0 >= self.x0 && o.x1 <= self.x1 && o.y0 >= self.y0 && o.y1 <= self.y1
    }
}

pub fn center(e: &Element) -> V {
    v(e.x + e.width / 2., e.y + e.height / 2.)
}

/// 线 / 自由画的点（世界坐标，未旋转）
pub fn abs_points(e: &Element) -> Vec<V> {
    e.pts().iter().map(|p| v(e.x + p[0], e.y + p[1])).collect()
}

/// 线的旋转中心：点的包围盒中心（官方以 x/y + 点包围盒为准）
pub fn linear_center(e: &Element) -> V {
    R::from_pts(abs_points(e)).map(|r| r.center()).unwrap_or(v(e.x, e.y))
}

pub fn rot_center(e: &Element) -> V {
    if e.points.is_some() && (e.is_linear() || e.kind == "freedraw") {
        linear_center(e)
    } else {
        center(e)
    }
}

/// 线的点（世界坐标，已旋转）
pub fn world_points(e: &Element) -> Vec<V> {
    let c = rot_center(e);
    abs_points(e).into_iter().map(|p| p.rot(e.angle, c)).collect()
}

/// 未旋转时的四角
pub fn corners(e: &Element) -> [V; 4] {
    let c = center(e);
    let (x0, y0, x1, y1) = (e.x, e.y, e.x + e.width, e.y + e.height);
    [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)].map(|p| p.rot(e.angle, c))
}

/// 轴对齐包围盒（考虑旋转）
pub fn bounds(e: &Element) -> R {
    if e.points.is_some() && (e.is_linear() || e.kind == "freedraw") {
        let r = R::from_pts(world_points(e)).unwrap_or(R::xywh(e.x, e.y, 0., 0.));
        return r.inflate(e.stroke_width / 2.);
    }
    if e.kind == "ellipse" && e.angle != 0. {
        // 旋转椭圆的精确包围盒
        let (a, b) = (e.width / 2., e.height / 2.);
        let (s, c) = e.angle.sin_cos();
        let hw = ((a * c).powi(2) + (b * s).powi(2)).sqrt();
        let hh = ((a * s).powi(2) + (b * c).powi(2)).sqrt();
        let cc = center(e);
        return R { x0: cc.x - hw, y0: cc.y - hh, x1: cc.x + hw, y1: cc.y + hh };
    }
    R::from_pts(corners(e)).unwrap()
}

pub fn bounds_of<'a>(els: impl IntoIterator<Item = &'a Element>) -> Option<R> {
    els.into_iter().map(bounds).reduce(|a, b| a.union(&b))
}

pub use crate::canvas::math::{dist_polyline, inside_poly};

/// 曲线（官方 roundness 线）用 Catmull-Rom 打平，命中测试与绑定都用它。
pub fn curve_points(pts: &[V], round: bool) -> Vec<V> {
    if !round || pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for i in 0..pts.len() - 1 {
        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { p2 };
        for k in 1..=10 {
            let t = k as f64 / 10.;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f64, b: f64, c: f64, d: f64| 0.5 * ((2. * b) + (-a + c) * t + (2. * a - 5. * b + 4. * c - d) * t2 + (-a + 3. * b - 3. * c + d) * t3);
            out.push(v(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    out
}

/// 命中测试。tol 为世界坐标下的容差。
pub fn hit(e: &Element, p: V, tol: f64) -> bool {
    if e.is_deleted {
        return false;
    }
    if !bounds(e).inflate(tol + 4.).contains(p) {
        return false;
    }
    let w = e.stroke_width / 2. + tol;
    match e.kind.as_str() {
        "line" | "arrow" => {
            let pts = curve_points(&world_points(e), e.roundness().is_some() && !e.elbowed());
            if dist_polyline(p, &pts) <= w {
                return true;
            }
            // 闭合且有填充的线：内部也算
            e.has_fill() && pts.len() > 2 && pts[0].dist(*pts.last().unwrap()) < 1. && inside_poly(p, &pts)
        }
        "freedraw" => dist_polyline(p, &world_points(e)) <= w + e.stroke_width,
        _ => {
            // 转到元素自身坐标系
            let c = center(e);
            let q = p.rot(-e.angle, c);
            let (x0, y0, x1, y1) = (e.x, e.y, e.x + e.width, e.y + e.height);
            // 与官网 shouldTestInside 一致：有填充、带绑定文字、文字 / 图片 / 嵌入才算内部；透明形状只认边框
            let filled = e.has_fill() || e.bound_text_id().is_some() || matches!(e.kind.as_str(), "text" | "image" | "iframe" | "embeddable" | "stickynote");
            match e.kind.as_str() {
                "ellipse" => {
                    let (a, b) = ((e.width / 2.).max(1.), (e.height / 2.).max(1.));
                    let d = ((q.x - c.x) / a).powi(2) + ((q.y - c.y) / b).powi(2);
                    let inner_a = (a - w).max(0.5);
                    let inner_b = (b - w).max(0.5);
                    let din = ((q.x - c.x) / inner_a).powi(2) + ((q.y - c.y) / inner_b).powi(2);
                    let outer = ((q.x - c.x) / (a + w)).powi(2) + ((q.y - c.y) / (b + w)).powi(2);
                    outer <= 1. && (filled || din >= 1. || d >= 1.)
                }
                "diamond" => {
                    let poly = [v(c.x, y0), v(x1, c.y), v(c.x, y1), v(x0, c.y), v(c.x, y0)];
                    dist_polyline(q, &poly) <= w || (filled && inside_poly(q, &poly))
                }
                "frame" | "magicframe" => {
                    // 框架：点边框或名称标签
                    let edge = dist_polyline(q, &[v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1), v(x0, y0)]) <= w + 2.;
                    edge || R::xywh(e.x, e.y - 24., e.width.min(220.), 22.).contains(q)
                }
                _ => {
                    let r = R::xywh(x0, y0, e.width, e.height);
                    if filled {
                        r.inflate(w).contains(q)
                    } else {
                        let poly = [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1), v(x0, y0)];
                        dist_polyline(q, &poly) <= w
                    }
                }
            }
        }
    }
}

// ───────────── 绑定 ─────────────

pub const BIND_DIST: f64 = 14.;

/// 从 from 指向目标中心的射线与目标轮廓的交点（考虑旋转）。
pub fn focus_point(t: &Element, from: V) -> V {
    let w = t.width.abs().max(0.0001);
    let h = t.height.abs().max(0.0001);
    let c = center(t);
    let lf = from.rot(-t.angle, c);
    let (dx, dy) = (lf.x - c.x, lf.y - c.y);
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return v(c.x + w / 2., c.y).rot(t.angle, c);
    }
    let (hw, hh) = (w / 2., h / 2.);
    let local = match t.kind.as_str() {
        "diamond" => {
            let s = 1. / (dx.abs() / hw + dy.abs() / hh).max(1e-9);
            v(c.x + dx * s, c.y + dy * s)
        }
        "ellipse" => {
            let s = 1. / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt().max(1e-9);
            v(c.x + dx * s, c.y + dy * s)
        }
        _ => {
            let tx = if dx.abs() > 1e-9 { hw / dx.abs() } else { f64::INFINITY };
            let ty = if dy.abs() > 1e-9 { hh / dy.abs() } else { f64::INFINITY };
            let s = tx.min(ty);
            v(c.x + dx * s, c.y + dy * s)
        }
    };
    local.rot(t.angle, c)
}

/// 端点应落的位置：轮廓交点再向外留 gap；官方新版带 fixedPoint 时按相对位置取点。
pub fn bound_point(t: &Element, b: &Binding, toward: V) -> V {
    if let Some([fx, fy]) = b.fixed_point {
        let c = center(t);
        let p = v(t.x + t.width * fx, t.y + t.height * fy).rot(t.angle, c);
        return p;
    }
    let p = focus_point(t, toward);
    let c = center(t);
    let d = p.sub(c);
    let l = d.len();
    if l < 1e-6 {
        return p;
    }
    p.add(d.scale(b.gap.max(0.) / l))
}

/// 元素轮廓（世界坐标折线，闭合）：绑定高亮沿真实形状画，而不是外接框。
pub fn outline_poly(e: &Element) -> Vec<V> {
    let c = center(e);
    let (w, h) = (e.width.abs(), e.height.abs());
    let (x0, y0) = (c.x - w / 2., c.y - h / 2.);
    let local: Vec<V> = match e.kind.as_str() {
        "ellipse" => (0..72)
            .map(|i| {
                let a = i as f64 / 72. * std::f64::consts::TAU;
                v(c.x + a.cos() * w / 2., c.y + a.sin() * h / 2.)
            })
            .collect(),
        "diamond" => vec![v(c.x, y0), v(x0 + w, c.y), v(c.x, y0 + h), v(x0, c.y)],
        _ => {
            let r = corner_radius(e).min(w / 2.).min(h / 2.);
            if r < 0.5 {
                vec![v(x0, y0), v(x0 + w, y0), v(x0 + w, y0 + h), v(x0, y0 + h)]
            } else {
                let mut out = vec![];
                // 四个圆角：右上、右下、左下、左上（顺时针）
                for (cx, cy, a0) in [(x0 + w - r, y0 + r, -90.), (x0 + w - r, y0 + h - r, 0.), (x0 + r, y0 + h - r, 90.), (x0 + r, y0 + r, 180.)] {
                    for k in 0..=8 {
                        let a = (a0 + 90. * k as f64 / 8.).to_radians();
                        out.push(v(cx + a.cos() * r, cy + a.sin() * r));
                    }
                }
                out
            }
        }
    };
    local.into_iter().map(|p| p.rot(e.angle, c)).collect()
}

/// 绑定落点：沿「中心 → toward」射线与轮廓的交点，再向外留 gap（与松手后 update_bound_arrows 一致）。
pub fn snap_point(t: &Element, toward: V, gap: f64) -> V {
    let p = focus_point(t, toward);
    let d = p.sub(center(t));
    let l = d.len();
    if l < 1e-6 {
        p
    } else {
        p.add(d.scale(gap / l))
    }
}

/// 找到 p 附近可绑定的元素（从上往下）。
pub fn find_bindable<'a>(doc: &'a SketchDoc, p: V, exclude: &[&str], tol: f64) -> Option<&'a Element> {
    doc.live().rev().filter(|e| e.is_bindable() && !e.locked && !exclude.contains(&e.id.as_str()) && !e.is_text()).find(|e| {
        let r = bounds(e).inflate(tol);
        r.contains(p) && (hit(e, p, tol) || inside_shape(e, p))
    })
}

/// 点是否在形状轮廓内（无论有无填充）
pub fn inside_shape(e: &Element, p: V) -> bool {
    let c = center(e);
    let q = p.rot(-e.angle, c);
    match e.kind.as_str() {
        "ellipse" => ((q.x - c.x) / (e.width / 2.).max(1.)).powi(2) + ((q.y - c.y) / (e.height / 2.).max(1.)).powi(2) <= 1.,
        "diamond" => inside_poly(q, &[v(c.x, e.y), v(e.x + e.width, c.y), v(c.x, e.y + e.height), v(e.x, c.y)]),
        _ => R::xywh(e.x, e.y, e.width, e.height).contains(q),
    }
}

/// 让所有绑定到 ids 中元素（或 ids 为空时所有）的线端点跟随目标。
pub fn update_bound_arrows(doc: &mut SketchDoc, moved: &[String]) {
    let jobs: Vec<usize> = doc
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_deleted && e.is_linear())
        .filter(|(_, e)| [true, false].iter().any(|s| e.binding(*s).is_some_and(|b| moved.is_empty() || moved.contains(&b.element_id))))
        .map(|(i, _)| i)
        .collect();
    for i in jobs {
        let e = doc.elements[i].clone();
        let mut pts = abs_points(&e);
        if pts.len() < 2 {
            continue;
        }
        if e.elbowed() {
            reroute_elbow(doc, i);
            continue;
        }
        // 旋转过的线：先转成世界坐标点、再去掉旋转
        let c = rot_center(&e);
        if e.angle != 0. {
            pts = pts.into_iter().map(|p| p.rot(e.angle, c)).collect();
        }
        let n = pts.len();
        let sb = e.binding(true);
        let eb = e.binding(false);
        let st = sb.as_ref().and_then(|b| doc.get(&b.element_id).cloned());
        let et = eb.as_ref().and_then(|b| doc.get(&b.element_id).cloned());
        // 两端都绑定时，先用目标中心作朝向，避免互相依赖
        if let (Some(b), Some(t)) = (&sb, &st) {
            let toward = if n == 2 { et.as_ref().map(center).unwrap_or(pts[1]) } else { pts[1] };
            pts[0] = bound_point(t, b, toward);
        }
        if let (Some(b), Some(t)) = (&eb, &et) {
            let toward = if n == 2 { st.as_ref().map(center).unwrap_or(pts[0]) } else { pts[n - 2] };
            pts[n - 1] = bound_point(t, b, toward);
        }
        if n == 2 {
            if let (Some(b), Some(t)) = (&sb, &st) {
                pts[0] = bound_point(t, b, pts[1]);
            }
        }
        let el = &mut doc.elements[i];
        el.angle = 0.;
        el.x = pts[0].x;
        el.y = pts[0].y;
        el.points = Some(pts.iter().map(|p| [p.x - pts[0].x, p.y - pts[0].y]).collect());
        el.normalize_points();
        el.bump();
    }
    // 箭头标签跟着箭头
    let label_owners: Vec<String> = doc.live().filter(|e| e.is_linear() && e.bound_text_id().is_some()).map(|e| e.id.clone()).collect();
    for id in label_owners {
        layout_bound_text(doc, &id);
    }
}

// ───────────── 折线箭头（elbow） ─────────────

/// 绑定目标朝向 toward 的那条边的中点（外推 gap），以及该边的外法线方向。
pub fn elbow_side_point(t: &Element, toward: V, gap: f64) -> (V, V) {
    let r = bounds(t);
    let c = r.center();
    let (hw, hh) = ((r.w() / 2.).max(1.), (r.h() / 2.).max(1.));
    let d = toward.sub(c);
    if (d.x / hw).abs() >= (d.y / hh).abs() {
        let s = if d.x >= 0. { 1. } else { -1. };
        (v(c.x + s * (hw + gap), c.y), v(s, 0.))
    } else {
        let s = if d.y >= 0. { 1. } else { -1. };
        (v(c.x, c.y + s * (hh + gap)), v(0., s))
    }
}

/// 正交走线：起点沿 ha 方向离开、终点沿 hb 的反方向进入（ha / hb 为 None 时按两点相对位置推断）。
pub fn elbow_route(a: V, ha: Option<V>, b: V, hb: Option<V>) -> Vec<V> {
    let d = b.sub(a);
    let dom = |d: V| if d.x.abs() >= d.y.abs() { v(d.x.signum(), 0.) } else { v(0., d.y.signum()) };
    let stub = 20.;
    let (a1, ha) = match ha {
        Some(h) => (a.add(h.scale(stub)), h),
        None => (a, dom(d)),
    };
    let (b1, hb) = match hb {
        Some(h) => (b.add(h.scale(stub)), h),
        None => (b, dom(a.sub(b))),
    };
    let mut pts = vec![a, a1];
    let horiz_a = ha.x != 0.;
    let horiz_b = hb.x != 0.;
    match (horiz_a, horiz_b) {
        (true, true) => {
            let mx = (a1.x + b1.x) / 2.;
            pts.extend([v(mx, a1.y), v(mx, b1.y)]);
        }
        (false, false) => {
            let my = (a1.y + b1.y) / 2.;
            pts.extend([v(a1.x, my), v(b1.x, my)]);
        }
        (true, false) => pts.push(v(b1.x, a1.y)),
        (false, true) => pts.push(v(a1.x, b1.y)),
    }
    pts.extend([b1, b]);
    // 去掉重合点与共线的中间点
    let mut out: Vec<V> = vec![];
    for p in pts {
        if out.last().is_some_and(|l: &V| l.dist(p) < 0.01) {
            continue;
        }
        if out.len() >= 2 {
            let (p0, p1) = (out[out.len() - 2], out[out.len() - 1]);
            let collinear = ((p0.x - p1.x).abs() < 0.01 && (p1.x - p.x).abs() < 0.01) || ((p0.y - p1.y).abs() < 0.01 && (p1.y - p.y).abs() < 0.01);
            if collinear {
                out.pop();
            }
        }
        out.push(p);
    }
    if out.len() < 2 {
        out = vec![a, b];
    }
    out
}

/// 按当前端点 / 绑定重新计算折线箭头的走线。
pub fn reroute_elbow(doc: &mut SketchDoc, i: usize) {
    let e = doc.elements[i].clone();
    let pts = world_points(&e);
    if pts.len() < 2 {
        return;
    }
    let (a0, b0) = (pts[0], pts[pts.len() - 1]);
    let st = e.binding(true).and_then(|b| doc.get(&b.element_id).cloned().map(|t| (t, b.gap)));
    let et = e.binding(false).and_then(|b| doc.get(&b.element_id).cloned().map(|t| (t, b.gap)));
    let toward_a = et.as_ref().map(|(t, _)| center(t)).unwrap_or(b0);
    let toward_b = st.as_ref().map(|(t, _)| center(t)).unwrap_or(a0);
    let (a, ha) = match &st {
        Some((t, gap)) => {
            let (p, h) = elbow_side_point(t, toward_a, *gap);
            (p, Some(h))
        }
        None => (a0, None),
    };
    let (b, hb) = match &et {
        Some((t, gap)) => {
            let (p, h) = elbow_side_point(t, toward_b, *gap);
            (p, Some(h))
        }
        None => (b0, None),
    };
    let route = elbow_route(a, ha, b, hb);
    let el = &mut doc.elements[i];
    el.angle = 0.;
    el.x = route[0].x;
    el.y = route[0].y;
    el.points = Some(route.iter().map(|p| [p.x - route[0].x, p.y - route[0].y]).collect());
    el.set("fixedSegments", serde_json::Value::Null);
    el.normalize_points();
    el.bump();
}

/// 线 / 箭头每一段的中点（世界坐标，曲线取曲线上的中点）—— 拖它可以把线弯过去。
pub fn segment_mids(e: &Element) -> Vec<V> {
    let pts = world_points(e);
    if pts.len() < 2 {
        return vec![];
    }
    let round = e.roundness().is_some() && !e.elbowed() && pts.len() >= 3;
    if round {
        let c = curve_points(&pts, true);
        (0..pts.len() - 1).map(|i| c[(i * 10 + 5).min(c.len() - 1)]).collect()
    } else {
        pts.windows(2).map(|w| v((w[0].x + w[1].x) / 2., (w[0].y + w[1].y) / 2.)).collect()
    }
}

// ───────────── 文字 ─────────────

/// 一行文字宽度（与导出 / 换行共用，保证屏幕、SVG、PNG 一致）。
/// 手写体用内嵌字体的真实步进；其他字体按字符类别估算。
pub fn text_width(s: &str, size: f64, family: i64) -> f64 {
    let hand = is_hand_family(family);
    s.chars()
        .map(|c| {
            if hand {
                if let Some(a) = super::fonts::hand_advance(c) {
                    return a * size;
                }
            }
            let w = if c.is_ascii() {
                match c {
                    'i' | 'l' | 'j' | '.' | ',' | '\'' | '!' | '|' | ':' | ';' => 0.32,
                    'm' | 'w' | 'M' | 'W' => 0.85,
                    ' ' => 0.33,
                    c if c.is_ascii_uppercase() => 0.66,
                    c if c.is_ascii_digit() => 0.56,
                    _ => 0.54,
                }
            } else if c as u32 >= 0x2e80 {
                1.0
            } else {
                0.62
            };
            w * size
        })
        .sum()
}

/// 手写字体族（Virgil / Excalifont）
pub fn is_hand_family(f: i64) -> bool {
    !matches!(f, 2 | 3 | 6 | 7 | 8 | 9)
}

/// 按最大宽度换行（中文逐字可断，英文按单词断）。
pub fn wrap_text(text: &str, size: f64, max_w: f64, family: i64) -> String {
    if max_w <= size {
        return text.to_string();
    }
    let mut out: Vec<String> = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        let mut word = String::new();
        let flush_word = |line: &mut String, word: &mut String, out: &mut Vec<String>| {
            if word.is_empty() {
                return;
            }
            if text_width(&format!("{line}{word}"), size, family) <= max_w || line.is_empty() {
                line.push_str(word);
            } else {
                out.push(std::mem::take(line).trim_end().to_string());
                *line = word.trim_start().to_string();
            }
            word.clear();
            // 单个超长单词：硬断
            while text_width(line, size, family) > max_w && line.chars().count() > 1 {
                let mut cut = String::new();
                for ch in line.chars() {
                    if text_width(&format!("{cut}{ch}"), size, family) > max_w && !cut.is_empty() {
                        break;
                    }
                    cut.push(ch);
                }
                let rest = line[cut.len()..].to_string();
                out.push(cut);
                *line = rest;
            }
        };
        for ch in para.chars() {
            if ch.is_ascii_alphanumeric() || (ch.is_ascii_punctuation() && !word.is_empty()) {
                word.push(ch);
            } else {
                flush_word(&mut line, &mut word, &mut out);
                word.push(ch);
                flush_word(&mut line, &mut word, &mut out);
            }
        }
        flush_word(&mut line, &mut word, &mut out);
        out.push(line);
    }
    out.join("\n")
}

pub fn text_size(text: &str, size: f64, line_h: f64, family: i64) -> (f64, f64) {
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lines.iter().map(|l| text_width(l, size, family)).fold(0., f64::max);
    (w.max(size * 0.3), lines.len().max(1) as f64 * size * line_h)
}

/// 独立文字：按内容更新宽高（autoResize）。
pub fn fit_text(e: &mut Element) {
    let text = e.text.clone().unwrap_or_default();
    let fs = e.font_size_or();
    let lh = e.line_height();
    let auto = e.get("autoResize").and_then(|v| v.as_bool()).unwrap_or(true);
    if auto {
        let (w, h) = text_size(&text, fs, lh, e.font_family());
        e.width = w;
        e.height = h;
    } else {
        let wrapped = wrap_text(&e.original_text(), fs, e.width, e.font_family());
        let (_, h) = text_size(&wrapped, fs, lh, e.font_family());
        e.text = Some(wrapped);
        e.height = h;
    }
}

/// 容器内可放文字的最大宽度（官方 getBoundTextMaxWidth）
pub fn container_text_max_w(c: &Element) -> f64 {
    let w = c.width.abs();
    match c.kind.as_str() {
        "ellipse" => (w / 2. * std::f64::consts::SQRT_2 - BOUND_TEXT_PADDING * 2.).max(10.),
        "diamond" => (w / 2. - BOUND_TEXT_PADDING * 2.).max(10.),
        "arrow" | "line" => 220.,
        "stickynote" => (w - STICKY_PADDING * 2.).max(10.),
        _ => (w - BOUND_TEXT_PADDING * 2.).max(10.),
    }
}

fn container_text_max_h(c: &Element) -> f64 {
    let h = c.height.abs();
    match c.kind.as_str() {
        "ellipse" => h / 2. * std::f64::consts::SQRT_2 - BOUND_TEXT_PADDING * 2.,
        "diamond" => h / 2. - BOUND_TEXT_PADDING * 2.,
        _ => h - BOUND_TEXT_PADDING * 2.,
    }
}

/// 按容器（或箭头）重新排版它的绑定文字：换行、必要时撑高容器、居中摆放。
pub fn layout_bound_text(doc: &mut SketchDoc, container: &str) {
    let Some(c) = doc.get(container).cloned() else { return };
    if c.is_sticky() {
        layout_sticky(doc, container);
        return;
    }
    let Some(tid) = c.bound_text_id() else { return };
    let Some(ti) = doc.index(&tid) else { return };
    let t = doc.elements[ti].clone();
    if t.is_deleted {
        return;
    }
    let fs = t.font_size_or();
    let lh = t.line_height();
    let max_w = container_text_max_w(&c);
    let wrapped = wrap_text(&t.original_text(), fs, max_w, t.font_family());
    let (tw, th) = text_size(&wrapped, fs, lh, t.font_family());
    let (x, y) = if c.is_linear() {
        let pts = world_points(&c);
        let mid = polyline_mid(&curve_points(&pts, c.roundness().is_some() && !c.elbowed()));
        (mid.x - tw / 2., mid.y - th / 2.)
    } else {
        // 容器放不下：撑高（官方行为）
        let need = th + BOUND_TEXT_PADDING * 2.;
        let max_h = container_text_max_h(&c);
        if th > max_h {
            if let Some(ci) = doc.index(container) {
                let ce = &mut doc.elements[ci];
                let ratio = match ce.kind.as_str() {
                    "ellipse" => std::f64::consts::SQRT_2,
                    "diamond" => 2.,
                    _ => 1.,
                };
                let new_h = (need * ratio).max(ce.height);
                ce.height = new_h;
                ce.bump();
            }
        }
        let c = doc.get(container).cloned().unwrap();
        let cc = center(&c);
        let x = match t.text_align() {
            "left" => cc.x - max_w / 2.,
            "right" => cc.x + max_w / 2. - tw,
            _ => cc.x - tw / 2.,
        };
        let y = match t.vertical_align() {
            "top" => c.y + BOUND_TEXT_PADDING + (c.height - container_text_max_h(&c) - BOUND_TEXT_PADDING * 2.) / 2.,
            "bottom" => c.y + c.height - BOUND_TEXT_PADDING - th - (c.height - container_text_max_h(&c) - BOUND_TEXT_PADDING * 2.) / 2.,
            _ => cc.y - th / 2.,
        };
        (x, y)
    };
    let angle = if c.is_linear() { 0. } else { c.angle };
    let te = &mut doc.elements[ti];
    if te.text.as_deref() != Some(&wrapped) || te.x != x || te.y != y || te.width != tw || te.height != th || te.angle != angle {
        te.text = Some(wrapped);
        te.x = x;
        te.y = y;
        te.width = tw;
        te.height = th;
        te.angle = angle;
        te.bump();
    }
}

pub fn polyline_mid(pts: &[V]) -> V {
    crate::canvas::math::polyline_mid(pts).unwrap_or_default()
}

/// 便签排版（官方 `getStickyNoteLayout`）：字号从用户设定（baseFontSize）往下缩到 16，
/// 还放不下才把便签撑高；便签不会矮于用户给的高度（baseHeight）。底部留 20px 日期栏。
pub fn layout_sticky(doc: &mut SketchDoc, id: &str) {
    let Some(c) = doc.get(id).cloned() else { return };
    let base_h = c.get("baseHeight").and_then(|v| v.as_f64()).filter(|h| *h > 0.).unwrap_or(c.height).max(STICKY_MIN);
    let inset_y = STICKY_PADDING * 2. + STICKY_FOOTER_H;
    let label = c.bound_text_id().and_then(|t| doc.index(&t)).map(|i| doc.elements[i].clone()).filter(|t| !t.is_deleted);
    let Some(t) = label else {
        if let Some(ce) = doc.get_mut(id) {
            if ce.height != base_h {
                ce.height = base_h;
                ce.bump();
            }
            ce.set("baseHeight", serde_json::json!(base_h));
        }
        return;
    };
    let fam = t.font_family();
    let lh = t.line_height();
    let base_fs = t.get("baseFontSize").and_then(|v| v.as_f64()).unwrap_or(t.font_size_or()).clamp(1., 512.);
    let min_fs = STICKY_MIN_FONT.min(base_fs);
    let max_w = (c.width - STICKY_PADDING * 2.).max(1.);
    let max_h = (base_h - inset_y).max(0.);
    let original = t.original_text();
    let blank = original.trim().is_empty();
    let mut fs = base_fs;
    let (mut wrapped, mut tw, mut th);
    loop {
        wrapped = wrap_text(&original, fs, max_w, fam);
        (tw, th) = text_size(&wrapped, fs, lh, fam);
        let fits = th <= max_h + 0.01 && tw <= max_w + 0.5;
        if blank || fits || fs <= min_fs {
            break;
        }
        fs = (fs - 2.).max(min_fs);
    }
    let h = if blank { base_h } else { base_h.max(th + inset_y) };
    let x = match t.text_align() {
        "left" => c.x + STICKY_PADDING,
        "right" => c.x + c.width - STICKY_PADDING - tw,
        _ => c.x + c.width / 2. - tw / 2.,
    };
    let body_bottom = c.y + h - STICKY_PADDING - STICKY_FOOTER_H;
    let y = match t.vertical_align() {
        "top" => c.y + STICKY_PADDING,
        "bottom" => body_bottom - th,
        _ => (c.y + h / 2. - th / 2.).min(body_bottom - th).max(c.y + STICKY_PADDING),
    };
    if let Some(ce) = doc.get_mut(id) {
        if ce.height != h {
            ce.height = h;
            ce.bump();
        }
        ce.set("baseHeight", serde_json::json!(base_h));
    }
    if let Some(te) = doc.get_mut(&t.id) {
        te.text = Some(wrapped);
        te.font_size = Some(fs);
        te.set("baseFontSize", serde_json::json!(base_fs));
        te.x = x;
        te.y = y;
        te.width = tw;
        te.height = th;
        te.angle = c.angle;
        te.stroke_color = c.stroke_color.clone();
        te.bump();
    }
}

/// 自适应圆角半径（官方 getCornerRadius）
pub fn corner_radius(e: &Element) -> f64 {
    let x = e.width.abs().min(e.height.abs());
    match e.roundness() {
        None => 0.,
        Some(2) => x * 0.25,
        Some(_) => {
            let fixed = e.roundness_value().unwrap_or(32.);
            if x <= fixed / 0.25 {
                x * 0.25
            } else {
                fixed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Element {
        let mut e = Element::new("rectangle", x, y);
        e.width = w;
        e.height = h;
        e
    }

    #[test]
    fn focus_point_on_rect_edge() {
        let r = rect(0., 0., 100., 50.);
        let p = focus_point(&r, v(300., 25.));
        assert!((p.x - 100.).abs() < 1e-6 && (p.y - 25.).abs() < 1e-6);
    }

    #[test]
    fn arrow_follows_moved_target() {
        let mut d = SketchDoc::new();
        let a = rect(0., 0., 100., 100.);
        let b = rect(300., 0., 100., 100.);
        let mut arr = Element::new("arrow", 104., 50.);
        arr.points = Some(vec![[0., 0.], [192., 0.]]);
        arr.set_binding(true, Some(&a.id));
        arr.set_binding(false, Some(&b.id));
        let bid = b.id.clone();
        d.elements = vec![a, b, arr];
        d.elements[1].y = 200.;
        update_bound_arrows(&mut d, &[bid]);
        let pts = world_points(&d.elements[2]);
        let end = *pts.last().unwrap();
        // 终点贴着 b 的轮廓（留 4px 间距）
        let b = &d.elements[1];
        assert!(bounds(b).inflate(5.).contains(end), "{end:?}");
        assert!(!bounds(b).inflate(-1.).contains(end));
    }

    #[test]
    fn hit_rotated_rect_outline() {
        let mut r = rect(0., 0., 100., 20.);
        r.angle = std::f64::consts::FRAC_PI_2;
        // 旋转 90° 后竖着：中心 (50,10)，上边缘在 y = -40
        assert!(hit(&r, v(50., -40.), 2.));
        assert!(!hit(&r, v(0., 10.), 2.));
    }

    #[test]
    fn wrap_cjk_and_words() {
        let s = wrap_text("hello world foo", 20., 70., FONT_NORMAL);
        assert!(s.lines().count() >= 2);
        let s = wrap_text("一二三四五六七八", 20., 60., FONT_NORMAL);
        assert!(s.lines().all(|l| text_width(l, 20., FONT_NORMAL) <= 60.));
    }
}
