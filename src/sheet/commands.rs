//! 命令分发：菜单、工具栏、右键菜单、快捷键都汇总到 `run(命令名)`。

use super::*;

impl SheetEditor {
    /// 统一命令入口：工具栏、菜单、右键菜单、快捷键、自动化测试都走这里。
    pub(crate) fn run(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (head, arg) = cmd.split_once(':').unwrap_or((cmd, ""));
        if !matches!(head, "painter" | "esc" | "border-color") {
            self.pop = None;
        }
        self.toast = None;
        match COMMANDS.iter().find(|(names, _)| names.contains(&head)) {
            Some((_, f)) => f(self, head, arg, window, cx),
            None => {
                if !self.run_obj(head, arg, window, cx) && !self.run_edit(head, arg, window, cx) && !self.run_cf(head, arg, window, cx) {
                    self.run_data(head, arg, window, cx);
                }
            }
        }
    }
}

type Cmd = fn(&mut SheetEditor, &str, &str, &mut Window, &mut Context<SheetEditor>);

/// 命令名 → 处理函数（多个别名共用一项）。
#[allow(unused_variables)]
static COMMANDS: &[(&[&str], Cmd)] = &[
    (&["up"], |this, head, arg, window, cx| this.move_by(-1, 0, false, cx)),
    (&["down"], |this, head, arg, window, cx| this.move_by(1, 0, false, cx)),
    (&["left"], |this, head, arg, window, cx| this.move_by(0, -1, false, cx)),
    (&["right"], |this, head, arg, window, cx| this.move_by(0, 1, false, cx)),
    (&["ext-up"], |this, head, arg, window, cx| this.move_by(-1, 0, true, cx)),
    (&["ext-down"], |this, head, arg, window, cx| this.move_by(1, 0, true, cx)),
    (&["ext-left"], |this, head, arg, window, cx| this.move_by(0, -1, true, cx)),
    (&["ext-right"], |this, head, arg, window, cx| this.move_by(0, 1, true, cx)),
    (&["edge-up"], |this, head, arg, window, cx| this.edge(-1, 0, false, cx)),
    (&["edge-down"], |this, head, arg, window, cx| this.edge(1, 0, false, cx)),
    (&["edge-left"], |this, head, arg, window, cx| this.edge(0, -1, false, cx)),
    (&["edge-right"], |this, head, arg, window, cx| this.edge(0, 1, false, cx)),
    (&["xedge-up"], |this, head, arg, window, cx| this.edge(-1, 0, true, cx)),
    (&["xedge-down"], |this, head, arg, window, cx| this.edge(1, 0, true, cx)),
    (&["xedge-left"], |this, head, arg, window, cx| this.edge(0, -1, true, cx)),
    (&["xedge-right"], |this, head, arg, window, cx| this.edge(0, 1, true, cx)),
    (&["tab"], |this, head, arg, window, cx| {
        if this.accept_completion(window, cx) || this.pop_tab(false, window, cx) {
            return;
        }
        this.move_by(0, 1, false, cx);
        window.focus(&this.focus);
    }),
    (&["stab"], |this, head, arg, window, cx| {
        if this.pop_tab(true, window, cx) {
            return;
        }
        this.move_by(0, -1, false, cx);
        window.focus(&this.focus);
    }),
    (&["enter"], |this, head, arg, window, cx| {
        // 日期弹层（含日期提醒）：回车保存并关闭
        if matches!(this.cpop, Some(objs::CellPop::Date(_))) {
            this.date_close(true, window, cx);
            return;
        }
        this.start_edit(None, window, cx)
    }),
    (&["senter"], |this, head, arg, window, cx| this.move_by(-1, 0, false, cx)),
    (&["edit"], |this, head, arg, window, cx| this.start_edit(None, window, cx)),
    (&["pgdn"], |this, head, arg, window, cx| {
        let n = ((f32::from(this.bounds.size.height) - HEAD_H) / (25. * this.zoom)).max(1.) as i32;
        this.move_by(n, 0, false, cx)
    }),
    (&["pgup"], |this, head, arg, window, cx| {
        let n = ((f32::from(this.bounds.size.height) - HEAD_H) / (25. * this.zoom)).max(1.) as i32;
        this.move_by(-n, 0, false, cx)
    }),
    (&["home"], |this, head, arg, window, cx| {
        let c = this.cur.c;
        this.move_by(0, 1 - c, false, cx)
    }),
    (&["end"], |this, head, arg, window, cx| {
        let (_, uc) = this.used();
        let c = this.cur.c;
        this.move_by(0, uc - c, false, cx)
    }),
    (&["top"], |this, head, arg, window, cx| {
        let (r, c) = (this.cur.r, this.cur.c);
        this.move_by(1 - r, 1 - c, false, cx)
    }),
    (&["bottom"], |this, head, arg, window, cx| {
        let (ur, uc) = this.used();
        let (r, c) = (this.cur.r, this.cur.c);
        this.move_by(ur - r, uc - c, false, cx)
    }),
    (&["scroll-to-cursor"], |this, head, arg, window, cx| {
        this.ensure_visible();
        cx.notify();
    }),
    (&["select-all"], |this, head, arg, window, cx| this.select(Cell { r: 1, c: 1 }, Cell { r: 1, c: 1 }, SelKind::All, cx)),
    (&["select-row"], |this, head, arg, window, cx| {
        let (a, c) = (this.anchor, this.cur);
        this.select(a, c, SelKind::Rows, cx)
    }),
    (&["select-col"], |this, head, arg, window, cx| {
        let (a, c) = (this.anchor, this.cur);
        this.select(a, c, SelKind::Cols, cx)
    }),
    (&["clear-content"], |this, head, arg, window, cx| {
        let a = this.sel_area();
        let (r0, c0, r1, c1) = (a.row, a.column, a.row + a.height - 1, a.column + a.width - 1);
        let inside = |r: i32, c: i32| r >= r0 && r <= r1 && c >= c0 && c <= c1;
        // 单元格对象（图片 / 附件 / 迷你图 / 日期提醒）和链接也算内容，退格一起清掉
        let objs = this.sx().cells.iter().any(|x| inside(x.r, x.c) && (!x.image.is_empty() || !x.files.is_empty() || x.spark.is_some() || x.remind.is_some()));
        let links: Vec<(i32, i32)> =
            this.model.borrow().get_links(this.sheet).map(|l| l.keys().copied().filter(|&(r, c)| inside(r, c)).collect()).unwrap_or_default();
        this.batch(|this| {
            let mut m = this.model.borrow_mut();
            m.range_clear_contents(&a).ok();
            for (r, c) in links {
                m.delete_cell_link(this.sheet, r, c).ok();
            }
            drop(m);
            if objs {
                this.edit_sx(|x| {
                    for cx_ in x.cells.iter_mut().filter(|x| inside(x.r, x.c)) {
                        cx_.image.clear();
                        cx_.files.clear();
                        cx_.spark = None;
                        cx_.remind = None;
                    }
                });
            }
        });
        this.hover_file = None;
        if this.side.is_some_and(|s| matches!(s, dialogs::Side::Spark(c) if inside(c.r, c.c))) {
            this.side = None;
        }
        this.changed(cx);
        this.sync_formula(cx);
    }),
    (&["clear-all"], |this, head, arg, window, cx| {
        let a = this.sel_area();
        this.model.borrow_mut().range_clear_all(&a).ok();
        this.changed(cx);
        this.sync_formula(cx);
    }),
    (&["esc"], |this, head, arg, window, cx| {
        if this.pop.take().is_some() || this.prompt.take().is_some() || this.cpop.take().is_some() || this.dlg.take().is_some() {
        } else if this.editing.take().is_some() {
            this.sync_formula(cx);
        } else if this.painter.take().is_some() {
        } else if this.find.is_some() {
            this.close_find(window, cx);
        } else if this.clip.as_ref().is_some_and(|c| c.cut) {
            this.clip = None;
        } else if this.side.take().is_some() {
        } else {
            cx.emit(SheetEvent::Close);
        }
        cx.notify();
    }),
    (&["close"], |this, head, arg, window, cx| {
        this.commit_edit(cx);
        this.cpop = None;
        this.release_file_view();
        cx.emit(SheetEvent::Close);
    }),
    (&["fullscreen"], |this, head, arg, window, cx| {
        this.commit_edit(cx);
        this.pop = None;
        cx.emit(SheetEvent::Fullscreen);
    }),
    (&["undo", "redo"], |this, head, arg, window, cx| {
        this.editing = None;
        {
            let mut m = this.model.borrow_mut();
            if head == "undo" {
                m.undo().ok()
            } else {
                m.redo().ok()
            };
        }
        this.apply_filter();
        this.changed(cx);
        this.sync_formula(cx);
    }),
    (&["copy"], |this, head, arg, window, cx| this.copy(false, cx)),
    (&["cut"], |this, head, arg, window, cx| this.copy(true, cx)),
    (&["paste"], |this, head, arg, window, cx| this.paste("normal", cx)),
    (&["paste-values"], |this, head, arg, window, cx| this.paste("values", cx)),
    (&["paste-format"], |this, head, arg, window, cx| this.paste("format", cx)),
    (&["bold"], |this, head, arg, window, cx| this.toggle_style("font.b", cx)),
    (&["italic"], |this, head, arg, window, cx| this.toggle_style("font.i", cx)),
    (&["underline"], |this, head, arg, window, cx| this.toggle_style("font.u", cx)),
    (&["strike"], |this, head, arg, window, cx| this.toggle_style("font.strike", cx)),
    (&["h-left"], |this, head, arg, window, cx| this.set_align(Some("left"), None, cx)),
    (&["h-center"], |this, head, arg, window, cx| this.set_align(Some("center"), None, cx)),
    (&["h-right"], |this, head, arg, window, cx| this.set_align(Some("right"), None, cx)),
    (&["v-top"], |this, head, arg, window, cx| this.style_path("alignment.vertical", "top", cx)),
    (&["v-middle"], |this, head, arg, window, cx| this.style_path("alignment.vertical", "center", cx)),
    (&["v-bottom"], |this, head, arg, window, cx| this.style_path("alignment.vertical", "bottom", cx)),
    (&["wrap", "overflow"], |this, head, arg, window, cx| {
        let a = this.sel_area();
        let on = head == "wrap";
        this.batch(|this| {
            this.set_align_raw(None, Some(on));
            this.edit_sx(|x| x.set_clip(a.row, a.column, a.row + a.height - 1, a.column + a.width - 1, false));
        });
        this.changed(cx);
    }),
    (&["font"], |this, head, arg, window, cx| {
        let name = arg.to_string();
        this.restyle(move |s| s.font.name = if name.is_empty() { "Inter".into() } else { name.clone() }, cx)
    }),
    (&["size"], |this, head, arg, window, cx| this.style_path("font.size", arg, cx)),
    (&["font-inc"], |this, head, arg, window, cx| this.style_path("font.size_delta", "1", cx)),
    (&["font-dec"], |this, head, arg, window, cx| this.style_path("font.size_delta", "-1", cx)),
    (&["color", "fill"], |this, head, arg, window, cx| {
        SheetEditor::remember_color(arg);
        this.style_path(if head == "color" { "font.color" } else { "fill.color" }, arg, cx)
    }),
    (&["opt-color", "spark-color", "spark-mark"], |this, head, arg, window, cx| this.apply_obj_color(head, arg, cx)),
    (&["border-color"], |this, head, arg, window, cx| {
        SheetEditor::remember_color(arg);
        this.border_color = if arg.is_empty() { "#1f2329".into() } else { arg.to_string() };
        cx.notify();
    }),
    (&["num"], |this, head, arg, window, cx| this.style_path("num_fmt", arg, cx)),
    (&["dec-inc"], |this, head, arg, window, cx| this.decimals(true, cx)),
    (&["dec-dec"], |this, head, arg, window, cx| this.decimals(false, cx)),
    (&["border"], |this, head, arg, window, cx| this.set_border(arg, cx)),
    (&["clear-format"], |this, head, arg, window, cx| {
        let a = this.sel_area();
        this.model.borrow_mut().range_clear_formatting(&a).ok();
        this.changed(cx);
    }),
    (&["painter", "painter-sticky"], |this, head, arg, window, cx| {
        if this.painter.is_some() {
            this.painter = None;
        } else {
            let a = this.sel_used();
            let m = this.model.borrow();
            let styles = (a.row..a.row + a.height)
                .map(|r| (a.column..a.column + a.width).map(|c| m.get_model().get_style_for_cell(this.sheet, r, c).unwrap_or_default()).collect())
                .collect();
            drop(m);
            this.painter = Some((styles, head == "painter-sticky"));
            this.toast = Some(if head == "painter-sticky" {
                "格式刷（连续使用，Esc 退出）：选择要应用格式的单元格".into()
            } else {
                "格式刷：选择要应用格式的单元格".into()
            });
        }
        cx.notify();
    }),
    (&["merge", "merge-rows", "merge-cols", "unmerge", "merge-toggle"], |this, head, arg, window, cx| {
        let a = this.sel_used();
        let merged = this.merged_at(this.cur).is_some();
        let unmerging = head == "unmerge" || (head == "merge-toggle" && merged);
        {
            let mut m = this.model.borrow_mut();
            if !unmerging {
                // 区域内多格有内容时只保留（每个合并块）左上方第一个非空值，其余清空后再合并；
                // ironcalc 遇到多个非空格会直接拒绝合并
                let parts = match head {
                    "merge-rows" => (a.row..a.row + a.height).map(|r| area(a.sheet, r, a.column, r, a.column + a.width - 1)).collect(),
                    "merge-cols" => (a.column..a.column + a.width).map(|c| area(a.sheet, a.row, c, a.row + a.height - 1, c)).collect(),
                    _ => vec![area(a.sheet, a.row, a.column, a.row + a.height - 1, a.column + a.width - 1)],
                };
                let mut dropped = false;
                for p in parts {
                    let filled: Vec<(i32, i32)> = (p.row..p.row + p.height)
                        .flat_map(|r| (p.column..p.column + p.width).map(move |c| (r, c)))
                        .filter(|&(r, c)| m.get_cell_content(p.sheet, r, c).map(|s| !s.is_empty()).unwrap_or(false))
                        .collect();
                    for &(r, c) in filled.iter().skip(1) {
                        m.range_clear_contents(&area(p.sheet, r, c, r, c)).ok();
                        dropped = true;
                    }
                }
                if dropped {
                    this.toast = Some("合并后仅保留左上角单元格的内容".into());
                }
            }
            match head {
                "merge-rows" => m.merge_cells_across(&a).ok(),
                "merge-cols" => m.merge_cells_down(&a).ok(),
                "unmerge" => m.unmerge_cells(&a).ok(),
                _ if head == "merge-toggle" && merged => m.unmerge_cells(&a).ok(),
                _ => m.merge_cells(&a).ok(),
            };
        }
        this.changed(cx);
    }),
    (&["fill-down"], |this, head, arg, window, cx| this.fill_dir(true, cx)),
    (&["fill-right"], |this, head, arg, window, cx| this.fill_dir(false, cx)),
    (&["insert-date", "insert-time"], |this, head, arg, window, cx| {
        let now = chrono::Local::now();
        let v = if head == "insert-date" { now.format("%Y-%m-%d").to_string() } else { now.format("%H:%M:%S").to_string() };
        if this.editing.is_some() {
            let t = format!("{}{v}", this.editing_text(cx).unwrap_or_default());
            this.set_edit_text(&t, cx);
        } else {
            let c = this.cur;
            this.set_value(c, &v, cx);
            this.sync_formula(cx);
        }
    }),
    (&["insert-fn"], |this, head, arg, window, cx| {
        // 插入函数：进入编辑并写好 `=NAME(`
        this.start_edit(Some(format!("={arg}(")), window, cx);
    }),
    (&["autosum"], |this, head, arg, window, cx| this.autosum(window, cx)),
    (&["find"], |this, head, arg, window, cx| this.open_find(false, window, cx)),
    (&["replace"], |this, head, arg, window, cx| this.open_find(true, window, cx)),
    (&["shortcuts"], |this, head, arg, window, cx| {
        this.pop = Some(Popup { kind: Pop::Shortcuts, at: point(px(0.), px(0.)) });
        cx.notify();
    }),
    (&["zoom-in", "zoom-out", "zoom-reset", "zoom"], |this, head, arg, window, cx| {
        const STEPS: [f32; 9] = [0.5, 0.75, 0.9, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0];
        let z = this.zoom;
        this.zoom = match head {
            "zoom-in" => STEPS.iter().copied().find(|s| *s > z + 0.01).unwrap_or(3.),
            "zoom-out" => STEPS.iter().rev().copied().find(|s| *s < z - 0.01).unwrap_or(0.5),
            "zoom" => arg.parse::<f32>().map(|v| v / 100.).unwrap_or(1.).clamp(0.5, 3.),
            _ => 1.,
        };
        this.scroll = (this.scroll.0 * this.zoom / z, this.scroll.1 * this.zoom / z);
        this.ensure_visible();
        cx.notify();
    }),
    (&["insert-before"], |this, head, arg, window, cx| {
        let w = if this.kind == SelKind::Cols { "col-left" } else { "row-above" };
        this.rows_op(w, 0, cx)
    }),
    (&["insert-after"], |this, head, arg, window, cx| {
        let w = if this.kind == SelKind::Cols { "col-right" } else { "row-below" };
        this.rows_op(w, 0, cx)
    }),
    (&["delete-sel"], |this, head, arg, window, cx| {
        let w = if this.kind == SelKind::Cols { "del-cols" } else { "del-rows" };
        this.rows_op(w, 0, cx)
    }),
    (
        &["row-above", "row-below", "col-left", "col-right", "del-rows", "del-cols", "hide-rows", "hide-cols", "unhide-rows", "unhide-cols"],
        |this, head, arg, window, cx| this.rows_op(head, arg.parse().unwrap_or(0), cx),
    ),
    (&["freeze-none"], |this, head, arg, window, cx| this.freeze(Some(0), Some(0), cx)),
    (&["freeze-rows"], |this, head, arg, window, cx| this.freeze(Some(arg.parse().unwrap_or(1)), None, cx)),
    (&["freeze-cols"], |this, head, arg, window, cx| this.freeze(None, Some(arg.parse().unwrap_or(1)), cx)),
    (&["freeze-here"], |this, head, arg, window, cx| {
        // 冻结至当前格：上方的行与左侧的列（「冻结至第 N 行 X 列」）
        let (r, c) = (this.cur.r, this.cur.c);
        this.freeze(Some(r), Some(c), cx)
    }),
    (&["freeze-row-here"], |this, head, arg, window, cx| {
        let r = this.cur.r;
        this.freeze(Some(r), None, cx)
    }),
    (&["freeze-col-here"], |this, head, arg, window, cx| {
        let c = this.cur.c;
        this.freeze(None, Some(c), cx)
    }),
    (&["sort-asc"], |this, head, arg, window, cx| this.sort(true, arg.parse().ok(), cx)),
    (&["sort-desc"], |this, head, arg, window, cx| this.sort(false, arg.parse().ok(), cx)),
    (&["filter"], |this, head, arg, window, cx| this.toggle_filter(cx)),
    (&["filter-clear"], |this, head, arg, window, cx| {
        if let Some(f) = &mut this.filter {
            if let Ok(c) = arg.parse::<i32>() {
                f.excl.remove(&c);
            } else {
                f.excl.clear();
            }
        }
        this.apply_filter();
        this.save_view(cx);
        cx.notify();
    }),
    (&["toolbar"], |this, head, arg, window, cx| {
        this.collapsed = !this.collapsed;
        cx.notify();
    }),
    (&["show-formulas"], |this, head, arg, window, cx| {
        this.show_formulas = !this.show_formulas;
        cx.notify();
    }),
    (&["grid"], |this, head, arg, window, cx| {
        let on = this.model.borrow().get_show_grid_lines(this.sheet).unwrap_or(true);
        this.model.borrow_mut().set_show_grid_lines(this.sheet, !on).ok();
        this.changed(cx);
    }),
    (&["f4"], |this, head, arg, window, cx| {
        if let Some(t) = this.editing_text(cx) {
            let n = t.chars().count();
            let res = this.model.borrow().cycle_reference(&t, n, n);
            if let Ok((new, ..)) = res {
                this.set_edit_text(&new, cx);
            }
        }
    }),
    (&["new-sheet"], |this, head, arg, window, cx| {
        this.model.borrow_mut().new_sheet().ok();
        let n = this.model.borrow().get_worksheets_properties().len() as u32;
        this.switch_sheet(n.saturating_sub(1), cx);
        this.changed(cx);
    }),
    (&["next-sheet", "prev-sheet"], |this, head, arg, window, cx| {
        let n = this.model.borrow().get_worksheets_properties().len() as u32;
        let s = if head == "next-sheet" { (this.sheet + 1) % n } else { (this.sheet + n - 1) % n };
        this.switch_sheet(s, cx);
    }),
    (&["sheet"], |this, head, arg, window, cx| this.switch_sheet(arg.parse().unwrap_or(0), cx)),
    (&["dup-sheet"], |this, head, arg, window, cx| {
        let s: u32 = arg.parse().unwrap_or(this.sheet);
        this.model.borrow_mut().duplicate_sheet(s).ok();
        this.changed(cx);
    }),
    (&["del-sheet"], |this, head, arg, window, cx| {
        let s: u32 = arg.parse().unwrap_or(this.sheet);
        if this.model.borrow().get_worksheets_properties().len() > 1 {
            this.model.borrow_mut().delete_sheet(s).ok();
            this.switch_sheet(s.saturating_sub(1), cx);
            this.changed(cx);
        }
    }),
    (&["move-sheet"], |this, head, arg, window, cx| {
        // move-sheet:<索引>,<新位置>
        let (a, b) = arg.split_once(',').unwrap_or(("0", "0"));
        let (a, b) = (a.parse::<u32>().unwrap_or(0), b.parse::<i64>().unwrap_or(0));
        let n = this.model.borrow().get_worksheets_properties().len() as i64;
        if b >= 0 && b < n {
            this.model.borrow_mut().move_sheet(a, b as u32).ok();
            this.switch_sheet(b as u32, cx);
            this.changed(cx);
        }
    }),
    (&["tab-color"], |this, head, arg, window, cx| {
        let (s, c) = arg.split_once(',').unwrap_or(("0", ""));
        let color = ironcalc::base::types::Color::from_param(c).unwrap_or_default();
        this.model.borrow_mut().set_sheet_color(s.parse().unwrap_or(0), &color).ok();
        this.changed(cx);
    }),
    (&["rename-sheet"], |this, head, arg, window, cx| {
        let s: u32 = arg.parse().unwrap_or(this.sheet);
        this.start_rename(s, window, cx);
    }),
    (&["hl"], |this, head, arg, window, cx| {
        let v = arg.to_string();
        set_view_prefs(&this.obj_id, |p| p.hl = (!v.is_empty()).then_some(v));
        cx.notify();
    }),
    (&["add-rows"], |this, head, arg, window, cx| {
        let n = this.add_rows.read(cx).text().trim().parse::<i32>().unwrap_or(20).clamp(1, 10000);
        let (ur, _) = this.used();
        this.extra_rows += n;
        this.geo_key.0 = u64::MAX;
        let c = this.cur.c;
        this.select(Cell { r: ur + 1, c }, Cell { r: ur + 1, c }, SelKind::Cells, cx);
        this.toast = Some(format!("已在末尾添加 {n} 行").into());
    }),
    (&["import"], |this, head, arg, window, cx| this.import(cx)),
    (&["export-xlsx"], |this, head, arg, window, cx| this.export(true, cx)),
    (&["export-csv"], |this, head, arg, window, cx| this.export(false, cx)),
];
