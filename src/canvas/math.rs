//! 两套白板引擎（画板 `board/`、草图 `sketch/`）共用的平面几何：与数据格式无关，只依赖「点有 x / y」。
//!
//! 画板用 `f32` 的 [`Pt`](super::Pt)，草图为了与 Excalidraw 数值一致用 `f64` 的 `sketch::geom::V`；
//! 这里的函数对两种标量 / 点类型泛型实现，命中测试、缩放手柄、对齐与分布只写一份。

use std::ops::{Add, Div, Mul, Sub};

/// 标量：`f32` 或 `f64`。
pub trait Scalar: Copy + PartialOrd + Default + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self> {
    fn of(v: f64) -> Self;
    fn sqrt(self) -> Self;
    fn max_value() -> Self;
}

impl Scalar for f32 {
    fn of(v: f64) -> Self {
        v as f32
    }
    fn sqrt(self) -> Self {
        f32::sqrt(self)
    }
    fn max_value() -> Self {
        f32::MAX
    }
}

impl Scalar for f64 {
    fn of(v: f64) -> Self {
        v
    }
    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }
    fn max_value() -> Self {
        f64::MAX
    }
}

/// 平面上的点。
pub trait P2: Copy {
    type S: Scalar;
    fn x(&self) -> Self::S;
    fn y(&self) -> Self::S;
    fn mk(x: Self::S, y: Self::S) -> Self;
}

impl P2 for super::Pt {
    type S = f32;
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn mk(x: f32, y: f32) -> Self {
        super::pt(x, y)
    }
}

fn clamp01<S: Scalar>(t: S) -> S {
    if t < S::of(0.) {
        S::of(0.)
    } else if t > S::of(1.) {
        S::of(1.)
    } else {
        t
    }
}

fn smin<S: Scalar>(a: S, b: S) -> S {
    if b < a {
        b
    } else {
        a
    }
}

pub fn dist<P: P2>(a: P, b: P) -> P::S {
    let (dx, dy) = (a.x() - b.x(), a.y() - b.y());
    (dx * dx + dy * dy).sqrt()
}

pub fn lerp<P: P2>(a: P, b: P, t: P::S) -> P {
    P::mk(a.x() + (b.x() - a.x()) * t, a.y() + (b.y() - a.y()) * t)
}

pub fn mid<P: P2>(a: P, b: P) -> P {
    lerp(a, b, P::S::of(0.5))
}

/// 点到线段的距离。
pub fn dist_seg<P: P2>(p: P, a: P, b: P) -> P::S {
    let (dx, dy) = (b.x() - a.x(), b.y() - a.y());
    let l2 = dx * dx + dy * dy;
    if l2 < P::S::of(1e-9) {
        return dist(p, a);
    }
    let t = clamp01(((p.x() - a.x()) * dx + (p.y() - a.y()) * dy) / l2);
    dist(p, P::mk(a.x() + dx * t, a.y() + dy * t))
}

/// 点到折线的最短距离（单点折线退化为到该点的距离）。
pub fn dist_polyline<P: P2>(p: P, pts: &[P]) -> P::S {
    match pts {
        [] => P::S::max_value(),
        [a] => dist(p, *a),
        _ => pts.windows(2).map(|w| dist_seg(p, w[0], w[1])).fold(P::S::max_value(), smin),
    }
}

/// 射线法判断点是否在多边形内。
pub fn inside_poly<P: P2>(p: P, poly: &[P]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y() > p.y()) != (b.y() > p.y()) && p.x() < (b.x() - a.x()) * (p.y() - a.y()) / (b.y() - a.y() + P::S::of(1e-12)) + a.x() {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 按长度取折线的中点。
pub fn polyline_mid<P: P2>(pts: &[P]) -> Option<P> {
    let first = *pts.first()?;
    let total = pts.windows(2).map(|w| dist(w[0], w[1])).fold(P::S::of(0.), |a, b| a + b);
    let half = total * P::S::of(0.5);
    let mut acc = P::S::of(0.);
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        if acc + l >= half && l > P::S::of(0.) {
            return Some(lerp(w[0], w[1], (half - acc) / l));
        }
        acc = acc + l;
    }
    Some(first)
}

/// 8 个缩放手柄：左上、上、右上、右、右下、下、左下、左。`q` 为按顺时针排列的四个角（可以是旋转后的）。
pub fn handles8<P: P2>(q: &[P; 4]) -> [P; 8] {
    [q[0], mid(q[0], q[1]), q[1], mid(q[1], q[2]), q[2], mid(q[2], q[3]), q[3], mid(q[3], q[0])]
}

/// 轴对齐矩形的四个角（左上起顺时针）。
pub fn corners<P: P2>(x0: P::S, y0: P::S, x1: P::S, y1: P::S) -> [P; 4] {
    [P::mk(x0, y0), P::mk(x1, y0), P::mk(x1, y1), P::mk(x0, y1)]
}

/// 对齐方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

impl Align {
    /// 按工具栏里的序号（左 / 水平居中 / 右 / 上 / 垂直居中 / 下）。
    pub fn from_index(k: u8) -> Align {
        [Align::Left, Align::HCenter, Align::Right, Align::Top, Align::VCenter, Align::Bottom][(k as usize).min(5)]
    }
}

/// 把 `(x0, y0, x1, y1)` 的单元对齐到整体包围盒 `all` 需要的位移。
pub fn align_delta<S: Scalar>(how: Align, all: (S, S, S, S), r: (S, S, S, S)) -> (S, S) {
    let half = S::of(0.5);
    let z = S::of(0.);
    match how {
        Align::Left => (all.0 - r.0, z),
        Align::HCenter => ((all.0 + all.2) * half - (r.0 + r.2) * half, z),
        Align::Right => (all.2 - r.2, z),
        Align::Top => (z, all.1 - r.1),
        Align::VCenter => (z, (all.1 + all.3) * half - (r.1 + r.3) * half),
        Align::Bottom => (z, all.3 - r.3),
    }
}

/// 等间距分布：输入已排好序的 `(起点, 长度)`，保持首尾不动，返回每个单元的新起点。
pub fn distribute<S: Scalar>(spans: &[(S, S)]) -> Vec<S> {
    if spans.len() < 3 {
        return spans.iter().map(|s| s.0).collect();
    }
    let start = spans[0].0;
    let end = spans.iter().map(|s| s.0 + s.1).fold(start, |a, b| if b > a { b } else { a });
    let total = spans.iter().map(|s| s.1).fold(S::of(0.), |a, b| a + b);
    let gap = (end - start - total) / S::of((spans.len() - 1) as f64);
    let mut cur = start;
    spans
        .iter()
        .map(|s| {
            let at = cur;
            cur = cur + s.1 + gap;
            at
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{pt, Pt};

    #[test]
    fn distances_and_polygons() {
        assert!((dist_seg(pt(0., 5.), pt(-10., 0.), pt(10., 0.)) - 5.).abs() < 1e-6);
        assert!((dist_seg(pt(13., 4.), pt(0., 0.), pt(10., 0.)) - 5.).abs() < 1e-6, "落在线段外取端点");
        assert!((dist_polyline(pt(5., 3.), &[pt(0., 0.), pt(10., 0.), pt(10., 10.)]) - 3.).abs() < 1e-6);
        let sq = [pt(0., 0.), pt(10., 0.), pt(10., 10.), pt(0., 10.)];
        assert!(inside_poly(pt(5., 5.), &sq) && !inside_poly(pt(15., 5.), &sq));
        let m: Pt = polyline_mid(&[pt(0., 0.), pt(10., 0.), pt(10., 10.)]).unwrap();
        assert!((m.x - 10.).abs() < 1e-5 && m.y.abs() < 1e-5);
    }

    #[test]
    fn handles_align_distribute() {
        let h = handles8::<Pt>(&corners(0., 0., 10., 20.));
        assert_eq!((h[1].x, h[1].y), (5., 0.));
        assert_eq!((h[7].x, h[7].y), (0., 10.));
        assert_eq!(align_delta(Align::Right, (0f64, 0., 100., 50.), (10., 10., 30., 20.)), (70., 0.));
        assert_eq!(align_delta(Align::VCenter, (0f64, 0., 100., 50.), (10., 10., 30., 20.)), (0., 10.));
        assert_eq!(distribute(&[(0f32, 10.), (12., 10.), (90., 10.)]), vec![0., 45., 90.]);
    }
}
