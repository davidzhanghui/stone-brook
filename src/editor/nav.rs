//! 光标移动、命中测试、鼠标选择。

use super::*;
use crate::model::BlockKind;
use unicode_segmentation::UnicodeSegmentation;

impl Editor {
    pub fn slot_end(&self, block: usize) -> Pos {
        let cell = self.stops(block) - 1;
        Pos::new(block, cell, self.slot_len((block, cell)))
    }

    fn next_stop(&self, p: Pos) -> Option<Pos> {
        if p.cell + 1 < self.stops(p.block) {
            return Some(Pos::new(p.block, p.cell + 1, 0));
        }
        self.visible_from(p.block + 1, true).map(|b| Pos::new(b, 0, 0))
    }

    fn prev_stop_end(&self, p: Pos) -> Option<Pos> {
        if p.cell > 0 {
            let c = p.cell - 1;
            return Some(Pos::new(p.block, c, self.slot_len((p.block, c))));
        }
        if p.block == 0 {
            return None;
        }
        self.visible_from(p.block - 1, false).map(|b| self.slot_end(b))
    }

    fn right_pos(&self) -> Pos {
        let h = self.core.head;
        let len = self.slot_len(h.slot());
        if h.offset < len {
            let t = &self.slot_text(h.slot()).unwrap().text;
            let n = t[h.offset..].grapheme_indices(true).nth(1).map(|(i, _)| h.offset + i).unwrap_or(len);
            Pos { offset: n, ..h }
        } else {
            match self.next_stop(h) {
                Some(p) => self.skip_covered(p, true),
                None => h,
            }
        }
    }

    /// 跳过被合并单元格覆盖的槽（这些槽没有渲染，也没有内容）。
    fn skip_covered(&self, mut p: Pos, forward: bool) -> Pos {
        for _ in 0..4096 {
            let Some(blk) = self.content.blocks.get(p.block) else { return p };
            // 表格里被合并盖住的格子、时间轴里隐藏的字段都跳过
            let skip = match &blk.kind {
                crate::model::BlockKind::Table { cols, merges, .. } => {
                    let cols = (*cols).max(1);
                    let (r, c) = (p.cell / cols, p.cell % cols);
                    matches!(crate::model::merge_at(merges, r, c), Some(m) if (m.row, m.col) != (r, c))
                }
                _ => blk.slot_hidden(p.cell),
            };
            if !skip {
                return p;
            }
            let np = if forward { self.next_stop(p) } else { self.prev_stop_end(p) };
            match np {
                Some(np) if np != p => p = np,
                _ => return p,
            }
        }
        p
    }

    fn left_pos(&self) -> Pos {
        let h = self.core.head;
        if h.offset > 0 {
            let t = &self.slot_text(h.slot()).unwrap().text;
            let p = t[..h.offset].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0);
            Pos { offset: p, ..h }
        } else {
            match self.prev_stop_end(h) {
                Some(p) => self.skip_covered(p, false),
                None => h,
            }
        }
    }

    pub(super) fn word_left_pos(&self) -> Pos {
        let h = self.core.head;
        if h.offset == 0 {
            return self.left_pos();
        }
        let t = &self.slot_text(h.slot()).unwrap().text;
        let o =
            crate::words::segments(t).into_iter().rev().find_map(|(i, w)| (i < h.offset && w.chars().any(|c| c.is_alphanumeric())).then_some(i)).unwrap_or(0);
        Pos { offset: o, ..h }
    }

    fn word_right_pos(&self) -> Pos {
        let h = self.core.head;
        let len = self.slot_len(h.slot());
        if h.offset >= len {
            return self.right_pos();
        }
        let t = &self.slot_text(h.slot()).unwrap().text;
        let o = crate::words::segments(t)
            .into_iter()
            .find_map(|(i, w)| {
                let end = i + w.len();
                (end > h.offset && w.chars().any(|c| c.is_alphanumeric())).then_some(end)
            })
            .unwrap_or(len);
        Pos { offset: o, ..h }
    }

    /// 视觉行的行首 / 行尾（考虑自动换行）。
    pub(super) fn line_edge(&self, end: bool) -> Pos {
        let h = self.core.head;
        let frame = self.frame.borrow();
        let len = self.slot_len(h.slot());
        if let Some(layout) = frame.layouts.get(&h.slot()) {
            let map = frame.maps.get(&h.slot()).cloned();
            let d = map.as_ref().map_or(h.offset, |m| m.m2d(h.offset));
            if let Some(p) = layout.position_for_index(d) {
                let b = layout.bounds();
                let y = p.y + layout.line_height() / 2.;
                let x = if end { b.right() + px(10000.) } else { b.left() - px(1.) };
                let ix = match layout.index_for_position(point(x, y)) {
                    Ok(i) | Err(i) => i,
                };
                let ix = map.as_ref().map_or(ix, |m| m.d2m(ix));
                return self.clamp(Pos { offset: ix.min(len), ..h });
            }
        }
        // 回退：逻辑行
        let t = self.slot_text(h.slot()).map(|t| t.text.as_str()).unwrap_or("");
        let o = if end { t[h.offset..].find('\n').map(|i| h.offset + i).unwrap_or(len) } else { t[..h.offset].rfind('\n').map(|i| i + 1).unwrap_or(0) };
        Pos { offset: o, ..h }
    }

    /// 垂直方向目标槽位：表格内按列上下移动，其他情况按块。
    fn vertical_neighbor(&self, p: Pos, down: bool) -> Option<(usize, usize)> {
        let b = &self.content.blocks[p.block];
        if let BlockKind::Table { rows, cols, .. } = b.kind {
            let (r, c) = (p.cell / cols, p.cell % cols);
            if down && r + 1 < rows {
                return Some((p.block, (r + 1) * cols + c));
            }
            if !down && r > 0 {
                return Some((p.block, (r - 1) * cols + c));
            }
        }
        if down {
            self.visible_from(p.block + 1, true).map(|b| (b, 0))
        } else if p.block > 0 {
            let prev = self.visible_from(p.block - 1, false)?;
            // 从下方进入表格时落在最后一行的同一列
            let cell = match self.content.blocks[prev].kind {
                BlockKind::Table { rows, cols, .. } => (rows - 1) * cols,
                _ => 0,
            };
            Some((prev, cell))
        } else {
            None
        }
    }

    fn vertical_pos(&mut self, down: bool) -> Pos {
        let h = self.core.head;
        let frame = self.frame.borrow();
        let layout = frame.layouts.get(&h.slot()).cloned();
        let hmap = frame.maps.get(&h.slot()).cloned();
        let hd = hmap.as_ref().map_or(h.offset, |m| m.m2d(h.offset));
        let (x, y, lh) = match layout.as_ref().and_then(|l| l.position_for_index(hd).map(|p| (l, p))) {
            Some((l, p)) => (self.core.goal_x.unwrap_or(p.x), p.y, l.line_height()),
            None => {
                drop(frame);
                let target = self.vertical_neighbor(h, down);
                return match target {
                    Some((b, c)) => Pos::new(b, c, if down { 0 } else { self.slot_len((b, c)) }),
                    None => h,
                };
            }
        };
        let layout = layout.unwrap();
        let bounds = layout.bounds();
        let target_y = if down { y + lh * 1.5 } else { y - lh * 0.5 };
        let goal = Some(x);
        if target_y > bounds.top() && target_y < bounds.bottom() {
            let ix = match layout.index_for_position(point(x, target_y)) {
                Ok(i) | Err(i) => i,
            };
            let ix = hmap.as_ref().map_or(ix, |m| m.d2m(ix));
            drop(frame);
            self.core.goal_x = goal;
            return self.clamp(Pos { offset: ix, ..h });
        }
        let Some(slot) = self.vertical_neighbor(h, down) else {
            drop(frame);
            return Pos { offset: if down { self.slot_len(h.slot()) } else { 0 }, ..h };
        };
        let target = frame.layouts.get(&slot).cloned();
        let tmap = frame.maps.get(&slot).cloned();
        drop(frame);
        self.core.goal_x = goal;
        match target {
            Some(t) => {
                let tb = t.bounds();
                let ty = if down { tb.top() + t.line_height() / 2. } else { tb.bottom() - t.line_height() / 2. };
                let ix = match t.index_for_position(point(x, ty)) {
                    Ok(i) | Err(i) => i,
                };
                let ix = tmap.as_ref().map_or(ix, |m| m.d2m(ix));
                self.clamp(Pos::new(slot.0, slot.1, ix))
            }
            None => Pos::new(slot.0, slot.1, if down { 0 } else { self.slot_len(slot) }),
        }
    }

    fn mv(&mut self, p: Pos, extend: bool, cx: &mut Context<Self>) {
        if !extend {
            self.core.goal_x = None;
        }
        self.set_cursor(p, extend, cx);
    }

    pub fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let p = if self.has_selection() { self.sel().0 } else { self.left_pos() };
        self.mv(p, false, cx)
    }
    pub fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let p = if self.has_selection() { self.sel().1 } else { self.right_pos() };
        self.mv(p, false, cx)
    }
    pub fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.left_pos();
        self.mv(p, true, cx)
    }
    pub fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.right_pos();
        self.mv(p, true, cx)
    }
    pub fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.word_left_pos();
        self.mv(p, false, cx)
    }
    pub fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.word_right_pos();
        self.mv(p, false, cx)
    }
    pub fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.word_left_pos();
        self.mv(p, true, cx)
    }
    pub fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.word_right_pos();
        self.mv(p, true, cx)
    }
    pub fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.line_edge(false);
        self.mv(p, false, cx)
    }
    pub fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.line_edge(true);
        self.mv(p, false, cx)
    }
    pub fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.line_edge(false);
        self.mv(p, true, cx)
    }
    pub fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.line_edge(true);
        self.mv(p, true, cx)
    }
    pub fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.mv(Pos::default(), false, cx)
    }
    pub fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.slot_end(self.content.blocks.len() - 1);
        self.mv(p, false, cx)
    }
    pub fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.mv(Pos::default(), true, cx)
    }
    pub fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.slot_end(self.content.blocks.len() - 1);
        self.mv(p, true, cx)
    }

    pub fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.pop.mention.is_some() {
            return self.mention_move(-1, cx);
        }
        if self.pop.slash.is_some() {
            let n = super::ops::filter_slash(&self.slash_query()).len().max(1);
            if let Some(sm) = self.pop.slash.as_mut() {
                sm.selected = (sm.selected + n - 1) % n;
            }
            cx.notify();
            return;
        }
        if self.has_selection() {
            let p = self.sel().0;
            self.core.anchor = p;
            self.core.head = p;
        }
        let p = self.vertical_pos(false);
        let g = self.core.goal_x;
        self.set_cursor(p, false, cx);
        self.core.goal_x = g;
    }

    pub fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.pop.mention.is_some() {
            return self.mention_move(1, cx);
        }
        if self.pop.slash.is_some() {
            let n = super::ops::filter_slash(&self.slash_query()).len().max(1);
            if let Some(sm) = self.pop.slash.as_mut() {
                sm.selected = (sm.selected + 1) % n;
            }
            cx.notify();
            return;
        }
        if self.has_selection() {
            let p = self.sel().1;
            self.core.anchor = p;
            self.core.head = p;
        }
        // 在最后一个非正文块的末尾按 ↓，自动补一个空段落，方便「跳出」代码块 / 表格
        let last = self.content.blocks.len() - 1;
        let h = self.core.head;
        if h.block == last && self.content.blocks[last].kind != BlockKind::Text && h.block > 0 {
            let at_end = self.vertical_pos(true) == h || self.content.blocks[last].kind.is_void();
            if at_end {
                self.checkpoint(EditKind::Other);
                self.insert_blocks(last + 1, vec![crate::model::Block::text("")]);
                self.changed(cx);
                self.set_cursor(Pos::new(last + 1, 0, 0), false, cx);
                return;
            }
        }
        let p = self.vertical_pos(true);
        let g = self.core.goal_x;
        self.set_cursor(p, false, cx);
        self.core.goal_x = g;
    }

    pub fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.vertical_pos(false);
        let g = self.core.goal_x;
        self.set_cursor(p, true, cx);
        self.core.goal_x = g;
    }

    pub fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.vertical_pos(true);
        let g = self.core.goal_x;
        self.set_cursor(p, true, cx);
        self.core.goal_x = g;
    }

    // ───────────── 命中测试 ─────────────

    /// 窗口坐标 → 文档位置。选取垂直方向最近、其次水平方向最近的文本槽。
    pub fn hit_test(&self, pt: Point<Pixels>) -> Option<Pos> {
        let frame = self.frame.borrow();
        let dist = |b: &Bounds<Pixels>| {
            let dy = if pt.y < b.top() {
                b.top() - pt.y
            } else if pt.y > b.bottom() {
                pt.y - b.bottom()
            } else {
                px(0.)
            };
            let dx = if pt.x < b.left() {
                b.left() - pt.x
            } else if pt.x > b.right() {
                pt.x - b.right()
            } else {
                px(0.)
            };
            (dy, dx)
        };
        let mut best: Option<((Pixels, Pixels), Pos)> = None;
        for (&(b, c), layout) in frame.layouts.iter() {
            let bounds = layout.bounds();
            let d = dist(&bounds);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                let clamped = point(pt.x.clamp(bounds.left(), bounds.right()), pt.y.clamp(bounds.top() + px(1.), bounds.bottom() - px(1.)));
                let ix = match layout.index_for_position(clamped) {
                    Ok(i) | Err(i) => i,
                };
                let ix = frame.maps.get(&(b, c)).map_or(ix, |m| m.d2m(ix));
                best = Some((d, Pos::new(b, c, ix)));
            }
        }
        for (&b, bounds) in frame.voids.iter() {
            let d = dist(bounds);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, Pos::new(b, 0, 0)));
            }
        }
        best.map(|(_, p)| self.clamp(p))
    }

    pub fn slot_mouse_down(&mut self, b: usize, c: usize, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let layout = self.frame.borrow().layouts.get(&(b, c)).cloned();
        let Some(layout) = layout else { return };
        let raw = match layout.index_for_position(ev.position) {
            Ok(i) | Err(i) => i,
        };
        let map = self.frame.borrow().maps.get(&(b, c)).cloned();
        // 点击行内公式：打开公式编辑浮层
        if let Some(m) = &map {
            if let Ok(i) = layout.index_for_position(ev.position) {
                if let Some(start) = m.formula_at(i) {
                    let tex = self.slot_text((b, c)).and_then(|t| t.math_spans().into_iter().find(|f| f.0 == start)).map(|f| f.1).unwrap_or_default();
                    self.core.head = Pos::new(b, c, start);
                    self.core.anchor = self.core.head;
                    self.sess.inline_math_at = Some((b, c, start));
                    self.open_prop_edit(b, "inline_math", &tex, "编辑 LaTeX，回车确认，留空删除", ev.position, window, cx);
                    return;
                }
            }
        }
        let ix = map.as_ref().map_or(raw, |m| m.d2m(raw));
        let p = self.clamp(Pos::new(b, c, ix));
        // 音频时间戳：单击即跳转播放
        if ev.click_count == 1 && !ev.modifiers.shift {
            let url = self.slot_text((b, c)).and_then(|t| t.runs().find(|(r, _)| r.start <= p.offset && p.offset < r.end).and_then(|(_, m)| m.link.clone()));
            if let Some((secs, name)) = url.as_deref().and_then(super::links::seek_link_target) {
                let name = name.to_string();
                self.seek_to_stamp(secs, &name, cx);
                return;
            }
        }
        // ⌘ + 点击打开链接
        if ev.modifiers.platform {
            let url = self.slot_text((b, c)).and_then(|t| t.runs().find(|(r, _)| r.start <= p.offset && p.offset < r.end).and_then(|(_, m)| m.link.clone()));
            if let Some(url) = url {
                self.open_link(&url, cx);
                return;
            }
        }
        self.mouse_select(p, ev.click_count, ev.modifiers.shift, cx);
    }

    pub fn mouse_select(&mut self, p: Pos, clicks: usize, shift: bool, cx: &mut Context<Self>) {
        self.core.selecting = true;
        self.core.goal_x = None;
        match clicks {
            2 => {
                let t = self.slot_text(p.slot()).map(|t| t.text.clone()).unwrap_or_default();
                let (mut s, mut e) = (p.offset, p.offset);
                for (i, w) in crate::words::segments(&t) {
                    if i <= p.offset && p.offset <= i + w.len() && !w.trim().is_empty() {
                        s = i;
                        e = i + w.len();
                        if p.offset < i + w.len() {
                            break;
                        }
                    }
                }
                self.core.anchor = Pos { offset: s, ..p };
                self.set_cursor(Pos { offset: e, ..p }, true, cx);
            }
            n if n >= 3 => {
                self.core.anchor = Pos { offset: 0, ..p };
                let len = self.slot_len(p.slot());
                self.set_cursor(Pos { offset: len, ..p }, true, cx);
            }
            _ => self.set_cursor(p, shift, cx),
        }
    }

    pub fn root_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.sess.diagram_drag.is_some() && ev.pressed_button == Some(MouseButton::Left) {
            self.diagram_drag_move(ev.position, cx);
            return;
        }
        // 表格列宽
        if self.sess.table_col_drag.is_some() {
            if ev.pressed_button == Some(MouseButton::Left) {
                self.drag_table_col(ev.position.x, cx);
            } else {
                // 在窗口外松开了鼠标：当作结束
                self.end_table_col_drag(cx);
            }
            return;
        }
        // 图片等比缩放
        if let Some(d) = self.sess.image_drag.as_mut() {
            if ev.pressed_button == Some(MouseButton::Left) {
                let dx = f32::from(ev.position.x - d.start_px.x);
                d.width = (d.start_w + dx * d.dir).clamp(48., 2400.);
                cx.notify();
            } else {
                self.end_image_drag(cx);
            }
            return;
        }
        // 裁剪框
        if self.sess.image_crop.as_ref().is_some_and(|c| c.drag.is_some()) {
            if ev.pressed_button == Some(MouseButton::Left) {
                self.crop_drag_move(ev.position, cx);
            } else if let Some(c) = self.sess.image_crop.as_mut() {
                c.drag = None;
            }
            return;
        }
        if !self.core.selecting || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(p) = self.hit_test(ev.position) {
            if p != self.core.head {
                self.core.head = p;
                self.touch_activity();
                cx.notify();
            }
        }
        // 拖动到边缘时自动滚动
        let vp = self.list.viewport_bounds();
        if ev.position.y < vp.top() + px(24.) {
            self.list.scroll_by(px(-24.));
            cx.notify();
        } else if ev.position.y > vp.bottom() - px(24.) {
            self.list.scroll_by(px(24.));
            cx.notify();
        }
    }

    pub fn root_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.diagram_drag_end(cx);
        self.end_table_col_drag(cx);
        self.end_image_drag(cx);
        if let Some(c) = self.sess.image_crop.as_mut() {
            c.drag = None;
        }
        if self.core.selecting {
            self.core.selecting = false;
            cx.notify();
        }
    }

    /// 松开缩放手柄：只点了一下没拖时，不把「自动宽度」钉成固定值，也不记撤销。
    fn end_image_drag(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.sess.image_drag.take() else { return };
        let w = d.width.round();
        if (w - d.start_w.round()).abs() >= 1. {
            self.set_image_width(d.block, Some(w), cx);
        } else {
            cx.notify();
        }
    }
}
