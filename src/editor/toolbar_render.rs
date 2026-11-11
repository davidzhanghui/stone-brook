//! 选中文字后的浮动工具栏（含悬停展开的子菜单）。

use super::*;
use crate::model::BlockKind;
use crate::theme::{Theme, PALETTE_NAMES};
use crate::ui::{icon, Tooltip};

impl Editor {
    pub(super) fn tb_hover_open(&mut self, m: Option<TbMenu>, cx: &mut Context<Self>) {
        if m.is_some() && self.pop.tb_menu != m {
            self.pop.tb_menu = m;
            self.tb.tb_more_heading = false;
            self.tb.tb_hover_count = 1;
            cx.notify();
        } else {
            self.tb.tb_hover_count += 1;
        }
    }

    pub(super) fn tb_hover_leave(&mut self, cx: &mut Context<Self>) {
        self.tb.tb_hover_count = self.tb.tb_hover_count.saturating_sub(1);
        self.tb.tb_hover_epoch += 1;
        let epoch = self.tb.tb_hover_epoch;
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            this.update(cx, |this, cx| {
                if this.tb.tb_hover_epoch == epoch && this.tb.tb_hover_count == 0 && this.pop.tb_menu.is_some() {
                    this.pop.tb_menu = None;
                    this.tb.tb_more_heading = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_toolbar(&self, at: Point<Pixels>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // 布局：[文本类型▾] [对齐▾] | B S I U 链接 代码 | [A▾] | 清除格式 评论
        const BTN: f32 = 28.;
        const DD: f32 = 40.;
        const GAP: f32 = 2.;
        const SEP: f32 = 9.;
        let slots = self.selected_slots();
        let all = |f: fn(&Marks) -> bool| !slots.is_empty() && slots.iter().all(|(s, r)| self.slot_text(*s).is_some_and(|t| t.all_marks(r.clone(), f)));
        let in_cells = self.sel_in_cells();
        let in_table = self.sel_valign().is_some() || slots.iter().any(|((b, _), _)| matches!(self.content.blocks[*b].kind, BlockKind::Table { .. }));
        let menu = self.pop.tb_menu;
        let tip = |s: &'static str| move |_: &mut Window, cx: &mut App| cx.new(|_| Tooltip(s.into())).into();
        let btn = |id: &'static str, ic: &'static str, on: bool, tp: &'static str| {
            div()
                .id(id)
                .size(px(BTN))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.active))
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 15., if on { t.accent } else { t.text }))
                .tooltip(tip(tp))
        };
        // 带下拉箭头的按钮
        let dd = |id: &'static str, lead: AnyElement, open: bool, tp: &'static str| {
            div()
                .id(id)
                .w(px(DD))
                .h(px(BTN))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(2.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(open, |d| d.bg(t.active))
                .hover(|s| s.bg(t.hover))
                .child(lead)
                .child(icon(if open { "chevron-up" } else { "chevron-down" }, 11., t.text_muted))
                .when(!open, |d| d.tooltip(tip(tp)))
        };
        let sep = || div().w(px(1.)).h(px(18.)).mx_1().bg(t.border);
        // 点击展开（已由悬停展开时保持不变，不会被点一下又收起）
        let toggle = |m: TbMenu| {
            cx.listener(move |this: &mut Self, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if this.pop.tb_menu != Some(m) {
                    this.pop.tb_menu = Some(m);
                    this.tb.tb_more_heading = false;
                    this.tb.tb_hover_count = 1;
                }
                cx.notify();
            })
        };
        // 悬停：移入下拉按钮即展开，移出按钮和面板后延时收起
        let hover = |m: Option<TbMenu>| {
            cx.listener(move |this: &mut Self, h: &bool, _, cx| {
                if *h {
                    this.tb_hover_open(m, cx);
                } else {
                    this.tb_hover_leave(cx);
                }
            })
        };

        // 当前文本类型图标
        let cur_kind = self.sel_kind();
        let kind_lead = |k: &Option<BlockKind>| -> AnyElement {
            match k {
                Some(BlockKind::Heading { level }) => {
                    div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(format!("H{level}")).into_any_element()
                }
                Some(BlockKind::Bullet) => icon("list", 15., t.text).into_any_element(),
                Some(BlockKind::Ordered) => icon("list-ordered", 15., t.text).into_any_element(),
                Some(BlockKind::Todo { .. }) => icon("square-check", 15., t.text).into_any_element(),
                Some(BlockKind::Code { .. }) => icon("braces", 15., t.text).into_any_element(),
                Some(BlockKind::Quote) => icon("quote", 15., t.text).into_any_element(),
                Some(BlockKind::Callout { .. }) => icon("callout", 15., t.text).into_any_element(),
                _ => icon("type", 15., t.text).into_any_element(),
            }
        };
        let cur_align = self.sel_align();
        let align_icon = |a: u8| match a {
            1 => "text-center",
            2 => "text-right",
            _ => "text-left",
        };
        let (lfg, lbg) = self.tb.last_color;
        let lfg_c = lfg.filter(|c| *c > 0).map(|c| t.fg_palette[c as usize & 7]).unwrap_or(t.text);
        let lbg_c = lbg.filter(|c| *c > 0).map(|c| t.text_bg(c));

        // 下拉面板的水平位置：跟随触发按钮
        let x0 = 5.;
        let align_x = if in_cells { x0 } else { x0 + DD + GAP };
        let color_x = align_x + DD + GAP + SEP + GAP + 6. * (BTN + GAP) + SEP + GAP;

        let bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GAP))
            .p(px(4.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .shadow_lg()
            .when(!in_cells, |d| {
                d.child(
                    dd("tb-kind", kind_lead(&cur_kind), menu == Some(TbMenu::Kind), "文本类型")
                        .on_hover(hover(Some(TbMenu::Kind)))
                        .on_mouse_down(MouseButton::Left, toggle(TbMenu::Kind)),
                )
            })
            .child(
                dd("tb-align", icon(align_icon(cur_align.unwrap_or(0)), 15., t.text).into_any_element(), menu == Some(TbMenu::Align), "对齐方式")
                    .on_hover(hover(Some(TbMenu::Align)))
                    .on_mouse_down(MouseButton::Left, toggle(TbMenu::Align)),
            )
            .child(sep())
            .child(btn("tb-b", "bold", all(|m| m.bold), "粗体 (⌘ + B)\nMarkdown: **文本**").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.bold(&Bold, w, cx)
                }),
            ))
            .child(btn("tb-s", "strike", all(|m| m.strike), "删除线 (⌘ + Shift + X)\nMarkdown: ~~文本~~").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.strike(&Strike, w, cx)
                }),
            ))
            .child(btn("tb-i", "italic", all(|m| m.italic), "斜体 (⌘ + I)\nMarkdown: *文本*").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.italic(&Italic, w, cx)
                }),
            ))
            .child(btn("tb-u", "underline", all(|m| m.underline), "下划线 (⌘ + U)").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.underline(&Underline, w, cx)
                }),
            ))
            .child(btn("tb-l", "link", all(|m| m.link.is_some()), "链接 (⌘ + K)").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.insert_link(&InsertLink, w, cx)
                }),
            ))
            .child(btn("tb-c", "code", all(|m| m.code), "行内代码 (⌘ + E)\nMarkdown: `代码`").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    cx.stop_propagation();
                    this.inline_code(&InlineCode, w, cx)
                }),
            ))
            .child(sep())
            .child(
                // 「A」直接套用上次的颜色，箭头展开颜色面板
                div()
                    .id("tb-color-apply")
                    .size(px(BTN))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .when(menu != Some(TbMenu::Color), |d| d.tooltip(tip("字体颜色 / 背景色")))
                    .on_hover(hover(Some(TbMenu::Color)))
                    .child(
                        div()
                            .px(px(3.))
                            .rounded(px(2.))
                            .when_some(lbg_c, |d, c| d.bg(c))
                            .text_size(px(14.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(lfg_c)
                            .child("A"),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let (fg, bg) = this.tb.last_color;
                            this.set_color(fg, bg, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id("tb-color")
                    .w(px(14.))
                    .h(px(BTN))
                    .ml(px(-GAP))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(menu == Some(TbMenu::Color), |d| d.bg(t.active))
                    .hover(|s| s.bg(t.hover))
                    .child(icon(if menu == Some(TbMenu::Color) { "chevron-up" } else { "chevron-down" }, 11., t.text_muted))
                    .on_hover(hover(Some(TbMenu::Color)))
                    .on_mouse_down(MouseButton::Left, toggle(TbMenu::Color)),
            )
            .child(sep())
            .child(btn("tb-clear", "eraser", false, "清除格式").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.clear_format(cx)
                }),
            ))
            .child(btn("tb-comment", "comment", false, "评论").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let b = this.sel().0.block;
                    this.pop.tb_menu = None;
                    this.start_comment(b, ev.position, window, cx);
                }),
            ));

        // 下拉面板通用外壳与菜单行
        let panel =
            |w: f32| div().w(px(w)).p(px(4.)).bg(t.panel_bg).border_1().border_color(t.border).rounded(px(8.)).shadow_lg().flex().flex_col().text_size(px(13.));
        let row = |id: ElementId, lead: AnyElement, label: &'static str, checked: bool, disabled: bool| {
            let fg = if disabled {
                t.text_muted
            } else if checked {
                t.accent
            } else {
                t.text
            };
            div()
                .id(id)
                .h(px(32.))
                .px_2()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(5.))
                .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                .text_color(fg)
                .child(div().w(px(18.)).flex().justify_center().child(lead))
                .child(div().flex_1().child(label))
                .when(checked, |d| d.child(icon("check", 14., t.accent)))
        };
        let menu_sep = || div().h(px(1.)).mx_1().my_1().bg(t.border);
        let h_lead = |s: &'static str, on: bool| {
            div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(if on { t.accent } else { t.text_secondary }).child(s).into_any_element()
        };
        let ic_lead = |ic: &'static str, on: bool, dis: bool| {
            icon(
                ic,
                15.,
                if dis {
                    t.text_muted
                } else if on {
                    t.accent
                } else {
                    t.text_secondary
                },
            )
            .into_any_element()
        };

        let menu_el: Option<AnyElement> = match menu {
            Some(TbMenu::Kind) if !in_cells => {
                let same = |k: &BlockKind| {
                    cur_kind
                        .as_ref()
                        .is_some_and(|c| std::mem::discriminant(c) == std::mem::discriminant(k) && (!matches!(k, BlockKind::Heading { .. }) || c == k))
                };
                let item = |id: usize, lead: AnyElement, label: &'static str, kind: BlockKind| {
                    let on = same(&kind);
                    row(("tbk", id).into(), lead, label, on, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pop.tb_menu = None;
                            this.set_sel_kind(kind.clone(), cx);
                        }),
                    )
                };
                let more_open = self.tb.tb_more_heading;
                let more_on = matches!(cur_kind, Some(BlockKind::Heading { level }) if level >= 4);
                Some(
                    panel(200.)
                        .child(item(0, ic_lead("type", same(&BlockKind::Text), false), "正文", BlockKind::Text))
                        .child(item(1, h_lead("H1", same(&BlockKind::Heading { level: 1 })), "一级标题", BlockKind::Heading { level: 1 }))
                        .child(item(2, h_lead("H2", same(&BlockKind::Heading { level: 2 })), "二级标题", BlockKind::Heading { level: 2 }))
                        .child(item(3, h_lead("H3", same(&BlockKind::Heading { level: 3 })), "三级标题", BlockKind::Heading { level: 3 }))
                        .child(
                            row(("tbk", 4usize).into(), h_lead("Hn", more_on), "其他标题", false, false)
                                .child(icon(if more_open { "chevron-down" } else { "chevron-right" }, 12., t.text_muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.tb.tb_more_heading = !this.tb.tb_more_heading;
                                        cx.notify();
                                    }),
                                ),
                        )
                        .when(more_open, |d| {
                            d.child(item(5, h_lead("H4", same(&BlockKind::Heading { level: 4 })), "四级标题", BlockKind::Heading { level: 4 }))
                                .child(item(6, h_lead("H5", same(&BlockKind::Heading { level: 5 })), "五级标题", BlockKind::Heading { level: 5 }))
                                .child(item(7, h_lead("H6", same(&BlockKind::Heading { level: 6 })), "六级标题", BlockKind::Heading { level: 6 }))
                        })
                        .child(item(8, ic_lead("list-ordered", same(&BlockKind::Ordered), false), "有序列表", BlockKind::Ordered))
                        .child(item(9, ic_lead("list", same(&BlockKind::Bullet), false), "无序列表", BlockKind::Bullet))
                        .child(item(10, ic_lead("square-check", same(&BlockKind::Todo { checked: false }), false), "任务", BlockKind::Todo { checked: false }))
                        .child(item(
                            11,
                            ic_lead("braces", same(&BlockKind::Code { lang: String::new() }), false),
                            "代码块",
                            BlockKind::Code { lang: String::new() },
                        ))
                        .child(menu_sep())
                        .child(item(12, ic_lead("quote", same(&BlockKind::Quote), false), "引用", BlockKind::Quote))
                        .child(item(
                            13,
                            ic_lead("callout", same(&BlockKind::Callout { emoji: String::new(), color: 0 }), false),
                            "高亮块",
                            BlockKind::Callout { emoji: "💡".into(), color: 5 },
                        ))
                        .into_any_element(),
                )
            }
            Some(TbMenu::Align) => {
                let cur_v = self.sel_valign();
                let h_item = |a: u8, ic: &'static str, label: &'static str| {
                    let on = cur_align == Some(a);
                    row(("tba", a as usize).into(), ic_lead(ic, on, false), label, on, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pop.tb_menu = None;
                            this.set_sel_align(a, cx);
                        }),
                    )
                };
                let v_item = |a: u8, ic: &'static str, label: &'static str| {
                    let on = cur_v == Some(a);
                    row(("tbv", a as usize).into(), ic_lead(ic, on, false), label, on, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pop.tb_menu = None;
                            this.set_cell_valign(a, cx);
                        }),
                    )
                };
                // 单元格 / 分栏内不支持缩进，置灰
                let ind_item = |id: usize, ic: &'static str, label: &'static str, d: i32| {
                    row(("tbi", id).into(), ic_lead(ic, false, in_cells), label, false, in_cells).when(!in_cells, |el| {
                        el.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.shift_indent(d, cx);
                            }),
                        )
                    })
                };
                Some(
                    panel(180.)
                        .child(h_item(0, "text-left", "左对齐"))
                        .child(h_item(1, "text-center", "居中对齐"))
                        .child(h_item(2, "text-right", "右对齐"))
                        .when(in_table, |d| {
                            d.child(menu_sep()).child(v_item(0, "arrow-up-to-line", "顶部对齐")).child(v_item(1, "fold-vertical", "垂直居中")).child(v_item(
                                2,
                                "arrow-down-to-line",
                                "底部对齐",
                            ))
                        })
                        .child(menu_sep())
                        .child(ind_item(0, "chevron-right", "增加缩进", 1))
                        .child(ind_item(1, "chevron-left", "减少缩进", -1))
                        .into_any_element(),
                )
            }
            Some(TbMenu::Color) => {
                let cur_fg = self.sel_uniform(|m| m.color);
                let cur_bg = self.sel_uniform(|m| m.bg);
                // 面板顺序：默认、灰、红、橙、黄、绿、蓝、紫
                const ORDER: [u8; 8] = [0, 7, 1, 2, 3, 4, 5, 6];
                let swatch = |id: ElementId, on: bool| {
                    div()
                        .id(id)
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .hover(|s| s.border_color(t.accent))
                };
                let fg_row = div().flex().flex_row().gap(px(4.)).children(ORDER.iter().map(|&c| {
                    swatch(("tbfg", c as usize).into(), cur_fg == Some(c))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if c == 0 { t.text } else { t.fg_palette[c as usize] })
                        .child("A")
                        .tooltip(tip(PALETTE_NAMES[c as usize]))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.tb.last_color.0 = Some(c);
                                this.set_color(Some(c), None, cx);
                            }),
                        )
                }));
                let bg_row = |cs: [u8; 8]| {
                    div().flex().flex_row().gap(px(4.)).children(cs.into_iter().map(|c| {
                        swatch(("tbbg", c as usize).into(), cur_bg == Some(c))
                            .bg(if c == 0 { t.panel_bg } else { t.text_bg(c) })
                            .when(c == 0, |d| d.child(icon("ban", 12., t.text_muted)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.tb.last_color.1 = Some(c);
                                    this.set_color(None, Some(c), cx);
                                }),
                            )
                    }))
                };
                Some(
                    panel(220.)
                        .p(px(10.))
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(t.text_secondary)
                        .child(crate::i18n::tr("字体颜色"))
                        .child(fg_row)
                        .child(div().mt_1().child(crate::i18n::tr("背景颜色")))
                        .child(bg_row(ORDER))
                        .child(bg_row([8, 9, 10, 11, 12, 13, 14, 15]))
                        .child(
                            div()
                                .id("tb-color-reset")
                                .mt_1()
                                .h(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.))
                                .border_1()
                                .border_color(t.border)
                                .cursor_pointer()
                                .hover(|s| s.bg(t.hover))
                                .text_color(t.text)
                                .child(crate::i18n::tr("恢复默认"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.set_color(Some(0), Some(0), cx);
                                    }),
                                ),
                        )
                        .into_any_element(),
                )
            }
            _ => None,
        };
        // 鼠标停在面板上时保持展开，移出后同样延时收起
        // （工具栏与面板之间的空隙也算在面板里，鼠标慢慢划过不会收起）
        // 水平位置跟随触发按钮；偏移放在外层，悬停区域才与面板重合
        let menu_x = match menu {
            Some(TbMenu::Align) => align_x,
            Some(TbMenu::Color) => color_x,
            _ => x0,
        };
        let menu_el = menu_el.map(|el| div().id("tb-menu-hover").ml(px(menu_x)).pt_1().on_hover(hover(menu)).child(el).into_any_element());

        deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(div().occlude().flex().flex_col().items_start().child(bar).children(menu_el)))
            .with_priority(3)
    }
}
