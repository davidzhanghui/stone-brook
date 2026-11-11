//! 画板的鼠标交互：选择、拖动（对齐吸附 / Shift 锁轴 / ⌥ 复制）、缩放手柄、绘制新元素、
//! 快捷创建（图形四周的蓝色圆点）、连线吸附到四边锚点、分区连带移动、右键平移与右键菜单。

use super::model::*;
use super::view::*;
use crate::canvas::paint::View;
use crate::canvas::{pt, rect, Arrow, Col, Pt, Rect, ShapeKind};
use crate::theme::Theme;
use gpui::*;
use std::collections::HashSet;

/// 叠加层（选择框、手柄、锚点、参考线……）所需的数据，在渲染时一次性算好，绘制阶段只读。
pub(super) struct Overlay {
    sel_rects: Vec<Rect>,
    /// 缩放手柄：(位置, 是否画成圆点)
    handles: Vec<(Pt, bool)>,
    /// 激活条预览（悬停生命线虚线 / 消息线将落在生命线上）
    ghost_bar: Option<Rect>,
    /// 激活条边缘的消息线锚点
    port: Option<Pt>,
    conn_ends: Vec<Pt>,
    /// 选中连线上的调整手柄：(位置, 种类, 是否悬停中)
    bends: Vec<(Pt, BendAt, bool)>,
    hover: Option<Rect>,
    /// 连线可吸附的锚点：(位置, 是否为当前吸附点)
    anchors: Vec<(Pt, bool)>,
    marquee: Option<Rect>,
    guides: Vec<(Pt, Pt)>,
    dots: Vec<(Pt, bool)>,
    ghost: Option<(Rect, Pt, Pt)>,
    size_tag: Option<(Pt, String)>,
    /// 悬停在空图形上的「输入文本」提示：(图形区域, 字号)
    hint: Option<(Rect, f32)>,
    /// 选中表格时上边 / 左边的细轨道（世界坐标）
    rails: Vec<Rect>,
    /// 轨道上各边界的圆点：(位置, 是否插入行, 是否悬停中)
    plus: Vec<(Pt, bool, bool)>,
    /// 悬停「+」对应的边界线（高亮）
    hot_line: Option<(Pt, Pt)>,
    /// 思维导图：选中节点的「+」/ 收起数量徽标（位置, 数量；0 表示「+」）
    mind_btns: Vec<(Pt, usize)>,
    /// 选中的单元格
    cell: Option<Rect>,
    /// 搜索结果：(区域, 是否当前)
    found: Vec<(Rect, bool)>,
    accent: Hsla,
    rail: Hsla,
    rail_dot: Hsla,
    guide: Hsla,
    panel: Hsla,
    text: Hsla,
}

impl Overlay {
    pub(super) fn paint(&self, view: View, _bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let accent = self.accent;
        for r in &self.sel_rects {
            // 选框紧贴图形边缘，不再在外面多套一圈
            window.paint_quad(outline(view.r(*r), accent, BorderStyle::default()));
        }
        if let Some(r) = self.hover {
            window.paint_quad(outline(view.r(r).dilate(px(2.)), accent.opacity(0.45), BorderStyle::default()));
        }
        for (h, round) in &self.handles {
            let c = view.p(*h);
            if *round {
                window.paint_quad(
                    fill(Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.))), gpui::white())
                        .corner_radii(px(5.))
                        .border_widths(px(1.5))
                        .border_color(accent),
                );
            } else {
                window.paint_quad(
                    fill(Bounds::new(point(c.x - px(4.5), c.y - px(4.5)), size(px(9.), px(9.))), gpui::white())
                        .corner_radii(px(2.))
                        .border_widths(px(1.5))
                        .border_color(accent),
                );
            }
        }
        if let Some(r) = self.ghost_bar {
            window.paint_quad(fill(view.r(r), accent.opacity(0.14)).corner_radii(px(2. * view.scale)));
        }
        // 激活条边缘锚点：白底蓝圈 +「+」
        if let Some(p) = self.port {
            let c = view.p(p);
            window.paint_quad(
                fill(Bounds::new(point(c.x - px(6.), c.y - px(6.)), size(px(12.), px(12.))), gpui::white())
                    .corner_radii(px(6.))
                    .border_widths(px(1.5))
                    .border_color(accent),
            );
            window.paint_quad(fill(Bounds::new(point(c.x - px(3.), c.y - px(0.6)), size(px(6.), px(1.2))), accent));
            window.paint_quad(fill(Bounds::new(point(c.x - px(0.6), c.y - px(3.)), size(px(1.2), px(6.))), accent));
        }
        // 连线调整手柄：折线段上是顺着线段的蓝色短条，直线 / 曲线上是白底蓝圈的小圆点
        for (p, at, hot) in &self.bends {
            let c = view.p(*p);
            let k = if *hot { 1.25 } else { 1. };
            match at {
                BendAt::Seg(_, vertical) => {
                    let (w, h) = if *vertical { (5. * k, 14. * k) } else { (14. * k, 5. * k) };
                    window.paint_quad(
                        fill(Bounds::new(point(c.x - px(w / 2. + 1.), c.y - px(h / 2. + 1.)), size(px(w + 2.), px(h + 2.))), gpui::white())
                            .corner_radii(px(w.min(h) / 2. + 1.)),
                    );
                    window.paint_quad(fill(Bounds::new(point(c.x - px(w / 2.), c.y - px(h / 2.)), size(px(w), px(h))), accent).corner_radii(px(w.min(h) / 2.)));
                }
                BendAt::Mid(_) | BendAt::Point(_) => {
                    let d = if matches!(at, BendAt::Point(_)) { 9. * k } else { 7. * k };
                    let bg = if matches!(at, BendAt::Point(_)) { gpui::white() } else { accent };
                    window.paint_quad(
                        fill(Bounds::new(point(c.x - px(d / 2.), c.y - px(d / 2.)), size(px(d), px(d))), bg)
                            .corner_radii(px(d / 2.))
                            .border_widths(px(1.5))
                            .border_color(if matches!(at, BendAt::Point(_)) { accent } else { gpui::white() }),
                    );
                }
            }
        }
        for e in &self.conn_ends {
            let c = view.p(*e);
            window.paint_quad(
                fill(Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.))), gpui::white())
                    .corner_radii(px(5.))
                    .border_widths(px(2.))
                    .border_color(accent),
            );
        }
        // 快捷创建：预览图形（虚线框）+ 预览连线
        if let Some((r, a, b)) = self.ghost {
            let (pa, pb) = (view.p(a), view.p(b));
            let horizontal = (pa.y - pb.y).abs() < px(1.);
            let line = if horizontal {
                Bounds::new(point(pa.x.min(pb.x), pa.y - px(0.75)), size((pb.x - pa.x).abs(), px(1.5)))
            } else {
                Bounds::new(point(pa.x - px(0.75), pa.y.min(pb.y)), size(px(1.5), (pb.y - pa.y).abs()))
            };
            window.paint_quad(fill(line, accent.opacity(0.6)));
            window.paint_quad(fill(view.r(r), accent.opacity(0.06)).corner_radii(px(8. * view.scale)).border_widths(px(1.5)).border_color(accent.opacity(0.6)));
        }
        for (p, hot) in &self.dots {
            let c = view.p(*p);
            let d = if *hot { 18. } else { 10. };
            window.paint_quad(
                fill(Bounds::new(point(c.x - px(d / 2.), c.y - px(d / 2.)), size(px(d), px(d))), if *hot { accent } else { accent.opacity(0.35) })
                    .corner_radii(px(d / 2.)),
            );
            if *hot {
                // 「+」
                window.paint_quad(fill(Bounds::new(point(c.x - px(4.), c.y - px(0.75)), size(px(8.), px(1.5))), gpui::white()));
                window.paint_quad(fill(Bounds::new(point(c.x - px(0.75), c.y - px(4.)), size(px(1.5), px(8.))), gpui::white()));
            }
        }
        for (p, hot) in &self.anchors {
            let c = view.p(*p);
            let d = if *hot { 12. } else { 8. };
            window.paint_quad(
                fill(Bounds::new(point(c.x - px(d / 2.), c.y - px(d / 2.)), size(px(d), px(d))), if *hot { accent } else { gpui::white() })
                    .corner_radii(px(d / 2.))
                    .border_widths(px(1.5))
                    .border_color(accent),
            );
        }
        if let Some(m) = self.marquee {
            window.paint_quad(fill(view.r(m), accent.opacity(0.08)).border_widths(px(1.)).border_color(accent));
        }
        // 对齐参考线：蓝色细虚线，只连在对齐的图形之间
        for (a, b) in &self.guides {
            let (pa, pb) = (view.p(*a), view.p(*b));
            let vertical = (pa.x - pb.x).abs() < px(0.5);
            let len = if vertical { pb.y - pa.y } else { pb.x - pa.x };
            let mut t = px(0.);
            while t < len {
                let l = px(4.).min(len - t);
                let d = if vertical {
                    Bounds::new(point(pa.x - px(0.5), pa.y + t), size(px(1.), l))
                } else {
                    Bounds::new(point(pa.x + t, pa.y - px(0.5)), size(l, px(1.)))
                };
                window.paint_quad(fill(d, self.guide));
                t += px(7.);
            }
        }
        if let Some(c) = self.cell {
            window.paint_quad(outline(view.r(c), accent, BorderStyle::default()).border_widths(px(2.)));
        }
        for (r, cur) in &self.found {
            let b = view.r(*r).dilate(px(3.));
            let (bg, line) = if *cur { (gpui::rgba(0xff880033), gpui::rgb(0xff8800)) } else { (gpui::rgba(0xffc60a33), gpui::rgba(0xffc60a99)) };
            window.paint_quad(fill(b, bg).corner_radii(px(3.)).border_widths(px(if *cur { 2. } else { 1. })).border_color(line));
        }
        // 表格：上 / 左两条细轨道 + 边界小圆点；悬停的圆点变成蓝色「+」、对应边界线高亮，并提示插入行 / 列
        for r in &self.rails {
            window.paint_quad(fill(view.r(*r), self.rail).corner_radii(px(2.)));
        }
        if let Some((a, b)) = self.hot_line {
            let (pa, pb) = (view.p(a), view.p(b));
            let line = if (pa.x - pb.x).abs() < px(0.5) {
                Bounds::new(point(pa.x - px(1.), pa.y), size(px(2.), pb.y - pa.y))
            } else {
                Bounds::new(point(pa.x, pa.y - px(1.)), size(pb.x - pa.x, px(2.)))
            };
            window.paint_quad(fill(line, accent));
        }
        for (p, row, hot) in &self.plus {
            let c = view.p(*p);
            if !*hot {
                window.paint_quad(fill(Bounds::new(point(c.x - px(2.), c.y - px(2.)), size(px(4.), px(4.))), self.rail_dot).corner_radii(px(2.)));
                continue;
            }
            window.paint_quad(fill(Bounds::new(point(c.x - px(8.), c.y - px(8.)), size(px(16.), px(16.))), accent).corner_radii(px(8.)));
            window.paint_quad(fill(Bounds::new(point(c.x - px(4.), c.y - px(0.75)), size(px(8.), px(1.5))), gpui::white()));
            window.paint_quad(fill(Bounds::new(point(c.x - px(0.75), c.y - px(4.)), size(px(1.5), px(8.))), gpui::white()));
            // 深色小提示
            let s: SharedString = if *row { "插入行" } else { "插入列" }.into();
            let run =
                TextRun { len: s.len(), font: gpui::font("PingFang SC"), color: gpui::white(), background_color: None, underline: None, strikethrough: None };
            let line = window.text_system().shape_line(s, px(11.), &[run], None);
            let w = line.width + px(14.);
            let b = Bounds::new(point(c.x - w / 2., c.y - px(36.)), size(w, px(22.)));
            window.paint_quad(fill(b, gpui::rgb(0x1f2329)).corner_radii(px(5.)));
            line.paint(point(b.origin.x + px(7.), b.origin.y + px(4.)), px(14.), window, cx).ok();
        }
        for (p, n) in &self.mind_btns {
            let c = view.p(*p);
            if *n == 0 {
                window.paint_quad(fill(Bounds::new(point(c.x - px(8.), c.y - px(8.)), size(px(16.), px(16.))), accent).corner_radii(px(8.)));
                window.paint_quad(fill(Bounds::new(point(c.x - px(4.), c.y - px(0.75)), size(px(8.), px(1.5))), gpui::white()));
                window.paint_quad(fill(Bounds::new(point(c.x - px(0.75), c.y - px(4.)), size(px(1.5), px(8.))), gpui::white()));
            } else {
                // 收起后的数量徽标
                let s: SharedString = n.to_string().into();
                let run =
                    TextRun { len: s.len(), font: gpui::font("PingFang SC"), color: accent, background_color: None, underline: None, strikethrough: None };
                let line = window.text_system().shape_line(s, px(11.), &[run], None);
                let w = (line.width + px(10.)).max(px(18.));
                let b = Bounds::new(point(c.x - w / 2., c.y - px(9.)), size(w, px(18.)));
                window.paint_quad(fill(b, gpui::white()).corner_radii(px(9.)).border_widths(px(1.2)).border_color(accent));
                line.paint(point(c.x - line.width / 2., c.y - px(7.)), px(14.), window, cx).ok();
            }
        }
        if let Some((r, font)) = self.hint {
            let s: SharedString = "输入文本".into();
            let fs = (font * view.scale).clamp(9., 40.);
            let run = TextRun {
                len: s.len(),
                font: gpui::font("PingFang SC"),
                color: gpui::rgb(0x8f959e).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(s, px(fs), &[run], None);
            let b = view.r(r);
            line.paint(point(b.origin.x + (b.size.width - line.width) / 2., b.origin.y + (b.size.height - px(fs * 1.38)) / 2.), px(fs * 1.38), window, cx).ok();
        }
        if let Some((p, text)) = &self.size_tag {
            let c = view.p(*p);
            let s: SharedString = text.clone().into();
            let run = TextRun { len: s.len(), font: gpui::font("PingFang SC"), color: self.text, background_color: None, underline: None, strikethrough: None };
            let line = window.text_system().shape_line(s, px(11.), &[run], None);
            let w = line.width + px(12.);
            let b = Bounds::new(point(c.x - w / 2., c.y + px(8.)), size(w, px(20.)));
            window.paint_quad(fill(b, self.panel).corner_radii(px(4.)));
            line.paint(point(b.origin.x + px(6.), b.origin.y + px(3.)), px(14.), window, cx).ok();
        }
    }
}

impl BoardEditor {
    pub(super) fn overlay_info(&self, t: &Theme) -> Overlay {
        let editing_id = self.editing.as_ref().map(|e| e.id);
        let lk = self.doc.lookup();
        let sel_rects: Vec<Rect> = self
            .doc
            .els
            .iter()
            .filter(|e| self.sel.contains(&e.id) && Some(e.id) != editing_id)
            .filter(|e| !(self.sel.len() == 1 && e.is_connector()))
            .map(|e| if super::seq::is_life(e) { super::seq::head_rect(e) } else { BoardDoc::bounds_in(&lk, e) })
            .collect();
        let handles = if matches!(self.drag, Some(Drag::Move { moved: true, .. })) {
            vec![]
        } else {
            self.single_resizable().map(|e| super::seq::handle_pts(e).into_iter().map(|(_, p, round)| (p, round)).collect()).unwrap_or_default()
        };
        let conn_ends: Vec<Pt> = self
            .single()
            .filter(|e| e.is_connector())
            .map(|e| {
                let p = self.doc.connector_points(e);
                vec![p[0], *p.last().unwrap()]
            })
            .unwrap_or_default();
        let hover = self
            .mind_target
            .or(self.hover.filter(|_| self.drag.is_none() && self.bar_ghost.is_none() && self.bar_port.is_none()))
            .and_then(|h| self.doc.get(h))
            .filter(|e| (!self.sel.contains(&e.id) || self.mind_target == Some(e.id)) && !e.is_container())
            .map(|e| if super::seq::is_life(e) { super::seq::head_rect(e) } else { self.doc.bounds_of(e) });
        // 连线吸附锚点：拖连线时目标图形的四边中点；连线工具下悬停的图形也显示
        let connecting = matches!(self.drag, Some(Drag::Connect { .. }) | Some(Drag::EndPoint { .. }) | Some(Drag::QuickDot { .. }))
            || matches!(self.tool, Tool::Connector(..));
        let mut anchors = if connecting {
            let target = self.snap_anchor.or(self.hover.filter(|_| matches!(self.tool, Tool::Connector(..))).map(|h| (h, None)));
            target
                .and_then(|(id, side)| self.doc.get(id).filter(|e| e.is_box()).map(|e| (e.rect(), side)))
                .map(|(r, side)| {
                    Side::ALL
                        .iter()
                        .map(|s| {
                            let (ax, ay) = s.anchor();
                            (pt(r.x + ax * r.w, r.y + ay * r.h), side == Some(*s))
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        // 消息线吸附到激活条上：落点高亮
        if let Some(super::seq::MsgSnap::Bar { id, ax, ay }) = self.msg_snap.filter(|_| connecting) {
            if let Some(e) = self.doc.get(id) {
                anchors.push((pt(e.x + ax * e.w, e.y + ay * e.h), true));
            }
        }
        let marquee = match &self.drag {
            Some(Drag::Marquee { start, cur }) => Some(Rect::from_points(*start, *cur)),
            _ => None,
        };
        let dots: Vec<(Pt, bool)> = self.quick_dots().into_iter().map(|(s, p)| (p, self.hover_dot == Some(s))).collect();
        let ghost = self.hover_dot.and_then(|side| {
            // 思维导图节点的圆点只用来拖出连线，不预览快捷创建
            let e = self.single().filter(|e| !e.is_mind())?;
            let r = quick_target(e.rect(), side);
            let (ax, ay) = side.anchor();
            let a = pt(e.x + ax * e.w, e.y + ay * e.h);
            let (bx, by) = opposite(side).anchor();
            Some((r, a, pt(r.x + bx * r.w, r.y + by * r.h)))
        });
        let resizing = matches!(self.drag, Some(Drag::Resize { .. }));
        let size_tag = (resizing || prefs().show_size)
            .then(|| self.single_resizable())
            .flatten()
            .filter(|_| self.editing.is_none())
            .map(|e| (pt(e.x + e.w / 2., e.y + e.h + 4. / self.zoom), format!("{:.0} × {:.0}", e.w, e.h)));
        let hint = self
            .hover
            .filter(|_| self.drag.is_none() && self.editing.is_none() && self.tool == Tool::Select && self.painting.is_none())
            .and_then(|h| self.doc.get(h))
            .filter(|e| matches!(e.kind, ElKind::Shape(k) if k != ShapeKind::Activation) | matches!(e.kind, ElKind::Sticky))
            .filter(|e| e.text.is_empty() && !e.locked && !e.is_table() && !e.is_mind() && !super::seq::is_life(e))
            .map(|e| (self.doc.bounds_of(e), e.font_size));
        let cell = self.cur_cell().and_then(|(id, r, c)| {
            let e = self.doc.get(id)?;
            let g = e.grid()?;
            Some(g.cell(r * g.ncols() + c))
        });
        let mut mind_btns: Vec<(Pt, usize)> = self.mind_badges().into_iter().map(|(_, p, n)| (p, n)).collect();
        mind_btns.extend(self.mind_buttons().into_iter().map(|(_, p, _)| (p, 0)));
        let (rails, hot_line) = self.table_rails();
        Overlay {
            rails,
            hot_line,
            plus: self.table_plus().into_iter().map(|(row, k, q)| (q, row, self.table_hot == Some((row, k)))).collect(),
            mind_btns,
            cell: if self.editing.is_some() { None } else { cell },
            found: self.find_rects(),
            sel_rects,
            handles,
            ghost_bar: self.ghost_bar(),
            port: self.bar_port.filter(|_| self.drag.is_none()).and_then(|(id, ax, ay)| self.doc.get(id).map(|e| pt(e.x + ax * e.w, e.y + ay * e.h))),
            conn_ends,
            bends: if self.drag.is_none() || matches!(self.drag, Some(Drag::Bend { .. })) {
                self.bend_handles().into_iter().map(|(p, at)| (p, at, self.hover_bend == Some(at))).collect()
            } else {
                vec![]
            },
            hover,
            anchors,
            marquee,
            guides: self.guides.clone(),
            dots,
            ghost,
            size_tag,
            hint,
            accent: t.accent,
            rail: if t.dark { gpui::hsla(0., 0., 1., 0.1) } else { gpui::hsla(0., 0., 0., 0.06) },
            rail_dot: if t.dark { gpui::hsla(0., 0., 1., 0.35) } else { gpui::hsla(0., 0., 0., 0.28) },
            guide: t.accent,
            panel: if t.dark { gpui::hsla(0., 0., 0.95, 1.) } else { gpui::rgb(0x1f2329).into() },
            text: if t.dark { gpui::rgb(0x1f2329).into() } else { gpui::white() },
        }
    }

    /// 选中连线的调整手柄：折线每段中间一个，拖动整段平移；直线 / 曲线在各段中点（拖出新拐点）和已有拐点上。
    /// 太短（屏幕上不到 28px）的段不显示；时序图消息线不显示。
    pub(super) fn bend_handles(&self) -> Vec<(Pt, BendAt)> {
        let Some(el) = self.single().filter(|e| e.is_connector() && !e.locked) else { return vec![] };
        if self.msg_end(el.id, 0).is_some() || self.msg_end(el.id, 1).is_some() {
            return vec![];
        }
        let ElKind::Connector { route, .. } = &el.kind else { return vec![] };
        let s = self.doc.connector_skeleton(el);
        let n = s.len();
        let long = |a: Pt, b: Pt| a.dist(b) * self.zoom >= 28.;
        let mut out = vec![];
        if route.is_elbow() {
            for k in 0..n.saturating_sub(1) {
                let (a, b) = (s[k], s[k + 1]);
                if long(a, b) {
                    out.push((a.lerp(b, 0.5), BendAt::Seg(k, (a.x - b.x).abs() < 0.5)));
                }
            }
            return out;
        }
        let drawn = self.doc.connector_points(el);
        for k in 0..n.saturating_sub(1) {
            if !long(s[k], s[k + 1]) {
                continue;
            }
            // 曲线：手柄放在这一段曲线的中间（每段采样 12 个点；没有拐点时整条 24 个点）
            let q = match route {
                Route::Curve if drawn.len() == 1 + 12 * (n - 1) => drawn[k * 12 + 6],
                Route::Curve if n == 2 && drawn.len() == 25 => drawn[12],
                _ => s[k].lerp(s[k + 1], 0.5),
            };
            out.push((q, BendAt::Mid(k)));
        }
        for i in 1..n.saturating_sub(1) {
            out.push((s[i], BendAt::Point(i)));
        }
        out
    }

    pub(super) fn bend_at(&self, screen: Point<Pixels>) -> Option<(Pt, BendAt)> {
        let v = self.view();
        self.bend_handles()
            .into_iter()
            .filter(|(q, _)| dist_px(v.p(*q), screen) < 9.)
            .min_by(|a, b| dist_px(v.p(a.0), screen).total_cmp(&dist_px(v.p(b.0), screen)))
    }

    /// 拖动连线调整手柄时的新走线骨架。
    fn bend_route(&self, orig: &[Pt], idx: usize, seg: bool, d: Pt) -> Vec<Pt> {
        if !seg {
            let mut v = orig.to_vec();
            v[idx] = v[idx].add(d);
            return v;
        }
        // 首段 / 末段：先在端头留一小段，再把其余部分整体挪开（端点仍贴在元素上）
        let mut v = orig.to_vec();
        let mut k = idx;
        let n = v.len();
        let stub = |a: Pt, b: Pt| {
            let len = a.dist(b);
            a.add(b.sub(a).scale(20f32.min(len / 3.) / len.max(1e-3)))
        };
        if k == n - 2 {
            let e = stub(v[n - 1], v[n - 2]);
            v.splice(n - 1..n - 1, [e, e]);
        }
        if k == 0 {
            let s0 = stub(v[0], v[1]);
            v.splice(1..1, [s0, s0]);
            k += 2;
        }
        let vertical = (v[k].x - v[k + 1].x).abs() < 0.5;
        // 吸附：挪到与两端点对齐（拉直）时贴上去
        let tol = 6. / self.zoom;
        let (first, last) = (v[0], v[v.len() - 1]);
        if vertical {
            let mut x = v[k].x + d.x;
            if let Some(t) = [orig[idx].x, first.x, last.x].into_iter().find(|t| (x - t).abs() < tol) {
                x = t;
            }
            v[k].x = x;
            v[k + 1].x = x;
        } else {
            let mut y = v[k].y + d.y;
            if let Some(t) = [orig[idx].y, first.y, last.y].into_iter().find(|t| (y - t).abs() < tol) {
                y = t;
            }
            v[k].y = y;
            v[k + 1].y = y;
        }
        v
    }

    /// 选中图形的缩放手柄：四角 / 边中点的手柄，以及整条边（边上任意位置都能拖）。
    pub(super) fn resize_handle_at(&self, screen: Point<Pixels>) -> Option<usize> {
        let el = self.single_resizable()?;
        let v = self.view();
        if let Some((i, _, _)) = super::seq::handle_pts(el).into_iter().find(|(_, h, _)| dist_px(v.p(*h), screen) < 8.) {
            return Some(i);
        }
        if super::seq::is_life(el) || super::seq::is_bar(el) {
            return None;
        }
        let (p, r, tol) = (self.to_world(screen), el.rect(), 5. / self.zoom);
        let (in_x, in_y) = (p.x >= r.x && p.x <= r.right(), p.y >= r.y && p.y <= r.bottom());
        if in_x && (p.y - r.y).abs() <= tol {
            Some(1)
        } else if in_x && (p.y - r.bottom()).abs() <= tol {
            Some(5)
        } else if in_y && (p.x - r.right()).abs() <= tol {
            Some(3)
        } else if in_y && (p.x - r.x).abs() <= tol {
            Some(7)
        } else {
            None
        }
    }

    pub(super) fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        match tool {
            Tool::Shape(k) => self.last_shape = k,
            Tool::Connector(r, a) => self.last_conn = (r, a),
            _ => {}
        }
        self.tool = tool;
        self.flyout = None;
        self.popover = None;
        self.conn_preset = None;
        self.painting = None;
        if !matches!(tool, Tool::Select | Tool::Hand) {
            self.sel.clear();
            self.cell_sel = None;
        }
        cx.notify();
    }

    // ───────────── 鼠标按下 ─────────────

    pub(super) fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.close_overlays();
        self.finish_edit(cx);
        let p = self.to_world(ev.position);
        let dark = Self::dark(cx);
        if self.space_down || self.tool == Tool::Hand || ev.button == MouseButton::Middle {
            self.drag = Some(Drag::Pan { start: ev.position, orig: self.offset });
            cx.notify();
            return;
        }
        // 格式刷：单击元素套用样式，单击空白处退出
        if self.painting.is_some() && self.tool == Tool::Select {
            match self.hit(p) {
                Some(id) => self.paint_style_on(id, cx),
                None => self.painting = None,
            }
            cx.notify();
            return;
        }
        match self.tool {
            Tool::Select => self.select_down(ev, p, window, cx),
            Tool::Lane(kind) => {
                self.snapshot();
                let el = self.doc.new_lane(kind, p, dark);
                let id = el.id;
                // 泳道是容器，放在最底层
                self.doc.els.insert(0, el);
                self.sel = [id].into();
                self.tool = Tool::Select;
                self.commit(cx);
            }
            Tool::Mind(layout) => self.insert_mind(p, layout, window, cx),
            Tool::Shape(_) | Tool::Text | Tool::Sticky | Tool::Section(..) => {
                self.snapshot();
                let kind = match self.tool {
                    Tool::Shape(k) => ElKind::Shape(k),
                    Tool::Text => ElKind::Text,
                    Tool::Section(..) => ElKind::Section,
                    _ => ElKind::Sticky,
                };
                let mut el = self.doc.new_el(kind, rect(p.x, p.y, 0., 0.), dark);
                self.style_new(&mut el);
                let id = el.id;
                if el.is_section() {
                    // 分区永远在最底层
                    self.doc.els.insert(0, el);
                } else {
                    self.doc.els.push(el);
                }
                self.sel = [id].into();
                if self.tool == Tool::Text {
                    if let Some(i) = self.doc.index(id) {
                        let e = &mut self.doc.els[i];
                        e.w = 120.;
                        e.h = 32.;
                        e.y -= 16.;
                    }
                    self.tool = Tool::Select;
                    self.start_edit(id, window, cx);
                } else {
                    self.drag = Some(Drag::Create { start: p, id });
                }
            }
            Tool::Connector(route, arrow) => {
                self.snapshot();
                let a = match self.snap_target(p, 0) {
                    Some((t, Some(side))) => {
                        let (ax, ay) = side.anchor();
                        End::Bound { el: t, ax, ay }
                    }
                    Some((t, None)) => End::Bound { el: t, ax: 0.5, ay: 0.5 },
                    None => End::Free { x: p.x, y: p.y },
                };
                let (start, end, dash) = self.conn_preset.unwrap_or((Arrow::None, if arrow { Arrow::Filled } else { Arrow::None }, false));
                let mut el = self.doc.new_el(ElKind::Connector { a, b: End::Free { x: p.x, y: p.y }, route, start, end }, Rect::default(), dark);
                el.dash = dash;
                let id = el.id;
                self.doc.els.push(el);
                self.drag = Some(Drag::Connect { id });
            }
            Tool::Pen => {
                self.snapshot();
                let mut el = self.doc.new_el(ElKind::Pen { pts: vec![[p.x, p.y, 0.5]] }, Rect::default(), dark);
                let palette = super::ui::PEN_COLORS;
                el.stroke = Col::rgb(palette[self.pen.color.min(palette.len() - 1)]);
                el.stroke_w = [1.4, 2.4, 4.][self.pen.width.min(2)];
                el.highlighter = self.pen.highlighter;
                let id = el.id;
                self.doc.els.push(el);
                self.drag = Some(Drag::Pen { id, last: p });
            }
            Tool::Eraser => {
                self.snapshot();
                self.drag = Some(Drag::Erase);
                self.erase_at(p);
            }
            Tool::Hand => {}
        }
        cx.notify();
    }

    /// 新建元素的默认样式（便签颜色 / 作者、文本字号……）。
    pub(super) fn style_new(&self, el: &mut El) {
        match el.kind {
            ElKind::Sticky => {
                el.fill = Some(Col::rgb(STICKY_COLORS[self.sticky_color.min(STICKY_COLORS.len() - 1)]));
                el.author = crate::settings::author();
                el.auto_font = true;
                el.font_size = 18.;
            }
            ElKind::Text => {
                el.font_size = 16.;
            }
            ElKind::Section => {
                el.text = "分区".into();
            }
            ElKind::Shape(_) => {
                el.font_size = 14.;
            }
            _ => {}
        }
    }

    fn select_down(&mut self, ev: &MouseDownEvent, p: Pt, window: &mut Window, cx: &mut Context<Self>) {
        self.click_edit = None;
        let v = self.view();
        // 0. 表格边界的「+」、思维导图节点旁的「+」/ 收起徽标
        for (row, k, q) in self.table_plus() {
            if dist_px(v.p(q), ev.position) < 10. {
                if let Some(id) = self.single_table().map(|e| e.id) {
                    self.table_insert_at(id, row, k, cx);
                }
                return;
            }
        }
        for (id, q, side) in self.mind_buttons() {
            if dist_px(v.p(q), ev.position) < 10. {
                self.mind_add_child(id, side, window, cx);
                return;
            }
        }
        for (id, q, _) in self.mind_badges() {
            if dist_px(v.p(q), ev.position) < 11. {
                self.sel = [id].into();
                self.mind_toggle_collapse(cx);
                return;
            }
        }
        // 0.5 表格 / 泳道的网格线：拖动调整行高列宽
        if let Some((id, vertical, k)) = self.grid_line_at(p) {
            if let Some(ElKind::Table { cols, rows, .. }) = self.doc.get(id).map(|e| &e.kind) {
                let orig = if vertical { cols[k - 1] } else { rows[k - 1] };
                self.snapshot();
                self.sel = [id].into();
                self.drag = Some(Drag::GridLine { id, vertical, k, orig, start: p });
                return;
            }
        }
        // 1. 快捷创建圆点
        if let Some(from) = self.single().map(|e| e.id) {
            for (side, q) in self.quick_dots() {
                if dist_px(v.p(q), ev.position) < 11. {
                    self.drag = Some(Drag::QuickDot { from, side, start: ev.position });
                    return;
                }
            }
        }
        // 1.5 激活条边缘锚点：拖出消息线
        if let Some((from, ax, ay)) = self.bar_port_at(p).filter(|_| !ev.modifiers.shift && ev.click_count < 2) {
            self.drag = Some(Drag::Port { from, ax, ay, start: ev.position });
            return;
        }
        // 2. 选中连线的两个端点
        if let Some(el) = self.single().filter(|e| e.is_connector() && !e.locked) {
            let id = el.id;
            let pts = self.doc.connector_points(el);
            for (which, q) in [(0usize, pts[0]), (1, *pts.last().unwrap())] {
                if dist_px(v.p(q), ev.position) < 9. {
                    self.snapshot();
                    self.drag = Some(Drag::EndPoint { id, which });
                    return;
                }
            }
        }
        // 2.5 选中连线上的调整手柄
        if let Some((q, at)) = self.bend_at(ev.position).filter(|_| ev.click_count < 2) {
            if let Some(el) = self.single() {
                let (id, mut orig) = (el.id, self.doc.connector_skeleton(el));
                let (idx, seg) = match at {
                    BendAt::Seg(k, _) => (k, true),
                    BendAt::Mid(k) => {
                        orig.insert(k + 1, q);
                        (k + 1, false)
                    }
                    BendAt::Point(i) => (i, false),
                };
                self.snapshot();
                self.drag = Some(Drag::Bend { id, orig, idx, seg, start: p, moved: false });
                return;
            }
        }
        // 3. 缩放手柄
        if let Some(el) = self.single_resizable() {
            let r = el.rect();
            if let Some(i) = self.resize_handle_at(ev.position) {
                {
                    // 生命线头部的下面两个角只改宽度
                    let i = match (super::seq::is_life(el), i) {
                        (true, 4) => 3,
                        (true, 6) => 7,
                        _ => i,
                    };
                    let (id, font) = (el.id, el.font_size);
                    let grid = match &el.kind {
                        ElKind::Table { cols, rows, .. } => Some((cols.clone(), rows.clone())),
                        _ => None,
                    };
                    let snap: Vec<El> =
                        if el.code.is_some() { self.doc.section_children(id).iter().filter_map(|k| self.doc.get(*k)).cloned().collect() } else { vec![] };
                    let kids = if el.is_lane() {
                        self.doc.section_children(id).iter().filter_map(|k| self.doc.get(*k)).map(|k| (k.id, k.x, k.y)).collect()
                    } else if super::seq::is_life(el) {
                        // 生命线上的激活条跟着虚线走
                        self.doc.bars_on(id).iter().filter_map(|k| self.doc.get(*k)).map(|k| (k.id, k.x, k.y)).collect()
                    } else {
                        vec![]
                    };
                    self.snapshot();
                    self.drag = Some(Drag::Resize { id, handle: i, orig: r, font, grid, kids, snap });
                    return;
                }
            }
        }
        // 3.5 生命线虚线：单击落下激活条，拖动则移动生命线
        if let Some((life, top)) = self.dash_hover(p).filter(|_| ev.click_count < 2 && !ev.modifiers.shift && !ev.modifiers.alt) {
            self.drag = Some(Drag::Dash { life, top, start: ev.position });
            return;
        }
        // 4. 元素
        // 代码绘图：先选中整张图，再单击才选中里面的元素；双击整张图打开代码编辑
        let hit = self.hit(p).map(|id| match self.code_owner(id) {
            Some(owner) if !self.sel.contains(&owner) && !self.sel.iter().any(|s| self.code_owner(*s) == Some(owner)) => owner,
            _ => id,
        });
        if let Some(owner) = hit.and_then(|id| if self.doc.get(id).is_some_and(|e| e.code.is_some()) { Some(id) } else { self.code_owner(id) }) {
            let only_owner = self.sel.len() == 1 && self.sel.contains(&owner);
            if ev.click_count >= 2 && only_owner && !self.doc.get(owner).is_some_and(|e| e.locked || self.section_title_rect(e).contains(p)) {
                let lang = super::code::CodeLang::from_u8(self.doc.get(owner).map(|e| e.code_lang).unwrap_or(0));
                self.open_code(Some(owner), lang, window, cx);
                return;
            }
        }
        match hit {
            Some(id) => {
                let el = self.doc.get(id).cloned();
                let was_selected = self.sel.len() == 1 && self.sel.contains(&id);
                // 表格：选中后再单击选中单元格，双击编辑单元格；泳道：双击泳道名编辑，双击空白处添加图形
                let cell = el.as_ref().and_then(|e| e.grid()).and_then(|g| g.cell_at(p));
                if let (Some(e), Some(i)) = (el.as_ref().filter(|e| e.is_table() && !e.locked), cell) {
                    if ev.click_count >= 2 {
                        if e.is_lane() && !e.is_head_cell(i) {
                            self.sel = [id].into();
                            self.open_picker(ev.position, p, None, cx);
                            return;
                        }
                        self.sel = [id].into();
                        self.start_edit_part(id, Part::Cell(i), window, cx);
                        return;
                    }
                    if was_selected || e.is_lane() && e.is_head_cell(i) {
                        self.sel = [id].into();
                        self.cell_sel = Some((id, i));
                        // 仍然可以拖动整个表格
                        self.snapshot();
                        let orig = self.move_set();
                        self.drag = Some(Drag::Move { start: p, orig, moved: false });
                        cx.notify();
                        return;
                    }
                } else if el.as_ref().is_some_and(|e| e.is_lane()) && ev.click_count >= 2 {
                    // 双击泳道图标题
                    self.sel = [id].into();
                    self.start_edit_part(id, Part::Main, window, cx);
                    return;
                }
                if ev.click_count >= 2 {
                    if let Some(el) = el.clone().filter(|e| !e.locked) {
                        // 双击分区内的空白：在该处添加图形（同双击空白画布）；双击分区标题：重命名
                        if el.is_section() && !self.section_title_rect(&el).contains(p) {
                            self.open_picker(ev.position, p, None, cx);
                            return;
                        }
                        if !matches!(el.kind, ElKind::Image { .. } | ElKind::Icon { .. } | ElKind::Pen { .. } | ElKind::Shape(ShapeKind::Activation)) {
                            self.sel = [id].into();
                            // 多格图形（类图 / 实体）：双击哪一格就编辑哪一格
                            match el.compartment_at(p) {
                                Some(i) => self.start_edit_part(id, Part::Cell(i), window, cx),
                                None => self.start_edit(id, window, cx),
                            }
                            return;
                        }
                    }
                }
                // 单击已选中的图形：没拖动就进入文字编辑
                if was_selected && ev.click_count < 2 && !ev.modifiers.shift && !ev.modifiers.alt {
                    self.click_edit = el
                        .as_ref()
                        .filter(|e| {
                            !e.locked
                                && !e.is_table()
                                && !e.is_mind()
                                && !e.is_section()
                                && !super::seq::is_life(e)
                                && (matches!(e.kind, ElKind::Shape(k) if k != ShapeKind::Activation) || matches!(e.kind, ElKind::Sticky | ElKind::Text))
                        })
                        .map(|e| e.id);
                }
                if ev.modifiers.shift {
                    if !self.sel.remove(&id) {
                        self.sel.insert(id);
                    }
                } else if !self.sel.contains(&id) {
                    self.sel = [id].into();
                }
                self.expand_groups();
                if el.as_ref().is_some_and(|e| e.locked) {
                    cx.notify();
                    return;
                }
                self.snapshot();
                // 思维导图的非根节点：拖动换父节点
                if let Some(e) = el.as_ref().filter(|e| e.mind_parent().is_some() && self.sel.len() == 1 && !ev.modifiers.alt) {
                    let orig = self.doc.mind_subtree(e.id).iter().filter_map(|k| self.doc.get(*k)).map(|k| (k.id, k.x, k.y)).collect();
                    self.drag = Some(Drag::MindMove { id: e.id, start: p, orig, moved: false, reclick: was_selected });
                    return;
                }
                // ⌥ 拖动：先原地复制一份，拖动的是副本
                if ev.modifiers.alt {
                    let els: Vec<El> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).cloned().collect();
                    self.insert_copies(els, 0.);
                }
                let orig = self.move_set();
                self.drag = Some(Drag::Move { start: p, orig, moved: false });
            }
            None => {
                if ev.click_count >= 2 {
                    self.open_picker(ev.position, p, None, cx);
                    return;
                }
                if !ev.modifiers.shift {
                    self.sel.clear();
                }
                self.drag = Some(Drag::Marquee { start: p, cur: p });
            }
        }
    }

    /// 拖动时要一起移动的元素：选区（未锁定）+ 选中分区里的元素，记下原始位置。
    pub(super) fn move_set(&self) -> Vec<(u64, f32, f32, Option<Vec<[f32; 3]>>, Option<(End, End)>)> {
        let mut ids: HashSet<u64> = self.sel.clone();
        for id in self.sel.iter() {
            ids.extend(self.doc.section_children(*id));
            // 生命线带着上面的激活条一起移动
            ids.extend(self.doc.bars_on(*id));
        }
        // 拖动思维导图根节点时整棵树一起动
        for id in ids.clone() {
            if self.doc.get(id).is_some_and(|e| e.is_mind() && e.mind_parent().is_none()) {
                ids.extend(self.doc.mind_subtree(id));
            }
        }
        self.doc
            .els
            .iter()
            .filter(|e| ids.contains(&e.id) && !e.locked)
            .map(|e| {
                let pen = match &e.kind {
                    ElKind::Pen { pts } => Some(pts.clone()),
                    _ => None,
                };
                let ends = match &e.kind {
                    ElKind::Connector { a, b, .. } => Some((a.clone(), b.clone())),
                    _ => None,
                };
                (e.id, e.x, e.y, pen, ends)
            })
            .collect()
    }

    pub(super) fn open_picker(&mut self, at: Point<Pixels>, world: Pt, conn: Option<u64>, cx: &mut Context<Self>) {
        self.picker = Some(Picker { at, world, conn });
        cx.notify();
    }

    pub(super) fn erase_at(&mut self, p: Pt) {
        // 橡皮擦只擦笔迹（手绘橡皮擦）
        let tol = 8. / self.zoom;
        let hit = self.doc.els.iter().rev().find(|e| match &e.kind {
            ElKind::Pen { pts } => {
                pts.windows(2).any(|q| dist_seg(p, pt(q[0][0], q[0][1]), pt(q[1][0], q[1][1])) < tol + e.stroke_w * if e.highlighter { 3. } else { 1. })
            }
            _ => false,
        });
        if let Some(id) = hit.map(|e| e.id) {
            self.doc.els.retain(|e| e.id != id);
        }
    }

    /// 删除端点所绑元素已不存在的连线。
    pub(super) fn remove_dangling(&mut self) {
        let ids: HashSet<u64> = self.doc.els.iter().map(|e| e.id).collect();
        self.doc.els.retain(|e| match &e.kind {
            ElKind::Connector { a, b, .. } => [a, b].iter().all(|end| match end {
                End::Bound { el, .. } => ids.contains(el),
                _ => true,
            }),
            _ => true,
        });
    }

    /// 连线吸附目标：鼠标附近的块状元素，以及靠近的那条边（离边中点足够近时）。
    pub(super) fn snap_target(&self, p: Pt, except: u64) -> Option<(u64, Option<Side>)> {
        let e = self.hit_box(p, except)?;
        let r = e.rect();
        let near = Side::ALL
            .iter()
            .map(|s| {
                let (ax, ay) = s.anchor();
                (*s, pt(r.x + ax * r.w, r.y + ay * r.h).dist(p) * self.zoom)
            })
            .filter(|(_, d)| *d < 18.)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s);
        Some((e.id, near))
    }

    // ───────────── 鼠标移动 ─────────────

    pub(super) fn mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.to_world(ev.position);
        self.mouse_world = p;
        // 右键拖动 = 平移画布
        if let Some((start, _, moved)) = self.right_down.as_mut() {
            if ev.pressed_button == Some(MouseButton::Right) {
                if !*moved && dist_px(*start, ev.position) > 4. {
                    *moved = true;
                    let s = *start;
                    self.drag = Some(Drag::Pan { start: s, orig: self.offset });
                }
            } else {
                self.right_down = None;
            }
        }
        if self.drag.is_none() {
            let h = self.hit(p);
            let v = self.view();
            let dot = self.quick_dots().into_iter().find(|(_, q)| dist_px(v.p(*q), ev.position) < 11.).map(|(s, _)| s);
            let snap = if matches!(self.tool, Tool::Connector(..)) { self.snap_target(p, 0) } else { None };
            let line = if self.tool == Tool::Select { self.grid_line_at(p).map(|x| x.1) } else { None };
            // 表格边界圆点：悬停时变成「+」
            let plus = self.table_plus().into_iter().find(|(_, _, q)| dist_px(v.p(*q), ev.position) < 10.).map(|(row, k, _)| (row, k));
            // 时序图：激活条边缘锚点优先，其次是虚线上的激活条预览
            let seq_ok = self.tool == Tool::Select && !self.space_down && self.editing.is_none() && self.painting.is_none();
            let port = if seq_ok && dot.is_none() { self.bar_port_at(p) } else { None };
            let ghost = if seq_ok && port.is_none() && dot.is_none() { self.dash_hover(p) } else { None };
            if port != self.bar_port || ghost != self.bar_ghost {
                self.bar_port = port;
                self.bar_ghost = ghost;
                cx.notify();
            }
            let handle = if self.tool == Tool::Select && !self.space_down && self.editing.is_none() && dot.is_none() {
                self.resize_handle_at(ev.position)
            } else {
                None
            };
            if handle != self.hover_handle {
                self.hover_handle = handle;
                cx.notify();
            }
            let bend = if self.tool == Tool::Select && !self.space_down && self.editing.is_none() { self.bend_at(ev.position).map(|x| x.1) } else { None };
            if bend != self.hover_bend {
                self.hover_bend = bend;
                cx.notify();
            }
            if h != self.hover || dot != self.hover_dot || snap != self.snap_anchor || line != self.hover_line || plus != self.table_hot {
                self.hover = h;
                self.hover_dot = dot;
                self.snap_anchor = snap;
                self.hover_line = line;
                self.table_hot = plus;
                cx.notify();
            }
            return;
        }
        if ev.pressed_button.is_none() {
            return;
        }
        let zoom = self.zoom;
        match self.drag.take() {
            Some(Drag::Pan { start, orig }) => {
                self.offset = pt(orig.x + f32::from(ev.position.x - start.x), orig.y + f32::from(ev.position.y - start.y));
                self.drag = Some(Drag::Pan { start, orig });
            }
            Some(Drag::QuickDot { from, side, start }) => self.mouse_move_quick_dot(ev, cx, from, p, side, start),
            Some(Drag::Dash { life, top, start }) => {
                if dist_px(start, ev.position) > 4. {
                    // 拖动虚线 = 移动生命线
                    self.bar_ghost = None;
                    self.sel = [life].into();
                    self.snapshot();
                    let orig = self.move_set();
                    let from = self.to_world(start);
                    self.drag = Some(Drag::Move { start: from, orig, moved: false });
                } else {
                    self.drag = Some(Drag::Dash { life, top, start });
                }
            }
            Some(Drag::Port { from, ax, ay, start }) => {
                if dist_px(start, ev.position) > 4. {
                    // 从激活条边缘拉出消息线：直线 + 实心箭头
                    self.snapshot();
                    let el = self.doc.new_el(
                        ElKind::Connector {
                            a: End::Bound { el: from, ax, ay },
                            b: End::Free { x: p.x, y: p.y },
                            route: Route::Straight,
                            start: Arrow::None,
                            end: Arrow::Filled,
                        },
                        Rect::default(),
                        Self::dark(cx),
                    );
                    let id = el.id;
                    self.doc.els.push(el);
                    self.bar_port = None;
                    self.drag_msg_end(id, 1, p);
                    self.drag = Some(Drag::Connect { id });
                } else {
                    self.drag = Some(Drag::Port { from, ax, ay, start });
                }
            }
            Some(Drag::Move { start, orig, .. }) => {
                let (mut dx, mut dy) = (p.x - start.x, p.y - start.y);
                // 生命线上的激活条只能沿虚线上下移动
                if let Some(cdy) = self.bar_move(&orig, dy) {
                    self.guides.clear();
                    for (id, _, y, _, _) in &orig {
                        if let Some(i) = self.doc.index(*id) {
                            self.doc.els[i].y = y + cdy;
                        }
                    }
                    self.drag = Some(Drag::Move { start, orig, moved: true });
                    cx.notify();
                    return;
                }
                // Shift：只沿水平或竖直方向移动
                if ev.modifiers.shift {
                    if dx.abs() > dy.abs() {
                        dy = 0.;
                    } else {
                        dx = 0.;
                    }
                }
                let (sdx, sdy) = self.snap_move(&orig, dx, dy, ev.modifiers.shift);
                dx += sdx;
                dy += sdy;
                for (id, x, y, pen, ends) in &orig {
                    if let Some(i) = self.doc.index(*id) {
                        let e = &mut self.doc.els[i];
                        e.x = x + dx;
                        e.y = y + dy;
                        if let (ElKind::Pen { pts }, Some(o)) = (&mut e.kind, pen) {
                            for (q, oq) in pts.iter_mut().zip(o) {
                                q[0] = oq[0] + dx;
                                q[1] = oq[1] + dy;
                            }
                        }
                        if let (ElKind::Connector { a, b, .. }, Some((oa, ob))) = (&mut e.kind, ends) {
                            for (cur, orig_end) in [(a, oa), (b, ob)] {
                                if let End::Free { x, y } = orig_end {
                                    *cur = End::Free { x: x + dx, y: y + dy };
                                }
                            }
                        }
                    }
                }
                self.drag = Some(Drag::Move { start, orig, moved: true });
            }
            Some(Drag::GridLine { id, vertical, k, orig, start }) => {
                let d = if vertical { p.x - start.x } else { p.y - start.y };
                self.drag_grid_line(id, vertical, k, orig, d);
                self.drag = Some(Drag::GridLine { id, vertical, k, orig, start });
            }
            Some(Drag::MindMove { id, start, orig, moved, reclick }) => {
                let (dx, dy) = (p.x - start.x, p.y - start.y);
                let moved = moved || (dx.abs() + dy.abs()) * zoom > 4.;
                if moved {
                    for (k, x, y) in &orig {
                        if let Some(i) = self.doc.index(*k) {
                            self.doc.els[i].x = x + dx;
                            self.doc.els[i].y = y + dy;
                        }
                    }
                    self.mind_target = self.mind_drop_target(id, p);
                }
                self.drag = Some(Drag::MindMove { id, start, orig, moved, reclick });
            }
            Some(Drag::Resize { id, handle, orig, font, grid, kids, snap }) => {
                if let Some(i) = self.doc.index(id) {
                    let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
                    match handle {
                        0 => (x0, y0) = (p.x, p.y),
                        1 => y0 = p.y,
                        2 => (x1, y0) = (p.x, p.y),
                        3 => x1 = p.x,
                        4 => (x1, y1) = (p.x, p.y),
                        5 => y1 = p.y,
                        6 => (x0, y1) = (p.x, p.y),
                        _ => x0 = p.x,
                    }
                    let mut r = Rect::from_points(pt(x0, y0), pt(x1, y1));
                    let e = &mut self.doc.els[i];
                    // 图片始终等比；其他元素按住 Shift 拖角时等比
                    let keep = matches!(e.kind, ElKind::Image { .. } | ElKind::Icon { .. }) || e.code.is_some() || (ev.modifiers.shift && handle % 2 == 0);
                    if keep {
                        let ratio = orig.h / orig.w.max(1.);
                        let w = r.w.max(16.);
                        let h = w * ratio;
                        // 以对角为锚点
                        let (ax, ay) = match handle {
                            0 => (orig.right(), orig.bottom()),
                            2 => (orig.x, orig.bottom()),
                            6 => (orig.right(), orig.y),
                            _ => (orig.x, orig.y),
                        };
                        r = rect(if ax > orig.x { ax - w } else { ax }, if ay > orig.y { ay - h } else { ay }, w, h);
                    }
                    e.x = r.x;
                    e.y = r.y;
                    e.w = r.w.max(16.);
                    e.h = r.h.max(16.);
                    // 文本框横向拉伸时只改宽度，纵向拉伸等比放大字号
                    if matches!(e.kind, ElKind::Text) && handle % 2 == 0 && orig.h > 1. {
                        e.font_size = (font * e.h / orig.h).clamp(8., 200.);
                    }
                    // 表格 / 泳道：行列等比缩放；泳道里的图形按比例跟着移动
                    if let (Some((c0, r0)), ElKind::Table { cols, rows, .. }) = (&grid, &mut e.kind) {
                        let (sw, sh) = (e.w / orig.w.max(1.), (e.h - (orig.h - r0.iter().sum::<f32>())) / r0.iter().sum::<f32>().max(1.));
                        *cols = c0.iter().map(|c| c * sw).collect();
                        *rows = r0.iter().map(|r| r * sh).collect();
                    }
                    // 生命线：虚线至少留一小段；激活条跟着虚线水平居中
                    if let Some((x, top, _)) = super::seq::dash_of(e) {
                        e.h = e.h.max(top - e.y + 20.);
                        for (k, _, _) in &kids {
                            if let Some(j) = self.doc.index(*k) {
                                let c = &mut self.doc.els[j];
                                c.x = x - c.w / 2.;
                            }
                        }
                        self.drag = Some(Drag::Resize { id, handle, orig, font, grid, kids, snap });
                        cx.notify();
                        return;
                    }
                    let (nx, ny, sx, sy) = (e.x, e.y, e.w / orig.w.max(1.), e.h / orig.h.max(1.));
                    for (k, kx, ky) in &kids {
                        if let Some(j) = self.doc.index(*k) {
                            let c = &mut self.doc.els[j];
                            c.x = nx + (kx - orig.x) * sx;
                            c.y = ny + (ky - orig.y) * sy;
                        }
                    }
                    // 代码分区：里面的图形（连同字号、线宽、笔迹、连线自由端）整体等比缩放
                    for o in &snap {
                        if let Some(j) = self.doc.index(o.id) {
                            self.doc.els[j] = o.scaled(orig.x, orig.y, nx, ny, sx, sy);
                        }
                    }
                }
                self.drag = Some(Drag::Resize { id, handle, orig, font, grid, kids, snap });
            }
            Some(Drag::Create { start, id }) => {
                if let Some(i) = self.doc.index(id) {
                    let mut r = Rect::from_points(start, p);
                    if ev.modifiers.shift {
                        let s = r.w.max(r.h);
                        r.w = s;
                        r.h = s;
                    }
                    let e = &mut self.doc.els[i];
                    e.x = r.x;
                    e.y = r.y;
                    e.w = r.w;
                    e.h = r.h;
                }
                self.drag = Some(Drag::Create { start, id });
            }
            Some(Drag::Marquee { start, .. }) => self.mouse_move_marquee(p, start),
            Some(Drag::Pen { id, last }) => {
                if p.dist(last) > 1.5 / zoom {
                    // 用速度模拟压力：越快越细
                    let speed = p.dist(last) * zoom;
                    let pressure = (1.0 - (speed / 40.).min(0.7)).clamp(0.3, 1.0);
                    if let Some(i) = self.doc.index(id) {
                        if let ElKind::Pen { pts } = &mut self.doc.els[i].kind {
                            pts.push([p.x, p.y, pressure]);
                        }
                    }
                    self.drag = Some(Drag::Pen { id, last: p });
                } else {
                    self.drag = Some(Drag::Pen { id, last });
                }
            }
            Some(Drag::Connect { id }) => {
                if !self.drag_msg_end(id, 1, p) {
                    let source = self.connector_source(id);
                    self.snap_anchor = self.snap_target(p, source);
                    self.set_end(id, 1, p);
                }
                self.drag = Some(Drag::Connect { id });
            }
            Some(Drag::EndPoint { id, which }) => {
                if !self.drag_msg_end(id, which, p) {
                    let other = self.connector_other(id, which);
                    self.snap_anchor = self.snap_target(p, other);
                    self.set_end(id, which, p);
                }
                self.drag = Some(Drag::EndPoint { id, which });
            }
            Some(Drag::Bend { id, orig, idx, seg, start, moved }) => {
                let d = p.sub(start);
                let moved = moved || d.x.abs() * zoom > 2. || d.y.abs() * zoom > 2.;
                if moved {
                    let pts = self.bend_route(&orig, idx, seg, d);
                    self.doc.set_route(id, &pts);
                }
                self.drag = Some(Drag::Bend { id, orig, idx, seg, start, moved });
            }
            Some(Drag::Erase) => {
                self.erase_at(p);
                self.drag = Some(Drag::Erase);
            }
            None => {}
        }
        cx.notify();
    }
    /// 从 `mouse_move` 拆出。
    fn mouse_move_quick_dot(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>, from: u64, p: Pt, side: Side, start: Point<Pixels>) {
        if dist_px(start, ev.position) > 5. {
            // 从圆点拉出连线
            self.snapshot();
            let (ax, ay) = side.anchor();
            // 思维导图节点拉出的是曲线（的「关联线」）
            let route = if self.doc.get(from).is_some_and(|e| e.is_mind()) { Route::Curve } else { Route::Elbow };
            let el = self.doc.new_el(
                ElKind::Connector { a: End::Bound { el: from, ax, ay }, b: End::Free { x: p.x, y: p.y }, route, start: Arrow::None, end: Arrow::Filled },
                Rect::default(),
                Self::dark(cx),
            );
            let id = el.id;
            self.doc.els.push(el);
            self.hover_dot = None;
            self.drag = Some(Drag::Connect { id });
        } else {
            self.drag = Some(Drag::QuickDot { from, side, start });
        }
    }

    /// 从 `mouse_move` 拆出。
    fn mouse_move_marquee(&mut self, p: Pt, start: Pt) {
        let m = Rect::from_points(start, p);
        let precise = prefs().precise_select;
        let lk = self.doc.lookup();
        self.sel = self
            .doc
            .els
            .iter()
            .filter(|e| lk.visible(e.id))
            .filter(|e| {
                let b = BoardDoc::bounds_in(&lk, e);
                let inside = b.x >= m.x && b.y >= m.y && b.right() <= m.right() && b.bottom() <= m.bottom();
                // 生命线：框到头部或虚线才算碰到，虚线两侧的空白不算
                let touch = match super::seq::dash_of(e) {
                    Some((x, y0, y1)) => super::seq::head_rect(e).intersects(&m) || (x >= m.x && x <= m.right() && m.y <= y1 && m.bottom() >= y0),
                    None => b.intersects(&m),
                };
                // 容器只在被完整框住时选中，避免一拖就选上整块背景
                if e.is_container() || precise {
                    inside
                } else {
                    touch
                }
            })
            .map(|e| e.id)
            .collect();
        self.expand_groups();
        self.drag = Some(Drag::Marquee { start, cur: p });
    }

    /// 移动时的对齐吸附：选区包围盒的左 / 中 / 右、上 / 中 / 下贴齐其他元素，并记录参考线。
    fn snap_move(&mut self, orig: &[(u64, f32, f32, Option<Vec<[f32; 3]>>, Option<(End, End)>)], dx: f32, dy: f32, lock: bool) -> (f32, f32) {
        self.guides.clear();
        let moving: HashSet<u64> = orig.iter().map(|o| o.0).collect();
        let Some(sb) = orig
            .iter()
            .filter_map(|o| self.doc.get(o.0).map(|e| (e, o)))
            .filter(|(e, _)| e.is_box() || e.is_section())
            .map(|(e, o)| rect(o.1, o.2, e.w, e.h))
            .reduce(|a, b| a.union(&b))
        else {
            return (0., 0.);
        };
        let moved = rect(sb.x + dx, sb.y + dy, sb.w, sb.h);
        let th = 6. / self.zoom;
        let (mut best_x, mut best_y): (Option<(f32, f32)>, Option<(f32, f32)>) = (None, None);
        for e in self.doc.els.iter().filter(|e| !moving.contains(&e.id) && (e.is_box() || e.is_section())) {
            let r = e.rect();
            for a in [moved.x, moved.center().x, moved.right()] {
                for b in [r.x, r.center().x, r.right()] {
                    let d = b - a;
                    if d.abs() < th && best_x.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                        best_x = Some((d, b));
                    }
                }
            }
            for a in [moved.y, moved.center().y, moved.bottom()] {
                for b in [r.y, r.center().y, r.bottom()] {
                    let d = b - a;
                    if d.abs() < th && best_y.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                        best_y = Some((d, b));
                    }
                }
            }
        }
        let mut out = (0., 0.);
        if let Some((d, _)) = best_x.filter(|_| !(lock && dx == 0.)) {
            out.0 = d;
        }
        if let Some((d, _)) = best_y.filter(|_| !(lock && dy == 0.)) {
            out.1 = d;
        }
        let fin = rect(moved.x + out.0, moved.y + out.1, moved.w, moved.h);
        let others: Vec<Rect> = self.doc.els.iter().filter(|e| !moving.contains(&e.id) && (e.is_box() || e.is_section())).map(|e| e.rect()).collect();
        self.guides = align_guides(fin, &others);
        out
    }

    fn connector_source(&self, id: u64) -> u64 {
        match self.doc.get(id).map(|e| &e.kind) {
            Some(ElKind::Connector { a: End::Bound { el, .. }, .. }) => *el,
            _ => 0,
        }
    }

    fn connector_other(&self, id: u64, which: usize) -> u64 {
        match self.doc.get(id).map(|e| &e.kind) {
            Some(ElKind::Connector { a, b, .. }) => match if which == 0 { b } else { a } {
                End::Bound { el, .. } => *el,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// 拖动连线端点：吸附到锚点时直接绑定（预览即为最终走线），否则跟随鼠标。
    fn set_end(&mut self, id: u64, which: usize, p: Pt) {
        let end = match self.snap_anchor {
            Some((t, Some(side))) => {
                let (ax, ay) = side.anchor();
                End::Bound { el: t, ax, ay }
            }
            Some((t, None)) => End::Bound { el: t, ax: 0.5, ay: 0.5 },
            None => End::Free { x: p.x, y: p.y },
        };
        if let Some(i) = self.doc.index(id) {
            if let ElKind::Connector { a, b, .. } = &mut self.doc.els[i].kind {
                *(if which == 0 { a } else { b }) = end;
            }
            // 拖动端点后不再沿用代码绘图算出的拐点
            self.doc.els[i].via.clear();
        }
    }

    // ───────────── 鼠标抬起 ─────────────

    pub(super) fn mouse_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.guides.clear();
        let p = self.mouse_world;
        match self.drag.take() {
            Some(Drag::Create { id, .. }) => {
                if let Some(i) = self.doc.index(id) {
                    let e = &mut self.doc.els[i];
                    if e.w < 6. && e.h < 6. {
                        // 单击：按默认尺寸放在点击处
                        let (w, h) = match (&e.kind, self.tool) {
                            (ElKind::Section, Tool::Section(w, h)) => (w, h),
                            (ElKind::Sticky, _) => (200., 200.),
                            (ElKind::Shape(k), _) => super::library::default_size(*k),
                            _ => (140., 72.),
                        };
                        e.x -= w / 2.;
                        e.y -= h / 2.;
                        e.w = w;
                        e.h = h;
                    }
                }
                let section = self.doc.get(id).is_some_and(|e| e.is_section());
                if !self.tool_lock {
                    self.tool = Tool::Select;
                }
                self.commit(cx);
                if !section {
                    self.start_edit(id, window, cx);
                }
            }
            Some(Drag::QuickDot { from, side, .. }) => {
                // 思维导图节点：单击圆点不复制节点（加节点用「+」）
                if !self.doc.get(from).is_some_and(|e| e.is_mind()) {
                    self.quick_create(from, side, window, cx);
                }
            }
            Some(Drag::Dash { life, top, .. }) => self.place_bar(life, top, cx),
            Some(Drag::Port { from, .. }) => {
                self.sel = [from].into();
                cx.notify();
            }
            Some(Drag::Connect { id }) => {
                // 从激活条拉出的消息线：落在生命线上时新建激活条，落在空白处保留悬空的箭头（不弹图形选择器）
                let msg = self.msg_end(id, 0).is_some();
                let new_bar = if msg { self.finish_msg(id, 1) } else { None };
                let from_quick = self.tool == Tool::Select && !msg;
                let (free_end, too_short) = match self.doc.get(id) {
                    Some(el) => {
                        let pts = self.doc.connector_points(el);
                        let short = pts.first().zip(pts.last()).is_some_and(|(a, b)| a.dist(*b) < 6.);
                        let free = matches!(el.kind, ElKind::Connector { b: End::Free { .. }, .. });
                        (free, short)
                    }
                    None => (false, true),
                };
                if too_short {
                    self.doc.els.retain(|e| e.id != id);
                    self.undo.pop_back();
                } else if from_quick && free_end {
                    // 从快捷创建圆点拖到空白处：弹出图形选择器，选中的图形接在线的末端
                    self.sel = [id].into();
                    let at = ev.position;
                    self.open_picker(at, p, Some(id), cx);
                } else if let Some(bar) = new_bar {
                    // 选中自动生成的激活条，方便接着拉高或继续连线
                    self.sel = [bar].into();
                } else {
                    self.sel = [id].into();
                }
                self.snap_anchor = None;
                self.msg_snap = None;
                if !self.tool_lock && matches!(self.tool, Tool::Connector(..)) {
                    self.tool = Tool::Select;
                }
                self.commit(cx);
            }
            Some(Drag::Bend { moved, .. }) => {
                if moved {
                    self.commit(cx);
                } else {
                    self.undo.pop_back();
                }
            }
            Some(Drag::EndPoint { id, which }) => {
                self.finish_msg(id, which);
                self.snap_anchor = None;
                self.msg_snap = None;
                self.commit(cx);
            }
            Some(Drag::Move { moved, .. }) => {
                if moved {
                    self.commit(cx);
                } else {
                    self.undo.pop_back();
                    if let Some(id) = self.click_edit.take() {
                        let part = self.doc.get(id).and_then(|e| e.compartment_at(self.mouse_world));
                        match part {
                            Some(i) => self.start_edit_part(id, Part::Cell(i), window, cx),
                            None => self.start_edit(id, window, cx),
                        }
                    }
                }
            }
            Some(Drag::Resize { id, .. }) | Some(Drag::GridLine { id, .. }) => {
                // 缩小到放不下文字时撑回来（图形 / 表格行不会比文字矮）
                let ts = cx.text_system().clone();
                if let Some(i) = self.doc.index(id) {
                    Self::fit_all(&mut self.doc.els[i], &ts);
                }
                self.commit(cx)
            }
            Some(Drag::Pen { .. }) | Some(Drag::Erase) => self.commit(cx),
            Some(Drag::MindMove { id, moved, reclick, .. }) => {
                let target = self.mind_target.take();
                if moved {
                    self.mind_drop(id, target);
                    self.commit(cx);
                } else {
                    self.undo.pop_back();
                    // 选中后再单击节点：进入编辑（思维导图）
                    if reclick {
                        self.start_edit(id, window, cx);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    // ───────────── 右键 ─────────────

    pub(super) fn right_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.close_overlays();
        self.finish_edit(cx);
        self.right_down = Some((ev.position, self.to_world(ev.position), false));
        cx.notify();
    }

    pub(super) fn right_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((_, world, moved)) = self.right_down.take() else { return };
        if moved {
            self.drag = None;
            cx.notify();
            return;
        }
        // 原地松开：右键菜单。点在元素上时先选中它
        match self.hit(world) {
            Some(id) => {
                if !self.sel.contains(&id) {
                    self.sel = [id].into();
                    self.expand_groups();
                }
                self.menu = Some((ev.position, true));
            }
            None => {
                self.sel.clear();
                self.menu = Some((ev.position, false));
            }
        }
        cx.notify();
    }

    // ───────────── 视图 ─────────────

    pub(super) fn scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = ev.delta.pixel_delta(px(20.));
        if ev.modifiers.platform || ev.modifiers.control {
            // 以鼠标为中心缩放
            let factor = (1. + f32::from(delta.y) * 0.004).clamp(0.5, 2.);
            self.zoom_at(ev.position, factor);
        } else {
            self.offset = pt(self.offset.x + f32::from(delta.x), self.offset.y + f32::from(delta.y));
        }
        self.popover = self.popover.filter(|p| matches!(p, Popover::Settings | Popover::Export | Popover::Zoom));
        cx.notify();
    }

    pub(super) fn zoom_at(&mut self, screen: Point<Pixels>, factor: f32) {
        let w = self.to_world(screen);
        self.zoom = (self.zoom * factor).clamp(0.1, 6.);
        let local = (f32::from(screen.x - self.canvas_bounds.origin.x), f32::from(screen.y - self.canvas_bounds.origin.y));
        self.offset = pt(local.0 - w.x * self.zoom, local.1 - w.y * self.zoom);
    }

    pub(super) fn zoom_to(&mut self, r: Option<Rect>) {
        let size = self.canvas_bounds.size;
        let (vw, vh) = (f32::from(size.width).max(200.), f32::from(size.height).max(200.));
        match r {
            Some(r) => {
                let r = r.inflate(80.);
                self.zoom = (vw / r.w).min(vh / r.h).clamp(0.1, 2.);
                self.offset = pt(vw / 2. - r.center().x * self.zoom, vh / 2. - r.center().y * self.zoom);
            }
            None => {
                self.zoom = 1.;
                self.offset = pt(vw / 2., vh / 2.);
            }
        }
    }

    pub(super) fn zoom_step(&mut self, f: f32, cx: &mut Context<Self>) {
        let c = self.canvas_bounds.center();
        self.zoom_at(c, f);
        cx.notify();
    }

    pub(super) fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        let r = self.selection_bounds().or_else(|| self.doc.bounds());
        self.zoom_to(r);
        cx.notify();
    }

    pub(super) fn zoom_100(&mut self, cx: &mut Context<Self>) {
        let c = self.canvas_bounds.center();
        let f = 1. / self.zoom;
        self.zoom_at(c, f);
        cx.notify();
    }

    /// 快捷创建：在 side 方向新建一个同样的图形并用直角连线连上，随后直接编辑文字。
    pub(super) fn quick_create(&mut self, from: u64, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.doc.get(from).cloned() else { return };
        self.snapshot();
        let mut r = quick_target(src.rect(), side);
        // 目标位置被占时继续往同方向让开
        for _ in 0..8 {
            if !self.doc.els.iter().any(|e| e.is_box() && e.rect().intersects(&r.inflate(-2.))) {
                break;
            }
            let step = side.dir();
            r.x += step.x * (r.w + 40.) + if step.x == 0. { r.w + 40. } else { 0. };
            r.y += step.y * (r.h + 40.);
        }
        let id = self.doc.alloc_id();
        let mut el = src.clone();
        el.id = id;
        el.x = r.x;
        el.y = r.y;
        el.text.clear();
        el.locked = false;
        el.group = None;
        if matches!(el.kind, ElKind::Sticky) {
            el.author = crate::settings::author();
        }
        self.doc.els.push(el);
        let (ax, ay) = side.anchor();
        let (bx, by) = opposite(side).anchor();
        let conn = self.doc.new_el(
            ElKind::Connector {
                a: End::Bound { el: from, ax, ay },
                b: End::Bound { el: id, ax: bx, ay: by },
                route: Route::Elbow,
                start: Arrow::None,
                end: Arrow::Filled,
            },
            Rect::default(),
            Self::dark(cx),
        );
        // 便签之间不连线（便签的快捷创建只复制便签）
        if !matches!(src.kind, ElKind::Sticky) {
            self.doc.els.push(conn);
        }
        self.sel = [id].into();
        self.hover_dot = None;
        self.commit(cx);
        self.start_edit(id, window, cx);
    }

    /// 图形选择器选中后：在记录的位置放置图形；若是从连线末端弹出的，把连线接上。
    pub(super) fn picker_choose(&mut self, kind: Option<ShapeKind>, sticky: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pk) = self.picker.take() else { return };
        let dark = Self::dark(cx);
        if pk.conn.is_none() {
            self.snapshot();
        }
        let (w, h) = match kind {
            Some(k) => super::library::default_size(k),
            _ if sticky => (200., 200.),
            _ => (140., 72.),
        };
        let ek = match kind {
            Some(k) => ElKind::Shape(k),
            None if sticky => ElKind::Sticky,
            None => ElKind::Text,
        };
        let mut el = self.doc.new_el(ek, rect(pk.world.x - w / 2., pk.world.y - h / 2., w, h), dark);
        self.style_new(&mut el);
        if matches!(el.kind, ElKind::Text) {
            el.h = 32.;
            el.y = pk.world.y - 16.;
        }
        // 从连线末端创建：图形朝向连线的那一边贴在落点上
        if let Some(cid) = pk.conn {
            if let Some(src) = self.doc.get(cid).and_then(|c| match &c.kind {
                ElKind::Connector { a: End::Bound { el, .. }, .. } => self.doc.get(*el).map(|e| e.rect().center()),
                _ => None,
            }) {
                let side = Side::facing(pk.world, src);
                let d = side.dir();
                el.x = pk.world.x - el.w / 2. - d.x * el.w / 2.;
                el.y = pk.world.y - el.h / 2. - d.y * el.h / 2.;
                let (ax, ay) = side.anchor();
                let nid = el.id;
                if let Some(i) = self.doc.index(cid) {
                    if let ElKind::Connector { b, .. } = &mut self.doc.els[i].kind {
                        *b = End::Bound { el: nid, ax, ay };
                    }
                }
            }
        }
        let id = el.id;
        self.doc.els.push(el);
        self.sel = [id].into();
        self.commit(cx);
        self.start_edit(id, window, cx);
    }
}

/// 对齐参考线：只画在对齐的图形之间，从最远的对齐图形连到被拖动的图形。
/// 某个方向上两条边都对齐时只画边线，不再画中线。
pub(super) fn align_guides(m: Rect, others: &[Rect]) -> Vec<(Pt, Pt)> {
    const EPS: f32 = 0.5;
    let mut out = vec![];
    for vertical in [true, false] {
        let (lo, mid, hi) = if vertical { (m.x, m.center().x, m.right()) } else { (m.y, m.center().y, m.bottom()) };
        let lines = |at: f32| -> Option<(Pt, Pt)> {
            // 与 at 对齐的图形（左 / 中 / 右任一条线）连同被拖动的图形，在另一个方向上的跨度
            let hits: Vec<&Rect> = others
                .iter()
                .filter(|r| {
                    let cs = if vertical { [r.x, r.center().x, r.right()] } else { [r.y, r.center().y, r.bottom()] };
                    cs.iter().any(|c| (c - at).abs() < EPS)
                })
                .collect();
            if hits.is_empty() {
                return None;
            }
            let (a, b) = hits.iter().fold(if vertical { (m.y, m.bottom()) } else { (m.x, m.right()) }, |(a, b), r| {
                if vertical {
                    (a.min(r.y), b.max(r.bottom()))
                } else {
                    (a.min(r.x), b.max(r.right()))
                }
            });
            Some(if vertical { (pt(at, a), pt(at, b)) } else { (pt(a, at), pt(b, at)) })
        };
        let edges: Vec<(Pt, Pt)> = [lo, hi].into_iter().filter_map(lines).collect();
        let both = edges.len() == 2;
        out.extend(edges);
        if !both {
            out.extend(lines(mid));
        }
    }
    out
}

/// 快捷创建的新图形位置：与原图形同尺寸，在 side 方向隔开一段距离。
pub(super) fn quick_target(r: Rect, side: Side) -> Rect {
    let gap = 80.;
    match side {
        Side::Right => rect(r.right() + gap, r.y, r.w, r.h),
        Side::Left => rect(r.x - gap - r.w, r.y, r.w, r.h),
        Side::Bottom => rect(r.x, r.bottom() + gap, r.w, r.h),
        Side::Top => rect(r.x, r.y - gap - r.h, r.w, r.h),
    }
}

pub(super) fn opposite(s: Side) -> Side {
    match s {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

fn dist_px(a: Point<Pixels>, b: Point<Pixels>) -> f32 {
    let (dx, dy) = (f32::from(a.x - b.x), f32::from(a.y - b.y));
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::align_guides;
    use crate::canvas::{pt, rect};

    #[test]
    fn guides_span_only_between_aligned_shapes() {
        // 与上方同宽的图形左右都对齐：两条竖线，从上方图形顶部连到拖动图形底部，不画中线
        let m = rect(100., 300., 110., 70.);
        let g = align_guides(m, &[rect(100., 100., 110., 110.), rect(500., 900., 50., 50.)]);
        assert_eq!(g, vec![(pt(100., 100.), pt(100., 370.)), (pt(210., 100.), pt(210., 370.))]);
        // 只有顶边对齐：一条横线，从左边图形连到拖动图形
        let g = align_guides(m, &[rect(-100., 300., 60., 40.)]);
        assert_eq!(g, vec![(pt(-100., 300.), pt(210., 300.))]);
        // 没有对齐：不画
        assert!(align_guides(m, &[rect(7., 9., 10., 10.)]).is_empty());
    }
}
