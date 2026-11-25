//! 多行文本输入（画板图形 / 便签 / 文本框的原位编辑），支持输入法、自动换行与对齐。
//!
//! 与 [`crate::line_input::LineInput`] 同一套思路（GPUI 官方 input 示例），区别是：
//! 文本按段落（`\n`）分行、每段按宽度自动折行；光标上下移动按视觉行走；
//! 可水平居中、垂直居中，编辑时的排版与画布上的显示一致（所见即所得）。

use crate::line_input::clamp_str_range;
use crate::theme::theme;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_area,
    [
        TBackspace,
        TDelete,
        TWordBack,
        TDeleteLineBack,
        TLeft,
        TRight,
        TUp,
        TDown,
        TWordLeft,
        TWordRight,
        TSelectLeft,
        TSelectRight,
        TSelectUp,
        TSelectDown,
        TSelectWordLeft,
        TSelectWordRight,
        THome,
        TEnd,
        TSelectHome,
        TSelectEnd,
        TDocStart,
        TDocEnd,
        TSelectDocStart,
        TSelectDocEnd,
        TSelectAll,
        TNewline,
        TCommit,
        TTab,
        TCopy,
        TCut,
        TPaste,
        TUndo,
        TRedo
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("TextArea");
    cx.bind_keys([
        KeyBinding::new("backspace", TBackspace, c),
        KeyBinding::new("shift-backspace", TBackspace, c),
        KeyBinding::new("ctrl-h", TBackspace, c),
        KeyBinding::new("delete", TDelete, c),
        KeyBinding::new("ctrl-d", TDelete, c),
        KeyBinding::new("alt-backspace", TWordBack, c),
        KeyBinding::new("cmd-backspace", TDeleteLineBack, c),
        KeyBinding::new("left", TLeft, c),
        KeyBinding::new("right", TRight, c),
        KeyBinding::new("up", TUp, c),
        KeyBinding::new("down", TDown, c),
        KeyBinding::new("ctrl-b", TLeft, c),
        KeyBinding::new("ctrl-f", TRight, c),
        KeyBinding::new("ctrl-p", TUp, c),
        KeyBinding::new("ctrl-n", TDown, c),
        KeyBinding::new("alt-left", TWordLeft, c),
        KeyBinding::new("alt-right", TWordRight, c),
        KeyBinding::new("shift-left", TSelectLeft, c),
        KeyBinding::new("shift-right", TSelectRight, c),
        KeyBinding::new("shift-up", TSelectUp, c),
        KeyBinding::new("shift-down", TSelectDown, c),
        KeyBinding::new("alt-shift-left", TSelectWordLeft, c),
        KeyBinding::new("alt-shift-right", TSelectWordRight, c),
        KeyBinding::new("cmd-left", THome, c),
        KeyBinding::new("cmd-right", TEnd, c),
        KeyBinding::new("ctrl-a", THome, c),
        KeyBinding::new("ctrl-e", TEnd, c),
        KeyBinding::new("home", THome, c),
        KeyBinding::new("end", TEnd, c),
        KeyBinding::new("cmd-shift-left", TSelectHome, c),
        KeyBinding::new("cmd-shift-right", TSelectEnd, c),
        KeyBinding::new("ctrl-shift-a", TSelectHome, c),
        KeyBinding::new("ctrl-shift-e", TSelectEnd, c),
        KeyBinding::new("shift-home", TSelectHome, c),
        KeyBinding::new("shift-end", TSelectEnd, c),
        KeyBinding::new("cmd-up", TDocStart, c),
        KeyBinding::new("cmd-down", TDocEnd, c),
        KeyBinding::new("cmd-shift-up", TSelectDocStart, c),
        KeyBinding::new("cmd-shift-down", TSelectDocEnd, c),
        KeyBinding::new("cmd-a", TSelectAll, c),
        // 画板：回车 / ⇧回车都是换行，Esc 或 ⌘回车结束编辑
        KeyBinding::new("enter", TNewline, c),
        KeyBinding::new("shift-enter", TNewline, c),
        KeyBinding::new("cmd-enter", TCommit, c),
        KeyBinding::new("escape", TCommit, c),
        KeyBinding::new("tab", TTab, c),
        KeyBinding::new("cmd-c", TCopy, c),
        KeyBinding::new("cmd-x", TCut, c),
        KeyBinding::new("cmd-v", TPaste, c),
        KeyBinding::new("cmd-z", TUndo, c),
        KeyBinding::new("cmd-shift-z", TRedo, c),
        KeyBinding::new("cmd-y", TRedo, c),
    ]);
}

pub enum TextAreaEvent {
    Changed,
    /// Esc / ⌘↵：结束编辑
    Commit,
    /// Tab（便签据此在右侧新建一张）
    Tab,
}

impl EventEmitter<TextAreaEvent> for TextArea {}

/// 排版参数：由宿主每帧设置（随缩放变化）。
#[derive(Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    pub line_height: f32,
    pub color: Hsla,
    pub bold: bool,
    /// 水平对齐：0 左、1 居中、2 右
    pub align: u8,
    /// 垂直对齐：0 顶、1 中、2 底
    pub v_align: u8,
    /// 斜体 / 下划线 / 删除线（`canvas::DECO_*` 位）
    pub deco: u8,
}

/// 一帧的排版结果：每段一个 WrappedLine 及其起点。
#[derive(Clone, Default)]
struct Layout {
    paras: Vec<(usize, WrappedLine, Point<Pixels>)>,
    line_h: Pixels,
    width: Pixels,
    align: u8,
}

impl Layout {
    /// 段内各视觉行的 (起点字节, 终点字节)。
    fn rows(line: &WrappedLine) -> Vec<(usize, usize)> {
        let mut starts = vec![0];
        for b in line.wrap_boundaries() {
            if let Some(g) = line.runs().get(b.run_ix).and_then(|r| r.glyphs.get(b.glyph_ix)) {
                starts.push(g.index);
            }
        }
        let mut out = Vec::with_capacity(starts.len());
        for (i, s) in starts.iter().enumerate() {
            out.push((*s, starts.get(i + 1).copied().unwrap_or(line.len())));
        }
        out
    }

    /// 某视觉行的水平偏移（居中 / 右对齐时）。
    fn row_offset(&self, line: &WrappedLine, row: usize) -> Pixels {
        if self.align == 0 {
            return px(0.);
        }
        let rows = Self::rows(line);
        let Some(&(_, end)) = rows.get(row) else { return px(0.) };
        let w = line.position_for_index(end, self.line_h).map(|p| p.x).unwrap_or(px(0.));
        let free = (self.width - w).max(px(0.));
        if self.align == 1 {
            free / 2.
        } else {
            free
        }
    }

    /// 全文字节偏移 → 光标左上角（元素坐标）。
    fn point_for(&self, ix: usize) -> Option<Point<Pixels>> {
        for (start, line, origin) in &self.paras {
            if ix >= *start && ix <= start + line.len() {
                let p = line.position_for_index(ix - start, self.line_h)?;
                let row = (p.y / self.line_h).round() as usize;
                return Some(point(origin.x + self.row_offset(line, row) + p.x, origin.y + p.y));
            }
        }
        None
    }

    /// 元素坐标 → 最近的全文字节偏移。
    fn index_for(&self, p: Point<Pixels>) -> usize {
        let Some(first) = self.paras.first() else { return 0 };
        // 找到 y 所在的段（在段之间 / 之外时取最近的段）
        let mut target = first;
        for para in &self.paras {
            if p.y >= para.2.y {
                target = para;
            }
        }
        let (start, line, origin) = target;
        let local_y = (p.y - origin.y).max(px(0.));
        let rows = Self::rows(line);
        let row = ((local_y / self.line_h) as usize).min(rows.len().saturating_sub(1));
        let x = p.x - origin.x - self.row_offset(line, row);
        let at = point(x.max(px(0.)), self.line_h * row as f32 + self.line_h / 2.);
        let local = match line.closest_index_for_position(at, self.line_h) {
            Ok(i) | Err(i) => i,
        };
        start + local.min(line.len())
    }
}

pub struct TextArea {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    style: TextStyle,
    layout: Layout,
    selecting: bool,
    /// 上下移动时保持的横坐标
    goal_x: Option<Pixels>,
    /// 样式分段（选中部分文字改样式，见 `canvas::rich`）；为空表示整段用 `style`
    spans: Vec<crate::canvas::Span>,
    /// 内容超出时可滚动（代码编辑框用）：滚动距离、最大滚动距离、上次已滚动到可见的选区
    scrollable: bool,
    scroll_y: Pixels,
    scroll_max: Pixels,
    revealed: Range<usize>,
    /// 代码编辑模式（代码绘图的编辑区）：等宽字体、左侧行号、当前行高亮，按该语言做语法着色
    code: Option<&'static str>,
    /// 字体（默认 PingFang SC；草图用手写体）
    font: Option<&'static str>,
    /// 本地撤销 / 重做栈（避免编辑代码时按 ⌘Z 冒泡到外层画板）
    undo: Vec<(String, Range<usize>, bool, Vec<crate::canvas::Span>)>,
    redo: Vec<(String, Range<usize>, bool, Vec<crate::canvas::Span>)>,
}

impl TextArea {
    /// 开启纵向滚动：滚轮滚动，光标移动 / 输入时自动滚到可见，右侧显示滚动条。
    pub fn set_scrollable(&mut self) {
        self.scrollable = true;
    }

    /// 开启代码编辑模式（lang 见 `highlight::highlight`）。
    pub fn set_font(&mut self, f: &'static str) {
        self.font = Some(f);
    }

    pub fn set_code(&mut self, lang: &'static str) {
        self.code = Some(lang);
    }

    pub fn new(text: &str, placeholder: &str, style: TextStyle, cx: &mut Context<Self>) -> Self {
        TextArea {
            focus: cx.focus_handle(),
            content: text.to_string(),
            placeholder: placeholder.to_string().into(),
            selected: 0..text.len(),
            reversed: false,
            marked: None,
            style,
            layout: Layout::default(),
            selecting: false,
            goal_x: None,
            spans: vec![],
            scrollable: false,
            scroll_y: px(0.),
            scroll_max: px(0.),
            revealed: 0..0,
            code: None,
            font: None,
            undo: vec![],
            redo: vec![],
        }
    }

    /// 整体样式对应的分段基准。
    fn base_fmt(&self) -> crate::canvas::Fmt {
        crate::canvas::Fmt { bold: self.style.bold, deco: self.style.deco, color: None }
    }

    pub fn spans(&self) -> &[crate::canvas::Span] {
        &self.spans
    }

    pub fn set_spans(&mut self, spans: Vec<crate::canvas::Span>) {
        self.spans = spans;
    }

    /// 当前选区（字节）。
    pub fn selection(&self) -> Range<usize> {
        clamp_str_range(&self.content, self.selected.clone())
    }

    /// 选区开头处文字的样式（工具栏据此显示加粗等状态）。
    pub fn fmt_at_selection(&self) -> crate::canvas::Fmt {
        let r = self.selection();
        crate::canvas::rich::fmt_at(&self.spans, self.content.len(), r.start + 1, self.base_fmt())
    }

    /// 选区内文字是否全部满足 `pred`。
    pub fn selection_all(&self, pred: impl Fn(&crate::canvas::Fmt) -> bool) -> bool {
        crate::canvas::rich::all(&self.spans, self.content.len(), self.base_fmt(), self.selection(), pred)
    }

    /// 给选区内的文字设置样式（选中几个字单独加粗 / 变色）。
    pub fn apply_fmt(&mut self, f: impl Fn(&mut crate::canvas::Fmt), cx: &mut Context<Self>) {
        let r = self.selection();
        if r.is_empty() {
            return;
        }
        let (len, base) = (self.content.len(), self.base_fmt());
        crate::canvas::rich::apply(&mut self.spans, len, base, r, f);
        crate::canvas::rich::normalize(&mut self.spans, len, base);
        self.changed(cx);
    }

    /// 记录一次撤销快照（最多保留 100 步）。
    fn push_undo(&mut self) {
        if self.undo.len() >= 100 {
            self.undo.remove(0);
        }
        self.undo.push((self.content.clone(), self.selected.clone(), self.reversed, self.spans.clone()));
        self.redo.clear();
    }

    /// 替换一段文字，样式分段随之移动；新文字继承前一个字的样式。
    fn splice(&mut self, range: Range<usize>, new: &str) {
        if !range.is_empty() || !new.is_empty() {
            self.push_undo();
        }
        let old_len = self.content.len();
        if !self.spans.is_empty() {
            let fmt = crate::canvas::rich::fmt_at(&self.spans, old_len, range.start, self.base_fmt());
            crate::canvas::rich::splice(&mut self.spans, old_len, range.clone(), new.len(), fmt);
        }
        self.content.replace_range(range, new);
        if self.content.is_empty() {
            self.spans.clear();
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    pub fn move_to_end(&mut self) {
        self.selected = self.content.len()..self.content.len();
    }

    /// 全选（新建思维导图节点时默认文字被选中，直接输入即替换）。
    pub fn select_all_text(&mut self) {
        self.selected = 0..self.content.len();
        self.reversed = false;
    }

    /// 在光标处插入文字（代码编辑器里 Tab 插入空格）。
    pub fn insert(&mut self, s: &str, cx: &mut Context<Self>) {
        let r = clamp_str_range(&self.content, self.selected.clone());
        self.splice(r.clone(), s);
        self.selected = r.start + s.len()..r.start + s.len();
        self.marked = None;
        self.changed(cx);
    }

    /// 整体替换内容（光标放到开头）。
    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.content = s.to_string();
        self.spans.clear();
        self.selected = 0..0;
        self.marked = None;
        self.changed(cx);
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.goal_x = None;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    fn move_to(&mut self, o: usize, cx: &mut Context<Self>) {
        self.selected = o..o;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, o: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = o
        } else {
            self.selected.end = o
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn prev_boundary(&self, o: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < o).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, o: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > o).then_some(i)).unwrap_or(self.content.len())
    }

    /// 上下移动一视觉行后的位置。
    fn vertical(&mut self, dir: f32) -> usize {
        let cur = self.cursor();
        let Some(p) = self.layout.point_for(cur) else { return cur };
        let x = *self.goal_x.get_or_insert(p.x);
        let y = p.y + self.layout.line_h * dir + self.layout.line_h / 2.;
        if y < self.layout.paras.first().map_or(px(0.), |p| p.2.y) {
            return 0;
        }
        let last = self.layout.paras.last().map(|(_, l, o)| o.y + l.size(self.layout.line_h).height);
        if last.is_some_and(|b| y > b) {
            return self.content.len();
        }
        self.layout.index_for(point(x, y))
    }

    fn prev_word_boundary(&self, o: usize) -> usize {
        if o == 0 {
            return 0;
        }
        crate::words::segments(&self.content)
            .into_iter()
            .rev()
            .find_map(|(i, w)| (i < o && w.chars().any(|c| c.is_alphanumeric())).then_some(i))
            .unwrap_or(0)
    }

    fn next_word_boundary(&self, o: usize) -> usize {
        if o >= self.content.len() {
            return self.content.len();
        }
        crate::words::segments(&self.content)
            .into_iter()
            .find_map(|(i, w)| {
                let end = i + w.len();
                (end > o && w.chars().any(|c| c.is_alphanumeric())).then_some(end)
            })
            .unwrap_or(self.content.len())
    }

    fn para_bounds(&self, o: usize) -> (usize, usize) {
        let start = self.content[..o].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = self.content[o..].find('\n').map(|i| o + i).unwrap_or(self.content.len());
        (start, end)
    }

    fn left(&mut self, _: &TLeft, _: &mut Window, cx: &mut Context<Self>) {
        let o = if self.selected.is_empty() { self.prev_boundary(self.cursor()) } else { self.selected.start };
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn right(&mut self, _: &TRight, _: &mut Window, cx: &mut Context<Self>) {
        let o = if self.selected.is_empty() { self.next_boundary(self.cursor()) } else { self.selected.end };
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn up(&mut self, _: &TUp, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.vertical(-1.);
        self.selected = o..o;
        self.reversed = false;
        cx.notify();
    }
    fn down(&mut self, _: &TDown, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.vertical(1.);
        self.selected = o..o;
        self.reversed = false;
        cx.notify();
    }
    fn word_left(&mut self, _: &TWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.prev_word_boundary(self.cursor());
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn word_right(&mut self, _: &TWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.next_word_boundary(self.cursor());
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn select_left(&mut self, _: &TSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.prev_boundary(self.cursor()), cx)
    }
    fn select_right(&mut self, _: &TSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.next_boundary(self.cursor()), cx)
    }
    fn select_up(&mut self, _: &TSelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.vertical(-1.);
        self.select_to(o, cx)
    }
    fn select_down(&mut self, _: &TSelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.vertical(1.);
        self.select_to(o, cx)
    }
    fn select_word_left(&mut self, _: &TSelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.prev_word_boundary(self.cursor()), cx)
    }
    fn select_word_right(&mut self, _: &TSelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.next_word_boundary(self.cursor()), cx)
    }
    fn home(&mut self, _: &THome, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.para_bounds(self.cursor()).0;
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn end(&mut self, _: &TEnd, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.para_bounds(self.cursor()).1;
        self.goal_x = None;
        self.move_to(o, cx)
    }
    fn select_home(&mut self, _: &TSelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.para_bounds(self.cursor()).0;
        self.goal_x = None;
        self.select_to(o, cx)
    }
    fn select_end(&mut self, _: &TSelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let o = self.para_bounds(self.cursor()).1;
        self.goal_x = None;
        self.select_to(o, cx)
    }
    fn doc_start(&mut self, _: &TDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(0, cx)
    }
    fn doc_end(&mut self, _: &TDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(self.content.len(), cx)
    }
    fn select_doc_start(&mut self, _: &TSelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(0, cx)
    }
    fn select_doc_end(&mut self, _: &TSelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.content.len(), cx)
    }
    fn select_all(&mut self, _: &TSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        self.goal_x = None;
        cx.notify()
    }
    fn backspace(&mut self, _: &TBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.prev_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn delete(&mut self, _: &TDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn word_back(&mut self, _: &TWordBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let start = self.prev_word_boundary(self.cursor());
            self.select_to(start, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn delete_line_back(&mut self, _: &TDeleteLineBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let start = self.para_bounds(self.cursor()).0;
            self.select_to(start, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn undo(&mut self, _: &TUndo, _: &mut Window, cx: &mut Context<Self>) {
        let Some((content, selected, reversed, spans)) = self.undo.pop() else { return };
        self.redo.push((self.content.clone(), self.selected.clone(), self.reversed, self.spans.clone()));
        self.content = content;
        self.selected = selected;
        self.reversed = reversed;
        self.spans = spans;
        self.marked = None;
        self.changed(cx);
    }
    fn redo(&mut self, _: &TRedo, _: &mut Window, cx: &mut Context<Self>) {
        let Some((content, selected, reversed, spans)) = self.redo.pop() else { return };
        self.undo.push((self.content.clone(), self.selected.clone(), self.reversed, self.spans.clone()));
        self.content = content;
        self.selected = selected;
        self.reversed = reversed;
        self.spans = spans;
        self.marked = None;
        self.changed(cx);
    }
    fn newline(&mut self, _: &TNewline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx)
    }
    fn commit(&mut self, _: &TCommit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextAreaEvent::Commit)
    }
    fn tab(&mut self, _: &TTab, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextAreaEvent::Tab)
    }
    fn copy(&mut self, _: &TCopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            let r = clamp_str_range(&self.content, self.selected.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[r].to_string()));
        }
    }
    fn cut(&mut self, _: &TCut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            let r = clamp_str_range(&self.content, self.selected.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[r].to_string()));
            self.replace_text_in_range(None, "", window, cx)
        }
    }
    fn paste(&mut self, _: &TPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace_text_in_range(None, &t.replace("\r\n", "\n"), window, cx);
        }
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 不让外层画布在冒泡阶段把焦点抢回去（见 line_input 的同类问题）
        window.prevent_default();
        cx.stop_propagation();
        window.focus(&self.focus);
        let ix = self.layout.index_for(ev.position);
        if ev.click_count >= 4 {
            self.selected = 0..self.content.len();
            self.reversed = false;
        } else if ev.click_count == 3 {
            let (s, e) = self.para_bounds(ix);
            self.selected = s..e;
            self.reversed = false;
        } else if ev.click_count == 2 {
            let (s, e) = self.word_at(ix);
            self.selected = s..e;
            self.reversed = false;
        } else if ev.modifiers.shift {
            self.select_to(ix, cx);
        } else {
            self.move_to(ix, cx);
            self.selecting = true;
        }
        self.goal_x = None;
        cx.notify();
    }

    fn word_at(&self, ix: usize) -> (usize, usize) {
        for (i, w) in crate::words::segments(&self.content) {
            if ix >= i && ix <= i + w.len() && w.chars().any(|c| !c.is_whitespace()) {
                return (i, i + w.len());
            }
        }
        (ix, ix)
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && ev.pressed_button == Some(MouseButton::Left) {
            let ix = self.layout.index_for(ev.position);
            self.select_to(ix, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn to_utf16(&self, o: usize) -> usize {
        self.content[..o.min(self.content.len())].encode_utf16().count()
    }
    fn from_utf16(&self, o: usize) -> usize {
        let mut u = 0;
        for (i, ch) in self.content.char_indices() {
            if u >= o {
                return i;
            }
            u += ch.len_utf16();
        }
        self.content.len()
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.from_utf16(r.start)..self.from_utf16(r.end)
    }
}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for TextArea {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = clamp_str_range(&self.content, self.range_from_utf16(&r));
        actual.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, new: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = clamp_str_range(&self.content, r.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone()));
        self.splice(range.clone(), new);
        self.selected = range.start + new.len()..range.start + new.len();
        self.reversed = false;
        self.marked = None;
        self.changed(cx);
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, new: &str, sel: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = clamp_str_range(&self.content, r.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone()));
        self.splice(range.clone(), new);
        self.marked = (!new.is_empty()).then(|| range.start..range.start + new.len());
        let to8 = |u: usize| {
            let mut c = 0;
            for (i, ch) in new.char_indices() {
                if c >= u {
                    return i;
                }
                c += ch.len_utf16();
            }
            new.len()
        };
        self.selected = sel.map(|s| range.start + to8(s.start)..range.start + to8(s.end)).unwrap_or(range.start + new.len()..range.start + new.len());
        self.reversed = false;
        self.changed(cx);
    }
    fn bounds_for_range(&mut self, r: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // 输入法候选框跟随光标；还没排版时退回到输入框位置。
        // 排版结果里的坐标已经是窗口坐标（段落起点取自元素 bounds），不能再加一次 bounds.origin，
        // 否则候选框会被推到右下方很远的地方。
        let range = self.range_from_utf16(&r);
        let Some(p) = self.layout.point_for(range.start) else { return Some(bounds) };
        Some(Bounds::new(p, size(px(2.), self.layout.line_h)))
    }
    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.to_utf16(self.layout.index_for(p)))
    }
}

/// 代码模式的着色分段：按语法高亮结果给关键字、字符串、注释、箭头等上色。
fn code_segs(lang: &str, text: &str, base: crate::canvas::Fmt, t: &crate::theme::Theme) -> Vec<(usize, crate::canvas::Fmt)> {
    use crate::highlight::Tok;
    let col = |tok: Tok| {
        let c = match tok {
            Tok::Keyword => t.syntax.keyword,
            Tok::String => t.syntax.string,
            Tok::Comment => t.syntax.comment,
            Tok::Number => t.syntax.number,
            Tok::Function => t.syntax.function,
            Tok::Type => t.syntax.typ,
            Tok::Arrow => t.syntax.number,
        };
        crate::canvas::Fmt { color: Some(crate::canvas::Col::from_hsla(c)), ..base }
    };
    let mut out = vec![];
    let mut at = 0;
    for (r, tok) in crate::highlight::highlight(lang, text) {
        if r.start < at || r.end > text.len() {
            continue;
        }
        if r.start > at {
            out.push((r.start - at, base));
        }
        out.push((r.end - r.start, col(tok)));
        at = r.end;
    }
    if at < text.len() {
        out.push((text.len() - at, base));
    }
    out
}

/// 负责排版、绘制与注册输入法处理器的元素。
struct AreaEl {
    input: Entity<TextArea>,
}

impl IntoElement for AreaEl {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

struct Prepaint {
    scroll: Pixels,
    scroll_max: Pixels,
    layout: Layout,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    /// 代码模式：行号（文字、位置）与当前行底色
    gutter: Vec<(ShapedLine, Point<Pixels>)>,
    cur_band: Option<PaintQuad>,
}

impl Element for AreaEl {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let t = theme(cx).clone();
        let input = self.input.read(cx);
        let st = input.style;
        let empty = input.content.is_empty();
        let text: SharedString = if empty { input.placeholder.clone() } else { input.content.clone().into() };
        let color = if empty { t.placeholder } else { st.color };
        let code = input.code;
        let mut font = gpui::font(if code.is_some() { "Menlo" } else { input.font.unwrap_or("PingFang SC") });
        font.weight = if st.bold { FontWeight::BOLD } else { FontWeight::NORMAL };
        // 按样式分段生成文字 run，再在输入法组字区间上叠加下划线
        let base_fmt = crate::canvas::Fmt { bold: st.bold, deco: if empty { 0 } else { st.deco }, color: None };
        let segs = match code {
            _ if empty => vec![(text.len(), base_fmt)],
            Some(lang) => code_segs(lang, &text, base_fmt, &t),
            None => crate::canvas::rich::resolve(&input.spans, text.len(), base_fmt),
        };
        let marked = input.marked.as_ref().filter(|_| !empty).map(|m| clamp_str_range(&text, m.clone()));
        let mut runs: Vec<TextRun> = vec![];
        let mut at = 0;
        for (l, f) in segs {
            let c = f.color.map(|c| c.hsla()).unwrap_or(color);
            let mut cuts = vec![at, at + l];
            if let Some(m) = &marked {
                cuts.extend([m.start, m.end].into_iter().filter(|x| *x > at && *x < at + l));
            }
            cuts.sort_unstable();
            for w in cuts.windows(2) {
                if w[1] <= w[0] {
                    continue;
                }
                let mut r = crate::canvas::paint::deco_run(w[1] - w[0], c, f.bold, f.deco);
                r.font.family = font.family.clone();
                r.font.weight = if f.bold { FontWeight::BOLD } else { FontWeight::NORMAL };
                if marked.as_ref().is_some_and(|m| w[0] >= m.start && w[1] <= m.end) {
                    r.underline = Some(UnderlineStyle { color: Some(c), thickness: px(1.), wavy: false });
                }
                runs.push(r);
            }
            at += l;
        }
        if runs.is_empty() {
            runs.push(TextRun { len: 0, ..crate::canvas::paint::deco_run(0, color, st.bold, 0) });
        }
        let line_h = px(st.line_height);
        // 代码模式：左侧留出行号栏
        let n_lines = input.content.split('\n').count().max(1);
        let gutter_w = if code.is_some() { px(st.font_size * 0.62 * (n_lines.to_string().len().max(2) as f32) + 26.) } else { px(0.) };
        let left = bounds.left() + gutter_w;
        let width = (bounds.size.width - gutter_w).max(px(4.));
        let lines = window.text_system().shape_text(text.clone(), px(st.font_size), &runs, Some(width), None).unwrap_or_default();
        let total = lines.iter().fold(px(0.), |acc, l| acc + l.size(line_h).height);
        let mut y = match st.v_align {
            1 => bounds.top() + ((bounds.size.height - total) / 2.).max(px(0.)),
            2 => bounds.top() + (bounds.size.height - total).max(px(0.)),
            _ => bounds.top(),
        };
        let mut paras = Vec::with_capacity(lines.len());
        let mut start = 0;
        for line in lines {
            let h = line.size(line_h).height;
            let len = line.len();
            paras.push((start, line, point(left, y)));
            start += len + 1;
            y += h;
        }
        let mut layout = Layout { paras, line_h, width, align: st.align };
        // 纵向滚动：超出部分可滚动，光标 / 选区变化时自动滚到可见
        let (mut scroll, mut scroll_max) = (px(0.), px(0.));
        if input.scrollable {
            scroll_max = (total - bounds.size.height).max(px(0.));
            scroll = input.scroll_y.clamp(px(0.), scroll_max);
            if input.selected != input.revealed {
                if let Some(c) = layout.point_for(input.cursor()) {
                    let (top, bottom) = (c.y - bounds.top(), c.y - bounds.top() + line_h);
                    if top < scroll {
                        scroll = top;
                    } else if bottom > scroll + bounds.size.height {
                        scroll = bottom - bounds.size.height;
                    }
                    scroll = scroll.clamp(px(0.), scroll_max);
                }
            }
            for para in layout.paras.iter_mut() {
                para.2.y -= scroll;
            }
        }

        let sel = input.selected.clone();
        let mut selection = vec![];
        let mut cursor = None;
        if !empty {
            if sel.is_empty() {
                if let Some(p) = layout.point_for(input.cursor()) {
                    cursor = Some(fill(Bounds::new(p, size(px(1.5), line_h)), t.accent));
                }
            } else {
                // 逐视觉行画选区
                for (pstart, line, origin) in &layout.paras {
                    for (row, (rs, re)) in Layout::rows(line).into_iter().enumerate() {
                        let (gs, ge) = (pstart + rs, pstart + re);
                        let (a, b) = (sel.start.max(gs), sel.end.min(ge));
                        if a >= b && !(sel.start <= gs && sel.end > ge) {
                            continue;
                        }
                        let off = layout.row_offset(line, row);
                        let xa = line.position_for_index(a - pstart, line_h).map(|p| p.x).unwrap_or_default();
                        let xb = line.position_for_index(b - pstart, line_h).map(|p| p.x).unwrap_or_default();
                        let y = origin.y + line_h * row as f32;
                        // 选区跨到下一段时，行尾补一小段表示换行符
                        let extra = if sel.end > ge && b == ge { px(5.) } else { px(0.) };
                        selection.push(fill(Bounds::from_corners(point(origin.x + off + xa, y), point(origin.x + off + xb + extra, y + line_h)), t.selection));
                    }
                }
            }
        } else if let Some((_, _, o)) = layout.paras.first() {
            // 空内容：光标落在占位文字的起点（居中时在中间）
            let x = match st.align {
                1 => bounds.left() + bounds.size.width / 2.,
                2 => bounds.right() - px(2.),
                _ => o.x,
            };
            cursor = Some(fill(Bounds::new(point(x, o.y), size(px(1.5), line_h)), t.accent));
        }
        // 行号与当前行底色
        let mut gutter = vec![];
        let mut cur_band = None;
        if code.is_some() {
            let cur = input.cursor();
            let num_font = gpui::font("Menlo");
            for (i, (pstart, line, origin)) in layout.paras.iter().enumerate() {
                let current = !empty && cur >= *pstart && cur <= pstart + line.len();
                if current && input.selected.is_empty() {
                    let h = line.size(line_h).height;
                    cur_band = Some(fill(Bounds::new(point(bounds.left(), origin.y), size(bounds.size.width, h)), t.accent.opacity(0.07)));
                }
                let label: SharedString = (i + 1).to_string().into();
                let color = if current { t.text } else { t.text_muted };
                let run = TextRun { len: label.len(), font: num_font.clone(), color, background_color: None, underline: None, strikethrough: None };
                let shaped = window.text_system().shape_line(label, px(st.font_size), &[run], None);
                let x = left - px(14.) - shaped.width;
                gutter.push((shaped, point(x, origin.y)));
            }
        }
        Prepaint { scroll, scroll_max, layout, cursor, selection, gutter, cur_band }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let scrollable = self.input.read(cx).scrollable;
        let mask = ContentMask { bounds };
        let mut thumb = None;
        if scrollable && p.scroll_max > px(0.5) && bounds.size.height > px(20.) {
            let view = bounds.size.height;
            let thumb_h = (view * (view / (view + p.scroll_max))).max(px(28.));
            let top = bounds.top() + (view - thumb_h) * (p.scroll / p.scroll_max);
            thumb = Some(Bounds::new(point(bounds.right() - px(7.), top), size(px(5.), thumb_h)));
        }
        window.with_content_mask(Some(mask), |window| {
            Self::paint_body(p, &focus, window, cx);
        });
        if let Some(t) = thumb {
            window.paint_quad(fill(t, theme(cx).text_muted.opacity(0.5)).corner_radii(px(2.5)));
        }
        let layout = std::mem::take(&mut p.layout);
        let (scroll, scroll_max) = (p.scroll, p.scroll_max);
        self.input.update(cx, |i, _| {
            i.layout = layout;
            if i.scrollable {
                i.scroll_y = scroll;
                i.scroll_max = scroll_max;
                i.revealed = i.selected.clone();
            }
        });
    }
}

impl AreaEl {
    fn paint_body(p: &mut Prepaint, focus: &FocusHandle, window: &mut Window, cx: &mut App) {
        if let Some(q) = p.cur_band.take() {
            window.paint_quad(q);
        }
        for (line, at) in &p.gutter {
            line.paint(*at, p.layout.line_h, window, cx).ok();
        }
        for q in p.selection.drain(..) {
            window.paint_quad(q);
        }
        let align = match p.layout.align {
            1 => TextAlign::Center,
            2 => TextAlign::Right,
            _ => TextAlign::Left,
        };
        for (_, line, origin) in &p.layout.paras {
            let b = Bounds::new(*origin, size(p.layout.width, line.size(p.layout.line_h).height));
            line.paint(*origin, p.layout.line_h, align, Some(b), window, cx).ok();
        }
        if focus.is_focused(window) {
            if let Some(c) = p.cursor.take() {
                window.paint_quad(c);
            }
        }
    }
}

impl Render for TextArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context("TextArea")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::word_back))
            .on_action(cx.listener(Self::delete_line_back))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                if this.scrollable && this.scroll_max > px(0.) {
                    let dy = ev.delta.pixel_delta(this.layout.line_h).y;
                    this.scroll_y = (this.scroll_y - dy).clamp(px(0.), this.scroll_max);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(AreaEl { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::{clamp_str_range, TDeleteLineBack, TDocStart, TRedo, TSelectDocEnd, TSelectEnd, TSelectHome, TSelectWordLeft, TUndo, TWordRight, TextArea, TextStyle};

    #[test]
    fn clamp_keeps_char_boundaries_for_multiline() {
        let s = "第一行\n第二行";
        assert_eq!(clamp_str_range(s, 1..100), 0..s.len());
    }

    #[gpui::test]
    fn selection_navigation_and_undo_shortcuts(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(crate::theme::Theme::light()));
        let style = TextStyle { font_size: 13., line_height: 20., color: gpui::black(), bold: false, align: 0, v_align: 0, deco: 0 };
        let win = cx.add_window(|_, cx| TextArea::new("flowchart TD\n    A --> B\n    B --> C", "", style, cx));
        let mut vcx = gpui::VisualTestContext::from_window(win.into(), cx);
        vcx.run_until_parked();

        win.update(&mut vcx, |ta, w, cx| {
            // 跳到文首、选到行尾
            ta.doc_start(&TDocStart, w, cx);
            assert_eq!(ta.selection(), 0..0);
            ta.select_end(&TSelectEnd, w, cx);
            assert_eq!(&ta.text()[ta.selection()], "flowchart TD");

            // 按词右移、按词左选
            ta.doc_start(&TDocStart, w, cx);
            ta.word_right(&TWordRight, w, cx);
            assert_eq!(ta.selection(), 9..9); // "flowchart" 末尾
            ta.select_word_left(&TSelectWordLeft, w, cx);
            assert_eq!(&ta.text()[ta.selection()], "flowchart");

            // 跳到第二行行尾，Cmd+Shift+Left 选到行首
            let line2_end = ta.text().find("--> B").unwrap() + "--> B".len();
            ta.move_to(line2_end, cx);
            ta.select_home(&TSelectHome, w, cx);
            assert_eq!(&ta.text()[ta.selection()], "    A --> B");

            // Cmd+Shift+Down 选到文尾
            ta.move_to(line2_end, cx);
            ta.select_doc_end(&TSelectDocEnd, w, cx);
            assert_eq!(&ta.text()[ta.selection()], "\n    B --> C");

            // Cmd+Backspace 删到行首 + Cmd+Z 撤销 + Cmd+Shift+Z 重做
            ta.move_to(line2_end, cx);
            ta.delete_line_back(&TDeleteLineBack, w, cx);
            assert!(!ta.text().contains("A --> B"));
            ta.undo(&TUndo, w, cx);
            assert!(ta.text().contains("A --> B"));
            ta.redo(&TRedo, w, cx);
            assert!(!ta.text().contains("A --> B"));
        })
        .unwrap();
    }
}
