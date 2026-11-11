//! 输入法（IME）接入。macOS 的 NSTextInputClient 以 UTF-16 偏移描述文本，
//! 这里把它们映射到「当前光标所在文本槽」的 UTF-8 偏移上。

use super::*;

fn to_utf16(s: &str, o: usize) -> usize {
    // o 可能落在多字节字符中间（偏移来自上一次的文本），先回退到 char 边界再切片
    let mut o = o.min(s.len());
    while o > 0 && !s.is_char_boundary(o) {
        o -= 1;
    }
    s[..o].encode_utf16().count()
}

fn from_utf16(s: &str, o: usize) -> usize {
    let mut u = 0;
    for (i, ch) in s.char_indices() {
        if u >= o {
            return i;
        }
        u += ch.len_utf16();
    }
    s.len()
}

impl Editor {
    fn head_text(&self) -> String {
        self.slot_text(self.core.head.slot()).map(|t| t.text.clone()).unwrap_or_default()
    }

    /// 用新文本替换当前槽内的区间，并按需标记为「输入法组字中」。
    fn ime_replace(&mut self, range: Range<usize>, text: &str, mark: bool, sel_in_new: Option<Range<usize>>, cx: &mut Context<Self>) {
        let h = self.core.head;
        // head 可能因为块被删除 / 文档切换而越界，越界时退回文末，别 panic
        if h.block >= self.content.blocks.len() {
            let last = self.content.blocks.len().saturating_sub(1);
            self.core.head = Pos::new(last, 0, self.slot_end(last).offset);
            self.core.anchor = self.core.head;
            if !text.is_empty() {
                self.insert_text(text, cx);
            }
            return;
        }
        if self.content.blocks[h.block].kind.is_void() {
            if !text.is_empty() {
                self.insert_text(text, cx);
            }
            return;
        }
        self.checkpoint(EditKind::Typing);
        let text = if h.block == 0 { text.replace('\n', " ") } else { text.to_string() };
        let marks = self.current_marks();
        let slot = self.slot_mut(h.slot());
        let range = range.start.min(slot.len())..range.end.min(slot.len());
        slot.delete(range.clone());
        slot.insert(range.start, &text, marks);
        self.core.marked = (mark && !text.is_empty()).then(|| range.start..range.start + text.len());
        let caret = match sel_in_new {
            Some(r) => range.start + from_utf16(&text, r.end),
            None => range.start + text.len(),
        };
        self.core.head = Pos { offset: caret, ..h };
        self.core.anchor = self.core.head;
        // 中文输入法上屏的 @ 不经过 after_char_typed
        if !mark && matches!(text.as_str(), "@" | "＠") {
            self.open_mention();
        }
        self.update_slash();
        self.changed(cx);
        self.reveal_head();
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let t = self.head_text();
        let range = from_utf16(&t, r.start)..from_utf16(&t, r.end);
        actual.replace(to_utf16(&t, range.start)..to_utf16(&t, range.end));
        Some(t[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let t = self.head_text();
        if self.core.anchor.slot() == self.core.head.slot() {
            let (s, e) = self.sel();
            Some(UTF16Selection { range: to_utf16(&t, s.offset)..to_utf16(&t, e.offset), reversed: self.core.head < self.core.anchor })
        } else {
            let o = to_utf16(&t, self.core.head.offset);
            Some(UTF16Selection { range: o..o, reversed: false })
        }
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let t = self.slot_text(self.core.head.slot())?;
        self.core.marked.as_ref().map(|r| to_utf16(&t.text, r.start)..to_utf16(&t.text, r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.core.marked = None;
    }

    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        if self.sess.read_only {
            cx.emit(EditorEvent::Toast(super::reading::READ_ONLY_HINT.into()));
            return;
        }
        let t = self.head_text();
        let range = r.map(|r| from_utf16(&t, r.start)..from_utf16(&t, r.end)).or(self.core.marked.clone());
        match range {
            // 普通按键输入：走统一的 insert_text（处理选区、Markdown 快捷语法、斜杠菜单）
            None => self.insert_text(text, cx),
            Some(range) => {
                self.ime_replace(range, text, false, None, cx);
                // 输入法上屏的单个字符也触发快捷语法（例如中文输入法下的 `、`）
                self.core.marked = None;
            }
        }
    }

    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, sel: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        if self.sess.read_only {
            return;
        }
        if self.has_selection() && self.core.anchor.slot() != self.core.head.slot() {
            self.checkpoint(EditKind::Typing);
            self.delete_selection_inner();
        }
        let t = self.head_text();
        let range = r.map(|r| from_utf16(&t, r.start)..from_utf16(&t, r.end)).or(self.core.marked.clone()).unwrap_or_else(|| {
            let (s, e) = self.sel();
            s.offset..e.offset
        });
        self.ime_replace(range, text, true, sel, cx);
    }

    fn bounds_for_range(&mut self, r: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let t = self.head_text();
        let start = from_utf16(&t, r.start);
        let lh = self.frame.borrow().layouts.get(&self.core.head.slot())?.line_height();
        let p = self.pos_for(self.core.head.slot(), start)?;
        Some(Bounds::new(p, size(px(2.), lh)))
    }

    fn character_index_for_point(&mut self, pt: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let p = self.hit_test(pt)?;
        if p.slot() != self.core.head.slot() {
            return None;
        }
        Some(to_utf16(&self.head_text(), p.offset))
    }
}
