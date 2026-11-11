//! 块菜单（左侧手柄菜单）与 `/` 斜杠菜单的渲染。

use super::ops::filter_slash;
use super::*;
use crate::model::BlockKind;
use crate::theme::Theme;
use crate::ui::icon;

impl Editor {
    pub(super) fn render_block_menu(&self, ix: usize, kind: &BlockKind, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds: Vec<(&str, BlockKind)> = vec![
            ("T", BlockKind::Text),
            ("H1", BlockKind::Heading { level: 1 }),
            ("H2", BlockKind::Heading { level: 2 }),
            ("H3", BlockKind::Heading { level: 3 }),
            ("•", BlockKind::Bullet),
            ("1.", BlockKind::Ordered),
            ("☑", BlockKind::Todo { checked: false }),
            ("❝", BlockKind::Quote),
            ("</>", BlockKind::Code { lang: "Plain Text".into() }),
            ("💡", BlockKind::Callout { emoji: "💡".into(), color: 5 }),
            ("✎", BlockKind::Notes),
        ];
        let textual = kind.is_textual();
        let cur_indent = self.content.blocks.get(ix).map(|b| b.indent).unwrap_or(0);
        let item = |id: &'static str, ic: &'static str, label: &'static str, hint: &'static str, danger: bool| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(6.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .text_color(if danger { t.fg_palette[1] } else { t.text })
                .child(icon(ic, 14., if danger { t.fg_palette[1] } else { t.text_secondary }))
                .child(div().flex_1().child(crate::i18n::tr(label)))
                .child(div().text_color(t.text_muted).text_size(px(11.)).child(hint))
        };
        div()
            .id("block-menu")
            .occlude()
            .w(px(232.))
            .p(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded(px(10.))
            .shadow_lg()
            .text_size(px(13.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(textual, |d| {
                d.child(div().px_2().pt_1().pb(px(4.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("转换为")))
                    .child(div().flex().flex_row().flex_wrap().gap(px(4.)).px_1().pb_1().children(kinds.into_iter().enumerate().map(|(i, (lbl, k))| {
                        let is_cur = std::mem::discriminant(&k) == std::mem::discriminant(kind) && (!matches!(k, BlockKind::Heading { .. }) || k == *kind);
                        div()
                            .id(("kind", i))
                            .w(px(38.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(if is_cur { t.accent } else { t.border })
                            .text_color(if is_cur { t.accent } else { t.text_secondary })
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(lbl)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_block_kind_at(ix, k.clone(), cx)))
                    })))
                    .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
            })
            .when(matches!(kind, BlockKind::Callout { .. }), |d| {
                d.child(div().px_2().pb(px(4.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("背景颜色")))
                    .child(div().flex().flex_row().gap(px(6.)).px_2().pb_2().children((1..8u8).map(|c| {
                        div()
                            .id(("cc", c as usize))
                            .size(px(20.))
                            .rounded_full()
                            .bg(t.callout_bg[c as usize])
                            .border_1()
                            .border_color(t.callout_border[c as usize])
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_callout(ix, None, Some(c), cx);
                            }))
                    })))
                    .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
            })
            .child(item("bm-cut", "scissors", "剪切", "⌘X", false).on_click(cx.listener(move |this, _, _, cx| {
                this.pop.block_menu = None;
                this.cut_block_at(ix, cx);
            })))
            .child(item("bm-copy", "copy", "复制", "⌘C", false).on_click(cx.listener(move |this, _, _, cx| {
                this.pop.block_menu = None;
                this.copy_block_at(ix, cx);
            })))
            .child(item("bm-dup", "copy", "创建副本", "⌘D", false).on_click(cx.listener(move |this, _, _, cx| this.duplicate_block_at(ix, cx))))
            .when(self.has_children(ix), |d| {
                let collapsed = self.content.blocks.get(ix).is_some_and(|b| b.collapsed);
                d.child(
                    item("bm-fold", if collapsed { "chevron-right" } else { "chevron-down" }, if collapsed { "展开子项" } else { "折叠子项" }, "⌘.", false)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pop.block_menu = None;
                            this.toggle_fold_at(ix, cx);
                        })),
                )
            })
            .child(item("bm-focus", "focus", if self.focus_ix() == Some(ix) { "退出聚焦" } else { "聚焦此项" }, "⌥⌘.", false).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.pop.block_menu = None;
                    this.focus_on(ix, cx);
                },
            )))
            .when(self.content.blocks.get(ix).is_some_and(|b| b.sync.is_some()), |d| {
                d.child(item("bm-unsync", "link", "取消同步", "", false).on_click(cx.listener(move |this, _, _, cx| this.unsync_at(ix, cx))))
            })
            .when(self.content.blocks.get(ix).is_some_and(|b| b.sync.is_none()), |d| {
                d.child(item("bm-sync", "link", "设为同步块", "", false).on_click(cx.listener(move |this, _, _, cx| {
                    this.core.head = Pos::new(ix, 0, 0);
                    this.core.anchor = this.core.head;
                    this.pop.block_menu = None;
                    this.make_sync(cx);
                })))
            })
            .child(item("bm-up", "arrow-up", "上移", "⌃⌘↑", false).on_click(cx.listener(move |this, _, _, cx| {
                this.core.head = Pos::new(ix, 0, 0);
                this.core.anchor = this.core.head;
                this.pop.block_menu = None;
                this.move_block(-1, cx);
            })))
            .child(item("bm-down", "arrow-down", "下移", "⌃⌘↓", false).on_click(cx.listener(move |this, _, _, cx| {
                this.core.head = Pos::new(ix, 0, 0);
                this.core.anchor = this.core.head;
                this.pop.block_menu = None;
                this.move_block(1, cx);
            })))
            .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
            .child(div().px_2().pb(px(4.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("缩进和对齐")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .px_1()
                    .pb_1()
                    .child(
                        div()
                            .id("bm-ind")
                            .h(px(26.))
                            .px_2()
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .border_1()
                            .border_color(t.border)
                            .hover(|s| s.bg(t.hover))
                            .child(icon("chevron-right", 13., t.text_secondary))
                            .child(div().text_size(px(12.)).child(crate::i18n::tr("缩进")))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.core.head = Pos::new(ix, 0, 0);
                                this.core.anchor = this.core.head;
                                this.pop.block_menu = None;
                                this.tab(&Tab, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("bm-outd")
                            .h(px(26.))
                            .px_2()
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .border_1()
                            .border_color(t.border)
                            .hover(|s| s.bg(t.hover))
                            .child(div().text_size(px(12.)).child(crate::i18n::tr("减少缩进")))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.core.head = Pos::new(ix, 0, 0);
                                this.core.anchor = this.core.head;
                                this.pop.block_menu = None;
                                this.shift_tab(&ShiftTab, window, cx);
                            })),
                    )
                    .children([("align-left", "左对齐", 0u8), ("align-center", "居中", 1), ("align-right", "右对齐", 2)].into_iter().map(|(ic, tip, a)| {
                        let cur = self.content.blocks[ix].align;
                        div()
                            .id(("bm-al", a as usize))
                            .size(px(26.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .border_1()
                            .border_color(if cur == a { t.accent } else { t.border })
                            .when(cur == a, |d| d.bg(t.accent_soft))
                            .hover(|s| s.bg(t.hover))
                            .tooltip(move |_, cx| cx.new(|_| crate::ui::Tooltip(tip.into())).into())
                            .child(icon(ic, 14., if cur == a { t.accent } else { t.text_secondary }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_align(ix, a, cx);
                                this.pop.block_menu = None;
                            }))
                    }))
                    .when(cur_indent < 8, |d| d)
                    .child(div().flex_1().flex().justify_end().items_center().pr_1().text_size(px(11.)).text_color(t.text_muted).child(if cur_indent > 0 {
                        format!("缩进 {cur_indent} 级")
                    } else {
                        String::new()
                    })),
            )
            .child(
                div()
                    .id("bm-comment")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(6.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon("comment", 14., t.text_secondary))
                    .child(div().flex_1().child(crate::i18n::tr("评论")))
                    .when(self.comments_on(ix) > 0, |d| d.child(div().text_size(px(11.)).text_color(t.accent).child(format!("{} 条", self.comments_on(ix)))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            this.pop.block_menu = None;
                            if this.comments_on(ix) > 0 {
                                this.open_comment_panel(ix, cx);
                            } else {
                                this.start_comment(ix, ev.position, window, cx);
                            }
                        }),
                    ),
            )
            .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
            .when(matches!(kind, BlockKind::Table { .. }), |d| {
                let (hr, hc) = match kind {
                    BlockKind::Table { header_row, header_col, .. } => (*header_row, *header_col),
                    _ => (true, false),
                };
                let (can_merge, in_merge) = match kind {
                    BlockKind::Table { cols, merges, .. } => {
                        let (s, e) = self.sel();
                        let can = self.has_selection() && s.block == ix && e.block == ix && s.cell != e.cell;
                        let cur = self.core.head.cell;
                        let m = self.core.head.block == ix && crate::model::merge_at(merges, cur / cols.max(&1), cur % cols.max(&1)).is_some();
                        (can, m)
                    }
                    _ => (false, false),
                };
                d.child(div().px_2().pb(px(4.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("表格")))
                    .when(can_merge, |d| {
                        d.child(item("bm-mg", "table", "合并单元格", "", false).on_click(cx.listener(move |this, _, _, cx| {
                            this.pop.block_menu = None;
                            this.table_merge_selection(ix, cx);
                        })))
                    })
                    .when(in_merge, |d| {
                        d.child(item("bm-sp", "columns", "拆分单元格", "", false).on_click(cx.listener(move |this, _, _, cx| {
                            this.pop.block_menu = None;
                            this.table_split_cell(ix, cx);
                        })))
                    })
                    .child(item("bm-thr", "list", "标题行", if hr { "开" } else { "关" }, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.pop.block_menu = None;
                        this.table_toggle_header_row(ix, cx);
                    })))
                    .child(item("bm-thc", "columns", "标题列", if hc { "开" } else { "关" }, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.pop.block_menu = None;
                        this.table_toggle_header_col(ix, cx);
                    })))
                    .child(item("bm-tew", "layout", "均分列宽", "", false).on_click(cx.listener(move |this, _, _, cx| {
                        this.pop.block_menu = None;
                        this.table_even_widths(ix, cx);
                    })))
                    .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
            })
            .child(item("bm-del", "trash", "删除", "", true).on_click(cx.listener(move |this, _, _, cx| this.delete_block_at(ix, cx))))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.pop.block_menu = None;
                cx.notify();
            }))
    }

    pub(super) fn render_slash(&self, at: Point<Pixels>, caret: Bounds<Pixels>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use super::ops::SlashIcon;
        let q = self.slash_query();
        let items = filter_slash(&q);
        let selected = self.pop.slash.as_ref().map(|s| s.selected).unwrap_or(0);
        let grouped = q.is_empty();
        let mut children: Vec<AnyElement> = vec![];
        let mut selected_child = 0;
        let mut last_group = "";
        if !grouped {
            children.push(div().px_2().py_1().text_size(px(11.)).text_color(t.text_muted).child(format!("搜索「{q}」· {} 项", items.len())).into_any_element());
        }
        for (i, it) in items.into_iter().enumerate() {
            if grouped && it.group != last_group {
                last_group = it.group;
                children.push(
                    div()
                        .px_2()
                        .pt(px(if i == 0 { 4. } else { 10. }))
                        .pb(px(4.))
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .child(crate::i18n::tr(it.group))
                        .into_any_element(),
                );
            }
            if i == selected {
                selected_child = children.len();
            }
            let sel = i == selected;
            let is_table = matches!(&it.action, super::ops::SlashAction::Table);
            let col: Hsla = rgb(it.color).into();
            let col = if t.dark { col.blend(gpui::hsla(0., 0., 1., 0.25)) } else { col };
            let icon_el: AnyElement = match it.icon {
                SlashIcon::Svg(name) => icon(name, 17., col).into_any_element(),
                SlashIcon::Text(tx) => {
                    div().text_size(px(if tx.len() > 2 { 9.5 } else { 12.5 })).font_weight(FontWeight::BOLD).text_color(col).child(tx).into_any_element()
                }
            };
            children.push(
                div()
                    .id(("slash-item", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px_2()
                    .h(px(36.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(t.hover))
                    .hover(|s| s.bg(t.hover))
                    .child(div().w(px(22.)).flex_none().flex().items_center().justify_center().child(icon_el))
                    .child(div().flex_1().text_size(px(14.)).text_color(t.text).child(crate::i18n::tr(it.label)))
                    .child(div().text_size(px(11.)).text_color(t.text_muted).child(it.hint))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if let Some(s) = &mut this.pop.slash {
                            if s.selected != i {
                                s.selected = i;
                                cx.notify();
                            }
                        }
                    }))
                    .when(is_table, |d| {
                        // hover「表格」：右侧弹出行列选择器；移出后（含浮层）延迟收起
                        d.on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                            if *hovered {
                                this.show_slash_table_picker(window.mouse_position(), cx);
                            } else {
                                this.schedule_close_table_picker(cx);
                            }
                        }))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.apply_slash(i, window, cx);
                        }),
                    )
                    .into_any_element(),
            );
        }
        // 只在选中项 / 关键词变化时滚到选中项，拖滚动条 / 滚轮时不被拉回去
        if let Some(s) = &self.pop.slash {
            let key = (selected, q.clone());
            if s.scrolled.borrow().as_ref() != Some(&key) {
                self.slash_scroll.scroll_to_item(selected_child);
                *s.scrolled.borrow_mut() = Some(key);
            }
        }
        let ghost = q.is_empty().then(|| {
            // 「输入关键词」占位，紧跟在 / 后面
            deferred(
                anchored()
                    .position(point(caret.right() + px(2.), caret.top()))
                    .child(div().text_size(px(15.)).line_height(caret.size.height).text_color(t.placeholder).child(crate::i18n::tr("输入关键词"))),
            )
            .with_priority(2)
        });
        let frame = self.frame.clone();
        div().children(ghost).child(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    // 外层不滚动：canvas 记录到的才是菜单真实区域
                    div()
                        .relative()
                        .child(
                            canvas(
                                move |bounds, _, _| {
                                    frame.borrow_mut().slash_bounds = Some(bounds);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                        .child(
                            div()
                                .id("slash")
                                .occlude()
                                .w(px(264.))
                                .max_h(px(420.))
                                .overflow_y_scroll()
                                .track_scroll(&self.slash_scroll)
                                .p(px(6.))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .rounded(px(10.))
                                .shadow_lg()
                                .children(children),
                        )
                        .child(crate::ui::scrollbar(self.slash_scroll.clone(), self.slash_bar.clone(), t.text_muted)),
                ),
            )
            .with_priority(3),
        )
    }
}
