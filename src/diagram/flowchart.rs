//! 流程图：`flowchart TD` / `graph LR`。
//!
//! 同时提供通用的 `Graph` → `Scene` 构建（类图 / 状态图 / ER 图也用它）。

use super::sugiyama::{self, Dir, LEdge, LNode, Spacing};
use super::*;
use crate::canvas::geom::boundary_point;
use crate::canvas::{rect, Align, Arrow, Item, LineItem, Pt, Section, ShapeItem, ShapeKind};
use std::collections::HashMap;

// ───────────────────────────── 通用图结构 ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct GNode {
    pub id: String,
    pub text: String,
    pub shape: ShapeKind,
    pub style: Style,
    pub sections: Vec<Section>,
    pub cluster: Option<usize>,
    /// 固定尺寸（开始 / 结束点、分叉条等）
    pub fixed: Option<(f32, f32)>,
    pub classes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GEdge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub start: Arrow,
    pub end: Arrow,
    pub dash: bool,
    pub dotted: bool,
    pub thick: bool,
    pub invisible: bool,
    pub minlen: usize,
    pub start_label: String,
    pub end_label: String,
    pub color: Option<Col>,
}

#[derive(Clone, Debug, Default)]
pub struct GCluster {
    pub title: String,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    pub clusters: Vec<GCluster>,
    pub index: HashMap<String, usize>,
    pub curved: bool,
}

impl Graph {
    pub fn new(dir: Dir) -> Self {
        Graph { dir, nodes: vec![], edges: vec![], clusters: vec![], index: HashMap::new(), curved: true }
    }

    /// 取得（或创建）节点。
    pub fn node(&mut self, id: &str, cluster: Option<usize>) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        let i = self.nodes.len();
        self.nodes.push(GNode { id: id.to_string(), text: id.to_string(), cluster, ..Default::default() });
        self.index.insert(id.to_string(), i);
        i
    }

    fn node_size(&self, n: &GNode, theme: &Theme, m: &mut dyn Measure) -> (f32, f32) {
        if let Some(s) = n.fixed {
            return s;
        }
        let fs = theme.font_size;
        if !n.sections.is_empty() {
            let mut w: f32 = 0.;
            let mut lines = 0;
            for s in &n.sections {
                for l in &s.lines {
                    w = w.max(m.width(l, fs, s.bold));
                    lines += 1;
                }
            }
            let h = lines as f32 * fs * 1.45 + n.sections.len() as f32 * 8. + 6.;
            return ((w + 28.).max(110.), h.max(40.));
        }
        let (tw, th) = text_size(m, &n.text, fs, n.style.bold);
        let (mut w, mut h) = (tw + 36., th + 20.);
        match n.shape {
            ShapeKind::Diamond => {
                // paint::text_area 对菱形左右各留 0.18*w（可用内宽 0.64*w），按菱形几何内接矩形反算宽高，避免文字异常折行或贴边
                let tw_pad = tw + 18.;
                let th_pad = th + 16.;
                w = (tw_pad / 0.62).max(tw + 56.);
                let rem = (1.0 - tw_pad / w).max(0.32);
                h = (th_pad / rem).max(th + 42.).max(w * 0.44);
            }
            ShapeKind::Circle | ShapeKind::DoubleCircle => {
                let d = (tw + 28.).max(th + 28.);
                w = d;
                h = d;
            }
            ShapeKind::Hexagon | ShapeKind::Parallelogram | ShapeKind::ParallelogramAlt | ShapeKind::Trapezoid | ShapeKind::TrapezoidAlt => w += 30.,
            ShapeKind::Cylinder => h += 18.,
            ShapeKind::Document => h += 12.,
            ShapeKind::Stadium | ShapeKind::Flag => w += 18.,
            ShapeKind::Ellipse => {
                w += 26.;
                h += 10.;
            }
            ShapeKind::Note => w += 6.,
            _ => {}
        }
        (w.max(56.), h.max(38.))
    }

    /// 连线端点落到节点边上的端口：返回每条边两端的精确朝外法向 `(da, db)`。
    /// - 常规流向从主轴两端进出；
    /// - 回边若走外侧走廊则从节点左右侧进出，避免穿插下方分支；
    /// - 多分支菱形的最左/最右外侧分支从菱形左右顶点出线，减轻底尖拥挤。
    fn assign_ports(&self, rects: &[crate::canvas::Rect], paths: &mut [Option<Vec<Pt>>]) -> Vec<Option<(Pt, Pt)>> {
        // (节点, 边) → [(边序号, 是否终点, 排序键)]；边：0 上、1 下、2 左、3 右
        let mut groups: BTreeMap<(usize, u8), Vec<(usize, bool, f32)>> = BTreeMap::new();
        let vertical = !self.dir.horizontal();
        let mut out_deg = vec![0usize; self.nodes.len()];
        for e in &self.edges {
            if e.from != e.to && !e.invisible {
                out_deg[e.from] += 1;
            }
        }
        let is_back = |e: &GEdge| -> bool {
            let (a, b) = (rects[e.from].center(), rects[e.to].center());
            match self.dir {
                Dir::TB => a.y > b.y + 10.,
                Dir::BT => a.y < b.y - 10.,
                Dir::LR => a.x > b.x + 10.,
                Dir::RL => a.x < b.x - 10.,
            }
        };
        let mut sides: Vec<(Option<u8>, Option<u8>)> = vec![(None, None); self.edges.len()];
        for (k, e) in self.edges.iter().enumerate() {
            let Some(pts) = &paths[k] else { continue };
            if pts.len() < 2 || e.from == e.to {
                continue;
            }
            let back = is_back(e);
            for (node, end, nb) in [(e.from, false, pts[1]), (e.to, true, pts[pts.len() - 2])] {
                let r = rects[node];
                let side = if vertical {
                    if back && nb.x < r.x - 12. {
                        2
                    } else if back && nb.x > r.right() + 12. {
                        3
                    } else if !end && self.nodes[node].shape == ShapeKind::Diamond && out_deg[node] >= 3 && nb.x < r.x - 24. {
                        2
                    } else if !end && self.nodes[node].shape == ShapeKind::Diamond && out_deg[node] >= 3 && nb.x > r.right() + 24. {
                        3
                    } else if nb.y >= r.bottom() {
                        1
                    } else if nb.y <= r.y {
                        0
                    } else if nb.x >= r.center().x {
                        3
                    } else {
                        2
                    }
                } else if back && nb.y < r.y - 12. {
                    0
                } else if back && nb.y > r.bottom() + 12. {
                    1
                } else if !end && self.nodes[node].shape == ShapeKind::Diamond && out_deg[node] >= 3 && nb.y < r.y - 24. {
                    0
                } else if !end && self.nodes[node].shape == ShapeKind::Diamond && out_deg[node] >= 3 && nb.y > r.bottom() + 24. {
                    1
                } else if nb.x >= r.right() {
                    3
                } else if nb.x <= r.x {
                    2
                } else if nb.y >= r.center().y {
                    1
                } else {
                    0
                };
                if end {
                    sides[k].1 = Some(side);
                } else {
                    sides[k].0 = Some(side);
                }
                let key = if side < 2 { nb.x } else { nb.y };
                groups.entry((node, side)).or_default().push((k, end, key));
            }
        }
        for ((node, side), mut list) in groups {
            list.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
            let (r, m) = (rects[node], list.len());
            let step = if m > 1 { (0.56 / (m - 1) as f32).min(0.22) } else { 0. };
            for (i, (k, end, _)) in list.into_iter().enumerate() {
                let f = 0.5 + (i as f32 - (m as f32 - 1.) / 2.) * step;
                let toward = match side {
                    0 => crate::canvas::pt(r.x + f * r.w, r.y),
                    1 => crate::canvas::pt(r.x + f * r.w, r.bottom()),
                    2 => crate::canvas::pt(r.x, r.y + f * r.h),
                    _ => crate::canvas::pt(r.right(), r.y + f * r.h),
                };
                let p = boundary_point(self.nodes[node].shape, r, toward);
                if let Some(pts) = paths[k].as_mut() {
                    let i = if end { pts.len() - 1 } else { 0 };
                    pts[i] = p;
                }
            }
        }
        let normal = |s: u8| match s {
            0 => crate::canvas::pt(0., -1.),
            1 => crate::canvas::pt(0., 1.),
            2 => crate::canvas::pt(-1., 0.),
            _ => crate::canvas::pt(1., 0.),
        };
        sides.into_iter().map(|(a, b)| Some((normal(a?), normal(b?)))).collect()
    }

    pub fn to_scene(&self, theme: &Theme, m: &mut dyn Measure, offsets: &BTreeMap<String, (f32, f32)>) -> Scene {
        let sizes: Vec<(f32, f32)> = self.nodes.iter().map(|n| self.node_size(n, theme, m)).collect();
        let lnodes: Vec<LNode> = self.nodes.iter().zip(&sizes).map(|(n, &(w, h))| LNode { w, h, cluster: n.cluster }).collect();
        let label_fs = theme.font_size - 2.;
        let ledges: Vec<LEdge> = self
            .edges
            .iter()
            .map(|e| LEdge {
                from: e.from,
                to: e.to,
                minlen: e.minlen.max(1),
                label_w: if e.label.is_empty() { 0. } else { text_size(m, &e.label, label_fs, false).0 },
            })
            .collect();
        let labeled = self.edges.iter().any(|e| !e.label.is_empty() || !e.start_label.is_empty());
        let sp = Spacing { node: if self.clusters.is_empty() { 48. } else { 52. }, rank: if !self.clusters.is_empty() || labeled { 78. } else { 56. } };
        let mut pair_count: HashMap<(usize, usize), usize> = HashMap::new();
        for e in &self.edges {
            *pair_count.entry((e.from.min(e.to), e.from.max(e.to))).or_default() += 1;
        }
        let _ = pair_count;
        let lay = sugiyama::layout(&lnodes, &ledges, self.dir, &sp);
        let rects: Vec<crate::canvas::Rect> = lay.pos.iter().zip(&sizes).map(|(c, &(w, h))| rect(c.x - w / 2., c.y - h / 2., w, h)).collect();
        let mut scene = Scene::default();

        // 子图边框：从最内层开始，包住成员节点与子子图
        let mut cbox: Vec<Option<crate::canvas::Rect>> = vec![None; self.clusters.len()];
        for ci in (0..self.clusters.len()).rev() {
            let mut b: Option<crate::canvas::Rect> = None;
            for (ni, n) in self.nodes.iter().enumerate() {
                if n.cluster == Some(ci) {
                    let r = rects[ni];
                    b = Some(b.map_or(r, |x| x.union(&r)));
                }
            }
            for (cj, c) in self.clusters.iter().enumerate() {
                if c.parent == Some(ci) {
                    if let Some(r) = cbox[cj] {
                        b = Some(b.map_or(r, |x| x.union(&r)));
                    }
                }
            }
            if let Some(r) = b {
                let title_h = if self.clusters[ci].title.is_empty() { 8. } else { theme.font_size * 1.9 };
                cbox[ci] = Some(crate::canvas::rect(r.x - 16., r.y - 12. - title_h, r.w + 32., r.h + 24. + title_h));
            }
        }
        for (ci, c) in self.clusters.iter().enumerate() {
            if let Some(r) = cbox[ci] {
                let mut s = ShapeItem::new(ShapeKind::RoundRect, r);
                s.radius = 8.;
                s.fill = Some(theme.cluster_fill.with_alpha(if theme.dark { 0.9 } else { 0.85 }));
                s.stroke = Some(theme.cluster_stroke);
                s.stroke_w = 1.2;
                s.text = c.title.clone();
                s.text_color = theme.text;
                s.font_size = theme.font_size - 1.;
                s.bold = true;
                s.align = Align::TopCenter;
                s.seed = 900 + ci as u64;
                scene.items.push(Item::Shape(s));
            }
        }

        // 连线：先算出每条边的折线，分配两端端口，再做 90° 正交圆角走廊布线
        let mut paths: Vec<Option<Vec<Pt>>> = vec![None; self.edges.len()];
        for (k, e) in self.edges.iter().enumerate() {
            if e.invisible {
                continue;
            }
            paths[k] = Some(lay.edges[k].clone());
        }
        let port_normals = self.assign_ports(&rects, &mut paths);
        if self.curved {
            self.route_orthogonal(&rects, &lay, &mut paths, &port_normals);
        }
        let mut edge_lines: Vec<LineItem> = vec![];
        let mut edge_ends = vec![];
        for (k, e) in self.edges.iter().enumerate() {
            let Some(pts) = paths[k].take() else { continue };
            let self_loop = e.from == e.to;
            let mut l = LineItem::new(pts);
            l.id = format!("edge:{k}");
            if self_loop {
                l.curved = true;
                l.basis = true;
            } else if self.curved {
                l.curved = false;
                l.basis = false;
                l.corner_radius = 8.;
            }
            l.stroke = e.color.unwrap_or(theme.line);
            l.width = if e.thick { 3. } else { 1.5 };
            l.dash = e.dash;
            l.dotted = e.dotted;
            l.start = e.start;
            l.end = e.end;
            l.label = e.label.clone();
            l.start_label = e.start_label.clone();
            l.end_label = e.end_label.clone();
            l.label_color = theme.text;
            l.label_bg = Some(theme.label_bg);
            l.font_size = label_fs;
            l.seed = 300 + k as u64;
            edge_lines.push(l);
            edge_ends.push((self.nodes[e.from].id.clone(), self.nodes[e.to].id.clone()));
        }
        place_edge_labels(&mut edge_lines, &rects, m);
        for l in edge_lines {
            scene.items.push(Item::Line(l));
        }

        // 节点
        for (i, n) in self.nodes.iter().enumerate() {
            let mut s = ShapeItem::new(n.shape, rects[i]);
            s.id = n.id.clone();
            s.fill = Some(n.style.fill.unwrap_or(theme.node_fill));
            s.stroke = Some(n.style.stroke.unwrap_or(theme.node_stroke));
            s.stroke_w = n.style.stroke_width.unwrap_or(1.5);
            s.dash = n.style.dash;
            s.text = n.text.clone();
            s.text_color = n.style.color.unwrap_or(theme.text);
            s.font_size = theme.font_size;
            s.bold = n.style.bold;
            s.sections = n.sections.clone();
            s.radius = 8.;
            s.seed = 10 + i as u64;
            match n.shape {
                ShapeKind::Note => {
                    s.fill = Some(n.style.fill.unwrap_or(theme.note_fill));
                    s.stroke = Some(n.style.stroke.unwrap_or(theme.note_stroke));
                }
                ShapeKind::StartDot | ShapeKind::EndDot | ShapeKind::Bar => {
                    s.stroke = Some(theme.text);
                    s.fill = Some(theme.text);
                }
                _ => {}
            }
            scene.items.push(Item::Shape(s));
        }
        apply_offsets(&mut scene, offsets, &edge_ends);
        scene.fit(16.);
        scene
    }

    /// 90° 正交圆角走廊布线：
    /// - 纵向/横向四方向统一在 `(main, cross)` 坐标系内完成；
    /// - 所有跨通道水平转折严格落在相邻两层之间的无节点走廊里，并按交叉最小化顺序分配互不重叠的轨道；
    /// - 菱形左右顶点出线直接做 L 形折转；
    /// - 回边沿最外侧通道走干净的 C 形包络，绝不穿插节点。
    fn route_orthogonal(&self, rects: &[crate::canvas::Rect], lay: &sugiyama::Layout, paths: &mut [Option<Vec<Pt>>], port_normals: &[Option<(Pt, Pt)>]) {
        let horiz = self.dir.horizontal();
        let rev_axis = matches!(self.dir, Dir::BT | Dir::RL);
        let main_of = |p: Pt| if horiz { p.x } else { p.y };
        let cross_of = |p: Pt| if horiz { p.y } else { p.x };
        let mk = |m: f32, c: f32| if horiz { crate::canvas::pt(m, c) } else { crate::canvas::pt(c, m) };
        let rect_main = |r: crate::canvas::Rect| if horiz { (r.x, r.right()) } else { (r.y, r.bottom()) };
        let rect_cross = |r: crate::canvas::Rect| if horiz { (r.y, r.bottom()) } else { (r.x, r.right()) };
        let cross_clear = |m: f32, c0: f32, c1: f32, skip_a: usize, skip_b: usize| -> bool {
            let (lo, hi) = (c0.min(c1), c0.max(c1));
            for (i, &r) in rects.iter().enumerate() {
                if i == skip_a || i == skip_b {
                    continue;
                }
                let (m0, m1) = rect_main(r);
                let (r0, r1) = rect_cross(r);
                if m > m0 - 6. && m < m1 + 6. && hi > r0 - 6. && lo < r1 + 6. {
                    return false;
                }
            }
            true
        };
        let nlayers = lay.layer_spans.len();
        let corridor = |b: usize| -> (f32, f32) {
            if rev_axis {
                (lay.layer_spans[b].0, lay.layer_spans[b + 1].1)
            } else {
                (lay.layer_spans[b].1, lay.layer_spans[b + 1].0)
            }
        };

        struct BandJog {
            band: usize,
            c_from: f32,
            c_to: f32,
            c_lo: f32,
            c_hi: f32,
        }
        enum EdgePlan {
            Direct(Vec<Pt>),
            Forward { p0: Pt, p1: Pt, l_turn: Option<Pt>, jogs: Vec<usize> },
        }

        let mut all_jogs: Vec<BandJog> = vec![];
        let mut plans: Vec<Option<EdgePlan>> = (0..self.edges.len()).map(|_| None).collect();

        // 计算全局横向边界（用于回边外侧绕行）
        let (mut global_c_min, mut global_c_max) = (f32::INFINITY, f32::NEG_INFINITY);
        for &r in rects {
            let (a, b) = rect_cross(r);
            global_c_min = global_c_min.min(a);
            global_c_max = global_c_max.max(b);
        }

        for (k, e) in self.edges.iter().enumerate() {
            if e.from == e.to {
                continue;
            }
            let Some(pts) = paths[k].as_ref() else { continue };
            if pts.len() < 2 {
                continue;
            }
            let p0 = pts[0];
            let p1 = *pts.last().unwrap();
            let lf = lay.node_layer[e.from];
            let lt = lay.node_layer[e.to];
            let (da, db) = port_normals[k].unwrap_or((crate::canvas::pt(0., 1.), crate::canvas::pt(0., -1.)));
            let da_cross = cross_of(da).abs() > 0.5;
            let db_cross = cross_of(db).abs() > 0.5;

            if lf >= lt {
                // 回边或同层边：沿外侧垂直/水平通道走干净 C 形包络
                let dummies = &pts[1..pts.len() - 1];
                let mut c_rail = if !dummies.is_empty() {
                    dummies.iter().map(|&p| cross_of(p)).sum::<f32>() / dummies.len() as f32
                } else if cross_of(da) < 0. || cross_of(p0) < (global_c_min + global_c_max) * 0.5 {
                    global_c_min - 28.
                } else {
                    global_c_max + 28.
                };
                // 保证通道位于起终点对应外侧之外，避免贴死节点
                let (fc0, fc1) = rect_cross(rects[e.from]);
                let (tc0, tc1) = rect_cross(rects[e.to]);
                if da_cross && cross_of(da) < 0. {
                    c_rail = c_rail.min(fc0 - 26.);
                } else if da_cross && cross_of(da) > 0. {
                    c_rail = c_rail.max(fc1 + 26.);
                }
                if db_cross && cross_of(db) < 0. {
                    c_rail = c_rail.min(tc0 - 26.);
                } else if db_cross && cross_of(db) > 0. {
                    c_rail = c_rail.max(tc1 + 26.);
                }
                let start_ok = da_cross && cross_clear(main_of(p0), cross_of(p0), c_rail, e.from, e.to);
                let end_ok = db_cross && cross_clear(main_of(p1), cross_of(p1), c_rail, e.from, e.to);
                let mut out = vec![p0];
                if start_ok {
                    out.push(mk(main_of(p0), c_rail));
                } else {
                    let (m0, m1) = rect_main(rects[e.from]);
                    let m_out = if main_of(da) > 0. { m1 + 18. } else { m0 - 18. };
                    out.push(mk(m_out, cross_of(p0)));
                    out.push(mk(m_out, c_rail));
                }
                if end_ok {
                    out.push(mk(main_of(p1), c_rail));
                } else {
                    let (m0, m1) = rect_main(rects[e.to]);
                    let m_in = if main_of(db) > 0. { m1 + 18. } else { m0 - 18. };
                    out.push(mk(m_in, c_rail));
                    out.push(mk(m_in, cross_of(p1)));
                }
                out.push(p1);
                plans[k] = Some(EdgePlan::Direct(out));
                continue;
            }

            // 正向边（lf < lt）
            let dummies = &pts[1..pts.len() - 1];
            let mut cur_c = cross_of(p0);
            let mut l_turn = None;
            if da_cross {
                let first_c = dummies.first().map(|&p| cross_of(p)).unwrap_or_else(|| cross_of(p1));
                let sgn = cross_of(da).signum();
                if (first_c - cur_c) * sgn >= 12. && cross_clear(main_of(p0), cur_c, first_c, e.from, e.to) {
                    l_turn = Some(mk(main_of(p0), first_c));
                    cur_c = first_c;
                }
            }
            let mut jogs = vec![];
            for b in lf..lt {
                let next_c = if b + 1 < lt { dummies.get(b - lf).map(|&p| cross_of(p)).unwrap_or_else(|| cross_of(p1)) } else { cross_of(p1) };
                if (cur_c - next_c).abs() >= 2.0 && b + 1 < nlayers {
                    let id = all_jogs.len();
                    all_jogs.push(BandJog { band: b, c_from: cur_c, c_to: next_c, c_lo: cur_c.min(next_c), c_hi: cur_c.max(next_c) });
                    jogs.push(id);
                    cur_c = next_c;
                }
            }
            plans[k] = Some(EdgePlan::Forward { p0, p1, l_turn, jogs });
        }

        // 按走廊分配不重叠轨道，并在每条走廊内以成对交叉最小化排序（消除扇出/扇入交叉）
        let mut jog_m = vec![0f32; all_jogs.len()];
        let mut by_band: Vec<Vec<usize>> = vec![vec![]; nlayers];
        for (id, j) in all_jogs.iter().enumerate() {
            if j.band + 1 < nlayers {
                by_band[j.band].push(id);
            }
        }
        let overlap = |a: &BandJog, b: &BandJog| a.c_lo < b.c_hi + 10. && b.c_lo < a.c_hi + 10.;
        let inside = |x: f32, a: &BandJog| x > a.c_lo + 1. && x < a.c_hi - 1.;
        for (b, ids) in by_band.iter_mut().enumerate() {
            if ids.is_empty() {
                continue;
            }
            // 先按跨度降序稳定预排，再做局部成对交换消除所有可避免的垂线-横线交叉
            ids.sort_by(|&ia, &ib| {
                let (a, b) = (&all_jogs[ia], &all_jogs[ib]);
                (b.c_hi - b.c_lo).partial_cmp(&(a.c_hi - a.c_lo)).unwrap()
            });
            for _ in 0..ids.len().min(12) {
                let mut swapped = false;
                for i in 0..ids.len().saturating_sub(1) {
                    let (a, bb) = (&all_jogs[ids[i]], &all_jogs[ids[i + 1]]);
                    if !overlap(a, bb) {
                        continue;
                    }
                    let c_ab = (inside(bb.c_from, a) as u8) + (inside(a.c_to, bb) as u8);
                    let c_ba = (inside(a.c_from, bb) as u8) + (inside(bb.c_to, a) as u8);
                    if c_ba < c_ab {
                        ids.swap(i, i + 1);
                        swapped = true;
                    }
                }
                if !swapped {
                    break;
                }
            }
            // 单调轨道着色：若 j 与排在前面的 p 重叠，则 track[j] 必须严格大于 track[p]
            let mut tracks = vec![0usize; ids.len()];
            for i in 0..ids.len() {
                let mut t = 0usize;
                for prev in 0..i {
                    if overlap(&all_jogs[ids[prev]], &all_jogs[ids[i]]) {
                        t = t.max(tracks[prev] + 1);
                    }
                }
                tracks[i] = t;
            }
            let num_tracks = tracks.iter().copied().max().unwrap_or(0) + 1;
            let (m_enter, m_exit) = corridor(b);
            let span = (m_exit - m_enter).abs();
            let margin = if num_tracks >= 4 { 12f32.min(span * 0.2) } else { 15f32.min(span * 0.28) };
            let dir_sign = if m_exit >= m_enter { 1. } else { -1. };
            let (m0, m1) = (m_enter + dir_sign * margin, m_exit - dir_sign * margin);
            for (idx, &jid) in ids.iter().enumerate() {
                let frac = if num_tracks == 1 { 0.5 } else { (tracks[idx] as f32 + 0.5) / num_tracks as f32 };
                jog_m[jid] = m0 + (m1 - m0) * frac;
            }
        }

        // 生成每条边的最终正交折线
        for (k, plan) in plans.into_iter().enumerate() {
            let Some(plan) = plan else { continue };
            let pts = match plan {
                EdgePlan::Direct(v) => v,
                EdgePlan::Forward { p0, p1, l_turn, jogs } => {
                    let mut v = vec![p0];
                    if let Some(lp) = l_turn {
                        v.push(lp);
                    }
                    for jid in jogs {
                        let m = jog_m[jid];
                        let j = &all_jogs[jid];
                        v.push(mk(m, j.c_from));
                        v.push(mk(m, j.c_to));
                    }
                    v.push(p1);
                    v
                }
            };
            paths[k] = Some(pts);
        }
    }
}

/// 在每条边的真实路径上为标签寻找不与节点及其他标签重叠、且尽量避开 90° 拐角的位置（短边优先占位）。
fn place_edge_labels(lines: &mut [LineItem], node_rects: &[crate::canvas::Rect], m: &mut dyn Measure) {
    let overlap_area = |a: crate::canvas::Rect, b: crate::canvas::Rect| -> f32 {
        let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0.);
        let h = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.);
        w * h
    };
    let mut occupied: Vec<crate::canvas::Rect> = node_rects.iter().map(|r| r.inflate(5.)).collect();
    let mut order: Vec<(usize, f32)> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.label.is_empty() && l.points.len() >= 2)
        .map(|(i, l)| {
            let len: f32 = l.points.windows(2).map(|w| w[0].dist(w[1])).sum();
            (i, len)
        })
        .collect();
    order.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    const TS: [f32; 13] = [0.50, 0.44, 0.56, 0.36, 0.64, 0.28, 0.72, 0.20, 0.80, 0.32, 0.68, 0.14, 0.86];
    for (idx, _) in order {
        let cmds = crate::canvas::paint::line_cmds(&lines[idx]);
        let (lw, lh) = text_size(m, &lines[idx].label, lines[idx].font_size, false);
        let (bw, bh) = (lw + 12., lh + 6.);
        let corners: Vec<Pt> = if lines[idx].points.len() > 2 { lines[idx].points[1..lines[idx].points.len() - 1].to_vec() } else { vec![] };
        let score = |p: Pt, occ: &[crate::canvas::Rect]| -> (f32, f32) {
            let r = rect(p.x - bw / 2., p.y - bh / 2., bw, bh);
            let ov: f32 = occ.iter().map(|o| overlap_area(r, *o)).sum();
            // 优先落在笔直段上，尽量不要骑在 90° 拐角上
            let corner_pen: f32 = corners.iter().map(|c| (18. - p.dist(*c)).max(0.)).sum();
            (ov, corner_pen)
        };
        let mut best_p = crate::canvas::geom::cmds_point_at(&cmds, 0.5);
        let mut best_s = (f32::INFINITY, f32::INFINITY);
        for &t in &TS {
            let p = crate::canvas::geom::cmds_point_at(&cmds, t);
            let s = score(p, &occupied);
            if s.0 < best_s.0 - 0.01 || ((s.0 - best_s.0).abs() <= 0.01 && s.1 < best_s.1) {
                best_s = s;
                best_p = p;
                if s.0 <= 0.01 && s.1 <= 0.01 {
                    break;
                }
            }
        }
        // 若路径上全部候选点仍存在重叠（极密区域），做轻量上下微调以彻底错开
        if best_s.0 > 0.01 {
            let base = best_p;
            for dy in [-bh, bh, -bh * 1.8, bh * 1.8] {
                let p = crate::canvas::pt(base.x, base.y + dy);
                let s = score(p, &occupied);
                if s.0 < best_s.0 {
                    best_s = s;
                    best_p = p;
                    if s.0 <= 0.01 {
                        break;
                    }
                }
            }
        }
        lines[idx].label_pos = Some(best_p);
        occupied.push(rect(best_p.x - bw / 2., best_p.y - bh / 2., bw, bh));
    }
}

// ───────────────────────────── 流程图解析 ─────────────────────────────

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }
    fn rest(&self) -> String {
        self.s[self.i.min(self.s.len())..].iter().collect()
    }
    fn starts(&self, pat: &str) -> bool {
        let p: Vec<char> = pat.chars().collect();
        self.s.len() >= self.i + p.len() && self.s[self.i..self.i + p.len()] == p[..]
    }
    fn skip_ws(&mut self) {
        while self.peek(0).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }
}

struct NodeRef {
    id: String,
    shape: Option<(ShapeKind, String)>,
    class: Option<String>,
}

const OPENERS: &[(&str, &str, ShapeKind)] = &[
    ("(((", ")))", ShapeKind::DoubleCircle),
    ("((", "))", ShapeKind::Circle),
    ("([", "])", ShapeKind::Stadium),
    ("[[", "]]", ShapeKind::Subroutine),
    ("[(", ")]", ShapeKind::Cylinder),
    ("{{", "}}", ShapeKind::Hexagon),
    ("[/", "/]", ShapeKind::Parallelogram),
    ("[/", "\\]", ShapeKind::Trapezoid),
    ("[\\", "\\]", ShapeKind::ParallelogramAlt),
    ("[\\", "/]", ShapeKind::TrapezoidAlt),
    ("(", ")", ShapeKind::RoundRect),
    ("[", "]", ShapeKind::Rect),
    ("{", "}", ShapeKind::Diamond),
    (">", "]", ShapeKind::Flag),
];

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-' || c == '#' || (!c.is_ascii() && !c.is_whitespace())
}

fn parse_node(p: &mut Parser, line: usize) -> DResult<Option<NodeRef>> {
    p.skip_ws();
    let start = p.i;
    while let Some(c) = p.peek(0) {
        // 连字符：如果后面是边的写法（--、-.、->）就停止
        if c == '-' && matches!(p.peek(1), Some('-') | Some('.') | Some('>')) {
            break;
        }
        if c == '.' && p.peek(1) == Some('-') {
            break;
        }
        if c == '=' || c == '~' || c == '&' || c == ':' || c == ';' {
            break;
        }
        if !is_id_char(c) {
            break;
        }
        p.i += 1;
    }
    let id: String = p.s[start..p.i].iter().collect();
    if id.is_empty() {
        return Ok(None);
    }
    let mut shape = None;
    for (open, close, kind) in OPENERS {
        if p.starts(open) {
            // 找到匹配的结束符（文本可以用引号包住）
            let after = p.i + open.chars().count();
            let rest: String = p.s[after..].iter().collect();
            let mut text_end = None;
            if rest.trim_start().starts_with('"') {
                if let Some(q1) = rest.find('"') {
                    if let Some(q2) = rest[q1 + 1..].find('"') {
                        let tail = &rest[q1 + 1 + q2 + 1..];
                        if tail.trim_start().starts_with(close) {
                            let off = q1 + 1 + q2 + 1 + (tail.len() - tail.trim_start().len());
                            text_end = Some((rest[..q1 + 1 + q2 + 1].to_string(), off));
                        }
                    }
                }
            }
            if text_end.is_none() {
                if let Some(pos) = rest.find(close) {
                    text_end = Some((rest[..pos].to_string(), pos));
                }
            }
            if let Some((text, off)) = text_end {
                // 位置是字节偏移，转换成字符数
                let consumed_chars = rest[..off].chars().count() + close.chars().count();
                p.i = after + consumed_chars;
                shape = Some((*kind, clean_text(&text)));
                break;
            } else if !matches!(*open, "[/" | "[\\") {
                return Err(DiagramError::new(line, format!("节点 {id} 的「{open}」没有闭合")));
            }
        }
    }
    let mut class = None;
    if p.starts(":::") {
        p.i += 3;
        let s = p.i;
        while p.peek(0).is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            p.i += 1;
        }
        class = Some(p.s[s..p.i].iter().collect());
    }
    Ok(Some(NodeRef { id, shape, class }))
}

struct EdgeTok {
    start: Arrow,
    end: Arrow,
    dash: bool,
    thick: bool,
    invisible: bool,
    label: String,
    minlen: usize,
}

fn head(c: Option<char>) -> Option<Arrow> {
    match c {
        Some('>') => Some(Arrow::Filled),
        Some('x') => Some(Arrow::Cross),
        Some('o') => Some(Arrow::Circle),
        _ => None,
    }
}

fn parse_edge(p: &mut Parser) -> Option<EdgeTok> {
    p.skip_ws();
    let save = p.i;
    let mut start = Arrow::None;
    if matches!(p.peek(0), Some('<') | Some('x') | Some('o')) && matches!(p.peek(1), Some('-') | Some('=') | Some('.')) {
        start = head(Some(p.peek(0).unwrap())).unwrap_or(Arrow::Filled);
        if p.peek(0) == Some('<') {
            start = Arrow::Filled;
        }
        p.i += 1;
    }
    let mut tok = EdgeTok { start, end: Arrow::None, dash: false, thick: false, invisible: false, label: String::new(), minlen: 1 };
    match p.peek(0) {
        Some('~') if p.starts("~~~") => {
            while p.peek(0) == Some('~') {
                p.i += 1;
            }
            tok.invisible = true;
            return Some(tok);
        }
        Some('-') if p.peek(1) == Some('.') => {
            tok.dash = true;
            p.i += 1;
            let mut dots = 0;
            while p.peek(0) == Some('.') {
                p.i += 1;
                dots += 1;
            }
            // 「-. 文本 .->」
            if dots == 1 && p.peek(0) == Some(' ') {
                let rest = p.rest();
                for term in [".->", ".-"] {
                    if let Some(pos) = rest.find(term) {
                        tok.label = clean_text(&rest[..pos]);
                        p.i += rest[..pos].chars().count() + 1;
                        while p.peek(0) == Some('.') {
                            p.i += 1;
                        }
                        break;
                    }
                }
            }
            tok.minlen = dots.max(1);
            if p.peek(0) == Some('-') {
                p.i += 1;
            }
            if let Some(h) = head(p.peek(0)) {
                tok.end = h;
                p.i += 1;
            }
        }
        Some(c @ ('-' | '=')) => {
            let mut n: usize = 0;
            while p.peek(0) == Some(c) {
                p.i += 1;
                n += 1;
            }
            if n < 2 && head(p.peek(0)).is_none() {
                p.i = save;
                return None;
            }
            tok.thick = c == '=';
            // 「-- 文本 -->」「== 文本 ==>」
            if n == 2 && head(p.peek(0)).is_none() && p.peek(0) == Some(' ') {
                let rest = p.rest();
                let terms: &[&str] = if c == '-' { &["-->", "---", "--x", "--o"] } else { &["==>", "===", "==x", "==o"] };
                if let Some((pos, term)) = terms.iter().filter_map(|t| rest.find(t).map(|p| (p, *t))).min_by_key(|x| x.0) {
                    tok.label = clean_text(&rest[..pos]);
                    p.i += rest[..pos].chars().count();
                    let mut m = 0;
                    while p.peek(0) == Some(c) {
                        p.i += 1;
                        m += 1;
                    }
                    let _ = (term, m);
                }
            } else {
                tok.minlen = n.saturating_sub(1).max(1);
            }
            if let Some(h) = head(p.peek(0)) {
                // 'o'/'x' 作为箭头时后面不能紧跟标识符字符（否则是节点名）
                let is_head = p.peek(0) == Some('>') || !p.peek(1).is_some_and(|c| c.is_alphanumeric());
                if is_head {
                    tok.end = h;
                    p.i += 1;
                }
            }
        }
        _ => {
            p.i = save;
            return None;
        }
    }
    // 「-->|文本|」
    p.skip_ws();
    if p.peek(0) == Some('|') {
        let rest = p.rest();
        if let Some(pos) = rest[1..].find('|') {
            tok.label = clean_text(&rest[1..1 + pos]);
            p.i += rest[..pos + 2].chars().count();
        }
    }
    Some(tok)
}

pub fn render(header: &str, body: &[(usize, String)], theme: &Theme, m: &mut dyn Measure, offsets: &BTreeMap<String, (f32, f32)>) -> DResult<Scene> {
    let g = parse(header, body)?;
    Ok(g.to_scene(theme, m, offsets))
}

pub fn parse(header: &str, body: &[(usize, String)]) -> DResult<Graph> {
    let dir = header.split_whitespace().nth(1).and_then(Dir::parse).unwrap_or(Dir::TB);
    let mut g = Graph::new(dir);
    let mut class_defs: HashMap<String, Style> = HashMap::new();
    let mut node_styles: Vec<(String, Style)> = vec![];
    let mut link_styles: Vec<(Option<usize>, Style)> = vec![];
    let mut stack: Vec<usize> = vec![];
    for (line, raw) in body {
        let line = *line;
        for stmt in raw.split(';') {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            let lower = stmt.to_lowercase();
            let word = lower.split_whitespace().next().unwrap_or("");
            match word {
                "subgraph" => {
                    let rest = stmt[8..].trim();
                    let title = match rest.find('[') {
                        Some(b) => clean_text(rest[b + 1..].trim_end_matches(']')),
                        None => clean_text(rest),
                    };
                    g.clusters.push(GCluster { title, parent: stack.last().copied() });
                    stack.push(g.clusters.len() - 1);
                    continue;
                }
                "end" => {
                    if stack.pop().is_none() {
                        return Err(DiagramError::new(line, "多余的 end"));
                    }
                    continue;
                }
                "direction" => continue,
                "classdef" => {
                    let rest = stmt[8..].trim();
                    if let Some((names, style)) = rest.split_once(char::is_whitespace) {
                        for n in names.split(',') {
                            class_defs.insert(n.trim().to_string(), Style::parse(style));
                        }
                    }
                    continue;
                }
                "class" => {
                    let rest = stmt[5..].trim();
                    if let Some((ids, cls)) = rest.rsplit_once(char::is_whitespace) {
                        for id in ids.split(',') {
                            let i = g.node(id.trim(), stack.last().copied());
                            g.nodes[i].classes.push(cls.trim().to_string());
                        }
                    }
                    continue;
                }
                "style" => {
                    let rest = stmt[5..].trim();
                    if let Some((id, style)) = rest.split_once(char::is_whitespace) {
                        node_styles.push((id.trim().to_string(), Style::parse(style)));
                    }
                    continue;
                }
                "linkstyle" => {
                    let rest = stmt[9..].trim();
                    if let Some((ids, style)) = rest.split_once(char::is_whitespace) {
                        let st = Style::parse(style);
                        for id in ids.split(',') {
                            link_styles.push((id.trim().parse().ok(), st.clone()));
                        }
                    }
                    continue;
                }
                "click" | "accTitle" | "accDescr" => continue,
                _ => {}
            }
            // 链式语句：A & B --> C -->|x| D
            let chars: Vec<char> = stmt.chars().collect();
            let mut p = Parser { s: &chars, i: 0 };
            let mut prev: Vec<usize> = vec![];
            let mut pending: Option<EdgeTok> = None;
            loop {
                let mut group = vec![];
                while let Some(nr) = parse_node(&mut p, line)? {
                    let cl = stack.last().copied();
                    let idx = g.node(&nr.id, cl);
                    if let Some((shape, text)) = nr.shape {
                        g.nodes[idx].shape = shape;
                        g.nodes[idx].text = text;
                    }
                    if let Some(c) = nr.class {
                        g.nodes[idx].classes.push(c);
                    }
                    group.push(idx);
                    p.skip_ws();
                    if p.peek(0) == Some('&') {
                        p.i += 1;
                        continue;
                    }
                    break;
                }
                if group.is_empty() {
                    if pending.is_some() || !p.eof() {
                        return Err(DiagramError::new(line, format!("连线缺少目标节点：{}", p.rest())));
                    }
                    break;
                }
                if let Some(tok) = pending.take() {
                    for &a in &prev {
                        for &b in &group {
                            g.edges.push(GEdge {
                                from: a,
                                to: b,
                                label: tok.label.clone(),
                                start: tok.start,
                                end: tok.end,
                                dash: tok.dash,
                                thick: tok.thick,
                                invisible: tok.invisible,
                                minlen: tok.minlen,
                                ..Default::default()
                            });
                        }
                    }
                }
                p.skip_ws();
                if p.eof() {
                    break;
                }
                let Some(tok) = parse_edge(&mut p) else {
                    return Err(DiagramError::new(line, format!("无法解析连线：{}", p.rest())));
                };
                pending = Some(tok);
                prev = group;
            }
        }
    }
    if !stack.is_empty() {
        return Err(DiagramError::new(body.last().map(|b| b.0).unwrap_or(0), "subgraph 缺少 end"));
    }
    // 样式
    for n in g.nodes.iter_mut() {
        let mut st = class_defs.get("default").cloned().unwrap_or_default();
        for c in &n.classes {
            if let Some(cs) = class_defs.get(c) {
                st.merge(cs);
            }
        }
        n.style = st;
    }
    for (id, st) in node_styles {
        if let Some(&i) = g.index.get(&id) {
            g.nodes[i].style.merge(&st);
        }
    }
    for (ix, st) in link_styles {
        match ix {
            Some(i) => {
                if let Some(e) = g.edges.get_mut(i) {
                    e.color = st.stroke;
                    e.dash |= st.dash;
                }
            }
            None => g.edges.iter_mut().for_each(|e| e.color = st.stroke),
        }
    }
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Graph {
        let (lines, _) = preprocess(src);
        parse(&lines[0].1, &lines[1..]).unwrap_or_else(|e| panic!("line {}: {}", e.line, e.msg))
    }

    #[test]
    fn shapes_and_edges() {
        let g = p("flowchart LR\n A[方形] --> B(圆角)\n B -.-> C{判断}\n C ==>|粗| D((圆))\n C -- 文字 --> E[(库)]\n E --- F>旗]\n F --x G[[子程序]]\n G <--> H{{六边形}}\n H ~~~ I[/平行/]");
        assert_eq!(g.nodes.len(), 9);
        let s = |id: &str| g.nodes[g.index[id]].shape;
        assert_eq!(s("A"), ShapeKind::Rect);
        assert_eq!(s("B"), ShapeKind::RoundRect);
        assert_eq!(s("C"), ShapeKind::Diamond);
        assert_eq!(s("D"), ShapeKind::Circle);
        assert_eq!(s("E"), ShapeKind::Cylinder);
        assert_eq!(s("F"), ShapeKind::Flag);
        assert_eq!(s("G"), ShapeKind::Subroutine);
        assert_eq!(s("H"), ShapeKind::Hexagon);
        assert_eq!(s("I"), ShapeKind::Parallelogram);
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges[1].dash);
        assert!(g.edges[2].thick && g.edges[2].label == "粗");
        assert_eq!(g.edges[3].label, "文字");
        assert_eq!(g.edges[4].end, Arrow::None);
        assert_eq!(g.edges[5].end, Arrow::Cross);
        assert_eq!(g.edges[6].start, Arrow::Filled);
        assert!(g.edges[7].invisible);
    }

    #[test]
    fn chains_groups_subgraphs_styles() {
        let g = p("graph TD\n a & b --> c --> d\n subgraph one [第一组]\n  x --> y\n  subgraph two\n   z\n  end\n end\n classDef hot fill:#f96,stroke:#333\n class a hot\n y:::hot\n style d fill:#ff0");
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.clusters.len(), 2);
        assert_eq!(g.clusters[1].parent, Some(0));
        assert_eq!(g.nodes[g.index["x"]].cluster, Some(0));
        assert_eq!(g.nodes[g.index["z"]].cluster, Some(1));
        assert!(g.nodes[g.index["a"]].style.fill.is_some());
        assert!(g.nodes[g.index["y"]].style.fill.is_some());
        assert!(g.nodes[g.index["d"]].style.fill.is_some());
    }

    #[test]
    fn quoted_text_with_brackets() {
        let g = p("flowchart TD\n A[\"包含 [方括号] 的文本\"] --> B[\"第一行<br>第二行\"]");
        assert_eq!(g.nodes[0].text, "包含 [方括号] 的文本");
        assert_eq!(g.nodes[1].text, "第一行\n第二行");
    }

    #[test]
    fn complex_branching_with_back_edges_renders_cleanly() {
        let src = r#"flowchart TD
    U["用户输入 + 选择模式"] --> CFG["Runtime 加载模式配置"]
    CFG --> P["1. 注入对应 System Prompt"]
    CFG --> T["2. 裁剪暴露给模型的 Tool 列表"]
    P --> LLM["主模型推理 (如 SWE-2 Flash)"]
    T --> LLM

    LLM -->|"仅文本回复"| OUT["输出给用户"]
    LLM -->|"发起 Tool Call"| ORG{"第1关: 组织级策略<br/>Org Deny / Ask Rules"}

    ORG -->|"命中 Deny"| BLOCK["直接拒绝，返回错误给模型"]
    ORG -->|"命中强制 Ask"| HITL["挂起循环，弹窗等人工审批"]
    ORG -->|"未拦截"| MODE{"第2关: 当前模式路由"}

    MODE -->|"Ask / Plan"| RO{"是否为只读工具?"}
    RO -->|"是 (read/grep/search)"| EXEC["执行工具并回传结果"]
    RO -->|"否 (写文件/危险命令)"| BLOCK

    MODE -->|"Bypass Permissions"| EXEC

    MODE -->|"Code (标准模式)"| CODE_CHK{"工作区内只读?"}
    CODE_CHK -->|"是"| EXEC
    CODE_CHK -->|"否 (写文件 / Shell)"| HITL

    MODE -->|"Smart 模式"| SMART_WS{"工作区内常规编辑?"}
    SMART_WS -->|"是"| EXEC
    SMART_WS -->|"否 (Shell / MCP / 外部写)"| HARD{"命中硬黑名单?<br/>rm / sudo / 装包 / 写git"}
    HARD -->|"是"| HITL
    HARD -->|"否"| GUARD["调用轻量安全判别模型<br/>(Safety Classifier)"]
    GUARD -->|"判定明确安全 (如 test/lint/build)"| EXEC
    GUARD -->|"不确定 / 高风险 / 模型超时"| HITL

    HITL -->|"用户批准"| EXEC
    HITL -->|"用户拒绝"| BLOCK
    EXEC --> LLM
    BLOCK --> LLM"#;
        let scene = crate::diagram::render(src, false, None, &mut EstimateMeasure, &BTreeMap::new()).unwrap();
        let shapes: Vec<&ShapeItem> = scene
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Shape(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(shapes.len(), 16);
        // 任意两个节点不重叠
        for i in 0..shapes.len() {
            for j in i + 1..shapes.len() {
                assert!(!shapes[i].rect.intersects(&shapes[j].rect), "{} overlaps {}", shapes[i].id, shapes[j].id);
            }
        }
        // 菱形内宽足够容纳单行文字，不发生意外折行
        for s in &shapes {
            if s.kind == ShapeKind::Diamond {
                let area = crate::canvas::paint::text_area(s.kind, s.rect, s.align);
                let (tw, _) = text_size(&mut EstimateMeasure, &s.text, s.font_size, s.bold);
                assert!(area.w >= tw, "diamond {} text_area.w={} < tw={}", s.id, area.w, tw);
            }
        }
        // 边标签互不重叠
        let label_boxes: Vec<(String, crate::canvas::Rect)> = scene
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Line(l) if !l.label.is_empty() => {
                    let p = l.label_pos.expect("labeled edge should have label_pos");
                    let (lw, lh) = text_size(&mut EstimateMeasure, &l.label, l.font_size, false);
                    Some((l.label.clone(), rect(p.x - lw / 2., p.y - lh / 2., lw, lh)))
                }
                _ => None,
            })
            .collect();
        assert!(label_boxes.len() >= 15);
        for i in 0..label_boxes.len() {
            for j in i + 1..label_boxes.len() {
                assert!(!label_boxes[i].1.intersects(&label_boxes[j].1), "label '{}' overlaps '{}'", label_boxes[i].0, label_boxes[j].0);
            }
        }
        if let Ok(path) = std::env::var("STONEBROOK_DUMP_PREVIEW_PNG") {
            let mut s = scene;
            s.bg = Some(Col::rgb(0xffffff));
            let svg = crate::canvas::svg::to_svg(&s);
            let png = crate::canvas::svg::svg_to_png(&svg, 2.).expect("png");
            std::fs::write(path, png).expect("write png");
        }
    }
}
