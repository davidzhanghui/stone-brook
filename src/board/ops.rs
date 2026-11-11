//! 画板命令：原位文字编辑、撤销 / 重做、剪贴板、层级、对齐与批量排版、锁定、分区、导出。

use super::model::*;
use super::view::*;
use crate::canvas::geom::estimate_width;
use crate::canvas::math;
use crate::canvas::{rect, Col, Rect, ShapeKind};
use crate::text_area::{TextArea, TextAreaEvent, TextStyle};
use crate::theme::Theme;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::collections::{HashMap, HashSet};

impl BoardEditor {
    // ───────────── 原位文字编辑 ─────────────

    /// 编辑区域（世界坐标）与排版参数（区域、水平对齐、垂直对齐 0 顶 1 中 2 底、字号、加粗），与画布上的显示保持一致。
    fn edit_geom(&self, el: &El, part: Part) -> (Rect, u8, u8, f32, bool) {
        // 多格图形的某一格：靠左上（类图的属性 / 方法）
        if let (Part::Cell(i), ElKind::Shape(_)) = (part, &el.kind) {
            if let Some(c) = el.compartments().get(i) {
                return (crate::canvas::geom::compartment_text_area(*c), 0, 0, el.font_size, false);
            }
        }
        if let (Part::Cell(i), Some(g)) = (part, el.grid()) {
            let a = el.cell_align(i);
            let v = if a.is_top() {
                0
            } else if a.is_bottom() {
                2
            } else {
                1
            };
            return (crate::canvas::paint::text_area(ShapeKind::Rect, g.cell(i), a), a.h(), v, el.cell_font(i), el.cell_bold(i));
        }
        let boxed = |k: ShapeKind, fs: f32| {
            let a = el.canvas_align();
            (crate::canvas::paint::text_area(k, el.rect(), a), a.h(), el.v_align_eff(), fs, el.bold)
        };
        match &el.kind {
            ElKind::Table { .. } => (
                crate::canvas::paint::text_area(ShapeKind::Rect, rect(el.x, el.y, el.w, el.title_h().max(LANE_TITLE_H)), crate::canvas::Align::Center),
                1,
                1,
                el.font_size + 2.,
                true,
            ),
            ElKind::Shape(k) => boxed(*k, el.font_size),
            ElKind::Sticky => boxed(ShapeKind::Sticky, el.effective_font()),
            ElKind::Text => boxed(ShapeKind::Rect, el.font_size),
            ElKind::Connector { .. } => {
                let m = self.doc.connector_mid(el);
                let fs = (el.font_size - 2.).max(10.);
                let w = (estimate_width(&el.text, fs, false) + 28.).clamp(90., 320.);
                (rect(m.x - w / 2., m.y - fs * 0.9, w, fs * 1.8), 1, 1, fs, false)
            }
            // 代码绘图的标题：左上角外侧的橙色标签
            ElKind::Section if el.code.is_some() => (code_title_rect(el, (estimate_width(&el.text, 13., true) + 24.).max(80.)), 1, 1, 13., true),
            ElKind::Section => {
                let w = (estimate_width(&el.text, 13., true) + 40.).clamp(120., el.w.max(120.));
                (rect(el.x, el.y - 26., w, 24.), 0, 1, 13., true)
            }
            _ => (el.rect(), 0, 0, el.font_size, false),
        }
    }

    /// 编辑框的文字修饰与颜色：表格单元格用单元格自己的设置。
    fn edit_deco_color(el: &El, part: Part, dark: bool) -> (u8, Hsla) {
        let (deco, col) = match part {
            Part::Main => (el.deco, el.text_color),
            Part::Cell(i) if el.is_table() => (el.cell_deco(i), el.cell_fmt(i).color.or(el.text_color)),
            Part::Cell(_) => (0, el.text_color),
        };
        if el.code.is_some() {
            return (deco, gpui::white());
        }
        let color =
            col.map(|c| c.hsla()).unwrap_or(if dark && !matches!(el.kind, ElKind::Sticky) { gpui::rgb(0xe3e4e6).into() } else { gpui::rgb(0x1f2329).into() });
        (deco, color)
    }

    pub(super) fn start_edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        // 表格：编辑选中的单元格（没有选中时编辑第一个）；泳道：编辑标题
        let part = match self.doc.get(id) {
            Some(e) if e.is_table() && !(e.is_lane() && !e.text.is_empty() && self.cell_sel.is_none()) => {
                Part::Cell(self.cell_sel.filter(|(t, _)| *t == id).map(|(_, i)| i).unwrap_or(0))
            }
            _ => Part::Main,
        };
        self.start_edit_part(id, part, window, cx)
    }

    pub(super) fn start_edit_part(&mut self, id: u64, part: Part, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        let Some(el) = self.doc.get(id).cloned() else { return };
        if matches!(el.kind, ElKind::Pen { .. } | ElKind::Image { .. } | ElKind::Icon { .. }) || el.locked {
            return;
        }
        if let (Part::Cell(i), true) = (part, el.is_table()) {
            self.cell_sel = Some((id, i));
        }
        let (_, align, v_align, font, bold) = self.edit_geom(&el, part);
        let main = matches!(part, Part::Main);
        let (deco, color) = Self::edit_deco_color(&el, part, Self::dark(cx));
        let style = TextStyle { font_size: font * self.zoom, line_height: font * 1.38 * self.zoom, color, bold, align, v_align, deco };
        let orig_spans = if main { el.spans.clone() } else { vec![] };
        let placeholder = match (&el.kind, part) {
            (ElKind::Connector { .. }, _) => "添加连线说明",
            (ElKind::Section, _) if el.code.is_some() => "输入标题",
            (ElKind::Section, _) => "分区标题",
            (ElKind::Text, _) => "输入文本",
            (ElKind::Table { .. }, Part::Main) => "泳道图标题",
            _ => "",
        };
        let text = el.part_text(part).to_string();
        let spans = orig_spans.clone();
        let area = cx.new(|cx| {
            let mut a = TextArea::new(&text, placeholder, style, cx);
            a.set_spans(spans);
            a
        });
        // 新建的空元素直接输入；已有文字时光标放到末尾（双击进入编辑）
        if !text.is_empty() {
            area.update(cx, |a, _| a.move_to_end());
        }
        let sub = cx.subscribe_in(&area, window, move |this, area, ev: &TextAreaEvent, window, cx| match ev {
            TextAreaEvent::Changed => {
                let text = area.read(cx).text().to_string();
                let floor = this.editing.as_ref().map(|ed| ed.orig_floor);
                this.live_text(id, part, text, floor, cx);
                if part == Part::Main {
                    let spans = area.read(cx).spans().to_vec();
                    if let Some(i) = this.doc.index(id) {
                        this.doc.els[i].set_spans(spans);
                    }
                }
                cx.notify();
            }
            TextAreaEvent::Commit => {
                this.finish_edit(cx);
                window.focus(&this.focus);
            }
            TextAreaEvent::Tab => match part {
                // 多格图形：Tab 编辑下一格（标题 → 第一格 → … → 回到标题）
                _ if this.doc.get(id).is_some_and(|e| !e.compartments().is_empty()) => {
                    let n = this.doc.get(id).map(|e| e.compartments().len()).unwrap_or(0);
                    let next = match part {
                        Part::Main => Part::Cell(0),
                        Part::Cell(i) if i + 1 < n => Part::Cell(i + 1),
                        _ => Part::Main,
                    };
                    this.start_edit_part(id, next, window, cx);
                }
                Part::Cell(i) => this.table_tab(id, i, window, cx),
                // 思维导图：编辑中按 Tab 结束编辑并添加子节点
                _ if this.doc.get(id).is_some_and(|e| e.is_mind()) => this.mind_add_child(id, 0, window, cx),
                _ => this.sticky_tab(id, window, cx),
            },
        });
        window.focus(&area.focus_handle(cx));
        // 双击进入编辑时，外层 track_focus 会在冒泡阶段把焦点抢回画布，这里阻止它
        window.prevent_default();
        self.editing = Some(Editing {
            id,
            part,
            area,
            orig_text: text,
            orig_spans,
            orig_rect: el.rect(),
            orig_floor: el.part_floor(part),
            orig_el: el.clone(),
            _sub: sub,
        });
        // 编辑中的元素即选中元素，属性栏据此显示文字格式按钮
        if self.sel.len() != 1 || !self.sel.contains(&id) {
            self.sel = [id].into();
        }
        self.popover = None;
        cx.notify();
    }

    /// 输入过程中实时写回文字：文本框随内容长宽，便签的自适应字号、思维导图的布局也随之变化。
    fn live_text(&mut self, id: u64, part: Part, text: String, floor: Option<f32>, cx: &App) {
        let Some(i) = self.doc.index(id) else { return };
        let e = &mut self.doc.els[i];
        let title_before = e.title_h();
        e.set_part_text(part, text);
        // 文字放不下时撑高图形 / 表格行，删字后缩回编辑前的高度
        let ts = cx.text_system().clone();
        let floor = floor.unwrap_or_else(|| e.part_floor(part));
        e.fit_text(part, floor, &mut |t, fs, b, w| crate::canvas::paint::count_lines(&ts, t, fs, b, w));
        if e.is_mind() {
            self.doc.layout_minds();
            return;
        }
        // 泳道图标题清空 = 删除标题栏
        if e.is_lane() && part == Part::Main {
            let d = title_before - e.title_h();
            e.y += d;
            e.h -= d;
            return;
        }
        if matches!(e.kind, ElKind::Text) {
            let fs = e.font_size;
            let widest = e.text.split('\n').map(|l| estimate_width(l, fs, e.bold)).fold(0f32, f32::max);
            e.w = (widest + 24.).clamp(60., 640.);
            let inner = e.w - 16.;
            let rows: f32 = e.text.split('\n').map(|l| (estimate_width(l, fs, e.bold) / inner).ceil().max(1.)).sum();
            e.h = (rows * fs * 1.38 + 12.).max(fs * 1.38 + 12.);
        }
    }

    /// 便签编辑中按 Tab：在右侧新建一张同色便签继续写。
    fn sticky_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.doc.get(id).filter(|e| matches!(e.kind, ElKind::Sticky)).cloned() else { return };
        self.finish_edit(cx);
        self.snapshot();
        let mut el = src.clone();
        el.id = self.doc.alloc_id();
        el.x = src.x + src.w + 24.;
        el.text.clear();
        el.author = crate::settings::author();
        let nid = el.id;
        self.doc.els.push(el);
        self.sel = [nid].into();
        self.commit(cx);
        self.start_edit(nid, window, cx);
    }

    pub(super) fn finish_edit(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.editing.take() else { return };
        let text = ed.area.read(cx).text().to_string();
        self.live_text(ed.id, ed.part, text.clone(), Some(ed.orig_floor), cx);
        let Some(i) = self.doc.index(ed.id) else { return };
        let spans_changed = ed.part == Part::Main && self.doc.els[i].spans != ed.orig_spans;
        let changed = self.doc.els[i].part_text(ed.part) != ed.orig_text || self.doc.els[i].rect() != ed.orig_rect || spans_changed;
        if changed {
            // 撤销点：编辑前的文字、样式分段与尺寸
            let mut before = self.doc.clone();
            before.els[i].set_part_text(ed.part, ed.orig_text.clone());
            if ed.part == Part::Main {
                before.els[i].spans = ed.orig_spans.clone();
            }
            let r = ed.orig_rect;
            (before.els[i].x, before.els[i].y, before.els[i].w, before.els[i].h) = (r.x, r.y, r.w, r.h);
            // 文字撑高了表格行：行高一起恢复
            if let (ElKind::Table { rows, .. }, ElKind::Table { rows: orig, .. }) = (&mut before.els[i].kind, &ed.orig_el.kind) {
                if rows.len() == orig.len() {
                    rows.clone_from(orig);
                }
            }
            self.undo.push_back(before);
            self.redo.clear();
        }
        // 空文本框不保留
        if matches!(self.doc.els[i].kind, ElKind::Text) && self.doc.els[i].text.trim().is_empty() {
            self.doc.els.remove(i);
            self.sel.remove(&ed.id);
        }
        self.commit(cx);
    }

    /// 编辑中的输入框：盖在元素的文字区域上，字号随缩放，看起来就是在原处打字。
    pub(super) fn render_edit_overlay(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ed = self.editing.as_ref()?;
        let el = self.doc.get(ed.id)?.clone();
        let (area, align, v_align, font, bold) = self.edit_geom(&el, ed.part);
        let (deco, color) = Self::edit_deco_color(&el, ed.part, t.dark);
        let style = TextStyle { font_size: font * self.zoom, line_height: font * 1.38 * self.zoom, color, bold, align, v_align, deco };
        let a = ed.area.clone();
        a.update(cx, |a, _| a.set_style(style));
        let r = self.view().r(area);
        let o = self.canvas_bounds.origin;
        let boxed = matches!(el.kind, ElKind::Connector { .. } | ElKind::Section);
        // 单元格编辑时给单元格描一圈蓝框
        let cell = match (ed.part, el.grid()) {
            (Part::Cell(i), Some(g)) => Some(self.view().r(g.cell(i))),
            _ => None,
        };
        Some(
            div()
                .absolute()
                // 必须显式贴到左上角：否则 absolute 元素落在前一个（占满全屏的画布）之后的静态位置，输入框跑出窗口外
                .inset_0()
                .when_some(cell, |d, c| {
                    d.child(div().absolute().left(c.origin.x - o.x).top(c.origin.y - o.y).w(c.size.width).h(c.size.height).border_2().border_color(t.accent))
                })
                .child(
                    div()
                        .absolute()
                        .left(r.origin.x - o.x)
                        .top(r.origin.y - o.y)
                        .w(r.size.width)
                        .h(r.size.height)
                        .when(boxed, |d| d.bg(t.bg).rounded(px(4.)).border_1().border_color(t.accent).px(px(4.)))
                        .child(a),
                )
                .into_any_element(),
        )
    }

    // ───────────── 基础命令 ─────────────

    pub(super) fn mutate_sel(&mut self, cx: &mut Context<Self>, f: impl Fn(&mut El)) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        for e in self.doc.els.iter_mut().filter(|e| self.sel.contains(&e.id)) {
            f(e);
        }
        // 编辑中改了整体样式：输入框里的样式分段跟着更新，否则下次输入会把旧分段写回去
        if let Some(ed) = self.editing.as_ref().filter(|ed| ed.part == Part::Main) {
            let spans = self.doc.get(ed.id).map(|e| e.spans.clone()).unwrap_or_default();
            ed.area.clone().update(cx, |a, _| a.set_spans(spans));
        }
        self.commit(cx);
    }

    /// 编辑文字时选中了部分文字：样式只改选中的部分。`pred` 判断是否已开启，
    /// `set(fmt, on)` 设置开关。返回 false 表示没有选中文字，由调用方改整个元素。
    pub(super) fn fmt_selection(
        &mut self,
        pred: impl Fn(&crate::canvas::Fmt) -> bool,
        set: impl Fn(&mut crate::canvas::Fmt, bool),
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ed) = self.editing.as_ref().filter(|ed| ed.part == Part::Main) else { return false };
        let area = ed.area.clone();
        if area.read(cx).selection().is_empty() {
            return false;
        }
        let on = !area.read(cx).selection_all(pred);
        area.update(cx, |a, cx| a.apply_fmt(|f| set(f, on), cx));
        cx.notify();
        true
    }

    pub(super) fn delete(&mut self, _: &BDelete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_sel(cx);
    }

    /// 删除选区；删除分区时连同分区里的内容（「移除分区」才保留内容）。
    pub(super) fn delete_sel(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        // 选中了单元格：清空单元格内容（表格）
        if let Some((id, i)) = self.cell_sel.filter(|(id, _)| self.single_table().is_some_and(|e| e.id == *id)) {
            if let Some(k) = self.doc.index(id) {
                if !self.doc.els[k].part_text(Part::Cell(i)).is_empty() {
                    self.snapshot();
                    self.doc.els[k].set_part_text(Part::Cell(i), String::new());
                    self.commit(cx);
                }
            }
            return;
        }
        self.snapshot();
        let mut ids = self.sel.clone();
        for id in self.sel.iter() {
            ids.extend(self.doc.section_children(*id));
            // 删除思维导图节点时连同子节点
            if self.doc.get(*id).is_some_and(|e| e.is_mind()) {
                ids.extend(self.doc.mind_subtree(*id));
            }
        }
        self.doc.els.retain(|e| !ids.contains(&e.id) || e.locked);
        self.remove_dangling();
        self.sel.clear();
        self.close_overlays();
        self.commit(cx);
    }

    /// 移除分区：只删分区本身，里面的元素保留。
    pub(super) fn remove_section(&mut self, cx: &mut Context<Self>) {
        let secs: HashSet<u64> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id) && e.is_section()).map(|e| e.id).collect();
        if secs.is_empty() {
            return;
        }
        self.snapshot();
        self.doc.els.retain(|e| !secs.contains(&e.id));
        self.sel.clear();
        self.close_overlays();
        self.commit(cx);
    }

    pub(super) fn do_undo(&mut self, _: &BUndo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(d) = self.undo.pop_back() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.sel.retain(|id| self.doc.get(*id).is_some());
            self.commit(cx);
        }
    }

    pub(super) fn do_redo(&mut self, _: &BRedo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(d) = self.redo.pop() {
            self.undo.push_back(std::mem::replace(&mut self.doc, d));
            self.sel.retain(|id| self.doc.get(*id).is_some());
            self.commit(cx);
        }
    }

    pub(super) fn select_all(&mut self, _: &BSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.sel = self.doc.els.iter().map(|e| e.id).collect();
        self.close_overlays();
        cx.notify();
    }

    /// 插入一组元素的副本（重新分配 id，内部连线跟着改指向），偏移 off，选中副本。
    pub(super) fn insert_copies(&mut self, els: Vec<El>, off: f32) {
        self.insert_copies_at(els, off, off);
    }

    /// 同上，按 (dx, dy) 平移（插入模板时用）。
    pub(super) fn insert_copies_at(&mut self, els: Vec<El>, dx: f32, dy: f32) {
        let mut map = HashMap::new();
        for e in &els {
            map.insert(e.id, self.doc.alloc_id());
        }
        let groups: HashMap<u64, u64> = els.iter().filter_map(|e| e.group).collect::<HashSet<_>>().into_iter().map(|g| (g, self.doc.alloc_id())).collect();
        let mut new_els = vec![];
        for mut e in els {
            e.id = map[&e.id];
            e.x += dx;
            e.y += dy;
            e.locked = false;
            e.group = e.group.and_then(|g| groups.get(&g).copied());
            // 思维导图：父节点也在副本里就跟着改，否则副本成为新的根节点
            if let Some(m) = e.mind.as_mut() {
                m.parent = map.get(&m.parent).copied().unwrap_or(0);
            }
            match &mut e.kind {
                ElKind::Pen { pts } => pts.iter_mut().for_each(|p| {
                    p[0] += dx;
                    p[1] += dy;
                }),
                ElKind::Connector { a, b, .. } => {
                    for end in [a, b] {
                        match end {
                            End::Bound { el, .. } => {
                                if let Some(n) = map.get(el) {
                                    *el = *n;
                                }
                            }
                            End::Free { x, y } => {
                                *x += dx;
                                *y += dy;
                            }
                        }
                    }
                }
                _ => {}
            }
            new_els.push(e);
        }
        self.sel = new_els.iter().map(|e| e.id).collect();
        // 容器副本放到底层，其余放到顶层
        let (secs, rest): (Vec<El>, Vec<El>) = new_els.into_iter().partition(|e| e.is_container());
        for (i, s) in secs.into_iter().enumerate() {
            self.doc.els.insert(i, s);
        }
        self.doc.els.extend(rest);
    }

    pub(super) fn paste_els(&mut self, els: Vec<El>, cx: &mut Context<Self>) {
        if els.is_empty() {
            return;
        }
        self.snapshot();
        // 选中了思维导图节点时，粘贴的节点成为它的子节点
        let target = self.single_mind().map(|e| e.id);
        self.insert_copies(els, 24.);
        if let Some(t) = target {
            let roots: Vec<u64> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id) && e.is_mind() && e.mind_parent().is_none()).map(|e| e.id).collect();
            let mut seq = self.doc.mind_children(t).len() as f32;
            for r in roots {
                if let Some(m) = self.doc.index(r).and_then(|i| self.doc.els[i].mind.as_mut()) {
                    m.parent = t;
                    m.seq = seq;
                    seq += 1.;
                }
                self.doc.restyle_mind_subtree(r);
            }
        }
        self.remove_dangling();
        self.commit(cx);
    }

    /// 复制选区；容器连同其内容、思维导图节点连同子节点一起复制。
    fn selection_with_children(&self) -> Vec<El> {
        let mut ids = self.sel.clone();
        for id in self.sel.iter() {
            ids.extend(self.doc.section_children(*id));
            if self.doc.get(*id).is_some_and(|e| e.is_mind()) {
                ids.extend(self.doc.mind_subtree(*id));
            }
        }
        self.doc.els.iter().filter(|e| ids.contains(&e.id)).cloned().collect()
    }

    pub(super) fn duplicate(&mut self, _: &BDuplicate, _: &mut Window, cx: &mut Context<Self>) {
        let els = self.selection_with_children();
        self.paste_els(els, cx);
    }

    pub(super) fn copy(&mut self, _: &BCopy, _: &mut Window, cx: &mut Context<Self>) {
        self.clip = self.selection_with_children();
        // 同时把文字放进系统剪贴板，方便粘到文档里
        let text: Vec<String> =
            self.clip.iter().flat_map(|e| e.parts().into_iter().map(|p| e.part_text(p).to_string()).collect::<Vec<_>>()).filter(|t| !t.is_empty()).collect();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.join("\n")));
        }
        self.close_overlays();
        cx.notify();
    }

    pub(super) fn paste(&mut self, _: &BPaste, _: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays();
        // 剪贴板里有图片时粘贴图片
        let c = crate::clipboard::read();
        if let Some((bytes, ext)) = c.image {
            if let Some(path) = crate::convert::save_image_bytes(&bytes, ext, &crate::store::data_dir().join("assets")) {
                self.add_image(path, cx);
                return;
            }
        }
        if !self.clip.is_empty() {
            let els = self.clip.clone();
            self.paste_els(els, cx);
            return;
        }
        // 外部文字：粘贴成文本框
        if let Some(text) = c.text.filter(|t| !t.trim().is_empty()) {
            let at = self.to_world(self.canvas_bounds.center());
            self.snapshot();
            let mut el = self.doc.new_el(ElKind::Text, rect(at.x, at.y, 120., 32.), Self::dark(cx));
            el.text = text.trim().to_string();
            let id = el.id;
            self.doc.els.push(el);
            self.live_text(id, Part::Main, text.trim().to_string(), None, cx);
            self.sel = [id].into();
            self.commit(cx);
        }
    }

    pub(super) fn group(&mut self, _: &BGroup, _: &mut Window, cx: &mut Context<Self>) {
        if self.sel.len() < 2 {
            return;
        }
        let g = self.doc.alloc_id();
        self.mutate_sel(cx, |e| e.group = Some(g));
    }

    pub(super) fn ungroup(&mut self, _: &BUngroup, _: &mut Window, cx: &mut Context<Self>) {
        self.mutate_sel(cx, |e| e.group = None);
    }

    pub(super) fn toggle_lock(&mut self, cx: &mut Context<Self>) {
        let lock = !self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).all(|e| e.locked);
        self.mutate_sel(cx, move |e| e.locked = lock);
    }

    pub(super) fn escape(&mut self, _: &BEscape, window: &mut Window, cx: &mut Context<Self>) {
        // 依次收起最上层的东西，收起一样就结束
        if self.close_overlays() || self.templates.take().is_some() {
        } else if self.quick_visible() {
            self.quick_tpl = false;
            self.quick_pick = None;
        } else if self.code_dlg.is_some() {
            self.close_code(window, cx);
        } else if self.painting.take().is_some() {
        } else if self.find.is_some() {
            self.close_find(window, cx);
        } else if self.editing.is_some() {
            self.finish_edit(cx);
        } else if self.tool != Tool::Select {
            self.tool = Tool::Select;
        } else if self.cell_sel.take().is_some() {
        } else if !self.sel.is_empty() {
            self.sel.clear();
        } else {
            self.close(cx);
            return;
        }
        cx.notify();
    }

    pub(super) fn nudge(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        // 表格里选中了单元格：方向键切换单元格
        if self.move_cell(dx.signum() as i32, dy.signum() as i32, cx) {
            return;
        }
        if self.sel.is_empty() {
            // 没有选中元素时方向键平移画布
            self.offset.x -= dx * 4.;
            self.offset.y -= dy * 4.;
            cx.notify();
            return;
        }
        self.snapshot();
        let ids: HashSet<u64> = self.move_set().into_iter().map(|o| o.0).collect();
        for e in self.doc.els.iter_mut().filter(|e| ids.contains(&e.id)) {
            e.x += dx;
            e.y += dy;
            match &mut e.kind {
                ElKind::Pen { pts } => pts.iter_mut().for_each(|p| {
                    p[0] += dx;
                    p[1] += dy;
                }),
                ElKind::Connector { a, b, .. } => {
                    for end in [a, b] {
                        if let End::Free { x, y } = end {
                            *x += dx;
                            *y += dy;
                        }
                    }
                }
                _ => {}
            }
        }
        self.commit(cx);
    }

    // ───────────── 插入 ─────────────

    pub(super) fn add_image(&mut self, path: String, cx: &mut Context<Self>) {
        let (w, h) = image::image_dimensions(&path).map(|(w, h)| (w as f32, h as f32)).unwrap_or((320., 240.));
        let s = (480. / w).min(1.);
        let c = self.to_world(self.canvas_bounds.center());
        self.snapshot();
        let el = self.doc.new_el(ElKind::Image { path }, rect(c.x - w * s / 2., c.y - h * s / 2., w * s, h * s), false);
        self.sel = [el.id].into();
        self.doc.els.push(el);
        self.tool = Tool::Select;
        self.commit(cx);
    }

    pub(super) fn pick_image(&mut self, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions {
                files: true, directories: false, multiple: false, prompt: Some("插入图片".into()), ..Default::default()
            });
        cx.spawn(async move |this: WeakEntity<BoardEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.first() {
                    if let Ok(bytes) = std::fs::read(p) {
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
                        if let Some(path) = crate::convert::save_image_bytes(&bytes, &ext, &crate::store::data_dir().join("assets")) {
                            this.update(cx, |this, cx| this.add_image(path, cx)).ok();
                        }
                    }
                }
            }
        })
        .detach();
    }

    /// 在视口中心添加一个形状（流程图形状库）。选中了一个形状时接在它下方并自动连线。
    #[cfg(feature = "dev")]
    pub(super) fn add_shape(&mut self, kind: ShapeKind, w: f32, h: f32, text: &str, cx: &mut Context<Self>) {
        let dark = Self::dark(cx);
        let c = self.to_world(self.canvas_bounds.center());
        let n = self.doc.els.iter().filter(|e| (e.x - (c.x - w / 2.)).abs() < 200. && (e.y - (c.y - h / 2.)).abs() < 400.).count() as f32;
        self.snapshot();
        let mut el = self.doc.new_el(ElKind::Shape(kind), rect(c.x - w / 2., c.y - h / 2. + n * 24., w, h), dark);
        el.text = text.to_string();
        el.font_size = 14.;
        let prev = self.single().filter(|e| e.is_box()).map(|e| e.id);
        let id = el.id;
        if let Some(pe) = prev.and_then(|p| self.doc.get(p)) {
            el.x = pe.x + pe.w / 2. - w / 2.;
            el.y = pe.y + pe.h + 56.;
        }
        self.doc.els.push(el);
        if let Some(prev) = prev.filter(|_| self.doc.flow) {
            let c = self.doc.new_el(
                ElKind::Connector {
                    a: End::Bound { el: prev, ax: 0.5, ay: 0.5 },
                    b: End::Bound { el: id, ax: 0.5, ay: 0.5 },
                    route: Route::Elbow,
                    start: crate::canvas::Arrow::None,
                    end: crate::canvas::Arrow::Filled,
                },
                Rect::default(),
                dark,
            );
            self.doc.els.push(c);
        }
        self.sel = [id].into();
        self.commit(cx);
    }

    // ───────────── 排列 ─────────────

    /// 对齐：left / hcenter / right / top / vcenter / bottom，以选区包围盒为基准。
    pub(super) fn align(&mut self, how: &'static str, cx: &mut Context<Self>) {
        let Some(b) = self.doc.els.iter().filter(|e| self.sel.contains(&e.id) && !e.is_connector()).map(|e| e.rect()).reduce(|a, b| a.union(&b)) else {
            return;
        };
        self.mutate_sel(cx, move |e| {
            if e.is_connector() || e.locked || matches!(e.kind, ElKind::Pen { .. }) {
                return;
            }
            let how = match how {
                "left" => math::Align::Left,
                "hcenter" => math::Align::HCenter,
                "right" => math::Align::Right,
                "top" => math::Align::Top,
                "vcenter" => math::Align::VCenter,
                _ => math::Align::Bottom,
            };
            let (dx, dy) = math::align_delta(how, (b.x, b.y, b.right(), b.bottom()), (e.x, e.y, e.x + e.w, e.y + e.h));
            e.x += dx;
            e.y += dy;
        });
    }

    pub(super) fn distribute(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        let mut items: Vec<(u64, f32, f32)> = self
            .doc
            .els
            .iter()
            .filter(|e| self.sel.contains(&e.id) && e.is_box())
            .map(|e| (e.id, if horizontal { e.x } else { e.y }, if horizontal { e.w } else { e.h }))
            .collect();
        if items.len() < 3 {
            cx.notify();
            return;
        }
        items.sort_by(|a, b| a.1.total_cmp(&b.1));
        let spans: Vec<(f32, f32)> = items.iter().map(|i| (i.1, i.2)).collect();
        self.snapshot();
        for ((id, _, _), at) in items.into_iter().zip(math::distribute(&spans)) {
            if let Some(i) = self.doc.index(id) {
                if horizontal {
                    self.doc.els[i].x = at;
                } else {
                    self.doc.els[i].y = at;
                }
            }
        }
        self.commit(cx);
    }

    /// 批量排版：等宽 / 等高 / 相同大小，以选区中最大的图形为准。
    pub(super) fn same_size(&mut self, w: bool, h: bool, cx: &mut Context<Self>) {
        let boxes: Vec<&El> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id) && e.is_box() && !e.locked).collect();
        if boxes.len() < 2 {
            return;
        }
        let mw = boxes.iter().map(|e| e.w).fold(0f32, f32::max);
        let mh = boxes.iter().map(|e| e.h).fold(0f32, f32::max);
        self.mutate_sel(cx, move |e| {
            if !e.is_box() || e.locked {
                return;
            }
            let c = e.rect().center();
            if w {
                e.w = mw;
                e.x = c.x - mw / 2.;
            }
            if h {
                e.h = mh;
                e.y = c.y - mh / 2.;
            }
        });
    }

    /// 层级：2 置于顶层、1 上移一层、-1 下移一层、-2 置于底层。
    pub(super) fn z_order(&mut self, level: i32, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        let sel = self.sel.clone();
        match level {
            2 | -2 => {
                let (s, rest): (Vec<El>, Vec<El>) = self.doc.els.drain(..).partition(|e| sel.contains(&e.id));
                self.doc.els = if level > 0 { rest.into_iter().chain(s).collect() } else { s.into_iter().chain(rest).collect() };
            }
            _ => {
                let n = self.doc.els.len();
                let idx: Vec<usize> = if level > 0 { (0..n).rev().collect() } else { (0..n).collect() };
                for i in idx {
                    if !sel.contains(&self.doc.els[i].id) {
                        continue;
                    }
                    let j = if level > 0 { i + 1 } else { i.wrapping_sub(1) };
                    if j < n && !sel.contains(&self.doc.els[j].id) {
                        self.doc.els.swap(i, j);
                    }
                }
            }
        }
        self.close_overlays();
        self.commit(cx);
    }

    // ───────────── 样式 ─────────────

    pub(super) fn set_fill(&mut self, c: Option<Col>, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            if !e.is_connector() && !matches!(e.kind, ElKind::Pen { .. } | ElKind::Image { .. } | ElKind::Icon { .. }) {
                // 保留原来的透明度
                let a = e.fill.map(|f| f.alpha()).unwrap_or(1.);
                e.fill = c.map(|c| c.with_alpha(a));
            }
        });
    }

    pub(super) fn set_stroke(&mut self, c: Col, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            // 保留原来的透明度
            e.stroke = c.with_alpha(e.stroke.alpha());
            e.no_stroke = false;
        });
    }

    /// 透明度滑杆拖动中：只改数值不进撤销栈（松手时由调用方 snapshot 一次）。
    pub(super) fn live_alpha(&mut self, stroke: bool, a: f32, cx: &mut Context<Self>) {
        let cell = if stroke { None } else { self.cur_cell() };
        for e in self.doc.els.iter_mut().filter(|e| self.sel.contains(&e.id)) {
            if stroke {
                e.stroke = e.stroke.with_alpha(a);
            } else if cell.is_none() {
                if let Some(f) = e.fill {
                    e.fill = Some(f.with_alpha(a));
                }
            }
        }
        cx.notify();
    }

    /// 线型：0 无、1 实线、2 虚线、3 点线。
    pub(super) fn set_line_style(&mut self, style: u8, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            e.no_stroke = style == 0 && !e.is_connector();
            e.dash = style == 2;
            e.dotted = style == 3;
        });
    }

    pub(super) fn set_stroke_w(&mut self, w: f32, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| e.stroke_w = w);
    }

    /// 文字放不下时把元素撑高（各段文字都检查，只增不减）。
    pub(super) fn fit_all(e: &mut El, ts: &std::sync::Arc<TextSystem>) {
        for p in e.parts() {
            let floor = e.part_floor(p);
            e.fit_text(p, floor, &mut |t, fs, b, w| crate::canvas::paint::count_lines(ts, t, fs, b, w));
        }
    }

    /// 改文字样式：选中了表格单元格时只改该单元格（表格），否则改选中的元素；改完后文字放不下的撑高。
    fn set_text_style(&mut self, cx: &mut Context<Self>, cell: impl FnOnce(&El, usize, &mut CellFmt), whole: impl Fn(&mut El)) {
        let ts = cx.text_system().clone();
        if let Some((id, r, c)) = self.cur_cell() {
            let Some(k) = self.doc.index(id) else { return };
            let i = r * self.doc.els[k].grid().map(|g| g.ncols()).unwrap_or(1) + c;
            self.snapshot();
            let snap = self.doc.els[k].clone();
            self.doc.els[k].cell_fmt_mut(i, |f| cell(&snap, i, f));
            Self::fit_all(&mut self.doc.els[k], &ts);
            self.commit(cx);
            return;
        }
        self.mutate_sel(cx, move |e| {
            whole(e);
            Self::fit_all(e, &ts);
        });
    }

    pub(super) fn set_font(&mut self, fs: Option<f32>, cx: &mut Context<Self>) {
        self.set_text_style(
            cx,
            |_, _, f| f.font_size = fs,
            move |e| match fs {
                Some(fs) => {
                    e.font_size = fs;
                    e.auto_font = false;
                    e.clear_cell_fmts(|f| f.font_size = None);
                }
                None => e.auto_font = true,
            },
        );
    }

    pub(super) fn set_text_color(&mut self, c: Col, cx: &mut Context<Self>) {
        if self.fmt_selection(|_| false, move |f, _| f.color = Some(c), cx) {
            return;
        }
        self.set_text_style(
            cx,
            |_, _, f| f.color = Some(c),
            move |e| {
                e.text_color = Some(c);
                e.spans_follow(|f| f.color = None);
                e.clear_cell_fmts(|f| f.color = None);
            },
        );
    }

    pub(super) fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        if self.fmt_selection(|f| f.bold, |f, on| f.bold = on, cx) {
            return;
        }
        let b = !self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).all(|e| e.bold);
        self.set_text_style(
            cx,
            |e, i, f| f.bold = Some(!e.cell_bold(i)),
            move |e| {
                e.bold = b;
                e.spans_follow(|f| f.bold = b);
                e.clear_cell_fmts(|f| f.bold = None);
            },
        );
    }

    /// 文字对齐：1 左、2 中、3 右。
    pub(super) fn set_text_align(&mut self, a: u8, cx: &mut Context<Self>) {
        self.set_text_style(
            cx,
            |_, _, f| f.align = Some(a),
            move |e| {
                e.text_align = a;
                e.clear_cell_fmts(|f| f.align = None);
            },
        );
    }

    /// 文字垂直对齐：1 顶、2 中、3 底。
    pub(super) fn set_v_align(&mut self, v: u8, cx: &mut Context<Self>) {
        self.set_text_style(
            cx,
            |_, _, f| f.v_align = Some(v),
            move |e| {
                e.v_align = v;
                e.clear_cell_fmts(|f| f.v_align = None);
            },
        );
    }

    /// 切换斜体 / 下划线 / 删除线（选区全部带该修饰时取消，否则加上）。
    pub(super) fn toggle_deco(&mut self, bit: u8, cx: &mut Context<Self>) {
        let set = move |f: &mut crate::canvas::Fmt, on: bool| if on { f.deco |= bit } else { f.deco &= !bit };
        if self.fmt_selection(move |f| f.deco & bit != 0, set, cx) {
            return;
        }
        let on = !self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).all(|e| e.deco & bit != 0);
        self.set_text_style(
            cx,
            move |e, i, f| {
                let mut d = e.cell_deco(i);
                let on = d & bit == 0;
                set_deco(&mut d, bit, on);
                f.deco = Some(d);
            },
            move |e| {
                set_deco(&mut e.deco, bit, on);
                e.spans_follow(|f| set(f, on));
                e.clear_cell_fmts(|f| f.deco = None);
            },
        );
    }

    /// 水平 / 垂直翻转（⇧H / ⇧V）：选区内元素位置按选区中线镜像；
    /// 有镜像版本的图形（箭头、括号、平行四边形、梯形、三角形）换成镜像图形，画笔笔迹逐点镜像。
    pub(super) fn flip(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        let Some(b) = self.selection_bounds() else { return };
        let (cx0, cy0) = (b.x + b.w / 2., b.y + b.h / 2.);
        let mirror = |k: ShapeKind| -> ShapeKind {
            use ShapeKind::*;
            match (horizontal, k) {
                (true, BlockArrow) => LeftArrow,
                (true, LeftArrow) => BlockArrow,
                (true, BraceLeft) => BraceRight,
                (true, BraceRight) => BraceLeft,
                (true, Parallelogram) => ParallelogramAlt,
                (true, ParallelogramAlt) => Parallelogram,
                (false, Trapezoid) => TrapezoidAlt,
                (false, TrapezoidAlt) => Trapezoid,
                (false, Triangle) => Merge,
                (false, Merge) => Triangle,
                _ => k,
            }
        };
        self.mutate_sel(cx, move |e| {
            if e.locked {
                return;
            }
            if horizontal {
                e.x = 2. * cx0 - e.x - e.w;
            } else {
                e.y = 2. * cy0 - e.y - e.h;
            }
            match &mut e.kind {
                ElKind::Shape(k) => *k = mirror(*k),
                ElKind::Pen { pts } => {
                    for p in pts.iter_mut() {
                        if horizontal {
                            p[0] = 2. * cx0 - p[0];
                        } else {
                            p[1] = 2. * cy0 - p[1];
                        }
                    }
                }
                ElKind::Connector { a, b, .. } => {
                    for end in [a, b] {
                        match end {
                            End::Free { x, y } => {
                                if horizontal {
                                    *x = 2. * cx0 - *x;
                                } else {
                                    *y = 2. * cy0 - *y;
                                }
                            }
                            End::Bound { ax, ay, .. } => {
                                if horizontal {
                                    *ax = 1. - *ax;
                                } else {
                                    *ay = 1. - *ay;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        });
    }

    // ───────────── 格式刷 ─────────────

    /// 复制样式（⌥⌘C）。
    pub(super) fn copy_style(&mut self, cx: &mut Context<Self>) {
        if let Some(e) = self.doc.els.iter().find(|e| self.sel.contains(&e.id)).cloned() {
            self.style_clip = Some(e);
            self.close_overlays();
            cx.notify();
        }
    }

    /// 粘贴样式（⌥⌘V）到选中元素。
    pub(super) fn paste_style(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.style_clip.clone() else { return };
        self.close_overlays();
        self.mutate_sel(cx, move |e| apply_style(e, &src));
    }

    /// 格式刷：复制当前样式，下一次单击的元素套用它（keep 为 true 时可连续刷，Esc 退出）。
    pub(super) fn start_painter(&mut self, keep: bool, cx: &mut Context<Self>) {
        if let Some(e) = self.doc.els.iter().find(|e| self.sel.contains(&e.id)).cloned() {
            self.style_clip = Some(e);
            self.painting = Some(keep);
            self.popover = None;
            cx.notify();
        }
    }

    /// 刷到一个元素上（连同同组元素）。
    pub(super) fn paint_style_on(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(src) = self.style_clip.clone() else { return };
        self.sel = [id].into();
        self.expand_groups();
        self.mutate_sel(cx, move |e| apply_style(e, &src));
        if self.painting == Some(false) {
            self.painting = None;
        }
    }

    pub(super) fn set_shape_kind(&mut self, k: ShapeKind, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            if matches!(e.kind, ElKind::Shape(_) | ElKind::Sticky) {
                if matches!(e.kind, ElKind::Sticky) {
                    e.mind = None;
                    // 便签换成图形：沿用便签底色，改用图形的描边
                    e.stroke = Col::rgb(0x3370ff);
                }
                e.kind = ElKind::Shape(k);
            }
        });
    }

    pub(super) fn set_route(&mut self, r: Route, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            if let ElKind::Connector { route, .. } = &mut e.kind {
                *route = r;
            }
        });
    }

    pub(super) fn set_arrow(&mut self, end: bool, a: crate::canvas::Arrow, cx: &mut Context<Self>) {
        self.mutate_sel(cx, move |e| {
            if let ElKind::Connector { start, end: en, .. } = &mut e.kind {
                if end {
                    *en = a;
                } else {
                    *start = a;
                }
            }
        });
    }

    /// 交换连线方向（起点和终点互换）。
    pub(super) fn swap_arrows(&mut self, cx: &mut Context<Self>) {
        self.mutate_sel(cx, |e| {
            if let ElKind::Connector { start, end, .. } = &mut e.kind {
                std::mem::swap(start, end);
            }
        });
    }

    pub(super) fn set_sticky_color(&mut self, i: usize, cx: &mut Context<Self>) {
        self.sticky_color = i;
        let c = Col::rgb(STICKY_COLORS[i.min(STICKY_COLORS.len() - 1)]);
        self.mutate_sel(cx, move |e| {
            if matches!(e.kind, ElKind::Sticky) {
                e.fill = Some(c);
            }
        });
    }

    pub(super) fn toggle_author(&mut self, cx: &mut Context<Self>) {
        let v = !self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).all(|e| e.show_author);
        self.mutate_sel(cx, move |e| {
            e.show_author = v;
            if v && e.author.is_empty() {
                e.author = crate::settings::author();
            }
        });
    }

    // ───────────── 导出 ─────────────

    /// 复制为图片（⇧⌘C）：只渲染选中的元素，PNG 写进剪贴板。
    pub(super) fn copy_as_image(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        let mut doc = self.doc.clone();
        let keep = self.sel.clone();
        doc.els.retain(|e| keep.contains(&e.id));
        let mut scene = doc.to_scene(false);
        scene.fit(16.);
        scene.bg = Some(Col::rgb(0xffffff));
        let svg = crate::canvas::svg::to_svg(&scene);
        if let Some(png) = crate::canvas::svg::svg_to_png(&svg, 2.) {
            cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(gpui::ImageFormat::Png, png)));
        }
        self.close_overlays();
        cx.notify();
    }

    pub(super) fn export(&mut self, png: bool, cx: &mut Context<Self>) {
        self.close_overlays();
        let mut scene = self.doc.to_scene(false);
        scene.fit(24.);
        scene.bg = Some(Col::rgb(0xffffff));
        let svg = crate::canvas::svg::to_svg(&scene);
        let name = format!("{}.{}", if self.doc.flow { "流程图" } else { "画板" }, if png { "png" } else { "svg" });
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |_: WeakEntity<BoardEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = if png {
                    crate::canvas::svg::svg_to_png(&svg, 2.).is_some_and(|b| std::fs::write(&path, b).is_ok())
                } else {
                    std::fs::write(&path, svg).is_ok()
                };
                if ok {
                    cx.update(|cx| cx.reveal_path(&path)).ok();
                }
            }
        })
        .detach();
        cx.notify();
    }
}

/// 格式刷：把 src 的样式套到 dst 上（只套两者都有意义的属性）。
pub(super) fn apply_style(dst: &mut El, src: &El) {
    let text_like = |e: &El| !matches!(e.kind, ElKind::Pen { .. } | ElKind::Image { .. } | ElKind::Icon { .. });
    if text_like(dst) && text_like(src) {
        dst.font_size = src.font_size;
        dst.bold = src.bold;
        dst.text_color = src.text_color;
        dst.text_align = src.text_align;
        dst.v_align = src.v_align;
        dst.deco = src.deco;
        dst.auto_font = src.auto_font && matches!(dst.kind, ElKind::Sticky);
    }
    let boxy = |e: &El| matches!(e.kind, ElKind::Shape(_) | ElKind::Sticky | ElKind::Text | ElKind::Section | ElKind::Table { .. });
    if boxy(dst) && boxy(src) {
        dst.fill = src.fill;
        dst.opacity = src.opacity;
    }
    let stroked = |e: &El| !matches!(e.kind, ElKind::Image { .. } | ElKind::Text);
    if stroked(dst) && stroked(src) {
        dst.stroke = src.stroke;
        dst.stroke_w = src.stroke_w;
        dst.dash = src.dash;
        dst.dotted = src.dotted;
        dst.no_stroke = src.no_stroke;
    }
    // 连线之间还复制线型与箭头
    if let (ElKind::Connector { route, start, end, .. }, ElKind::Connector { route: r2, start: s2, end: e2, .. }) = (&mut dst.kind, &src.kind) {
        *route = *r2;
        *start = *s2;
        *end = *e2;
    }
    // 便签之间复制颜色；同为图形时连形状一起（格式刷不改形状）
}

fn set_deco(deco: &mut u8, bit: u8, on: bool) {
    if on {
        *deco |= bit
    } else {
        *deco &= !bit
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{BoardDoc, ElKind};
    use crate::canvas::{rect, Arrow, Col, Rect, ShapeKind};

    #[test]
    fn format_painter_copies_compatible_style_only() {
        let mut b = BoardDoc::new(false);
        let mut src = b.new_el(ElKind::Shape(ShapeKind::Diamond), rect(0., 0., 10., 10.), false);
        src.fill = Some(Col::rgb(0xfde2e2));
        src.stroke = Col::rgb(0xf54a45);
        src.font_size = 24.;
        src.bold = true;
        src.dash = true;
        let mut dst = b.new_el(ElKind::Shape(ShapeKind::Rect), rect(0., 0., 10., 10.), false);
        super::apply_style(&mut dst, &src);
        assert_eq!((dst.fill, dst.stroke, dst.font_size, dst.bold, dst.dash), (src.fill, src.stroke, 24., true, true));
        // 形状本身不变
        assert_eq!(dst.kind, ElKind::Shape(ShapeKind::Rect));
        // 连线只取线条与文字样式，并复制箭头
        let mut line = b.new_el(
            ElKind::Connector {
                a: super::End::Free { x: 0., y: 0. },
                b: super::End::Free { x: 1., y: 1. },
                route: super::Route::Straight,
                start: Arrow::None,
                end: Arrow::Filled,
            },
            Rect::default(),
            false,
        );
        super::apply_style(&mut line, &src);
        assert!(line.fill.is_none() && line.dash && line.stroke == src.stroke);
    }
}
