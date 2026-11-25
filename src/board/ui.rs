//! 画板的固定界面：左侧竖向工具栏（悬停浮出图形 / 连线 / 便签 / 分区面板）、右上角操作栏、
//! 底部画笔栏、右下角缩放栏与小地图。布局与画板编辑态一致。

use super::model::*;
use super::view::*;
use crate::canvas::paint::{paint_scene, View};
use crate::canvas::{rect, Col, Item, Scene, ShapeItem, ShapeKind};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

/// 流程图形状库
#[cfg(feature = "dev")]
pub const FLOW_SHAPES: [(ShapeKind, &str, f32, f32); 10] = [
    (ShapeKind::Stadium, "开始 / 结束", 120., 48.),
    (ShapeKind::Rect, "过程", 130., 56.),
    (ShapeKind::Diamond, "判断", 130., 80.),
    (ShapeKind::Parallelogram, "输入 / 输出", 140., 56.),
    (ShapeKind::Subroutine, "子流程", 130., 56.),
    (ShapeKind::Document, "文档", 130., 64.),
    (ShapeKind::Cylinder, "数据库", 110., 70.),
    (ShapeKind::RoundRect, "圆角矩形", 130., 56.),
    (ShapeKind::Hexagon, "准备", 140., 56.),
    (ShapeKind::Circle, "连接点", 56., 56.),
];

/// 基础图形（「图形」面板）
pub const BOARD_SHAPES: [(ShapeKind, &str); 20] = [
    (ShapeKind::RoundRect, "圆角矩形 R"),
    (ShapeKind::Rect, "矩形 S"),
    (ShapeKind::Ellipse, "圆形 O"),
    (ShapeKind::Diamond, "菱形 D"),
    (ShapeKind::Triangle, "三角形"),
    (ShapeKind::Stadium, "全圆角矩形"),
    (ShapeKind::Parallelogram, "平行四边形"),
    (ShapeKind::Trapezoid, "梯形"),
    (ShapeKind::Pentagon, "五边形"),
    (ShapeKind::Hexagon, "六边形"),
    (ShapeKind::Octagon, "八边形"),
    (ShapeKind::Star, "五角星"),
    (ShapeKind::RightTriangle, "直角三角形"),
    (ShapeKind::Cross, "十字形"),
    (ShapeKind::BlockArrow, "箭头"),
    (ShapeKind::DoubleArrow, "双向箭头"),
    (ShapeKind::Bubble, "对话气泡"),
    (ShapeKind::Cylinder, "圆柱体"),
    (ShapeKind::Document, "文档"),
    (ShapeKind::Note, "注释"),
];

pub const PEN_COLORS: [u32; 7] = [0x1f2329, 0xf54a45, 0xff8800, 0xf5c518, 0x34c724, 0x3370ff, 0x7f3bf5];

/// 分区预设尺寸
pub const SECTION_SIZES: [(&str, f32, f32); 4] = [("16:9", 800., 450.), ("4:3", 640., 480.), ("1:1", 480., 480.), ("A4", 420., 594.)];

/// 小尺寸的图形预览（工具栏 / 面板里的图标）。
pub fn shape_preview(kind: ShapeKind, w: f32, h: f32, stroke: Hsla, fill: Option<Hsla>) -> impl IntoElement {
    let mut s = ShapeItem::new(kind, rect(1.5, 1.5, w - 3., h - 3.));
    s.fill = fill.map(Col::from_hsla);
    s.stroke = Some(Col::from_hsla(stroke));
    s.stroke_w = 1.4;
    s.radius = (h / 5.).min(4.);
    let scene = Arc::new(Scene { items: vec![Item::Shape(s)], width: w, height: h, ..Default::default() });
    div()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .child(canvas(|b, _, _| b, move |b, _, win, cx| paint_scene(&scene, View { origin: b.origin, scale: 1. }, b, win, cx)).size_full())
}

/// 面板容器（白底、圆角、阴影）。
pub(super) fn panel(t: &Theme) -> Div {
    div().occlude().rounded(px(10.)).bg(t.panel_bg).border_1().border_color(t.border).shadow_lg()
}

impl BoardEditor {
    fn tool_icon_btn(&self, id: &'static str, ic: &'static str, on: bool, tip: &'static str, t: &Theme) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(on, |d| d.bg(t.accent_soft))
            .hover(|s| s.bg(t.hover))
            .child(icon(ic, 18., if on { t.accent } else { t.text }))
            .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
    }

    /// 带浮出面板的工具按钮：悬停展开面板，右下角的小三角提示还有更多选项。
    fn fly_btn(&self, id: &'static str, content: AnyElement, on: bool, fly: Flyout, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let open = self.flyout == Some(fly);
        div()
            .id(id)
            .relative()
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(on || open, |d| d.bg(if on { t.accent_soft } else { t.hover }))
            .hover(|s| s.bg(t.hover))
            .child(content)
            .child(
                div()
                    .absolute()
                    .right(px(3.))
                    .bottom(px(3.))
                    .size(px(0.))
                    .border_l(px(3.))
                    .border_t(px(3.))
                    .border_color(gpui::transparent_black())
                    .border_r(px(0.))
                    .child(div().absolute().right_0().bottom_0().size(px(4.)).bg(t.text_muted).rounded(px(1.))),
            )
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if *h && this.flyout != Some(fly) {
                    this.flyout = Some(fly);
                    this.popover = None;
                    cx.notify();
                }
            }))
    }

    pub(super) fn render_left_toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.tool;
        let sep = || div().h(px(1.)).w(px(24.)).my(px(4.)).bg(t.border);
        let close_fly = |this: &mut Self, cx: &mut Context<Self>| {
            if this.flyout.take().is_some() {
                cx.notify();
            }
        };
        let shape_on = matches!(tool, Tool::Shape(_));
        let conn_on = matches!(tool, Tool::Connector(..));
        let (route, _) = self.last_conn;
        let conn_icon = match route {
            Route::Straight => "conn-straight",
            Route::Elbow => "conn-elbow",
            Route::RoundElbow => "conn-round-elbow",
            Route::Curve => "conn-curve",
        };
        let last_shape = self.last_shape;
        let ink = if shape_on { t.accent } else { t.text };
        let sticky_c = Col::rgb(STICKY_COLORS[self.sticky_color.min(8)]).hsla();
        let on_col = |on: bool| if on { t.accent } else { t.text };
        let bar = panel(t)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .p(px(5.))
            .child(
                self.tool_icon_btn("bt-sel", "pointer", tool == Tool::Select, "选择 V", t)
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            close_fly(this, cx)
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Select, cx))),
            )
            .child(sep())
            .child(
                self.fly_btn("bt-shape", shape_preview(last_shape, 20., 16., ink, None).into_any_element(), shape_on, Flyout::Shapes, t, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tool(Tool::Shape(this.last_shape), cx))),
            )
            .child(
                self.tool_icon_btn("bt-text", "type", tool == Tool::Text, "文本 T", t)
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            close_fly(this, cx)
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Text, cx))),
            )
            .child(
                self.fly_btn(
                    "bt-sticky",
                    div().size(px(17.)).rounded(px(3.)).bg(sticky_c).border_1().border_color(t.border_strong).shadow_sm().into_any_element(),
                    tool == Tool::Sticky,
                    Flyout::Sticky,
                    t,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Sticky, cx))),
            )
            .child(self.fly_btn("bt-conn", icon(conn_icon, 18., on_col(conn_on)).into_any_element(), conn_on, Flyout::Connectors, t, cx).on_click(cx.listener(
                |this, _, _, cx| {
                    let (r, a) = this.last_conn;
                    this.set_tool(Tool::Connector(r, a), cx)
                },
            )))
            .child(
                self.fly_btn(
                    "bt-section",
                    icon("section", 18., on_col(matches!(tool, Tool::Section(..)))).into_any_element(),
                    matches!(tool, Tool::Section(..)),
                    Flyout::Section,
                    t,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Section(800., 450.), cx))),
            )
            .child(
                self.fly_btn(
                    "bt-table",
                    icon("table", 18., on_col(matches!(tool, Tool::Lane(_)))).into_any_element(),
                    matches!(tool, Tool::Lane(_)),
                    Flyout::Table,
                    t,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.insert_table(3, 3, cx))),
            )
            .child(
                self.tool_icon_btn("bt-pen", "pencil", matches!(tool, Tool::Pen | Tool::Eraser), "画笔 P", t)
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            close_fly(this, cx)
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Pen, cx))),
            )
            .child(
                self.fly_btn(
                    "bt-mind",
                    icon("mindmap", 18., on_col(matches!(tool, Tool::Mind(_)))).into_any_element(),
                    matches!(tool, Tool::Mind(_)),
                    Flyout::Mind,
                    t,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Mind(0), cx))),
            )
            .child(
                self.tool_icon_btn("bt-icons", "smile-plus", self.icons.is_some(), "图标", t)
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            close_fly(this, cx)
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_icons(window, cx))),
            )
            .child(sep())
            .child(
                self.tool_icon_btn("bt-tpl", "layout-template", self.templates.is_some(), "模板", t)
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            close_fly(this, cx)
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.open_templates(cx))),
            )
            .child(self.fly_btn("bt-more", icon("more", 18., t.text).into_any_element(), false, Flyout::More, t, cx));

        div()
            .absolute()
            .left(px(if self.library {
                16. + 5. * 42. + 26. + 10.
            } else if self.icons.is_some() {
                16. + super::library::ICON_PANEL_W + 10.
            } else {
                16.
            }))
            .top_0()
            .bottom_0()
            .flex()
            .flex_col()
            .justify_center()
            .child(div().relative().child(bar).children(self.render_flyout(t, cx)))
    }

    fn render_flyout(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fly = self.flyout?;
        // 面板贴在工具栏右侧，与对应按钮大致对齐（按钮 36 + 间距 2，分隔线约 11）
        let slot = |i: f32, seps: f32| 5. + i * 38. + seps * 11.;
        let top = match fly {
            Flyout::Shapes => slot(1., 1.),
            Flyout::Sticky => slot(3., 1.),
            Flyout::Connectors => slot(4., 1.),
            Flyout::Section => slot(5., 1.),
            Flyout::Table => slot(6., 1.),
            Flyout::Mind => slot(1., 1.),
            Flyout::More => slot(10., 2.) - 120.,
        };
        let title = |s: &'static str| div().px(px(4.)).pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(s);
        let row = |id: (&'static str, usize), ic: AnyElement, label: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(13.))
                .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                .hover(|s| s.bg(t.hover))
                .child(ic)
                .child(label)
        };
        let body: AnyElement =
            match fly {
                Flyout::Shapes => div()
                    .w(px(5. * 42. + 8.))
                    .child(title("图形"))
                    .child(
                        div().flex().flex_wrap().gap(px(2.)).children(
                            super::library::basic_shapes()
                                .enumerate()
                                .map(|(i, (k, tip))| self.lib_cell(("fly-shape", i), super::library::LibItem::Shape(k), tip, t, cx)),
                        ),
                    )
                    .child(
                        div()
                            .id("fly-more-shapes")
                            .mt(px(8.))
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(8.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(t.border)
                            .cursor_pointer()
                            .text_size(px(13.))
                            .hover(|s| s.bg(t.hover))
                            .child("更多图形")
                            .child(div().px(px(5.)).rounded(px(3.)).bg(t.hover).text_size(px(11.)).text_color(t.text_muted).child("M"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library = true;
                                this.icons = None;
                                this.flyout = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
                Flyout::Connectors => {
                    let crow = |id: usize, ic: &'static str, label: &'static str, r: Route, arrow: bool| {
                        let on = self.last_conn == (r, arrow) && self.conn_preset.is_none();
                        row(("fc", id), icon(ic, 17., if on { t.accent } else { t.text }).into_any_element(), label, on)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_tool(Tool::Connector(r, arrow), cx)))
                    };
                    div()
                        .w(px(168.))
                        .child(title("连线 L"))
                        .child(crow(1, "conn-straight", "直线连接线", Route::Straight, true))
                        .child(crow(2, "conn-elbow", "直角连接线", Route::Elbow, true))
                        .child(crow(3, "conn-curve", "曲线连接线", Route::Curve, true))
                        .child(crow(4, "conn-line", "直线", Route::Straight, false))
                        .into_any_element()
                }
                Flyout::Sticky => div()
                    .w(px(3. * 46. + 12.))
                    .child(title("便签 N"))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(STICKY_COLORS.iter().enumerate().map(|(i, c)| {
                        let on = self.sticky_color == i;
                        div()
                            .id(("fly-sticky", i))
                            .size(px(40.))
                            .rounded(px(4.))
                            .bg(Col::rgb(*c).hsla())
                            .shadow_sm()
                            .border_2()
                            .border_color(if on { t.accent } else { gpui::transparent_black() })
                            .cursor_pointer()
                            .hover(|s| s.border_color(t.accent.opacity(0.5)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.sticky_color = i;
                                this.set_tool(Tool::Sticky, cx);
                            }))
                    })))
                    .into_any_element(),
                Flyout::Section => {
                    div()
                        .w(px(180.))
                        .child(title("分区 ⇧S"))
                        .children(SECTION_SIZES.iter().enumerate().map(|(i, (name, w, h))| {
                            let (w, h) = (*w, *h);
                            let on = self.tool == Tool::Section(w, h);
                            div()
                                .id(("fly-sec", i))
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
                                .child(div().w(px(22.)).flex().justify_center().child(
                                    div().w(px(22. * (w / w.max(h)))).h(px(16. * (h / w.max(h)))).border_1().border_color(t.text_secondary).rounded(px(2.)),
                                ))
                                .child(div().flex_1().child(*name))
                                .child(div().text_size(px(11.)).text_color(t.text_muted).child(format!("{w:.0}×{h:.0}")))
                                .on_click(cx.listener(move |this, _, _, cx| this.set_tool(Tool::Section(w, h), cx)))
                        }))
                        .child(div().px(px(10.)).pt(px(6.)).text_size(px(11.5)).text_color(t.text_muted).child("选中尺寸后单击画布放置，或直接拖出任意大小"))
                        .into_any_element()
                }
                Flyout::Table => {
                    // 同 Word：在网格上移动选择行列数
                    let (pr, pc) = self.table_pick;
                    let cell = |r: usize, c: usize| {
                        let on = r < pr && c < pc;
                        div()
                            .id(("tpick", r * 10 + c))
                            .size(px(18.))
                            .rounded(px(2.))
                            .border_1()
                            .border_color(if on { t.accent } else { t.border_strong })
                            .when(on, |d| d.bg(t.accent_soft))
                            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                                if *h && this.table_pick != (r + 1, c + 1) {
                                    this.table_pick = (r + 1, c + 1);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| this.insert_table(r + 1, c + 1, cx)))
                    };
                    let lane_row = |k: u8, label: &'static str| {
                        row(("fly-lane", k as usize), super::library::item_preview(super::library::LibItem::Lane(k), t.text), label, self.tool == Tool::Lane(k))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_tool(Tool::Lane(k), cx)))
                    };
                    div()
                        .w(px(8. * 21. + 8.))
                        .child(title(if pr > 0 { "表格" } else { "插入表格" }))
                        .child(div().flex().flex_col().gap(px(3.)).children((0..8).map(|r| div().flex().gap(px(3.)).children((0..8).map(move |c| cell(r, c))))))
                        .child(div().pt(px(6.)).px(px(4.)).text_size(px(12.)).text_color(t.text_secondary).child(if pr > 0 {
                            format!("{pr} 行 × {pc} 列")
                        } else {
                            "移动鼠标选择行列数".into()
                        }))
                        .child(div().h(px(1.)).my(px(8.)).bg(t.border))
                        .child(title("泳道"))
                        .child(lane_row(0, "垂直泳道"))
                        .child(lane_row(1, "水平泳道"))
                        .child(lane_row(2, "矩阵泳道"))
                        .into_any_element()
                }
                Flyout::Mind => {
                    // 思维导图 / 树状图 / 时间线三组缩略图卡片，放不下时面板内滚动
                    let cur = match self.tool {
                        Tool::Mind(k) => Some(k),
                        _ => None,
                    };
                    // 面板较高：顶部与第一个工具按钮对齐，高度不超出工具栏，放不下就在面板里滚动
                    let room = slot(10., 2.) - slot(1., 1.) - 40.;
                    div()
                        .w(px(2. * 92. + 8.))
                        .child(div().id("fly-mind-scroll").max_h(px(room)).overflow_y_scroll().child(self.mind_layout_cards(
                            "fly-mind",
                            cur,
                            t,
                            cx,
                            |this, k, cx| this.set_tool(Tool::Mind(k), cx),
                        )))
                        .child(div().px(px(4.)).pt(px(8.)).text_size(px(11.5)).text_color(t.text_muted).child("选择后单击画布放置 · ↵ 兄弟节点 · Tab 子节点"))
                        .into_any_element()
                }
                Flyout::More => div()
                    .w(px(190.))
                    .child(title("更多"))
                    .child(row(("fly-more", 0), icon("image", 17., t.text).into_any_element(), "图片", false).on_click(cx.listener(|this, _, _, cx| {
                        this.flyout = None;
                        this.pick_image(cx)
                    })))
                    .child(
                        row(("fly-more", 4), icon("sitemap", 17., t.text).into_any_element(), "Mermaid 绘图", false)
                            .on_click(cx.listener(|this, _, window, cx| this.open_code(None, super::code::CodeLang::Mermaid, window, cx))),
                    )
                    .child(
                        row(("fly-more", 1), icon("code-xml", 17., t.text).into_any_element(), "PlantUML 绘图", false)
                            .on_click(cx.listener(|this, _, window, cx| this.open_code(None, super::code::CodeLang::Puml, window, cx))),
                    )
                    .child(
                        row(("fly-more", 2), icon("search", 17., t.text).into_any_element(), "搜索替换  ⌘F", false)
                            .on_click(cx.listener(|this, _, window, cx| this.open_find(window, cx))),
                    )
                    .child(row(("fly-more", 3), icon("shapes", 17., t.text).into_any_element(), "更多图形  M", self.library).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.library = !this.library;
                            this.icons = None;
                            this.flyout = None;
                            cx.notify();
                        },
                    )))
                    .into_any_element(),
            };
        Some(div().absolute().left(px(56.)).top(px(top - 20.)).child(panel(t).id("board-flyout").p(px(8.)).child(body).on_hover(cx.listener(
            |this, h: &bool, _, cx| {
                if !*h && this.flyout.take().is_some() {
                    cx.notify();
                }
            },
        ))))
        .map(|d| d.into_any_element())
    }

    pub(super) fn render_topbar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let btn = |id: &'static str, ic: &'static str, tip: &'static str, enabled: bool| {
            div()
                .id(id)
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .child(icon(ic, 17., if enabled { t.text } else { t.text_muted.opacity(0.5) }))
        };
        let text_btn = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if on { t.accent } else { t.text })
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .child(label)
        };
        let sep = || div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(t.border);
        let can_undo = !self.undo.is_empty();
        let can_redo = !self.redo.is_empty();
        let title = if self.preview {
            "以思维导图查看"
        } else if self.doc.flow {
            "流程图"
        } else {
            "画板"
        };
        div()
            .absolute()
            .top(px(12.))
            // 左侧让开 macOS 的红黄绿窗口按钮
            .left(px(88.))
            .right(px(16.))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                panel(t)
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(if self.doc.flow { "workflow" } else { "pen-tool" }, 16., t.accent))
                    .child(div().text_size(px(13.)).child(title))
                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(format!("{} 个元素", self.doc.els.len()))),
            )
            .child(
                panel(t)
                    .h(px(40.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(btn("tb-find", "search", "搜索替换 ⌘F", true).on_click(cx.listener(|this, _, w, cx| this.open_find(w, cx))))
                    .child(sep())
                    .child(btn("tb-undo", "restore", "撤销 ⌘Z", can_undo).on_click(cx.listener(|this, _, w, cx| this.do_undo(&BUndo, w, cx))))
                    .child(btn("tb-redo", "redo", "重做 ⇧⌘Z", can_redo).on_click(cx.listener(|this, _, w, cx| this.do_redo(&BRedo, w, cx))))
                    .child(sep())
                    .child(text_btn("tb-rough", "手绘风格", self.doc.rough).on_click(cx.listener(|this, _, _, cx| {
                        this.snapshot();
                        this.doc.rough = !this.doc.rough;
                        this.commit(cx);
                    })))
                    .child(
                        btn("tb-set", "settings", "画板设置", true)
                            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| this.toggle_popover(Popover::Settings, ev.position(), cx))),
                    )
                    .child(
                        text_btn("tb-export", "导出", self.popover == Some(Popover::Export))
                            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| this.toggle_popover(Popover::Export, ev.position(), cx))),
                    )
                    .child(sep())
                    .when(self.preview, |d| {
                        d.child(text_btn("tb-insert", "插入到文档", false).on_click(cx.listener(|this, _, _, cx| {
                            this.finish_edit(cx);
                            this.commit(cx);
                            cx.emit(BoardEvent::Insert);
                        })))
                    })
                    .when(!self.preview, |d| d.child(text_btn("tb-cancel", "取消", false).on_click(cx.listener(|this, _, _, cx| this.request_cancel(cx)))))
                    .child(
                        div()
                            .id("tb-done")
                            .h(px(30.))
                            .px(px(14.))
                            .mx(px(2.))
                            .flex()
                            .items_center()
                            .rounded(px(7.))
                            .cursor_pointer()
                            .bg(t.accent)
                            .text_color(gpui::white())
                            .text_size(px(13.))
                            .hover(|s| s.opacity(0.9))
                            .child("完成")
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
    }

    pub(super) fn toggle_popover(&mut self, p: Popover, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.flyout = None;
        self.menu = None;
        if self.popover == Some(p) {
            self.popover = None;
        } else {
            self.popover = Some(p);
            self.pop_at = at;
        }
        cx.notify();
    }

    /// 点击「取消」且存在未保存改动时的防误触确认弹窗。
    pub(super) fn render_cancel_confirm(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.cancel_confirm {
            return None;
        }
        let title = if self.created_blank { "放弃新建画板？" } else { "放弃本次修改？" };
        let desc = if self.created_blank {
            "当前画板内容不会被保存，文档中的画板块也将一并移除。"
        } else {
            "画板将恢复到进入编辑前的状态，本次所做的修改会丢失。"
        };
        Some(
            div()
                .id("board-cancel-backdrop")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.35))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.cancel_confirm = false;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("board-cancel-card")
                        .w(px(348.))
                        .p(px(20.))
                        .rounded(px(12.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title))
                        .child(div().text_size(px(13.)).text_color(t.text_secondary).child(desc))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .justify_end()
                                .gap(px(8.))
                                .pt(px(4.))
                                .child(
                                    div()
                                        .id("board-cancel-stay")
                                        .h(px(30.))
                                        .px(px(14.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .border_1()
                                        .border_color(t.border)
                                        .text_size(px(13.))
                                        .text_color(t.text)
                                        .hover(|s| s.bg(t.hover))
                                        .child("继续编辑")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_confirm = false;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id("board-cancel-discard")
                                        .h(px(30.))
                                        .px(px(14.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .bg(t.fg_palette[1])
                                        .text_size(px(13.))
                                        .text_color(gpui::white())
                                        .hover(|s| s.opacity(0.9))
                                        .child("放弃")
                                        .on_click(cx.listener(|this, _, _, cx| this.confirm_cancel(cx))),
                                ),
                        ),
                ),
        )
    }

    /// 底部画笔栏：画笔 / 荧光笔 / 橡皮擦、颜色、粗细（手绘工具栏）。
    pub(super) fn render_pen_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !matches!(self.tool, Tool::Pen | Tool::Eraser) {
            return None;
        }
        let pen = self.pen;
        let erasing = self.tool == Tool::Eraser;
        let kind_btn = |id: &'static str, ic: &'static str, tip: &'static str, on: bool| {
            div()
                .id(id)
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .child(icon(ic, 18., if on { t.accent } else { t.text }))
        };
        let sep = || div().w(px(1.)).h(px(22.)).mx(px(6.)).bg(t.border);
        Some(
            div()
                .absolute()
                .bottom(px(20.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    panel(t)
                        .h(px(48.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(kind_btn("pb-pen", "pencil", "画笔 P", !erasing && !pen.highlighter).on_click(cx.listener(|this, _, _, cx| {
                            this.pen.highlighter = false;
                            this.set_tool(Tool::Pen, cx);
                        })))
                        .child(kind_btn("pb-hl", "highlighter", "荧光笔 ⇧P", !erasing && pen.highlighter).on_click(cx.listener(|this, _, _, cx| {
                            this.pen.highlighter = true;
                            this.set_tool(Tool::Pen, cx);
                        })))
                        .child(
                            kind_btn("pb-er", "eraser2", "橡皮擦 E（点击或划过笔迹即可擦除）", erasing)
                                .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Eraser, cx))),
                        )
                        .child(sep())
                        .children(PEN_COLORS.iter().enumerate().map(|(i, c)| {
                            let on = pen.color == i && !erasing;
                            div()
                                .id(("pb-c", i))
                                .size(px(26.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .cursor_pointer()
                                .border_2()
                                .border_color(if on { t.accent } else { gpui::transparent_black() })
                                .child(div().size(px(16.)).rounded_full().bg(Col::rgb(*c).hsla()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pen.color = i;
                                    if this.tool == Tool::Eraser {
                                        this.set_tool(Tool::Pen, cx);
                                    }
                                    cx.notify();
                                }))
                        }))
                        .child(sep())
                        .children([4f32, 7., 11.].into_iter().enumerate().map(|(i, d)| {
                            let on = pen.width == i;
                            div()
                                .id(("pb-w", i))
                                .size(px(30.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .when(on, |s| s.bg(t.accent_soft))
                                .hover(|s| s.bg(t.hover))
                                .child(div().size(px(d)).rounded_full().bg(if on { t.accent } else { t.text }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pen.width = i;
                                    cx.notify();
                                }))
                        }))
                        .child(sep())
                        .child(
                            div()
                                .id("pb-exit")
                                .h(px(30.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .hover(|s| s.bg(t.hover))
                                .child("退出手绘")
                                .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Select, cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_zoom_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let btn = |id: &'static str, ic: &'static str, tip: &'static str, on: bool| {
            div()
                .id(id)
                .size(px(30.))
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
        let p = prefs();
        div().absolute().right(px(16.)).bottom(px(16.)).child(
            panel(t)
                .h(px(40.))
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(1.))
                .child(btn("zb-hand", "hand", "移动画布 H（或按住空格 / 右键拖动）", self.tool == Tool::Hand).on_click(cx.listener(|this, _, _, cx| {
                    let t = if this.tool == Tool::Hand { Tool::Select } else { Tool::Hand };
                    this.set_tool(t, cx)
                })))
                .child(div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(t.border))
                .child(btn("zb-out", "minus", "缩小 ⌘-", false).on_click(cx.listener(|this, _, _, cx| this.zoom_step(0.8, cx))))
                .child(
                    div()
                        .id("zb-pct")
                        .w(px(54.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .hover(|s| s.bg(t.hover))
                        .child(format!("{:.0}%", self.zoom * 100.))
                        .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| this.toggle_popover(Popover::Zoom, ev.position(), cx))),
                )
                .child(btn("zb-in", "plus", "放大 ⌘=", false).on_click(cx.listener(|this, _, _, cx| this.zoom_step(1.25, cx))))
                .child(div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(t.border))
                .child(btn("zb-map", "map", "小地图", p.minimap).on_click(cx.listener(|_, _, _, cx| {
                    let mut p = PREFS.lock().unwrap();
                    p.minimap = !p.minimap;
                    cx.notify();
                }))),
        )
    }

    pub(super) fn render_minimap(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !prefs().minimap {
            return None;
        }
        let b = self.doc.bounds()?;
        let (mw, mh) = (200f32, 130f32);
        let vp_w = crate::canvas::Rect {
            x: -self.offset.x / self.zoom,
            y: -self.offset.y / self.zoom,
            w: f32::from(self.canvas_bounds.size.width) / self.zoom,
            h: f32::from(self.canvas_bounds.size.height) / self.zoom,
        };
        let all = b.union(&vp_w).inflate(40.);
        let s = (mw / all.w).min(mh / all.h);
        let mut scene = self.doc.to_scene(t.dark);
        scene.rough = false;
        let scene = Arc::new(scene);
        let vp_col = t.accent;
        Some(
            div().absolute().right(px(16.)).bottom(px(64.)).child(
                panel(t)
                    .w(px(mw))
                    .h(px(mh))
                    .overflow_hidden()
                    .child(
                        canvas(
                            |b, _, _| b,
                            move |bounds, _, window, cx| {
                                let view = View { origin: point(bounds.origin.x - px(all.x * s), bounds.origin.y - px(all.y * s)), scale: s };
                                paint_scene(&scene, view, bounds, window, cx);
                                window.paint_quad(fill(view.r(vp_w), vp_col.opacity(0.06)).border_widths(px(1.5)).border_color(vp_col));
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            // 点击小地图：把视口中心移到对应位置
                            let cb = this.canvas_bounds;
                            let mini_origin = (f32::from(cb.right()) - 16. - mw, f32::from(cb.bottom()) - 64. - mh);
                            let wx = all.x + (f32::from(ev.position.x) - mini_origin.0) / s;
                            let wy = all.y + (f32::from(ev.position.y) - mini_origin.1) / s;
                            this.offset = crate::canvas::pt(f32::from(cb.size.width) / 2. - wx * this.zoom, f32::from(cb.size.height) / 2. - wy * this.zoom);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            ),
        )
    }
}
