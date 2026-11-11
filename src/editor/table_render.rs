//! 表格块与分栏块的渲染，以及插入表格时的尺寸选择器。

use super::render::*;
use super::*;
use crate::model::{BlockKind, RichText};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};

impl Editor {
    /// 表格：光标进入表格后，上方出现列手柄条、左侧出现行手柄条。
    /// 点手柄选中整列 / 整行（左上角选中全表），停在手柄条的边界上出现「+」与插入线，
    /// 拖动列边线调整列宽；选中多格后可合并。手柄占用网格里单独的一行一列，
    /// 行高天然与内容一致，不需要额外测量。
    pub(super) fn render_table(&self, ix: usize, kind: &BlockKind, focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::model::merge_origin;
        /// 行手柄列宽 / 列手柄行高（含与表格的间隙）
        const HANDLE: f32 = 16.;
        const BAR: f32 = 11.;
        let BlockKind::Table { rows, cols, cells, header_row, header_col, merges, widths } = kind else {
            return div().into_any_element();
        };
        let (rows, cols) = ((*rows).max(1), (*cols).max(1));
        let bid = self.content.blocks[ix].id;
        let active = focused && self.core.head.block == ix;
        let rect = if focused { self.table_sel_rect(ix) } else { None };
        let in_merge = active && rect.is_none() && crate::model::merge_at(merges, self.core.head.cell / cols, self.core.head.cell % cols).is_some();
        let is_head = |r: usize, c: usize| (r == 0 && *header_row) || (c == 0 && *header_col);
        // 列宽：拖动中用预览值，其次用保存的比例，否则均分
        let weights: Vec<f32> = match self.sess.table_col_drag.as_ref().filter(|d| d.block == ix) {
            Some(d) => d.cur.clone(),
            None => widths.clone().filter(|w| w.len() == cols).unwrap_or_else(|| vec![1.; cols]),
        };
        let mut template = vec![-HANDLE];
        template.extend(weights.iter().copied());
        let geo = self.caches.table_geo.borrow().get(&bid).cloned().unwrap_or_default();
        let geo_ok = geo.cols.len() == cols && geo.rows.len() == rows;
        let hover = self.sess.table_insert_hover.filter(|h| h.block == ix);
        let all_sel = rect == Some((0, 0, rows - 1, cols - 1));
        let bar_color = |on: bool| if on { t.accent } else { t.border };
        let geo_store = self.caches.table_geo.clone();

        let mut grid = div().id(("tb-grid", bid as usize)).relative().w_full().grid().grid_col_weights(template);
        // 整张表的区域（含手柄）
        {
            let geo_store = geo_store.clone();
            grid = grid.child(
                canvas(
                    move |b, _, _| {
                        geo_store.borrow_mut().entry(bid).or_default().grid = b;
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }
        // 跨行合并格撑高最后一行：taffy 不会按跨多行的格子扩展行高，这里用上一帧量到的内容高度补上。
        // 只依赖「合并区其余行」的高度，所以结果稳定，不会来回抖动。
        let mut row_min = vec![0f32; rows];
        if geo.rows.len() == rows {
            for m in merges.iter().filter(|m| m.row_span > 1 && m.row + m.row_span <= rows) {
                let Some(need) = geo.need.get(&(m.row * cols + m.col)) else { continue };
                let last = m.row + m.row_span - 1;
                let others: f32 = (m.row..last).map(|r| f32::from(geo.rows[r].size.height)).sum();
                row_min[last] = row_min[last].max(need - others);
            }
        }
        // 不在编辑中、也没有合并格的表格不需要手柄与几何：只留一行占位，版面保持一致，元素少一大截
        let full_handles = active || !merges.is_empty() || !geo_ok;
        if !full_handles {
            grid = grid.child(div().row_start(1).row_end(2).col_start(1).col_end((cols + 2) as i16).h(px(HANDLE)));
        } else {
            // 左上角：选中全表
            grid = grid.child(
                div().row_start(1).row_end(2).col_start(1).col_end(2).pr(px(HANDLE - BAR)).pb(px(HANDLE - BAR)).child(
                    div()
                        .id(("tb-all", bid as usize))
                        .size(px(BAR))
                        .rounded(px(3.))
                        .bg(bar_color(all_sel))
                        .cursor_pointer()
                        .when(!active, |d| d.invisible())
                        .hover(|s| s.bg(t.accent))
                        .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("选中整张表格").into())).into())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                window.focus(&this.focus);
                                this.table_select(ix, (0, 0, rows - 1, cols - 1), cx);
                            }),
                        ),
                ),
            );
            // 列手柄
            for c in 0..cols {
                let geo_store = geo_store.clone();
                grid = grid.child(self.render_table_part_2(ix, t, cx, active, bid, c, cols, geo_store.clone(), rows, rect));
            }
            // 行手柄
            for r in 0..rows {
                let geo_store = geo_store.clone();
                grid = grid.child(self.render_table_part(ix, t, cx, active, bid, cols, geo_store.clone(), r, &row_min, rows, rect));
            }
        }
        // 单元格：支持跨行跨列（合并单元格）
        for r in 0..rows {
            for c in 0..cols {
                let (orow, ocol, rs, cs) = merge_origin(merges, r, c);
                if (orow, ocol) != (r, c) {
                    continue;
                }
                let cell = r * cols + c;
                let sel = rect.is_some_and(|(r1, c1, r2, c2)| r >= r1 && r <= r2 && c >= c1 && c <= c2);
                let head = is_head(r, c);
                let spec = SlotSpec { weight: if head { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }, size: 14., line_h: 23., ..SlotSpec::body(t) };
                let text = cells.get(cell).cloned().unwrap_or_default();
                grid = grid.child(
                    div()
                        .relative()
                        .row_start((r + 2) as i16)
                        .row_end((r + rs + 2) as i16)
                        .col_start((c + 2) as i16)
                        .col_end((c + cs + 2) as i16)
                        .min_w(px(0.))
                        .px(px(10.))
                        .py(px(6.))
                        .border_color(t.border_strong)
                        .border_r_1()
                        .border_b_1()
                        .when(r == 0, |d| d.border_t_1())
                        .when(c == 0, |d| d.border_l_1())
                        .when(head, |d| d.bg(t.table_header))
                        .when(sel, |d| d.bg(t.accent_soft))
                        // 垂直对齐：单元格撑满整行高度，内容按 valign 上 / 中 / 下排布
                        .flex()
                        .flex_col()
                        .when(text.valign == 1, |d| d.justify_center())
                        .when(text.valign == 2, |d| d.justify_end())
                        .child(div().relative().w_full().child(self.render_slot(ix, cell, &text, spec, focused, t)).children((rs > 1).then(|| {
                            // 跨行合并格：记下内容实际高度，下一帧据此撑高它的最后一行
                            let geo_store = geo_store.clone();
                            canvas(
                                move |b, window, _| {
                                    let need = f32::from(b.size.height) + 14.;
                                    let mut g = geo_store.borrow_mut();
                                    let old = g.entry(bid).or_default().need.insert(cell, need);
                                    if old.is_none_or(|o| (o - need).abs() > 0.5) {
                                        window.refresh();
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                        })))
                        .on_mouse_down(MouseButton::Left, self.slot_click(ix, cell, cx)),
                );
            }
        }

        // ── 覆盖层（依赖上一帧几何）──
        // 阅读模式：不显示行列手柄、列宽拖柄与插入「+」
        if geo_ok && active && !self.sess.read_only {
            let ox = geo.grid.left();
            let oy = geo.grid.top();
            let body_top = geo.rows[0].top() - oy;
            let body_h = geo.rows[rows - 1].bottom() - geo.rows[0].top();
            let body_left = geo.cols[0].left() - ox;
            let body_w = geo.cols[cols - 1].right() - geo.cols[0].left();
            let col_x = |k: usize| if k < cols { geo.cols[k].left() - ox } else { geo.cols[cols - 1].right() - ox };
            let row_y = |k: usize| if k < rows { geo.rows[k].top() - oy } else { geo.rows[rows - 1].bottom() - oy };
            // 列宽拖柄：每条内部列边线
            for c in 0..cols.saturating_sub(1) {
                let dragging = self.sess.table_col_drag.as_ref().is_some_and(|d| d.block == ix && d.col == c);
                grid = grid.child(
                    div()
                        .id(("tb-rs", bid as usize * 64 + c))
                        .absolute()
                        .left(col_x(c + 1) - px(4.))
                        .top(body_top)
                        .w(px(8.))
                        .h(body_h)
                        .flex()
                        .justify_center()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .group("tb-rs")
                        .child(div().w(px(2.)).h_full().when(!dragging, |d| d.invisible().group_hover("tb-rs", |s| s.visible())).bg(t.accent))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.start_table_col_drag(ix, c, ev.position, cx);
                            }),
                        ),
                );
            }
            // 插入热区：列手柄条上的每条边界、行手柄条上的每条边界
            let zone = |id: usize, h: TableInsertHover| {
                let on = hover == Some(h);
                div()
                    .id(("tb-ins", id))
                    .absolute()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(16.))
                            .rounded_full()
                            .bg(t.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .shadow_sm()
                            .when(!on, |d| d.invisible())
                            .child(icon("plus", 10., gpui::white())),
                    )
                    .tooltip(move |_, cx| cx.new(|_| Tooltip(if h.col { "插入列" } else { "插入行" }.into())).into())
                    .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                        let next = if *over { Some(h) } else { this.sess.table_insert_hover.filter(|x| *x != h) };
                        if this.sess.table_insert_hover != next {
                            this.sess.table_insert_hover = next;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.checkpoint(EditKind::Other);
                            if h.col {
                                this.table_insert_col(h.block, h.at, cx);
                            } else {
                                this.table_insert_row(h.block, h.at, cx);
                            }
                        }),
                    )
            };
            for k in 0..=cols {
                let h = TableInsertHover { block: ix, col: true, at: k };
                grid = grid.child(zone(bid as usize * 1024 + k, h).left(col_x(k) - px(9.)).top(px(-5.)).w(px(18.)).h(px(BAR + 10.)));
            }
            for k in 0..=rows {
                let h = TableInsertHover { block: ix, col: false, at: k };
                grid = grid.child(zone(bid as usize * 1024 + 512 + k, h).top(row_y(k) - px(9.)).left(px(-5.)).h(px(18.)).w(px(BAR + 10.)));
            }
            // 插入预览线
            if let Some(h) = hover {
                grid = grid.child(if h.col {
                    div().absolute().left(col_x(h.at) - px(1.)).top(body_top).w(px(2.)).h(body_h).bg(t.accent)
                } else {
                    div().absolute().top(row_y(h.at) - px(1.)).left(body_left).h(px(2.)).w(body_w).bg(t.accent)
                });
            }
        }

        let toolbar = active.then(|| {
            let btn = |id: &'static str, ic: &'static str, tip: &'static str, op: &'static str, on: bool, danger: bool| {
                let color = if on { t.accent } else { t.text_secondary };
                let danger_c = t.fg_palette[1];
                div()
                    .id(id)
                    .size(px(28.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.accent_soft))
                    .hover(move |s| if danger { s.bg(danger_c.opacity(0.1)) } else { s.bg(t.hover) })
                    .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                    .child(icon(ic, 16., color))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.table_op(ix, op, cx);
                        }),
                    )
            };
            let sep = || div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(t.border);
            let text_btn = |id: &'static str, ic: &'static str, label: &'static str, op: &'static str| {
                div()
                    .id(id)
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(t.text)
                    .hover(|s| s.bg(t.hover))
                    .child(icon(ic, 15., t.text_secondary))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.table_op(ix, op, cx);
                        }),
                    )
            };
            div()
                .absolute()
                .top(px(-44.))
                .right_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(1.))
                .p(px(3.))
                .rounded(px(8.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .shadow_md()
                .occlude()
                .when(rect.is_some(), |d| d.child(text_btn("tb-mg", "cells-merge", "合并单元格", "merge")).child(sep()))
                .when(in_merge, |d| d.child(text_btn("tb-sp", "cells-split", "拆分单元格", "split")).child(sep()))
                .child(btn("tb-ra", "row-above", "在上方插入行", "row_above", false, false))
                .child(btn("tb-rb", "row-below", "在下方插入行", "row_below", false, false))
                .child(btn("tb-cl", "col-left", "在左侧插入列", "col_left", false, false))
                .child(btn("tb-cr", "col-right", "在右侧插入列", "col_right", false, false))
                .child(sep())
                .child(btn("tb-hr", "header-row", "标题行", "header_row", *header_row, false))
                .child(btn("tb-hc", "header-col", "标题列", "header_col", *header_col, false))
                .when(widths.is_some(), |d| d.child(btn("tb-ev", "cols-even", "均分列宽", "even_widths", false, false)))
                .child(sep())
                .child(btn("tb-dr", "row-del", "删除所在行", "del_row", false, true))
                .child(btn("tb-dc", "col-del", "删除所在列", "del_col", false, true))
                .child(btn("tb-dt", "trash", "删除表格", "del_table", false, true))
        });

        div()
            .id(("table-wrap", ix))
            .relative()
            // 左侧行手柄挂在正文边距里，表格本体与正文左对齐
            .ml(px(-HANDLE))
            .child(grid)
            .children(toolbar)
            .into_any_element()
    }
    /// 从 `render_table` 拆出。
    fn render_table_part_2(
        &self,
        ix: usize,
        t: &Theme,
        cx: &mut Context<Self>,
        active: bool,
        bid: u64,
        c: usize,
        cols: usize,
        geo_store: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<u64, TableGeo>>>,
        rows: usize,
        rect: Option<(usize, usize, usize, usize)>,
    ) -> AnyElement {
        const HANDLE: f32 = 16.;
        const BAR: f32 = 11.;
        let col_sel = |c: usize| rect.is_some_and(|(r1, c1, r2, c2)| r1 == 0 && r2 == rows - 1 && c >= c1 && c <= c2);
        let bar_color = |on: bool| if on { t.accent } else { t.border };
        div()
            .row_start(1)
            .row_end(2)
            .col_start((c + 2) as i16)
            .col_end((c + 3) as i16)
            .pb(px(HANDLE - BAR))
            .child(
                canvas(
                    move |b, window, _| {
                        let mut g = geo_store.borrow_mut();
                        let g = g.entry(bid).or_default();
                        g.cols.resize(cols, Bounds::default());
                        let old = std::mem::replace(&mut g.cols[c], b);
                        // 覆盖层用上一帧的几何：编辑中几何变了就补一帧
                        if active && old.size != b.size {
                            window.refresh();
                        }
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
                    .id(("tb-ch", bid as usize * 64 + c))
                    .h(px(BAR))
                    .w_full()
                    .when(c == 0, |d| d.rounded_l(px(3.)))
                    .when(c == cols - 1, |d| d.rounded_r(px(3.)))
                    .border_r_1()
                    .border_color(t.bg)
                    .bg(bar_color(col_sel(c)))
                    .cursor_pointer()
                    .when(!active, |d| d.invisible())
                    .hover(|s| s.bg(if col_sel(c) { t.accent } else { t.border_strong }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.table_select(ix, (0, c, rows - 1, c), cx);
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 从 `render_table` 拆出。
    fn render_table_part(
        &self,
        ix: usize,
        t: &Theme,
        cx: &mut Context<Self>,
        active: bool,
        bid: u64,
        cols: usize,
        geo_store: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<u64, TableGeo>>>,
        r: usize,
        row_min: &[f32],
        rows: usize,
        rect: Option<(usize, usize, usize, usize)>,
    ) -> AnyElement {
        const HANDLE: f32 = 16.;
        const BAR: f32 = 11.;
        let row_sel = |r: usize| rect.is_some_and(|(r1, c1, r2, c2)| c1 == 0 && c2 == cols - 1 && r >= r1 && r <= r2);
        let bar_color = |on: bool| if on { t.accent } else { t.border };
        div()
            .row_start((r + 2) as i16)
            .row_end((r + 3) as i16)
            .col_start(1)
            .col_end(2)
            .when(row_min[r] > 0., |d| d.min_h(px(row_min[r])))
            .pr(px(HANDLE - BAR))
            .child(
                canvas(
                    move |b, window, _| {
                        let mut g = geo_store.borrow_mut();
                        let g = g.entry(bid).or_default();
                        g.rows.resize(rows, Bounds::default());
                        let old = std::mem::replace(&mut g.rows[r], b);
                        if active && old.size != b.size {
                            window.refresh();
                        }
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
                    .id(("tb-rh", bid as usize * 4096 + r))
                    .w(px(BAR))
                    .h_full()
                    .when(r == 0, |d| d.rounded_t(px(3.)))
                    .when(r == rows - 1, |d| d.rounded_b(px(3.)))
                    .border_b_1()
                    .border_color(t.bg)
                    .bg(bar_color(row_sel(r)))
                    .cursor_pointer()
                    .when(!active, |d| d.invisible())
                    .hover(|s| s.bg(if row_sel(r) { t.accent } else { t.border_strong }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.table_select(ix, (r, 0, r, cols - 1), cx);
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 行列选择器的落点：贴菜单右侧，右侧放不下翻到左侧，再不行贴窗口右边。
    pub(crate) fn table_picker_pos(&self, window: &Window) -> Point<Pixels> {
        const PW: f32 = 198.;
        const PH: f32 = 272.;
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let mouse_y = self.pop.table_picker.as_ref().map(|tp| f32::from(tp.y)).unwrap_or(0.);
        let y = mouse_y.min(vh - PH - 8.).max(8.);
        let f = self.frame.borrow();
        let anchor_x = f32::from(self.slash_at.x);
        let (mleft, mright) = match f.slash_bounds {
            // 偏差过大说明是上一次菜单的残留值，退回锚点推算
            Some(b) if (f32::from(b.left()) - anchor_x).abs() < 400. => (f32::from(b.left()), f32::from(b.right())),
            _ => (anchor_x, anchor_x + 264.),
        };
        let right = mright + 8.;
        let left = mleft - 8. - PW;
        let x = if right + PW <= vw - 8. {
            right
        } else if left >= 8. {
            left
        } else {
            (vw - 8. - PW).max(8.)
        };
        point(px(x), px(y))
    }

    /// 斜杠菜单插入表格时的行列选择器（常见风格：9 列 × 10 行）。
    pub(super) fn render_table_picker(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        const ROWS: usize = 10;
        const COLS: usize = 9;
        let tp = self.pop.table_picker.as_ref()?;
        let pos = self.table_picker_pos(window);
        let cur_r = tp.rows.clamp(1, ROWS);
        let cur_c = tp.cols.clamp(1, COLS);
        let mut grid = div().flex().flex_col().gap(px(2.));
        for r in 0..ROWS {
            let mut line = div().flex().flex_row().gap(px(2.));
            for c in 0..COLS {
                let on = r < cur_r && c < cur_c;
                line = line.child(
                    div()
                        .id(("tp-cell", r * COLS + c))
                        .size(px(18.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .when(on, |d| d.bg(t.accent_soft))
                        .cursor_pointer()
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if !*hovered {
                                return;
                            }
                            this.keep_table_picker();
                            if let Some(tp) = this.pop.table_picker.as_mut() {
                                let (nr, nc) = (r + 1, c + 1);
                                if tp.rows != nr || tp.cols != nc {
                                    tp.rows = nr;
                                    tp.cols = nc;
                                    cx.notify();
                                }
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.insert_table_from_slash(r + 1, c + 1, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(line);
        }
        Some(
            deferred(
                anchored().position(pos).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("table-picker")
                        .occlude()
                        .w(px(198.))
                        .rounded(px(10.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .p(px(10.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.keep_table_picker();
                            } else {
                                this.schedule_close_table_picker(cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.cancel_table_picker(cx)))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .text_size(px(12.))
                                .child(div().text_color(t.text_secondary).child(crate::i18n::tr("插入支持富文本的表格")))
                                .child(div().text_color(t.accent).font_weight(FontWeight::MEDIUM).child(format!("{cur_r} × {cur_c}"))),
                        )
                        .child(grid)
                        .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("移动鼠标选择行列，点击插入"))),
                ),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }

    pub(super) fn render_columns(&self, ix: usize, cols: &[RichText], focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let active = focused && self.core.head.block == ix;
        let n = cols.len();
        let toolbar = active.then(|| {
            let btn = |id: &'static str, label: &'static str, add: bool, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(t.text_secondary)
                    .hover(|s| s.bg(t.hover))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.columns_op(ix, add, cx);
                        }),
                    )
            };
            div()
                .absolute()
                .top(px(-34.))
                .right_0()
                .flex()
                .flex_row()
                .p(px(3.))
                .rounded(px(6.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .shadow_md()
                .text_size(px(12.))
                .occlude()
                .child(btn("col-add", "增加一栏", true, cx))
                .child(btn("col-del", "减少一栏", false, cx))
        });
        div()
            .relative()
            .w_full()
            .flex()
            .flex_row()
            .gap(px(14.))
            .children(cols.iter().enumerate().map(|(c, rt)| {
                let in_col = active && self.core.head.cell == c;
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if in_col {
                        t.accent
                    } else if active {
                        t.border_strong
                    } else {
                        t.border
                    })
                    .child(self.render_slot(
                        ix,
                        c,
                        rt,
                        SlotSpec {
                            placeholder: (focused && rt.is_empty() && self.core.head == Pos::new(ix, c, 0)).then(|| format!("第 {} 栏", c + 1).into()),
                            ..SlotSpec::body(t)
                        },
                        focused,
                        t,
                    ))
                    .on_mouse_down(MouseButton::Left, self.slot_click(ix, c, cx))
            }))
            .children(toolbar)
            .when(n == 0, |d| d.child(crate::i18n::tr("空分栏")))
            .into_any_element()
    }
}
