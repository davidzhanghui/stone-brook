//! 时间轴块：节点 = 彩色卡片（标题 + 详情）+ 时间，配色按顺序循环；
//! 水平 / 垂直排列，卡片交替或同侧；悬停轴上两点之间出「+」插入节点、末端「+」追加，悬停圆点出删除；
//! 右上角切换排列方式、显示 / 隐藏标题·详情·时间。

use super::blocks::GpuiMeasure;
use super::render::SlotSpec;
use super::*;
use crate::canvas::Measure;
use crate::model::{Block, BlockKind, TimelineItem, TIMELINE_COLORS, TL_HIDE_DETAIL, TL_HIDE_TIME, TL_HIDE_TITLE, TL_SLOTS};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};

const CARD_W: f32 = 160.;
/// 水平排列时相邻节点的间距：交替 / 同侧
const GAP_ALT: f32 = 176.;
const GAP_SAME: f32 = 200.;
/// 轴线所占的行高（水平）/ 列宽（垂直）
const AXIS: f32 = 28.;
/// 垂直排列时轴两侧区域的宽度
const SIDE_W: f32 = 200.;
/// 水平排列末端「+」区域
const END_W: f32 = 56.;

#[derive(Clone, Copy, PartialEq)]
pub enum TlMenuKind {
    /// 排列方式
    Layout,
    /// 显示字段
    Fields,
}

/// 时间轴右上角的下拉菜单。
pub struct TlMenu {
    pub block: usize,
    pub kind: TlMenuKind,
    /// 排列方式菜单里悬停展开的子菜单：Some(是否垂直)
    pub sub: Option<bool>,
}

/// 第 k 个节点的 (卡片底色, 圆点色)
fn colors(k: usize, t: &Theme) -> (Hsla, Hsla) {
    let (bg, dot) = TIMELINE_COLORS[k % TIMELINE_COLORS.len()];
    let dot: Hsla = rgb(dot).into();
    (if t.dark { dot.opacity(0.22) } else { rgb(bg).into() }, dot)
}

fn first_visible(hide: u8) -> usize {
    (0..TL_SLOTS).find(|i| hide & (1 << i) == 0).unwrap_or(0)
}

/// 虚线轴：水平时画在区域垂直居中处；垂直时按 from..to（0..1 的比例）画在水平居中处。
fn dashed(color: Hsla, horizontal: bool, from: f32, to: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let (dash, gap): (f32, f32) = (4., 3.);
            let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
            let (start, end) = if horizontal { (0., w) } else { (h * from, h * to) };
            let mut p = start;
            while p < end {
                let l = dash.min(end - p);
                let r = if horizontal {
                    Bounds::new(point(b.origin.x + px(p), b.origin.y + px(h / 2. - 0.5)), size(px(l), px(1.)))
                } else {
                    Bounds::new(point(b.origin.x + px(w / 2. - 0.5), b.origin.y + px(p)), size(px(1.), px(l)))
                };
                window.paint_quad(fill(r, color));
                p += dash + gap;
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Editor {
    pub(super) fn render_timeline(&mut self, ix: usize, blk: &Block, focused: bool, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let BlockKind::Timeline { items, vertical, same_side, hide } = &blk.kind else { return div().into_any_element() };
        let (vertical, same, hide) = (*vertical, *same_side, *hide);
        let id = blk.id as usize;
        let selected = focused && self.block_selected(ix);
        let in_block = focused && self.core.head.block == ix;
        let avail = self.avail_w.get() - 2.;

        let body = if vertical {
            let w = SIDE_W * 2. + AXIS;
            div().pl(px(((avail - w) / 2.).max(0.))).child(self.tl_vertical(ix, items, same, hide, focused, t, window, cx)).into_any_element()
        } else {
            let gap = if same { GAP_SAME } else { GAP_ALT };
            let w = items.len() as f32 * gap + END_W;
            let inner = div().flex_none().pl(px(((avail - w) / 2.).max(0.))).child(self.tl_horizontal(ix, items, same, hide, focused, t, window, cx));
            if w > avail {
                // 放不下时横向滚动；纵向滚轮仍交给文档
                let mut sc = div().id(("tl-scroll", id)).w_full().overflow_x_scroll().pb(px(6.)).child(inner);
                sc.style().restrict_scroll_to_axis = Some(true);
                sc.into_any_element()
            } else {
                inner.into_any_element()
            }
        };

        div()
            .id(("tl", id))
            .group("tl")
            .relative()
            .w_full()
            .pt(px(44.))
            .pb(px(20.))
            .rounded(px(8.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .child(body)
            .child(self.tl_toolbar(ix, vertical, hide, in_block, t, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    // 点空白处：光标进第一个节点
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, first_visible(hide), 0), 1, ev.modifiers.shift, cx);
                }),
            )
            .into_any_element()
    }

    /// 水平排列：上排 / 轴线 / 下排三行，每个节点占一列。
    #[allow(clippy::too_many_arguments)]
    fn tl_horizontal(&self, ix: usize, items: &[TimelineItem], same: bool, hide: u8, focused: bool, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let n = items.len();
        let gap = if same { GAP_SAME } else { GAP_ALT };
        let (mut top, mut bottom) = (div().flex().flex_row().items_end(), div().flex().flex_row().items_start());
        for (k, it) in items.iter().enumerate() {
            let card = self.tl_card(ix, k, it, hide, focused, t, cx);
            let time = self.tl_time(ix, k, it, hide, focused, t, window, cx);
            let col = || div().w(px(gap)).flex_none().flex().flex_col().items_center();
            // 交替：偶数节点卡片在上、时间在下；奇数相反。同侧：卡片都在上
            let (a, b) = if same || k % 2 == 0 { (card, time) } else { (time, card) };
            top = top.child(col().pb(px(4.)).children(a));
            bottom = bottom.child(col().pt(px(4.)).children(b));
        }

        let x = |k: usize| k as f32 * gap + gap / 2.;
        let mut axis = div().relative().h(px(AXIS)).w(px(n as f32 * gap + END_W));
        if n > 1 {
            axis = axis.child(div().absolute().top_0().left(px(x(0))).w(px(x(n - 1) - x(0))).h(px(AXIS)).child(dashed(t.border_strong, true, 0., 1.)));
        }
        // 两点之间插入；末端追加
        for k in 0..n.saturating_sub(1) {
            axis = axis.child(self.tl_insert_zone(ix, k + 1, (x(k) + 36., 0.), (gap - 48., AXIS), true, false, t, cx));
        }
        if n > 0 {
            let x0 = x(n - 1) + 36.;
            axis = axis.child(self.tl_insert_zone(ix, n, (x0, 0.), (n as f32 * gap + END_W - x0, AXIS), true, true, t, cx));
        }
        for k in 0..n {
            axis = axis.child(div().absolute().left(px(x(k) - 11.)).top(px(AXIS / 2. - 11.)).child(self.tl_dot(ix, k, true, t, cx)));
        }
        div().flex().flex_col().child(top).child(axis).child(bottom)
    }

    /// 垂直排列：每个节点一行，轴在中间。
    #[allow(clippy::too_many_arguments)]
    fn tl_vertical(&self, ix: usize, items: &[TimelineItem], same: bool, hide: u8, focused: bool, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let n = items.len();
        let mut col = div().flex().flex_col().w(px(SIDE_W * 2. + AXIS));
        for (k, it) in items.iter().enumerate() {
            let card = self.tl_card(ix, k, it, hide, focused, t, cx);
            let time = self.tl_time(ix, k, it, hide, focused, t, window, cx);
            // 交替：偶数节点卡片在左；同侧：时间在左、卡片在右
            let (l, r) = if !same && k % 2 == 0 { (card, time) } else { (time, card) };
            let (from, to) = (if k == 0 { 0.5 } else { 0. }, if k + 1 == n { 0.5 } else { 1. });
            col = col.child(
                div()
                    .relative()
                    .flex()
                    .flex_row()
                    .child(div().w(px(SIDE_W)).flex_none().py(px(5.)).flex().flex_col().items_end().justify_center().children(l))
                    .child(
                        div()
                            .w(px(AXIS))
                            .flex_none()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(dashed(t.border_strong, false, from, to))
                            .child(self.tl_dot(ix, k, false, t, cx)),
                    )
                    .child(div().w(px(SIDE_W)).flex_none().py(px(5.)).flex().flex_col().items_start().justify_center().children(r))
                    .when(k + 1 < n, |d| d.child(self.tl_insert_zone(ix, k + 1, (SIDE_W, -1.), (AXIS, 0.), false, false, t, cx))),
            );
        }
        col.child(div().relative().h(px(26.)).child(self.tl_insert_zone(ix, n, (SIDE_W, 2.), (AXIS, 22.), false, true, t, cx)))
    }

    /// 彩色卡片：标题 + 详情（两者都隐藏时没有卡片）。
    #[allow(clippy::too_many_arguments)]
    fn tl_card(&self, ix: usize, k: usize, it: &TimelineItem, hide: u8, focused: bool, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        if hide & (TL_HIDE_TITLE | TL_HIDE_DETAIL) == TL_HIDE_TITLE | TL_HIDE_DETAIL {
            return None;
        }
        let (bg, _) = colors(k, t);
        Some(
            div()
                .w(px(CARD_W))
                .p(px(8.))
                .rounded(px(6.))
                .bg(bg)
                .flex()
                .flex_col()
                .gap(px(2.))
                .when(hide & TL_HIDE_TITLE == 0, |d| d.child(self.tl_field(ix, k * TL_SLOTS, &it.title, focused, t, cx)))
                .when(hide & TL_HIDE_DETAIL == 0, |d| d.child(self.tl_field(ix, k * TL_SLOTS + 1, &it.detail, focused, t, cx))),
        )
    }

    /// 时间标签：按文字宽度收紧，好在轴上居中 / 靠右对齐。
    #[allow(clippy::too_many_arguments)]
    fn tl_time(&self, ix: usize, k: usize, it: &TimelineItem, hide: u8, focused: bool, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        if hide & TL_HIDE_TIME != 0 {
            return None;
        }
        let text = if it.time.is_empty() { "输入时间" } else { it.time.text.as_str() };
        let mut m = GpuiMeasure { window };
        let w = text.split('\n').map(|l| m.width(l, 12., false)).fold(0., f32::max) + 14.;
        Some(div().w(px(w.min(CARD_W))).child(self.tl_field(ix, k * TL_SLOTS + 2, &it.time, focused, t, cx)))
    }

    /// 一个可编辑字段：编辑中描蓝框。
    fn tl_field(&self, ix: usize, cell: usize, rt: &crate::model::RichText, focused: bool, t: &Theme, cx: &mut Context<Self>) -> Div {
        let editing = focused && self.core.head.slot() == (ix, cell);
        let (size, line_h, weight, color, ph) = match cell % TL_SLOTS {
            0 => (14., 22., FontWeight::SEMIBOLD, t.text, "输入标题"),
            1 => (12., 18., FontWeight::NORMAL, t.text_secondary, "输入详情"),
            _ => (12., 18., FontWeight::NORMAL, t.text_muted, "输入时间"),
        };
        let spec = SlotSpec { size, line_h, weight, color, placeholder: Some(ph.into()), ..SlotSpec::body(t) };
        div()
            .px(px(4.))
            .rounded(px(3.))
            .border_1()
            .border_color(if editing { t.accent } else { gpui::transparent_black() })
            .when(editing, |d| d.bg(t.panel_bg))
            .child(self.render_slot(ix, cell, rt, spec, focused, t))
            .on_mouse_down(MouseButton::Left, self.slot_click(ix, cell, cx))
    }

    /// 轴上的圆点；悬停放大并在旁边出删除按钮（水平在右侧，垂直在上方）。
    fn tl_dot(&self, ix: usize, k: usize, horizontal: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, c) = colors(k, t);
        let (w, h, dot_at, del_at) = if horizontal { (46., 22., (0., 0.), (22., 0.)) } else { (22., 46., (0., 24.), (0., 0.)) };
        div()
            .id(("tl-dot", k))
            .group("tl-dot")
            .relative()
            .w(px(w))
            .h(px(h))
            .when(!horizontal, |d| d.mt(px(-24.)))
            .child(
                div()
                    .absolute()
                    .left(px(dot_at.0))
                    .top(px(dot_at.1))
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(8.)).rounded_full().bg(c).group_hover("tl-dot", |s| s.size(px(12.)).border_2().border_color(t.bg))),
            )
            .child(
                div()
                    .id(("tl-del", k))
                    .absolute()
                    .left(px(del_at.0))
                    .top(px(del_at.1))
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .invisible()
                    .group_hover("tl-dot", |s| s.visible())
                    .hover(|s| s.bg(t.hover))
                    .child(icon("trash", 12., t.text_secondary))
                    .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("删除节点").into())).into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.tl_remove(ix, k, cx);
                        }),
                    ),
            )
    }

    /// 插入热区：悬停时轴线变蓝实线、中间（末端在最右）出「+」，点击在 `at` 处插入节点。
    #[allow(clippy::too_many_arguments)]
    fn tl_insert_zone(
        &self,
        ix: usize,
        at: usize,
        pos: (f32, f32),
        sz: (f32, f32),
        horizontal: bool,
        end: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (w, h) = sz;
        // 垂直排列两节点之间：热区跨在两行的分界上
        let (w, h, y) = if !horizontal && !end { (w, 18., pos.1 - 9.) } else { (w, h, pos.1) };
        let plus_at = if horizontal { (if end { w - 18. } else { w / 2. - 9. }, h / 2. - 9.) } else { (w / 2. - 9., if end { h - 18. } else { h / 2. - 9. }) };
        let line = if horizontal {
            div().absolute().left_0().top(px(h / 2. - 1.)).w(px(if end { w - 9. } else { w })).h(px(2.))
        } else {
            div().absolute().top_0().left(px(w / 2. - 1.)).h(px(if end { h - 9. } else { h })).w(px(2.))
        };
        div()
            .id(("tl-ins", at))
            .group("tl-ins")
            .absolute()
            .left(px(pos.0))
            .when(!horizontal && !end, |d| d.bottom(px(-9.)))
            .when(horizontal || end, |d| d.top(px(y)))
            .w(px(w))
            .h(px(h))
            .cursor_pointer()
            .child(line.bg(t.accent).invisible().group_hover("tl-ins", |s| s.visible()))
            .child(
                div()
                    .absolute()
                    .left(px(plus_at.0))
                    .top(px(plus_at.1))
                    .size(px(18.))
                    .rounded_full()
                    .bg(t.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .invisible()
                    .group_hover("tl-ins", |s| s.visible())
                    .child(icon("plus", 12., gpui::white())),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.tl_insert(ix, at, cx);
                }),
            )
    }

    /// 右上角：排列方式、显示字段（悬停块或光标在块内时显示）。
    fn tl_toolbar(&self, ix: usize, vertical: bool, hide: u8, in_block: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = self.pop.tl_menu.as_ref().filter(|m| m.block == ix).map(|m| (m.kind, m.sub));
        let pop = menu.map(|(kind, sub)| match kind {
            TlMenuKind::Layout => self.tl_layout_menu(ix, sub, t, cx).into_any_element(),
            TlMenuKind::Fields => self.tl_fields_menu(ix, hide, t, cx).into_any_element(),
        });
        let btn = |key: &'static str, name: &'static str, tip: &'static str, kind: TlMenuKind| {
            div()
                .id((key, ix))
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .when(menu.is_some_and(|m| m.0 == kind), |d| d.bg(t.hover))
                .hover(|s| s.bg(t.hover))
                .child(icon(name, 15., t.text_secondary))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let same = this.pop.tl_menu.as_ref().is_some_and(|m| m.block == ix && m.kind == kind);
                        this.pop.tl_menu = (!same).then_some(TlMenu { block: ix, kind, sub: None });
                        cx.notify();
                    }),
                )
        };
        let (mut lay_pop, mut fld_pop) = (None, None);
        match menu.map(|m| m.0) {
            Some(TlMenuKind::Layout) => lay_pop = pop,
            Some(TlMenuKind::Fields) => fld_pop = pop,
            None => {}
        }
        let anchor = |p: Option<AnyElement>| div().absolute().top(px(30.)).right_0().children(p);
        div()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .flex()
            .flex_row()
            .gap(px(2.))
            .when(menu.is_none() && !in_block, |d| d.invisible().group_hover("tl", |s| s.visible()))
            .child(
                div()
                    .relative()
                    .child(btn("tl-layout", if vertical { "arrow-down" } else { "arrow-right" }, "排列方式", TlMenuKind::Layout))
                    .child(anchor(lay_pop)),
            )
            .child(div().relative().child(btn("tl-fields", "settings", "显示设置", TlMenuKind::Fields)).child(anchor(fld_pop)))
    }

    fn tl_menu_shell(&self, ix: usize, kind: TlMenuKind, w: f32, t: &Theme, cx: &mut Context<Self>, body: Div) -> impl IntoElement {
        deferred(
            anchored().anchor(Corner::TopRight).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("tl-menu")
                    .occlude()
                    .w(px(w))
                    .p(px(4.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .child(body)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // 推迟到本次点击分发完再关：点在工具栏按钮上时由按钮自己切换
                    .on_mouse_down_out(cx.listener(move |_, _, window, cx| {
                        cx.defer_in(window, move |this, _, cx| {
                            if this.pop.tl_menu.as_ref().is_some_and(|m| m.block == ix && m.kind == kind) {
                                this.pop.tl_menu = None;
                                cx.notify();
                            }
                        })
                    })),
            ),
        )
        .with_priority(4)
    }

    fn tl_menu_item(key: &'static str, t: &Theme) -> Stateful<Div> {
        div()
            .id(key)
            .h(px(32.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .text_size(px(13.))
            .text_color(t.text)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
    }

    /// 排列方式：水平 / 垂直，悬停在左侧展开「交替 / 同侧」。
    fn tl_layout_menu(&self, ix: usize, sub: Option<bool>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (cur_v, cur_same) = match self.content.blocks.get(ix).map(|b| &b.kind) {
            Some(BlockKind::Timeline { vertical, same_side, .. }) => (*vertical, *same_side),
            _ => (false, false),
        };
        let mut body = div().relative().flex().flex_col();
        for (row, (v, name, ic)) in [(false, "水平排列", "arrow-right"), (true, "垂直排列", "arrow-down")].into_iter().enumerate() {
            body = body.child(
                Self::tl_menu_item(name, t)
                    .when(sub == Some(v), |d| d.bg(t.hover))
                    .child(icon(ic, 14., t.text_secondary))
                    .child(div().flex_1().child(name))
                    .child(icon("chevron-right", 13., t.text_muted))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            if let Some(m) = this.pop.tl_menu.as_mut().filter(|m| m.sub != Some(v)) {
                                m.sub = Some(v);
                                cx.notify();
                            }
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(m) = this.pop.tl_menu.as_mut() {
                                m.sub = Some(v);
                            }
                            cx.notify();
                        }),
                    ),
            );
            if sub == Some(v) {
                let subm = div()
                    .id("tl-sub")
                    .absolute()
                    .top(px(row as f32 * 32. - 4.))
                    .left(px(-112.))
                    .w(px(104.))
                    .p(px(4.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .children([(false, "交替"), (true, "同侧")].into_iter().map(|(s, label)| {
                        let on = cur_v == v && cur_same == s;
                        Self::tl_menu_item(label, t).child(div().flex_1().child(label)).when(on, |d| d.child(icon("check", 14., t.accent))).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.pop.tl_menu = None;
                                this.update_kind(
                                    ix,
                                    |k| {
                                        if let BlockKind::Timeline { vertical, same_side, .. } = k {
                                            (*vertical, *same_side) = (v, s);
                                        }
                                    },
                                    cx,
                                );
                            }),
                        )
                    }));
                body = body.child(subm);
            }
        }
        self.tl_menu_shell(ix, TlMenuKind::Layout, 140., t, cx, body)
    }

    /// 显示设置：标题 / 详情 / 时间，点眼睛切换显示。
    fn tl_fields_menu(&self, ix: usize, hide: u8, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let body =
            div().flex().flex_col().children([("标题", TL_HIDE_TITLE), ("详情", TL_HIDE_DETAIL), ("时间", TL_HIDE_TIME)].into_iter().map(|(name, bit)| {
                let shown = hide & bit == 0;
                Self::tl_menu_item(name, t)
                    .child(div().flex_1().child(name))
                    .child(icon(if shown { "eye" } else { "eye-off" }, 14., if shown { t.text_secondary } else { t.text_muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.tl_toggle_field(ix, bit, cx);
                        }),
                    )
            }));
        self.tl_menu_shell(ix, TlMenuKind::Fields, 140., t, cx, body)
    }

    fn tl_hide(&self, ix: usize) -> u8 {
        match self.content.blocks.get(ix).map(|b| &b.kind) {
            Some(BlockKind::Timeline { hide, .. }) => *hide,
            _ => 0,
        }
    }

    fn tl_insert(&mut self, ix: usize, at: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        let hide = self.tl_hide(ix);
        if let BlockKind::Timeline { items, .. } = &mut self.block_mut(ix).kind {
            items.insert(at.min(items.len()), TimelineItem::default());
        }
        self.core.head = Pos::new(ix, at * TL_SLOTS + first_visible(hide), 0);
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    fn tl_remove(&mut self, ix: usize, k: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        let mut empty = false;
        if let BlockKind::Timeline { items, .. } = &mut self.block_mut(ix).kind {
            if k < items.len() {
                items.remove(k);
            }
            empty = items.is_empty();
        }
        if empty {
            self.remove_blocks(ix..ix + 1);
            self.core.head = self.clamp(Pos::new(ix, 0, 0));
        } else {
            let hide = self.tl_hide(ix);
            let k = k.min(self.content.blocks[ix].slot_count() / TL_SLOTS - 1);
            self.core.head = Pos::new(ix, k * TL_SLOTS + first_visible(hide), 0);
        }
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    fn tl_toggle_field(&mut self, ix: usize, bit: u8, cx: &mut Context<Self>) {
        let hide = self.tl_hide(ix) ^ bit;
        if hide & 7 == 7 {
            cx.emit(EditorEvent::Toast("至少保留一项".into()));
            return;
        }
        self.update_kind(
            ix,
            |k| {
                if let BlockKind::Timeline { hide: h, .. } = k {
                    *h = hide;
                }
            },
            cx,
        );
        // 光标所在字段被隐藏：挪到同节点第一个可见字段
        for p in [&mut self.core.head, &mut self.core.anchor] {
            if p.block == ix && hide & (1 << (p.cell % TL_SLOTS)) != 0 {
                *p = Pos::new(ix, p.cell / TL_SLOTS * TL_SLOTS + first_visible(hide), 0);
            }
        }
        cx.notify();
    }
}
