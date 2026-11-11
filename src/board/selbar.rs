//! 浮动属性栏（选中元素上方，按元素类型变化画板）、各个下拉面板、右键菜单、图形选择器。

use super::model::*;
use super::ui::{panel, shape_preview, BOARD_SHAPES};
use super::view::*;
use crate::canvas::paint::{paint_scene, View};
use crate::canvas::{pt, Arrow, Col, Item, LineItem, Scene, ShapeKind, DECO_ITALIC, DECO_STRIKE, DECO_UNDERLINE};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

/// 线条 / 边框颜色（画板：上排黑白灰，下排彩色）
pub const STROKE_COLORS: [u32; 12] = [0x1f2329, 0x646a73, 0x8f959e, 0xbbbfc4, 0xdee0e3, 0xffffff, 0x7f3bf5, 0x3370ff, 0x14c0ff, 0x34c724, 0xffc60a, 0xf54a45];
/// 填充色（上排浅色，下排深色），None 为无填充
pub const FILL_COLORS: [Option<u32>; 18] = [
    None,
    Some(0xffffff),
    Some(0xf2f3f5),
    Some(0xece2fe),
    Some(0xe1eaff),
    Some(0xd9f3fd),
    Some(0xd9f5d6),
    Some(0xfff5cc),
    Some(0xfde2e2),
    Some(0x1f2329),
    Some(0x646a73),
    Some(0xbbbfc4),
    Some(0x7f3bf5),
    Some(0x3370ff),
    Some(0x245bdb),
    Some(0x34c724),
    Some(0xd9a200),
    Some(0xf54a45),
];
/// 文字颜色
const TEXT_COLORS: [u32; 12] = [0x1f2329, 0x646a73, 0xbbbfc4, 0xffffff, 0x7f3bf5, 0x3370ff, 0x14c0ff, 0x34c724, 0xffc60a, 0xff8800, 0xf54a45, 0xf14ba9];
/// 字号（画板的字号下拉）
const FONT_SIZES: [f32; 12] = [12., 14., 16., 18., 20., 24., 28., 36., 48., 64., 72., 96.];
/// 端点样式（常用 7 种，其余收在「更多」里）
const ARROWS: [(Arrow, &str); 7] = [
    (Arrow::None, "无"),
    (Arrow::Open, "线形箭头"),
    (Arrow::Filled, "实心箭头"),
    (Arrow::Hollow, "空心三角"),
    (Arrow::DiamondHollow, "空心菱形"),
    (Arrow::DiamondFilled, "实心菱形"),
    (Arrow::Circle, "圆点"),
];
const MORE_ARROWS: [(Arrow, &str); 5] =
    [(Arrow::Cross, "叉号"), (Arrow::One, "一"), (Arrow::ZeroOrOne, "零或一"), (Arrow::OneOrMany, "一或多"), (Arrow::ZeroOrMany, "零或多")];
/// 连线类型（「连线样式」面板第一行）
const ROUTES: [(Route, &str, &str); 4] = [
    (Route::Elbow, "conn-elbow", "直角连线"),
    (Route::RoundElbow, "conn-round-elbow", "圆角直角连线"),
    (Route::Straight, "conn-straight", "直线"),
    (Route::Curve, "conn-curve", "曲线"),
];

/// 选区的类型（决定属性栏内容）。
#[derive(Clone, Copy, PartialEq)]
enum SelKind {
    Shape,
    Sticky,
    Text,
    Connector,
    Image,
    Icon,
    Pen,
    Section,
    Table,
    Mixed,
}

fn line_preview(start: Arrow, end: Arrow, dash: bool, dotted: bool, w: f32, color: Hsla) -> impl IntoElement {
    let mut l = LineItem::new(vec![pt(4., 9.), pt(52., 9.)]);
    l.stroke = Col::from_hsla(color);
    l.width = w;
    l.start = start;
    l.end = end;
    l.dash = dash;
    l.dotted = dotted;
    let scene = Arc::new(Scene { items: vec![Item::Line(l)], width: 56., height: 18., ..Default::default() });
    div()
        .w(px(56.))
        .h(px(18.))
        .flex_none()
        .child(canvas(|b, _, _| b, move |b, _, win, cx| paint_scene(&scene, View { origin: b.origin, scale: 1. }, b, win, cx)).size_full())
}

impl BoardEditor {
    fn sel_kind(&self) -> Option<SelKind> {
        let els: Vec<&El> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).collect();
        let first = els.first()?;
        let k = |e: &El| match e.kind {
            ElKind::Shape(_) => SelKind::Shape,
            ElKind::Sticky => SelKind::Sticky,
            ElKind::Text => SelKind::Text,
            ElKind::Connector { .. } => SelKind::Connector,
            ElKind::Image { .. } | ElKind::Path { .. } => SelKind::Image,
            ElKind::Icon { .. } => SelKind::Icon,
            ElKind::Pen { .. } => SelKind::Pen,
            ElKind::Section => SelKind::Section,
            ElKind::Table { .. } => SelKind::Table,
        };
        let k0 = k(first);
        if els.iter().all(|e| k(e) == k0) && !(els.len() > 1 && matches!(k0, SelKind::Image | SelKind::Section | SelKind::Table)) {
            Some(k0)
        } else {
            Some(SelKind::Mixed)
        }
    }

    /// 工具栏显示的文字样式：编辑中选中了部分文字时取选中文字的样式，否则取元素整体样式。
    fn shown_fmt(&self, e: &El, cx: &App) -> (bool, u8, Option<Col>) {
        if let Some(ed) = self.editing.as_ref().filter(|ed| ed.id == e.id && ed.part == Part::Main) {
            let a = ed.area.read(cx);
            if !a.selection().is_empty() {
                let f = a.fmt_at_selection();
                return (f.bold, f.deco, f.color.or(e.text_color));
            }
        }
        if let Some(i) = self.shown_cell(e) {
            return (e.cell_bold(i), e.cell_deco(i), e.cell_fmt(i).color.or(e.text_color));
        }
        (e.bold, e.deco, e.text_color)
    }

    /// 选中了表格的某个单元格时，工具栏显示 / 修改该单元格的文字样式。
    fn shown_cell(&self, e: &El) -> Option<usize> {
        let (id, r, c) = self.cur_cell()?;
        (id == e.id).then(|| r * e.grid().map(|g| g.ncols()).unwrap_or(1) + c)
    }

    /// 工具栏显示的字号与水平 / 垂直对齐（0 左/顶、1 中、2 右/底）。
    fn shown_layout(&self, e: &El) -> (f32, u8, u8) {
        match self.shown_cell(e) {
            Some(i) => {
                let a = e.cell_align(i);
                (
                    e.cell_font(i),
                    a.h(),
                    if a.is_top() {
                        0
                    } else if a.is_bottom() {
                        2
                    } else {
                        1
                    },
                )
            }
            None => (e.font_size, e.h_align(), e.v_align_eff()),
        }
    }

    fn first_sel(&self) -> Option<&El> {
        self.doc.els.iter().find(|e| self.sel.contains(&e.id))
    }

    /// 浮动属性栏 + 当前展开的下拉面板。
    pub(super) fn render_selbar(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        let mut bar_bottom = None;
        // 原位编辑文字时也显示属性栏，只保留文字相关的按钮（选中图形里的文字时弹出文字格式栏）。
        // 连线说明、分区标题的文字样式不可调，编辑时不显示。
        let text_edit = self.editing.as_ref().is_some_and(|ed| self.sel.len() == 1 && self.sel.contains(&ed.id));
        let editing_ok =
            self.editing.is_none() || (text_edit && matches!(self.sel_kind(), Some(SelKind::Shape | SelKind::Sticky | SelKind::Text | SelKind::Table)));
        let show = !self.sel.is_empty()
            && matches!(self.tool, Tool::Select | Tool::Hand)
            && self.drag.is_none()
            && editing_ok
            && self.menu.is_none()
            && self.picker.is_none()
            && self.templates.is_none()
            && self.code_dlg.is_none();
        if let (true, Some(kind), Some(b)) = (show, self.sel_kind(), self.selection_bounds()) {
            let r = self.view().r(b);
            let cb = self.canvas_bounds;
            // 放在选区上方，空间不够时放到下方
            // 与选区留出快捷创建圆点的位置
            // 表格上方还有插入行列的轨道和「插入列」提示，再多留一些
            // 编辑文字时没有快捷创建圆点，靠近一些
            let gap = if self.single_table().is_some() {
                px(98.)
            } else if text_edit {
                px(56.)
            } else {
                px(72.)
            };
            let above = r.top() - gap > cb.top() + px(60.);
            let top = if above { r.top() - gap } else { r.bottom() + px(32.) };
            bar_bottom = Some(top + px(44.));
            let cxm = r.center().x - cb.origin.x;
            let w = f32::from(cb.size.width);
            let left = (f32::from(cxm) - 450.).clamp(8. - 450. + 200., w - 450. - 200.);
            out.push(
                div()
                    .absolute()
                    .top(top - cb.origin.y)
                    .left(px(left))
                    .w(px(900.))
                    .flex()
                    .justify_center()
                    .child(self.render_bar(kind, text_edit, t, cx))
                    .into_any_element(),
            );
        }
        if let Some(p) = self.render_popover(bar_bottom, t, window, cx) {
            out.push(p);
        }
        out
    }

    fn bar_btn(&self, id: &'static str, tip: &'static str, on: bool, t: &Theme) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(32.))
            .min_w(px(32.))
            .px(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(t.text)
            .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
            .hover(|s| s.bg(t.hover))
            .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
    }

    /// 带 ▾ 的下拉触发按钮：悬停即展开，移开后自动收起；点击则固定展开 / 收起。
    fn drop_btn(&self, id: &'static str, tip: &'static str, pop: Popover, content: AnyElement, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let open = self.popover == Some(pop);
        self.bar_btn(id, tip, open, t)
            .child(content)
            .child(icon("chevron-down", 11., t.text_muted))
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| this.hover_drop_btn(pop, *hovered, window, cx)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if this.popover == Some(pop) && this.pop_by_hover {
                    // 悬停展开后再点击：固定住，不再随鼠标移开收起
                    this.pop_by_hover = false;
                    cx.notify();
                } else {
                    this.pop_by_hover = false;
                    this.toggle_popover(pop, ev.position(), cx)
                }
            }))
    }

    fn hover_drop_btn(&mut self, pop: Popover, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.pop_btn_hover = hovered;
        if !hovered {
            self.schedule_pop_close(cx);
            return;
        }
        if self.popover != Some(pop) && self.menu.is_none() {
            self.flyout = None;
            self.popover = Some(pop);
            self.pop_at = window.mouse_position();
            self.pop_by_hover = true;
            self.pop_panel_hover = false;
            cx.notify();
        }
    }

    /// 鼠标离开按钮和下拉面板一小段时间后收起（留出从按钮移到面板的时间）。
    pub(super) fn schedule_pop_close(&mut self, cx: &mut Context<Self>) {
        if !self.pop_by_hover {
            return;
        }
        self.pop_close_gen += 1;
        let gen = self.pop_close_gen;
        cx.spawn(async move |this: WeakEntity<BoardEditor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(220)).await;
            this.update(cx, |this, cx| {
                if this.pop_close_gen == gen && this.pop_by_hover && !this.pop_btn_hover && !this.pop_panel_hover && this.popover.is_some() {
                    this.popover = None;
                    this.pop_by_hover = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn render_bar(&self, kind: SelKind, text_edit: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.first_sel().cloned();
        let sep = || div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(t.border);
        let fill_dot = |c: Option<Col>| {
            div()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(t.border_strong)
                .when_some(c, |d, c| d.bg(c.hsla()))
                .when(c.is_none(), |d| d.child(div().absolute().w(px(18.)).h(px(1.5)).bg(gpui::rgb(0xf54a45))))
        };
        let stroke_sq = |c: Col, none: bool| div().size(px(16.)).rounded(px(3.)).border_2().border_color(if none { t.border } else { c.hsla() });
        let font_label = |e: &El| if e.auto_font && matches!(e.kind, ElKind::Sticky) { "自动".to_string() } else { format!("{:.0}", self.shown_layout(e).0) };
        let text_a = |c: Option<Col>| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(div().text_size(px(14.)).line_height(px(14.)).child("A"))
                .child(div().w(px(14.)).h(px(3.)).rounded(px(1.)).bg(c.map(|c| c.hsla()).unwrap_or(t.text)))
        };
        // 点属性栏不把焦点抢回画布：编辑文字时改完样式可以接着打字
        let mut bar = panel(t)
            .h(px(44.))
            .px(px(4.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(1.))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
        let Some(e) = first else { return bar };
        let locked = self.doc.els.iter().filter(|x| self.sel.contains(&x.id)).all(|x| x.locked);
        let align_icon = match self.shown_layout(&e).1 {
            0 => "text-left",
            2 => "text-right",
            _ => "text-center",
        };
        let (shown_bold, shown_deco, shown_color) = self.shown_fmt(&e, cx);
        // 文本格式按钮（「A≡」：加粗 / 斜体 / 下划线 / 删除线与对齐都收在里面）
        let fmt_on = shown_bold || shown_deco != 0;
        let fmt_icon = div()
            .flex()
            .items_end()
            .gap(px(1.))
            .child(div().text_size(px(14.)).line_height(px(15.)).when(fmt_on, |d| d.text_color(t.accent)).child("A"))
            .child(icon(align_icon, 10., if fmt_on { t.accent } else { t.text }));
        let text_group = |bar: Div, cx: &mut Context<Self>| {
            bar.child(self.drop_btn("sb-tc", "文字颜色", Popover::TextColor, text_a(shown_color).into_any_element(), t, cx))
                .child(self.drop_btn("sb-fs", "字号", Popover::FontSize, div().min_w(px(22.)).child(font_label(&e)).into_any_element(), t, cx))
                .child(self.drop_btn("sb-al", "文本格式", Popover::Align, fmt_icon.into_any_element(), t, cx))
        };
        // 编辑文字中：只有文字颜色、字号、文本格式
        if text_edit {
            return text_group(bar, cx);
        }
        // 思维导图节点：子主题、布局、收起
        if e.is_mind() && self.sel.len() == 1 {
            let has_kids = !self.doc.mind_children(e.id).is_empty();
            let collapsed = e.mind.as_ref().is_some_and(|m| m.collapsed);
            bar = bar
                .child(self.bar_btn("sb-mchild", "添加子主题 Tab", false, t).child(icon("plus", 14., t.text)).child("子主题").on_click(cx.listener(
                    |this, _, w, cx| {
                        if let Some(id) = this.single_mind().map(|e| e.id) {
                            this.mind_add_child(id, 0, w, cx);
                        }
                    },
                )))
                .child(self.drop_btn("sb-mlayout", "布局与分支线", Popover::MindLayout, icon("mindmap", 15., t.text).into_any_element(), t, cx))
                .when(has_kids, |b| {
                    b.child(
                        self.bar_btn("sb-mfold", if collapsed { "展开子节点 ⌘." } else { "收起子节点 ⌘." }, collapsed, t)
                            .child(icon(if collapsed { "chevron-right" } else { "chevron-down" }, 15., if collapsed { t.accent } else { t.text }))
                            .on_click(cx.listener(|this, _, _, cx| this.mind_toggle_collapse(cx))),
                    )
                })
                .child(sep());
        }
        match kind {
            SelKind::Shape if self.doc.els.iter().filter(|x| self.sel.contains(&x.id)).all(super::seq::is_bar) => {
                // 激活条：只有填充和边框
                bar = bar
                    .child(self.drop_btn(
                        "sb-fill",
                        "填充",
                        Popover::Fill,
                        fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                        t,
                        cx,
                    ))
                    .child(self.drop_btn("sb-border", "边框", Popover::Border, stroke_sq(e.stroke, e.no_stroke).into_any_element(), t, cx));
            }
            SelKind::Shape => {
                bar = bar
                    .child(self.drop_btn(
                        "sb-shape",
                        "切换图形",
                        Popover::ShapeKind,
                        shape_preview(e.shape_kind(), 20., 15., t.text, None).into_any_element(),
                        t,
                        cx,
                    ))
                    .child(sep())
                    .child(self.drop_btn(
                        "sb-fill",
                        "填充",
                        Popover::Fill,
                        fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                        t,
                        cx,
                    ))
                    .child(self.drop_btn("sb-border", "边框", Popover::Border, stroke_sq(e.stroke, e.no_stroke).into_any_element(), t, cx))
                    .child(sep());
                bar = text_group(bar, cx);
            }
            SelKind::Sticky => {
                bar = bar
                    .child(
                        self.drop_btn(
                            "sb-sticky",
                            "便签颜色",
                            Popover::StickyColor,
                            div()
                                .size(px(16.))
                                .rounded(px(3.))
                                .bg(e.fill.map(|c| c.hsla()).unwrap_or(t.hover))
                                .border_1()
                                .border_color(t.border_strong)
                                .into_any_element(),
                            t,
                            cx,
                        ),
                    )
                    .child(sep());
                bar = text_group(bar, cx);
                bar = bar.child(sep()).child(
                    self.bar_btn("sb-author", "显示作者", e.show_author, t)
                        .child(icon("users", 15., if e.show_author { t.accent } else { t.text }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_author(cx))),
                );
            }
            SelKind::Text => {
                bar = text_group(bar, cx);
                bar = bar.child(sep()).child(self.drop_btn(
                    "sb-fill",
                    "背景色",
                    Popover::Fill,
                    fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                    t,
                    cx,
                ));
            }
            SelKind::Connector => {
                bar = self.render_bar_connector(t, cx, bar, &e);
            }
            SelKind::Image => {}
            // 彩色素材（表情、头像、技术图标）不能改颜色
            SelKind::Icon if matches!(&e.kind, ElKind::Icon { name } if super::iconlib::is_color(name)) => {}
            SelKind::Icon => {
                bar = bar.child(self.drop_btn(
                    "sb-icol",
                    "图标颜色",
                    Popover::Border,
                    div().size(px(16.)).rounded_full().bg(e.stroke.hsla()).border_1().border_color(t.border_strong).into_any_element(),
                    t,
                    cx,
                ));
            }
            SelKind::Pen => {
                bar =
                    bar.child(self.drop_btn("sb-pc", "笔迹颜色", Popover::Border, stroke_sq(e.stroke, false).into_any_element(), t, cx)).child(self.drop_btn(
                        "sb-pw",
                        "粗细",
                        Popover::LineStyle,
                        div().w(px(18.)).h(px(e.stroke_w.clamp(1.5, 5.))).rounded_full().bg(t.text).into_any_element(),
                        t,
                        cx,
                    ));
            }
            SelKind::Table => {
                let (lane, head_row, head_col) = match &e.kind {
                    ElKind::Table { lane, head_row, head_col, .. } => (*lane, *head_row, *head_col),
                    _ => (false, false, false),
                };
                if lane {
                    bar = bar
                        .child(
                            self.bar_btn("sb-ltitle", if e.text.is_empty() { "添加泳道图标题" } else { "编辑标题（双击标题）" }, false, t)
                                .child(icon("type", 14., t.text))
                                .child(if e.text.is_empty() { "添加标题" } else { "标题" })
                                .on_click(cx.listener(|this, _, w, cx| this.lane_title(w, cx))),
                        )
                        .child(sep());
                }
                let cell_fill = self.cur_cell().and_then(|(_, r, c)| match &e.kind {
                    ElKind::Table { fills, cols, .. } => fills.get(r * cols.len() + c).copied().filter(|f| *f != 0).map(Col),
                    _ => None,
                });
                bar = bar
                    .child(self.drop_btn(
                        "sb-rows",
                        if lane { "插入 / 删除泳道与阶段" } else { "插入 / 删除行列" },
                        Popover::TableOps,
                        icon("row-below", 15., t.text).into_any_element(),
                        t,
                        cx,
                    ))
                    .child(
                        self.bar_btn("sb-hrow", if lane { "首行作为泳道名" } else { "标题行" }, head_row, t)
                            .child(icon("header-row", 15., if head_row { t.accent } else { t.text }))
                            .on_click(cx.listener(|this, _, _, cx| this.table_toggle_head(true, cx))),
                    )
                    .child(
                        self.bar_btn("sb-hcol", if lane { "首列作为泳道名" } else { "标题列" }, head_col, t)
                            .child(icon("header-col", 15., if head_col { t.accent } else { t.text }))
                            .on_click(cx.listener(|this, _, _, cx| this.table_toggle_head(false, cx))),
                    )
                    .child(sep())
                    .child(
                        self.drop_btn(
                            "sb-fill",
                            if self.cur_cell().is_some() { "单元格填充" } else { "填充" },
                            Popover::Fill,
                            fill_dot(if self.cur_cell().is_some() { cell_fill } else { e.fill })
                                .relative()
                                .flex()
                                .items_center()
                                .justify_center()
                                .into_any_element(),
                            t,
                            cx,
                        ),
                    )
                    .child(self.drop_btn("sb-border", "边框", Popover::Border, stroke_sq(e.stroke, e.no_stroke).into_any_element(), t, cx))
                    .child(sep());
                bar = text_group(bar, cx);
            }
            SelKind::Section if e.code.is_some() => {
                bar = self.render_bar_section(t, cx, bar, e.clone());
            }
            SelKind::Section => {
                bar = bar
                    .child(self.bar_btn("sb-rename", "重命名标题（双击标题）", false, t).child(icon("pencil", 14., t.text)).child("重命名").on_click(
                        cx.listener(|this, _, w, cx| {
                            if let Some(id) = this.single().map(|e| e.id) {
                                this.start_edit(id, w, cx);
                            }
                        }),
                    ))
                    .child(self.drop_btn(
                        "sb-fill",
                        "背景色",
                        Popover::Fill,
                        fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                        t,
                        cx,
                    ))
                    .child(
                        self.bar_btn("sb-unsec", "移除分区（保留分区内的内容）", false, t)
                            .child("移除分区")
                            .on_click(cx.listener(|this, _, _, cx| this.remove_section(cx))),
                    );
            }
            SelKind::Mixed => {
                let has_box = self.doc.els.iter().any(|x| self.sel.contains(&x.id) && matches!(x.kind, ElKind::Shape(_)));
                let grouped = self.doc.els.iter().filter(|x| self.sel.contains(&x.id)).all(|x| x.group.is_some() && x.group == e.group);
                bar = bar.child(self.drop_btn("sb-arr", "对齐与排版", Popover::Arrange, icon("align-h-left", 16., t.text).into_any_element(), t, cx)).when(
                    has_box,
                    |b| {
                        b.child(sep())
                            .child(self.drop_btn(
                                "sb-fill",
                                "填充",
                                Popover::Fill,
                                fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                                t,
                                cx,
                            ))
                            .child(self.drop_btn("sb-border", "边框", Popover::Border, stroke_sq(e.stroke, e.no_stroke).into_any_element(), t, cx))
                    },
                );
                bar = bar.child(sep()).child(
                    self.bar_btn("sb-group", if grouped { "取消组合 ⇧⌘G" } else { "组合 ⌘G" }, grouped, t)
                        .child(icon("group", 15., if grouped { t.accent } else { t.text }))
                        .on_click(cx.listener(move |this, _, w, cx| if grouped { this.ungroup(&BUngroup, w, cx) } else { this.group(&BGroup, w, cx) })),
                );
            }
        }
        let painting = self.painting.is_some();
        // 图片和彩色素材没有可刷的样式
        let plain = kind == SelKind::Image || matches!(&e.kind, ElKind::Icon { name } if super::iconlib::is_color(name));
        // 代码绘图只留「更多」（锁定在更多菜单里）
        let code = kind == SelKind::Section && e.code.is_some();
        bar.when(!plain, |b| b.child(sep()))
            .when(!plain && !code, |b| {
                b.child(
                    self.bar_btn("sb-paint", "格式刷（双击可连续使用，Esc 退出）· 复制样式 ⌥⌘C", painting, t)
                        .child(icon("paintbrush", 15., if painting { t.accent } else { t.text }))
                        .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                            if this.painting.is_some() && ev.click_count() < 2 {
                                this.painting = None;
                                cx.notify();
                            } else {
                                this.start_painter(ev.click_count() >= 2, cx);
                            }
                        })),
                )
            })
            .when(!code, |b| {
                b.child(
                    self.bar_btn("sb-lock", if locked { "解锁 ⇧⌘L" } else { "锁定 ⇧⌘L" }, locked, t)
                        .child(icon(if locked { "lock" } else { "unlock" }, 15., if locked { t.accent } else { t.text }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_lock(cx))),
                )
            })
            .child(
                self.bar_btn("sb-more", "更多", self.popover == Some(Popover::More), t)
                    .child(icon("more", 16., t.text))
                    .on_hover(cx.listener(|this, hovered: &bool, window, cx| this.hover_drop_btn(Popover::More, *hovered, window, cx)))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        if this.popover == Some(Popover::More) && this.pop_by_hover {
                            this.pop_by_hover = false;
                            cx.notify();
                        } else {
                            this.pop_by_hover = false;
                            this.toggle_popover(Popover::More, ev.position(), cx)
                        }
                    })),
            )
    }
    /// 从 `render_bar` 拆出。
    fn render_bar_connector(&self, t: &Theme, cx: &mut Context<Self>, mut bar: Div, e: &El) -> Div {
        let sep = || div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(t.border);
        let (route, start, end) = match &e.kind {
            ElKind::Connector { route, start, end, .. } => (*route, *start, *end),
            _ => (Route::Straight, Arrow::None, Arrow::None),
        };
        let route_icon = ROUTES.iter().find(|r| r.0 == route).map(|r| r.1).unwrap_or("conn-straight");
        // 起点箭头 · 箭头方向 · 终点箭头 | 颜色 · 连线样式（类型 / 线型 / 粗细 / 跨线）| 连线说明
        let short = |a: Arrow, at_end: bool| {
            let mut l = LineItem::new(vec![pt(3., 9.), pt(23., 9.)]);
            l.stroke = Col::from_hsla(t.text);
            l.width = 1.5;
            (l.start, l.end) = if at_end { (Arrow::None, a) } else { (a, Arrow::None) };
            let scene = Arc::new(Scene { items: vec![Item::Line(l)], width: 26., height: 18., ..Default::default() });
            div()
                .w(px(26.))
                .h(px(18.))
                .flex_none()
                .child(canvas(|b, _, _| b, move |b, _, win, cx| paint_scene(&scene, View { origin: b.origin, scale: 1. }, b, win, cx)).size_full())
        };
        bar = bar
            .child(self.drop_btn("sb-start", "起点", Popover::StartArrow, short(start, false).into_any_element(), t, cx))
            .child(
                self.bar_btn("sb-swap", "箭头方向", false, t).child(icon("rotate", 14., t.text)).on_click(cx.listener(|this, _, _, cx| this.swap_arrows(cx))),
            )
            .child(self.drop_btn("sb-end", "终点", Popover::EndArrow, short(end, true).into_any_element(), t, cx))
            .child(sep())
            .child(self.drop_btn(
                "sb-lc",
                "颜色",
                Popover::Border,
                div().size(px(16.)).rounded_full().border_2().border_color(e.stroke.hsla()).into_any_element(),
                t,
                cx,
            ))
            .child(self.drop_btn("sb-ls", "连线样式", Popover::LineStyle, icon(route_icon, 16., t.text).into_any_element(), t, cx))
            .child(sep())
            .child(
                self.bar_btn("sb-cap", "添加连线说明 ↵", false, t)
                    .child(div().text_size(px(11.)).line_height(px(11.)).mr(px(-3.)).child("+"))
                    .child(icon("type", 15., t.text))
                    .on_click(cx.listener(|this, _, w, cx| {
                        if let Some(id) = this.single().map(|e| e.id) {
                            this.start_edit(id, w, cx);
                        }
                    })),
            );
        bar
    }

    /// 从 `render_bar` 拆出。
    fn render_bar_section(&self, t: &Theme, cx: &mut Context<Self>, mut bar: Div, e: El) -> Div {
        let sep = || div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(t.border);
        let fill_dot = |c: Option<Col>| {
            div()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(t.border_strong)
                .when_some(c, |d, c| d.bg(c.hsla()))
                .when(c.is_none(), |d| d.child(div().absolute().w(px(18.)).h(px(1.5)).bg(gpui::rgb(0xf54a45))))
        };
        let stroke_sq = |c: Col, none: bool| div().size(px(16.)).rounded(px(3.)).border_2().border_color(if none { t.border } else { c.hsla() });
        // 代码绘图：编辑代码 | 标题 | 填充 · 边框
        let lang = super::code::CodeLang::from_u8(e.code_lang);
        let has_title = !e.text.is_empty();
        bar = bar
            .child(
                self.bar_btn("sb-code", if lang == super::code::CodeLang::Mermaid { "编辑 Mermaid 代码" } else { "编辑 PlantUML 代码" }, false, t)
                    .child(icon("square-code", 17., t.text))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(id) = this.single().map(|e| e.id) {
                            this.open_code(Some(id), lang, w, cx);
                        }
                    })),
            )
            .child(sep())
            .child(
                self.bar_btn("sb-ctitle", if has_title { "隐藏标题" } else { "添加标题" }, has_title, t)
                    .child(icon("square-type", 17., if has_title { t.accent } else { t.text }))
                    .on_click(cx.listener(|this, _, w, cx| this.code_title(w, cx))),
            )
            .child(sep())
            .child(self.drop_btn(
                "sb-fill",
                "填充",
                Popover::Fill,
                fill_dot(e.fill).relative().flex().items_center().justify_center().into_any_element(),
                t,
                cx,
            ))
            .child(self.drop_btn("sb-border", "边框", Popover::Border, stroke_sq(e.stroke, e.no_stroke).into_any_element(), t, cx));
        bar
    }

    // ───────────── 下拉面板 ─────────────

    /// 圆形色块网格（调色板），每行 `cols` 个。
    #[allow(clippy::too_many_arguments)]
    fn color_grid(
        &self,
        id: &'static str,
        colors: Vec<Option<u32>>,
        cols: usize,
        cur: Option<Col>,
        t: &Theme,
        on_pick: impl Fn(&mut Self, Option<Col>, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().flex().flex_wrap().gap(px(8.)).w(px(cols as f32 * 30. - 8.)).children(colors.into_iter().enumerate().map(move |(i, c)| {
            let col = c.map(Col::rgb);
            let on = col.map(|c| c.0 | 0xff) == cur.map(|c| c.0 | 0xff);
            let f = on_pick.clone();
            div()
                .id((id, i))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .when(on, |d| d.border_2().border_color(t.accent))
                .hover(|s| s.opacity(0.85))
                .child(
                    div()
                        .size(px(if on { 14. } else { 22. }))
                        .rounded_full()
                        .border_1()
                        .border_color(t.border)
                        .when_some(col, |d, c| d.bg(c.hsla()))
                        .when(col.is_none(), |d| d.flex().items_center().justify_center().child(icon("ban", if on { 10. } else { 14. }, t.text_muted))),
                )
                .on_click(cx.listener(move |this, _, _, cx| f(this, col, cx)))
        }))
    }

    /// 不透明度滑杆：点按或拖动设置，松手记一次撤销。
    fn alpha_slider(&self, stroke: bool, value: f32, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let set = move |this: &mut Self, x: Pixels, cx: &mut Context<Self>| {
            let b = this.slider_bounds;
            if b.size.width > px(0.) {
                let v = ((x - b.origin.x) / b.size.width).clamp(0.05, 1.);
                this.live_alpha(stroke, (v * 100.).round() / 100., cx);
            }
        };
        div()
            .pt(px(12.))
            .child(div().flex().justify_between().text_size(px(12.)).text_color(t.text_muted).child("不透明度").child(format!("{:.0}%", value * 100.)))
            .child(
                div()
                    .id(if stroke { "alpha-s" } else { "alpha-f" })
                    .relative()
                    .h(px(18.))
                    .mt(px(4.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .child(canvas(move |b, _, cx| entity.update(cx, |this, _| this.slider_bounds = b), |_, _, _, _| {}).absolute().inset_0())
                    .child(div().w_full().h(px(4.)).rounded_full().bg(t.hover).child(div().h_full().w(relative(value)).rounded_full().bg(t.accent)))
                    .child(
                        div()
                            .absolute()
                            .top(px(1.))
                            .left(relative(value))
                            .ml(px(-8.))
                            .size(px(16.))
                            .rounded_full()
                            .bg(gpui::white())
                            .border_1()
                            .border_color(t.border_strong)
                            .shadow_sm(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.snapshot();
                            this.slider_drag = true;
                            set(this, ev.position.x, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        if this.slider_drag && ev.pressed_button == Some(MouseButton::Left) {
                            set(this, ev.position.x, cx);
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.slider_drag = false;
                            this.commit(cx);
                        }),
                    ),
            )
    }

    fn menu_row(&self, id: &'static str, label: &'static str, key: &'static str, danger: bool, t: &Theme) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(32.))
            .px(px(10.))
            .mx(px(4.))
            .flex()
            .items_center()
            .gap(px(16.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(if danger { gpui::rgb(0xf54a45).into() } else { t.text })
            .hover(|s| s.bg(t.hover))
            .child(div().flex_1().child(label))
            .child(div().text_size(px(12.)).text_color(t.text_muted).child(key))
    }

    fn render_popover(&self, bar_bottom: Option<Pixels>, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pop = self.popover?;
        let first = self.first_sel().cloned();
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);
        let body: AnyElement = match pop {
            Popover::ShapeKind => div()
                .w(px(5. * 42.))
                .child(title("更改图形"))
                .child(div().flex().flex_wrap().gap(px(2.)).children(BOARD_SHAPES.iter().enumerate().map(|(i, (k, tip))| {
                    let k = *k;
                    div()
                        .id(("pk-shape", i))
                        .size(px(40.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .tooltip(move |_, cx| cx.new(|_| Tooltip((*tip).into())).into())
                        .child(shape_preview(k, 26., 20., t.text, None))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.popover = None;
                            this.set_shape_kind(k, cx)
                        }))
                })))
                .into_any_element(),
            Popover::Fill => {
                let cell = self.single_table().is_some() && self.cur_cell().is_some();
                let cur = first.as_ref().and_then(|e| e.fill);
                let alpha = cur.map(|c| c.alpha()).unwrap_or(1.);
                div()
                    .w(px(9. * 30.))
                    .child(self.color_grid(
                        "pf",
                        FILL_COLORS.to_vec(),
                        9,
                        cur,
                        t,
                        |this, c, cx| {
                            if this.single_table().is_some() {
                                this.set_cell_fill(c, cx)
                            } else {
                                this.set_fill(c, cx)
                            }
                        },
                        cx,
                    ))
                    .when(!cell && cur.is_some(), |d| d.child(self.alpha_slider(false, alpha, t, cx)))
                    .into_any_element()
            }
            Popover::Border => self.render_popover_border(t, cx),
            Popover::TextColor => self.render_popover_text_color(t, cx),
            Popover::FontSize => self.render_popover_font_size(t, cx),
            Popover::Align => self.render_popover_align(t, cx),
            Popover::Route => self.render_popover_route(t, cx),
            Popover::StartArrow | Popover::EndArrow => {
                let is_end = pop == Popover::EndArrow;
                let cur = match first.as_ref().map(|e| &e.kind) {
                    Some(ElKind::Connector { start, end, .. }) => {
                        if is_end {
                            *end
                        } else {
                            *start
                        }
                    }
                    _ => Arrow::None,
                };
                // 当前用的是「更多」里的样式时直接展开
                let more = self.arrow_more || MORE_ARROWS.iter().any(|m| m.0 == cur);
                let list: Vec<(Arrow, &str)> = ARROWS.iter().chain(MORE_ARROWS.iter().filter(|_| more)).copied().collect();
                div()
                    .w(px(150.))
                    .children(list.into_iter().enumerate().map(|(i, (a, name))| {
                        let on = cur == a;
                        div()
                            .id(("par", i))
                            .h(px(30.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                            .hover(|s| s.bg(t.hover))
                            .child(if is_end {
                                line_preview(Arrow::None, a, false, false, 1.5, t.text)
                            } else {
                                line_preview(a, Arrow::None, false, false, 1.5, t.text)
                            })
                            .child(div().flex_1().child(name))
                            .when(on, |d| d.child(icon("check", 13., t.accent)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.popover = None;
                                this.arrow_more = false;
                                this.set_arrow(is_end, a, cx)
                            }))
                    }))
                    .when(!more, |d| {
                        d.child(
                            div()
                                .id("par-more")
                                .h(px(28.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(12.))
                                .text_color(t.accent)
                                .hover(|s| s.bg(t.hover))
                                .child("更多")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.arrow_more = true;
                                    this.pop_by_hover = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .into_any_element()
            }
            Popover::LineStyle if first.as_ref().is_some_and(|e| e.is_connector()) => self.render_popover_line_style(t, cx),
            Popover::LineStyle => {
                let e = first.clone();
                let style = e
                    .as_ref()
                    .map(|e| {
                        if e.dotted {
                            3
                        } else if e.dash {
                            2
                        } else {
                            1
                        }
                    })
                    .unwrap_or(1);
                let sw = e.as_ref().map(|e| e.stroke_w).unwrap_or(1.8);
                let is_pen = e.as_ref().is_some_and(|e| matches!(e.kind, ElKind::Pen { .. }));
                div()
                    .w(px(180.))
                    .when(!is_pen, |d| {
                        d.child(title("线型")).child(div().flex().flex_col().gap(px(2.)).pb(px(8.)).children(
                            [(1u8, false, false), (2, true, false), (3, false, true)].into_iter().map(|(s, dash, dot)| {
                                let on = style == s;
                                div()
                                    .id(("pls", s as usize))
                                    .h(px(28.))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .when(on, |d| d.bg(t.accent_soft))
                                    .hover(|s| s.bg(t.hover))
                                    .child(line_preview(Arrow::None, Arrow::None, dash, dot, 2., if on { t.accent } else { t.text }))
                                    .on_click(cx.listener(move |this, _, _, cx| this.set_line_style(s, cx)))
                            }),
                        ))
                    })
                    .child(title("粗细"))
                    .child(div().flex().gap(px(4.)).children([1.0f32, 1.8, 3., 4.5].into_iter().enumerate().map(|(i, w)| {
                        let on = (sw - w).abs() < 0.3;
                        div()
                            .id(("plw", i))
                            .w(px(38.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .border_1()
                            .border_color(if on { t.accent } else { t.border })
                            .hover(|s| s.bg(t.hover))
                            .child(div().w(px(22.)).h(px(w)).rounded_full().bg(if on { t.accent } else { t.text }))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_stroke_w(w, cx)))
                    })))
                    .into_any_element()
            }
            Popover::StickyColor => {
                let cur = first.as_ref().and_then(|e| e.fill);
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .w(px(3. * 34. + 12.))
                    .children(STICKY_COLORS.iter().enumerate().map(|(i, c)| {
                        let on = cur.map(|x| x.0 | 0xff) == Some(Col::rgb(*c).0);
                        div()
                            .id(("psk", i))
                            .size(px(30.))
                            .rounded(px(4.))
                            .bg(Col::rgb(*c).hsla())
                            .shadow_sm()
                            .cursor_pointer()
                            .border_2()
                            .border_color(if on { t.accent } else { gpui::transparent_black() })
                            .on_click(cx.listener(move |this, _, _, cx| this.set_sticky_color(i, cx)))
                    }))
                    .into_any_element()
            }
            Popover::Arrange => self.render_popover_arrange(t, cx),
            Popover::More => self.menu_items(true, t, cx).into_any_element(),
            Popover::TableOps => self.render_popover_table_ops(t, cx),
            Popover::MindLayout => self.render_popover_mind_layout(t, cx),
            Popover::Settings => self.render_popover_settings(t, cx),
            Popover::Export => div()
                .w(px(200.))
                .py(px(4.))
                .child(self.menu_row("pe-png", "下载为 PNG 图片", "", false, t).on_click(cx.listener(|this, _, _, cx| this.export(true, cx))))
                .child(self.menu_row("pe-svg", "下载为 SVG 矢量图", "", false, t).on_click(cx.listener(|this, _, _, cx| this.export(false, cx))))
                .child(self.menu_row("pe-mmd", "复制为 Mermaid", "", false, t).on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    cx.write_to_clipboard(ClipboardItem::new_string(super::to_mermaid(&this.doc)));
                    cx.notify();
                })))
                .into_any_element(),
            Popover::Zoom => div()
                .w(px(180.))
                .py(px(4.))
                .child(self.menu_row("pz-fit", "适应画布", "⇧1", false, t).on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    this.zoom_fit(cx)
                })))
                .child(self.menu_row("pz-100", "缩放至 100%", "⌘0", false, t).on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    this.zoom_100(cx)
                })))
                .child(self.menu_row("pz-50", "缩放至 50%", "", false, t).on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    let f = 0.5 / this.zoom;
                    this.zoom_step(f, cx)
                })))
                .child(self.menu_row("pz-200", "缩放至 200%", "", false, t).on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    let f = 2. / this.zoom;
                    this.zoom_step(f, cx)
                })))
                .into_any_element(),
        };
        // 属性栏的下拉贴在栏下方；顶栏 / 缩放栏的下拉贴在点击处下方（缩放栏的向上弹）
        let vp = window.viewport_size();
        let at = match pop {
            Popover::Settings | Popover::Export => point(self.pop_at.x - px(180.), self.pop_at.y + px(26.)),
            Popover::Zoom => point(self.pop_at.x - px(60.), self.pop_at.y - px(180.)),
            _ => point(self.pop_at.x - px(24.), bar_bottom.unwrap_or(self.pop_at.y + px(20.)) + px(6.)),
        };
        let at = point(at.x.min(vp.width - px(280.)).max(px(8.)), at.y.max(px(8.)));
        let pad = if matches!(pop, Popover::More | Popover::Settings | Popover::Export | Popover::Zoom) { px(0.) } else { px(10.) };
        Some(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    panel(t)
                        .id("board-popover")
                        .p(pad)
                        .child(body)
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            this.pop_panel_hover = *hovered;
                            if !*hovered {
                                this.schedule_pop_close(cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if this.popover.take().is_some() {
                                cx.notify();
                            }
                        })),
                ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
    /// 从 `render_popover` 拆出。
    fn render_popover_text_color(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);

        let cur = first.as_ref().map(|e| self.shown_fmt(e, cx).2.unwrap_or(Col::rgb(0x1f2329)));
        // 一排带颜色的「A」
        div()
            .child(title("文字颜色"))
            .child(div().flex().flex_wrap().gap(px(6.)).w(px(6. * 30.)).children(TEXT_COLORS.iter().enumerate().map(|(i, c)| {
                let col = Col::rgb(*c);
                let on = cur.is_some_and(|x| x.0 | 0xff == col.0 | 0xff);
                div()
                    .id(("ptc", i))
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(if on { t.accent } else { t.border })
                    .when(on, |d| d.border_2())
                    .when(*c == 0xffffff, |d| d.bg(gpui::rgb(0x8f959e)))
                    .hover(|s| s.border_color(t.accent))
                    .text_size(px(14.))
                    .text_color(col.hsla())
                    .child("A")
                    .on_click(cx.listener(move |this, _, _, cx| this.set_text_color(col, cx)))
            })))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_mind_layout(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);

        let (layout, branch) = first.as_ref().map(|e| self.doc.mind_root_style_pub(e.id)).unwrap_or((0, 0));
        let opt = |id: (&'static str, usize), ic: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(13.))
                .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 16., if on { t.accent } else { t.text }))
                .child(label)
        };
        div()
            .w(px(2. * 92. + 8.))
            .child(
                div()
                    .id("pml-scroll")
                    .max_h(px(330.))
                    .overflow_y_scroll()
                    .child(self.mind_layout_cards("pml", Some(layout), t, cx, |this, k, cx| this.mind_set_layout(k, cx))),
            )
            .child(div().pt(px(8.)).child(title("分支线")))
            .child(opt(("pmb", 0), "conn-curve", "曲线", branch == 0).on_click(cx.listener(|this, _, _, cx| this.mind_set_branch(0, cx))))
            .child(opt(("pmb", 1), "conn-elbow", "直角", branch == 1).on_click(cx.listener(|this, _, _, cx| this.mind_set_branch(1, cx))))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_arrange(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);

        let cell = |id: usize, ic: &'static str, tip: &'static str, f: fn(&mut Self, &mut Context<Self>)| {
            div()
                .id(("parr", id))
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .child(icon(ic, 17., t.text))
                .on_click(cx.listener(move |this, _, _, cx| f(this, cx)))
        };
        div()
            .child(title("对齐"))
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(cell(0, "align-h-left", "左对齐", |this, cx| this.align("left", cx)))
                    .child(cell(1, "align-h-mid", "水平居中", |this, cx| this.align("hcenter", cx)))
                    .child(cell(2, "align-h-right", "右对齐", |this, cx| this.align("right", cx)))
                    .child(cell(3, "align-v-top", "顶对齐", |this, cx| this.align("top", cx)))
                    .child(cell(4, "align-v-mid", "垂直居中", |this, cx| this.align("vcenter", cx)))
                    .child(cell(5, "align-v-bottom", "底对齐", |this, cx| this.align("bottom", cx))),
            )
            .child(div().pt(px(8.)).child(title("分布与批量排版")))
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(cell(6, "columns", "水平等间距", |this, cx| this.distribute(true, cx)))
                    .child(cell(7, "list", "垂直等间距", |this, cx| this.distribute(false, cx)))
                    .child(cell(8, "same-size", "等宽", |this, cx| this.same_size(true, false, cx)))
                    .child(cell(9, "align-v-mid", "等高", |this, cx| this.same_size(false, true, cx)))
                    .child(cell(10, "maximize", "相同大小", |this, cx| this.same_size(true, true, cx))),
            )
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_line_style(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);

        // 连线样式：连线类型 / 线型 / 粗细 / 跨线
        let e = first.clone().unwrap();
        let route = match &e.kind {
            ElKind::Connector { route, .. } => *route,
            _ => Route::Straight,
        };
        let style = if e.dotted {
            3
        } else if e.dash {
            2
        } else {
            1
        };
        let cell = |id: (&'static str, usize), on: bool| {
            div()
                .id(id)
                .flex_1()
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
        };
        let row = || div().flex().gap(px(4.)).pb(px(8.));
        div()
            .w(px(196.))
            .child(title("连线样式"))
            .child(row().children(ROUTES.iter().enumerate().map(|(i, (r, ic, tip))| {
                let r = *r;
                let on = route == r;
                cell(("pcr", i), on)
                    .tooltip(move |_, cx| cx.new(|_| Tooltip((*tip).into())).into())
                    .child(icon(ic, 16., if on { t.accent } else { t.text }))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_route(r, cx)))
            })))
            .child(row().children([(1u8, false, false), (2, true, false), (3, false, true)].into_iter().map(|(s, dash, dot)| {
                let on = style == s;
                let mut l = LineItem::new(vec![pt(2., 9.), pt(30., 9.)]);
                l.stroke = Col::from_hsla(if on { t.accent } else { t.text });
                l.width = 2.;
                l.end = Arrow::None;
                l.dash = dash;
                l.dotted = dot;
                let scene = Arc::new(Scene { items: vec![Item::Line(l)], width: 32., height: 18., ..Default::default() });
                cell(("pcs", s as usize), on)
                    .child(
                        div()
                            .w(px(32.))
                            .h(px(18.))
                            .child(canvas(|b, _, _| b, move |b, _, win, cx| paint_scene(&scene, View { origin: b.origin, scale: 1. }, b, win, cx)).size_full()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.set_line_style(s, cx)))
            })))
            .child(row().children([1.0f32, 1.8, 3., 4.5].into_iter().enumerate().map(|(i, w)| {
                let on = (e.stroke_w - w).abs() < 0.3;
                let d = w + 3.;
                cell(("pcw", i), on)
                    .child(div().size(px(d)).rounded_full().bg(if on { t.accent } else { t.text }))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_stroke_w(w, cx)))
            })))
            .child(
                // 跨线（交叉处画小拱桥）暂未支持
                div()
                    .id("pc-jump")
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .rounded(px(5.))
                    .bg(t.hover)
                    .opacity(0.45)
                    .text_size(px(12.))
                    .tooltip(|_, cx| cx.new(|_| Tooltip("跨线（即将支持）".into())).into())
                    .child(icon("line-jump", 15., t.text))
                    .child("跨线"),
            )
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_border(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();
        let title = |s: &'static str| div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);

        let e = first.clone();
        let cur = e.as_ref().map(|e| e.stroke);
        let is_box = e.as_ref().is_some_and(|e| matches!(e.kind, ElKind::Shape(_) | ElKind::Section | ElKind::Table { .. }));
        let style = e
            .as_ref()
            .map(|e| {
                if e.no_stroke {
                    0
                } else if e.dotted {
                    3
                } else if e.dash {
                    2
                } else {
                    1
                }
            })
            .unwrap_or(1);
        let sw = e.as_ref().map(|e| e.stroke_w).unwrap_or(1.8);
        let alpha = cur.map(|c| c.alpha()).unwrap_or(1.);
        // 分段按钮组（浅灰底，选中项白底蓝字）
        let seg = |children: Vec<AnyElement>| div().flex().p(px(2.)).gap(px(2.)).rounded(px(6.)).bg(t.hover).children(children);
        let seg_btn = |id: (&'static str, usize), on: bool, tip: &'static str| {
            div()
                .id(id)
                .flex_1()
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.bg))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
        };
        let line_glyph = |s: u8, on: bool| {
            let c = if on { t.accent } else { t.text };
            match s {
                0 => icon("ban", 15., c).into_any_element(),
                1 => div().w(px(18.)).h(px(2.)).bg(c).into_any_element(),
                2 => div().flex().gap(px(3.)).children((0..3).map(|_| div().w(px(4.)).h(px(2.)).bg(c))).into_any_element(),
                _ => div().flex().gap(px(2.)).children((0..5).map(|_| div().w(px(2.)).h(px(2.)).bg(c))).into_any_element(),
            }
        };
        div()
            .w(px(6. * 30.))
            .when(is_box, |d| {
                d.child(title("边框样式"))
                    .child(seg([(0u8, "无边框"), (1, "实线"), (2, "虚线"), (3, "点线")]
                        .into_iter()
                        .map(|(s, tip)| {
                            seg_btn(("pbs", s as usize), style == s, tip)
                                .child(line_glyph(s, style == s))
                                .on_click(cx.listener(move |this, _, _, cx| this.set_line_style(s, cx)))
                                .into_any_element()
                        })
                        .collect()))
                    .child(div().h(px(6.)))
                    .child(seg([(1.0f32, "细"), (1.8, "标准"), (3., "粗"), (4.5, "特粗")]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (w, tip))| {
                            let on = (sw - w).abs() < 0.3;
                            seg_btn(("pbw", i), on, tip)
                                .child(div().size(px(3. + i as f32 * 1.6)).rounded_full().bg(if on { t.accent } else { t.text }))
                                .on_click(cx.listener(move |this, _, _, cx| this.set_stroke_w(w, cx)))
                                .into_any_element()
                        })
                        .collect()))
                    .child(div().h(px(1.)).my(px(10.)).bg(t.border))
            })
            .child(self.color_grid(
                "psc",
                STROKE_COLORS.iter().map(|c| Some(*c)).collect(),
                6,
                cur,
                t,
                move |this, c, cx| {
                    if let Some(c) = c {
                        this.set_stroke(c, cx)
                    }
                },
                cx,
            ))
            .child(self.alpha_slider(true, alpha, t, cx))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_route(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();

        let cur = match first.as_ref().map(|e| &e.kind) {
            Some(ElKind::Connector { route, .. }) => *route,
            _ => Route::Straight,
        };
        div()
            .w(px(150.))
            .children(ROUTES.into_iter().enumerate().map(|(i, (r, ic, name))| {
                let on = cur == r;
                div()
                    .id(("prt", i))
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                    .hover(|s| s.bg(t.hover))
                    .child(icon(ic, 16., if on { t.accent } else { t.text }))
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.popover = None;
                        this.set_route(r, cx)
                    }))
            }))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_font_size(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();

        let e = first.clone();
        let sticky = e.as_ref().is_some_and(|e| matches!(e.kind, ElKind::Sticky));
        let cur = e.as_ref().map(|e| if e.auto_font && sticky { 0. } else { self.shown_layout(e).0 }).unwrap_or(16.);
        let row = |i: usize, label: String, v: Option<f32>, on: bool| {
            div()
                .id(("pfs", i))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(13.))
                .when(on, |d| d.text_color(t.accent))
                .hover(|s| s.bg(t.hover))
                .child(label)
                .when(on, |d| d.child(icon("check", 13., t.accent)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.popover = None;
                    this.set_font(v, cx)
                }))
        };
        div()
            .w(px(76.))
            .when(sticky, |d| d.child(row(99, "自动".into(), None, cur == 0.)))
            .children(FONT_SIZES.iter().enumerate().map(|(i, s)| row(i, format!("{s:.0}"), Some(*s), (cur - s).abs() < 0.5)))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_table_ops(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();

        let lane = first.as_ref().is_some_and(|e| e.is_lane());
        let has_cell = self.cur_cell().is_some();
        let sep = || div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.border);
        let r = |id: &'static str, label: &'static str, how: u8| {
            self.menu_row(id, label, "", false, t).on_click(cx.listener(move |this, _, _, cx| this.table_insert(how, cx)))
        };
        div()
            .w(px(200.))
            .py(px(4.))
            .child(r("pt-up", if lane { "在上方插入一行" } else { "在上方插入行" }, 0))
            .child(r("pt-down", if lane { "在下方插入一行" } else { "在下方插入行" }, 1))
            .child(r("pt-left", "在左侧插入列", 2))
            .child(r("pt-right", "在右侧插入列", 3))
            .child(sep())
            .child(
                self.menu_row("pt-delr", "删除所在行", "", true, t)
                    .when(!has_cell, |d| d.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.table_delete(true, cx))),
            )
            .child(
                self.menu_row("pt-delc", "删除所在列", "", true, t)
                    .when(!has_cell, |d| d.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.table_delete(false, cx))),
            )
            .when(!has_cell, |d| d.child(div().px(px(14.)).py(px(4.)).text_size(px(11.5)).text_color(t.text_muted).child("先单击选中一个单元格")))
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_settings(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let p = prefs();
        let toggle = |id: &'static str, label: &'static str, key: &'static str, on: bool, f: fn(&mut BoardPrefs)| {
            div()
                .id(id)
                .h(px(34.))
                .px(px(10.))
                .mx(px(4.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(13.))
                .hover(|s| s.bg(t.hover))
                .child(div().flex_1().child(label))
                .child(div().text_size(px(12.)).text_color(t.text_muted).child(key))
                .child(
                    div()
                        .w(px(30.))
                        .h(px(18.))
                        .rounded_full()
                        .bg(if on { t.accent } else { t.border_strong })
                        .flex()
                        .items_center()
                        .px(px(2.))
                        .when(on, |d| d.justify_end())
                        .child(div().size(px(14.)).rounded_full().bg(gpui::white())),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    f(&mut PREFS.lock().unwrap());
                    cx.notify();
                }))
        };
        div()
            .w(px(250.))
            .py(px(4.))
            .child(toggle("ps-grid", "网格背景", "⇧G", p.grid, |p| p.grid = !p.grid))
            .child(toggle("ps-quick", "快捷创建", "⇧Q", p.quick_create, |p| p.quick_create = !p.quick_create))
            .child(toggle("ps-size", "显示尺寸", "", p.show_size, |p| p.show_size = !p.show_size))
            .child(toggle("ps-precise", "精准框选", "", p.precise_select, |p| p.precise_select = !p.precise_select))
            .child(toggle("ps-map", "小地图", "", p.minimap, |p| p.minimap = !p.minimap))
            .when(self.doc.flow, |d| {
                d.child(div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.border)).child(self.menu_row("ps-layout", "自动整理布局", "", false, t).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.popover = None;
                        this.snapshot();
                        super::auto_layout(&mut this.doc);
                        this.commit(cx);
                        let b = this.doc.bounds();
                        this.zoom_to(b);
                    }),
                ))
            })
            .into_any_element()
    }

    /// 从 `render_popover` 拆出。
    fn render_popover_align(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let first = self.first_sel().cloned();

        // 文本格式（「A≡」）：加粗 / 删除线 / 下划线 / 斜体，水平对齐，垂直对齐
        let e = first.clone();
        let (_, h, v) = e.as_ref().map(|e| self.shown_layout(e)).unwrap_or((16., 1, 1));
        let (bold, deco, _) = e.as_ref().map(|e| self.shown_fmt(e, cx)).unwrap_or_default();
        let btn = |id: (&'static str, usize), ic: &'static str, tip: &'static str, on: bool| {
            div()
                .id(id)
                .flex_1()
                .h(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .child(icon(ic, 16., if on { t.accent } else { t.text }))
        };
        let sep = || div().h(px(1.)).my(px(6.)).bg(t.border);
        div()
            .w(px(180.))
            .child(
                div()
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(btn(("pfmt", 0), "bold", "加粗 ⌘B", bold).on_click(cx.listener(|this, _, _, cx| this.toggle_bold(cx))))
                            .child(
                                btn(("pfmt", 1), "strikethrough", "删除线 ⌘⇧X", deco & DECO_STRIKE != 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_deco(DECO_STRIKE, cx))),
                            )
                            .child(
                                btn(("pfmt", 2), "underline", "下划线 ⌘U", deco & DECO_UNDERLINE != 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_deco(DECO_UNDERLINE, cx))),
                            )
                            .child(
                                btn(("pfmt", 3), "italic", "斜体 ⌘I", deco & DECO_ITALIC != 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_deco(DECO_ITALIC, cx))),
                            ),
                    )
                    .child(sep()),
            )
            .child(
                div().flex().gap(px(2.)).children(
                    [(1u8, "text-left", "左对齐"), (2, "text-center", "水平居中"), (3, "text-right", "右对齐")].into_iter().map(|(a, ic, tip)| {
                        btn(("pal", a as usize), ic, tip, h + 1 == a).on_click(cx.listener(move |this, _, _, cx| this.set_text_align(a, cx)))
                    }),
                ),
            )
            .child(div().h(px(2.)))
            .child(
                div().flex().gap(px(2.)).children(
                    [(1u8, "arrow-up-to-line", "顶部对齐"), (2, "fold-vertical", "垂直居中"), (3, "arrow-down-to-line", "底部对齐")]
                        .into_iter()
                        .map(|(a, ic, tip)| btn(("pva", a as usize), ic, tip, v + 1 == a).on_click(cx.listener(move |this, _, _, cx| this.set_v_align(a, cx)))),
                ),
            )
            .into_any_element()
    }

    /// 右键菜单 / 「更多」里的操作。on_sel 为 false 时是画布空白处的菜单。
    fn menu_items(&self, on_sel: bool, t: &Theme, cx: &mut Context<Self>) -> Div {
        let sep = || div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.border);
        let done = |this: &mut Self| {
            this.menu = None;
            this.popover = None;
        };
        if !on_sel {
            return div()
                .w(px(200.))
                .py(px(4.))
                .child(self.menu_row("m-paste", "粘贴", "⌘V", false, t).on_click(cx.listener(move |this, _, w, cx| {
                    done(this);
                    this.paste(&BPaste, w, cx)
                })))
                .child(self.menu_row("m-all", "全选", "⌘A", false, t).on_click(cx.listener(move |this, _, w, cx| {
                    done(this);
                    this.select_all(&BSelectAll, w, cx)
                })))
                .child(sep())
                .child(self.menu_row("m-grid", if prefs().grid { "隐藏网格" } else { "显示网格" }, "⇧G", false, t).on_click(cx.listener(
                    move |this, _, _, cx| {
                        done(this);
                        let mut p = PREFS.lock().unwrap();
                        p.grid = !p.grid;
                        cx.notify();
                    },
                )))
                .child(self.menu_row("m-fit", "适应画布", "⇧1", false, t).on_click(cx.listener(move |this, _, _, cx| {
                    done(this);
                    this.zoom_fit(cx)
                })));
        }
        let locked = self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).all(|e| e.locked);
        let grouped = self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).any(|e| e.group.is_some());
        let has_section = self.doc.els.iter().any(|e| self.sel.contains(&e.id) && e.is_section());
        let multi = self.sel.len() > 1;
        let mind = self.single_mind().is_some();
        div()
            .w(px(210.))
            .py(px(4.))
            .child(self.menu_row("m-copy", "复制", "⌘C", false, t).on_click(cx.listener(move |this, _, w, cx| {
                done(this);
                this.copy(&BCopy, w, cx)
            })))
            .child(self.menu_row("m-cimg", "复制为图片", "⇧⌘C", false, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.copy_as_image(cx)
            })))
            .child(self.menu_row("m-paste", "粘贴", "⌘V", false, t).on_click(cx.listener(move |this, _, w, cx| {
                done(this);
                this.paste(&BPaste, w, cx)
            })))
            .child(self.menu_row("m-dup", "创建副本", "⌘D", false, t).on_click(cx.listener(move |this, _, w, cx| {
                done(this);
                this.duplicate(&BDuplicate, w, cx)
            })))
            .child(sep())
            // 层级：悬停展开二级菜单
            .child(
                div()
                    .id("m-layer-wrap")
                    .relative()
                    .on_hover(cx.listener(|this, h: &bool, _, cx| {
                        this.layer_sub = *h;
                        cx.notify();
                    }))
                    .child(self.menu_row("m-layer", "层级", "", false, t).when(self.layer_sub, |d| d.bg(t.hover)).child(icon(
                        "chevron-right",
                        13.,
                        t.text_muted,
                    )))
                    .when(self.layer_sub, |d| {
                        d.child(
                            div().absolute().top(px(-4.)).left(px(206.)).child(
                                panel(t)
                                    .w(px(180.))
                                    .py(px(4.))
                                    .child(self.menu_row("m-top", "置于顶层", "⌘]", false, t).on_click(cx.listener(move |this, _, _, cx| {
                                        done(this);
                                        this.z_order(2, cx)
                                    })))
                                    .child(self.menu_row("m-up", "上移一层", "", false, t).on_click(cx.listener(move |this, _, _, cx| {
                                        done(this);
                                        this.z_order(1, cx)
                                    })))
                                    .child(self.menu_row("m-down", "下移一层", "", false, t).on_click(cx.listener(move |this, _, _, cx| {
                                        done(this);
                                        this.z_order(-1, cx)
                                    })))
                                    .child(self.menu_row("m-bottom", "置于底层", "⌘[", false, t).on_click(cx.listener(move |this, _, _, cx| {
                                        done(this);
                                        this.z_order(-2, cx)
                                    }))),
                            ),
                        )
                    }),
            )
            .child(sep())
            .child(self.menu_row("m-cstyle", "复制样式", "⌥⌘C", false, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.copy_style(cx)
            })))
            // 没有复制过样式时置灰
            .child(self.menu_row("m-pstyle", "粘贴样式", "⌥⌘V", false, t).when(self.style_clip.is_none(), |d| d.opacity(0.4)).on_click(cx.listener(
                move |this, _, _, cx| {
                    done(this);
                    this.paste_style(cx)
                },
            )))
            .child(sep())
            .child(self.menu_row("m-fliph", "水平翻转", "⇧H", false, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.flip(true, cx)
            })))
            .child(self.menu_row("m-flipv", "垂直翻转", "⇧V", false, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.flip(false, cx)
            })))
            .when(mind, |d| {
                d.child(sep())
                    .child(self.menu_row("m-mchild", "添加子节点", "Tab", false, t).on_click(cx.listener(move |this, _, w, cx| {
                        done(this);
                        if let Some(id) = this.single_mind().map(|e| e.id) {
                            this.mind_add_child(id, 0, w, cx);
                        }
                    })))
                    .child(self.menu_row("m-msib", "添加兄弟节点", "↵", false, t).on_click(cx.listener(move |this, _, w, cx| {
                        done(this);
                        if let Some(id) = this.single_mind().map(|e| e.id) {
                            this.mind_add_sibling(id, w, cx);
                        }
                    })))
                    .child(self.menu_row("m-mparent", "添加父节点", "⇧Tab", false, t).on_click(cx.listener(move |this, _, w, cx| {
                        done(this);
                        if let Some(id) = this.single_mind().map(|e| e.id) {
                            this.mind_add_parent(id, w, cx);
                        }
                    })))
                    .child(self.menu_row("m-mfold", "收起 / 展开子节点", "⌘.", false, t).on_click(cx.listener(move |this, _, _, cx| {
                        done(this);
                        this.mind_toggle_collapse(cx)
                    })))
            })
            .child(sep())
            .when(multi || grouped, |d| {
                d.child(self.menu_row("m-group", if grouped { "取消组合" } else { "组合" }, if grouped { "⇧⌘G" } else { "⌘G" }, false, t).on_click(
                    cx.listener(move |this, _, w, cx| {
                        done(this);
                        if grouped {
                            this.ungroup(&BUngroup, w, cx)
                        } else {
                            this.group(&BGroup, w, cx)
                        }
                    }),
                ))
            })
            .child(self.menu_row("m-lock", if locked { "解锁" } else { "锁定" }, "⇧⌘L", false, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.toggle_lock(cx)
            })))
            .when(has_section, |d| {
                d.child(self.menu_row("m-unsec", "移除分区", "", false, t).on_click(cx.listener(move |this, _, _, cx| {
                    done(this);
                    this.remove_section(cx)
                })))
            })
            .child(sep())
            .child(self.menu_row("m-del", "删除", "⌫", true, t).on_click(cx.listener(move |this, _, _, cx| {
                done(this);
                this.delete_sel(cx)
            })))
    }

    pub(super) fn render_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (at, on_sel) = self.menu?;
        Some(
            deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(panel(t).child(self.menu_items(on_sel, t, cx)).on_mouse_down_out(
                cx.listener(|this, _, _, cx| {
                    if this.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )))
            .with_priority(4)
            .into_any_element(),
        )
    }

    /// 图形选择器：常用图形 + 便签 + 文本，选中后放在记录的位置。
    pub(super) fn render_picker(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pk = self.picker?;
        let shapes: [(ShapeKind, &str); 10] = [
            (ShapeKind::RoundRect, "圆角矩形"),
            (ShapeKind::Rect, "矩形"),
            (ShapeKind::Ellipse, "圆形"),
            (ShapeKind::Diamond, "菱形"),
            (ShapeKind::Triangle, "三角形"),
            (ShapeKind::Stadium, "全圆角矩形"),
            (ShapeKind::Parallelogram, "平行四边形"),
            (ShapeKind::Hexagon, "六边形"),
            (ShapeKind::Cylinder, "圆柱体"),
            (ShapeKind::Document, "文档"),
        ];
        let cell =
            |i: usize| div().id(("pick", i)).size(px(40.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer().hover(|s| s.bg(t.hover));
        let body = div()
            .w(px(5. * 42. + 4.))
            .child(div().pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(if pk.conn.is_some() { "连接到新图形" } else { "添加图形" }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(2.))
                    .children(shapes.into_iter().enumerate().map(|(i, (k, tip))| {
                        cell(i)
                            .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                            .child(shape_preview(k, 26., 20., t.text, None))
                            .on_click(cx.listener(move |this, _, w, cx| this.picker_choose(Some(k), false, w, cx)))
                    }))
                    .child(
                        cell(20)
                            .tooltip(|_, cx| cx.new(|_| Tooltip("便签".into())).into())
                            .child(div().size(px(20.)).rounded(px(3.)).bg(Col::rgb(STICKY_COLORS[0]).hsla()).shadow_sm())
                            .on_click(cx.listener(|this, _, w, cx| this.picker_choose(None, true, w, cx))),
                    )
                    .child(
                        cell(21)
                            .tooltip(|_, cx| cx.new(|_| Tooltip("文本".into())).into())
                            .child(icon("type", 18., t.text))
                            .on_click(cx.listener(|this, _, w, cx| this.picker_choose(None, false, w, cx))),
                    ),
            );
        Some(
            deferred(anchored().position(point(pk.at.x + px(8.), pk.at.y + px(8.))).snap_to_window_with_margin(px(8.)).child(
                panel(t).p(px(10.)).child(body).on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    // 点到别处：若是从连线拉出来的，保留那条悬空的连线
                    if this.picker.take().is_some() {
                        cx.notify();
                    }
                })),
            ))
            .with_priority(4)
            .into_any_element(),
        )
    }
}
