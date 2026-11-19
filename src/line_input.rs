//! 单行输入框（支持输入法），用于快速打开、查找、链接编辑。
//! 基于 GPUI 官方 input 示例改写，增加了事件与外观。

use crate::theme::theme;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    line_input,
    [
        IBackspace,
        IDelete,
        ILeft,
        IRight,
        ISelectLeft,
        ISelectRight,
        ISelectAll,
        IHome,
        IEnd,
        IPaste,
        ICut,
        ICopy,
        ISubmit,
        IShiftSubmit,
        ICancel,
        IUp,
        IDown,
        IWordBack,
        ISelectWordLeft,
        ISelectWordRight,
        ISelectHome,
        ISelectEnd
    ]
);

pub enum InputEvent {
    Changed,
    Submit { shift: bool },
    Cancel,
    Up,
    Down,
}

impl EventEmitter<InputEvent> for LineInput {}

pub struct LineInput {
    focus: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// 横向滚动量：文字比框宽时左移，保证光标始终可见
    scroll: Pixels,
    selecting: bool,
}

impl LineInput {
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        LineInput {
            focus: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.to_string().into(),
            selected: 0..0,
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            scroll: px(0.),
            selecting: false,
        }
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.content = s.to_string().into();
        self.selected = 0..s.len();
        self.reversed = false;
        cx.notify();
    }

    /// 光标移到末尾、不选中（非聚焦的输入框初始化时用，避免整段高亮）。
    pub fn collapse_to_end(&mut self, cx: &mut Context<Self>) {
        let n = self.content.len();
        self.selected = n..n;
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, o: usize, cx: &mut Context<Self>) {
        self.selected = o..o;
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

    fn left(&mut self, _: &ILeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.prev_boundary(self.cursor()), cx)
        } else {
            self.move_to(self.selected.start, cx)
        }
    }
    fn right(&mut self, _: &IRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.next_boundary(self.selected.end), cx)
        } else {
            self.move_to(self.selected.end, cx)
        }
    }
    fn select_left(&mut self, _: &ISelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.prev_boundary(self.cursor()), cx)
    }
    fn select_right(&mut self, _: &ISelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx)
    }
    fn select_word_left(&mut self, _: &ISelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_word_start(&self.content, self.cursor()), cx)
    }
    fn select_word_right(&mut self, _: &ISelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word_end(&self.content, self.cursor()), cx)
    }
    fn select_home(&mut self, _: &ISelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx)
    }
    fn select_end(&mut self, _: &ISelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx)
    }
    fn select_all(&mut self, _: &ISelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify()
    }
    fn home(&mut self, _: &IHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx)
    }
    fn end(&mut self, _: &IEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx)
    }
    fn backspace(&mut self, _: &IBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.prev_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn word_back(&mut self, _: &IWordBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            let c = self.cursor();
            let start =
                self.content.split_word_bound_indices().rev().find_map(|(i, w)| (i < c && w.chars().any(|ch| ch.is_alphanumeric())).then_some(i)).unwrap_or(0);
            self.select_to(start, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn delete(&mut self, _: &IDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn paste(&mut self, _: &IPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace_text_in_range(None, &t.replace('\n', " "), window, cx);
        }
    }
    fn copy(&mut self, _: &ICopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.clamp_range(self.selected.clone())].to_string()));
        }
    }
    fn cut(&mut self, _: &ICut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.clamp_range(self.selected.clone())].to_string()));
            self.replace_text_in_range(None, "", window, cx)
        }
    }
    fn submit(&mut self, _: &ISubmit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit { shift: false })
    }
    fn shift_submit(&mut self, _: &IShiftSubmit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit { shift: true })
    }
    fn cancel(&mut self, _: &ICancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel)
    }
    fn up(&mut self, _: &IUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Up)
    }
    fn down(&mut self, _: &IDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Down)
    }

    fn index_for_mouse(&self, p: Point<Pixels>) -> usize {
        let (Some(b), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        if p.y < b.top() {
            return 0;
        }
        // 空内容时排版的是占位文字，按它算下标会把光标放到占位文字中间
        if self.content.is_empty() || p.y > b.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(p.x - b.left() + self.scroll).min(self.content.len())
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

    /// 把字节区间夹进文本范围并对齐到 char 边界（见 [`clamp_str_range`]）。
    fn clamp_range(&self, r: Range<usize>) -> Range<usize> {
        clamp_str_range(&self.content, r)
    }
}

/// 把字节区间夹进 `0..=len` 并对齐到 char 边界。
///
/// 输入法（尤其第三方 IME）回传的 range 是以「它上次看到的文本」为基准的，
/// 中间只要我们自己改过 content（提交、粘贴、清空），range 就可能越界，
/// 甚至在多字节字符中间 —— 直接拿去切片 `&content[a..b]` 会 panic 让整个应用退出。
pub(crate) fn clamp_str_range(s: &str, r: Range<usize>) -> Range<usize> {
    let len = s.len();
    let mut a = r.start.min(len);
    while a > 0 && !s.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = r.end.min(len).max(a);
    while b < len && !s.is_char_boundary(b) {
        b += 1;
    }
    a..b
}

/// 词的边界：字母 / 数字 / 下划线连成一个词。汉字连续段也算一个词
/// （UAX#29 把汉字拆成单字，双击只选中一个字，不符合直觉）。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 双击选中的区间：点中的词；点在空白 / 标点上时选相邻的词（优先左边）。
pub(crate) fn word_range_at(text: &str, o: usize) -> Range<usize> {
    let mut o = o.min(text.len());
    while o < text.len() && !text.is_char_boundary(o) {
        o += 1;
    }
    let at = text[o..].chars().next();
    let prev = text[..o].char_indices().next_back();
    let word = at.is_some_and(is_word_char) || prev.is_some_and(|(_, c)| is_word_char(c));
    let same = |c: char| is_word_char(c) == word;
    let (mut start, mut end) = (o, o);
    if let Some(c) = at {
        if same(c) {
            end = o + c.len_utf8();
        }
    }
    if let Some((i, c)) = prev {
        if same(c) {
            start = i;
        }
    }
    while let Some((i, c)) = text[..start].char_indices().next_back() {
        if same(c) {
            start = i;
        } else {
            break;
        }
    }
    while end < text.len() {
        let c = text[end..].chars().next().unwrap();
        if same(c) {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    start..end
}

/// 光标左边那个词的起点（alt+shift+← 用）。
fn prev_word_start(text: &str, o: usize) -> usize {
    let mut i = o.min(text.len());
    while let Some((j, c)) = text[..i].char_indices().next_back() {
        if is_word_char(c) {
            break;
        }
        i = j;
    }
    while let Some((j, c)) = text[..i].char_indices().next_back() {
        if is_word_char(c) {
            i = j;
        } else {
            break;
        }
    }
    i
}

/// 光标右边那个词的终点（alt+shift+→ 用）。
fn next_word_end(text: &str, o: usize) -> usize {
    let mut i = o.min(text.len());
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if is_word_char(c) {
            break;
        }
        i += c.len_utf8();
    }
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if is_word_char(c) {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    i
}

impl EntityInputHandler for LineInput {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.clamp_range(self.range_from_utf16(&r));
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
        let range = self.clamp_range(r.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone()));
        self.content = (self.content[0..range.start].to_owned() + new + &self.content[range.end..]).into();
        self.selected = range.start + new.len()..range.start + new.len();
        self.marked = None;
        self.changed(cx);
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, new: &str, sel: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.clamp_range(r.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone()));
        self.content = (self.content[0..range.start].to_owned() + new + &self.content[range.end..]).into();
        self.marked = (!new.is_empty()).then(|| range.start..range.start + new.len());
        self.selected = sel
            .map(|s| {
                let base = range.start;
                let sub: String = new.to_string();
                let to8 = |u: usize| {
                    let mut c = 0;
                    for (i, ch) in sub.char_indices() {
                        if c >= u {
                            return i;
                        }
                        c += ch.len_utf16();
                    }
                    sub.len()
                };
                base + to8(s.start)..base + to8(s.end)
            })
            .unwrap_or(range.start + new.len()..range.start + new.len());
        self.changed(cx);
    }
    fn bounds_for_range(&mut self, r: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // 还没排版过（刚打开就开始输入）：至少给出输入框本身的位置，别让候选框落到屏幕角落
        let Some(line) = self.last_layout.as_ref() else { return Some(bounds) };
        let range = self.range_from_utf16(&r);
        Some(Bounds::from_corners(
            point(bounds.left() - self.scroll + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() - self.scroll + line.x_for_index(range.end), bounds.bottom()),
        ))
    }
    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let line = self.last_layout.as_ref()?;
        let ix = line.index_for_x(p.x - local.x + self.scroll)?;
        Some(self.to_utf16(ix))
    }
}

struct TextEl {
    input: Entity<LineInput>,
}

struct Prepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll: Pixels,
}

impl IntoElement for TextEl {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextEl {
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
        style.size.height = window.line_height().into();
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
        let content = input.content.clone();
        let sel = input.selected.clone();
        let cursor = input.cursor();
        let scroll0 = input.scroll;
        let style = window.text_style();
        let (text, color) = if content.is_empty() { (input.placeholder.clone(), t.placeholder) } else { (content, t.text) };
        let run = TextRun { len: text.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = if let Some(m) = input.marked.as_ref() {
            let m = input.clamp_range(m.clone());
            vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.), wavy: false }), ..run.clone() },
                TextRun { len: text.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect()
        } else {
            vec![run]
        };
        let size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(text, size, &runs, None);
        let cx_ = line.x_for_index(cursor);
        // 光标跑出可视区时滚动；内容变短时回收多余的滚动量。占位文字不滚动（超出部分被裁掉）
        let w = bounds.size.width - px(2.);
        let scroll = if input.content.is_empty() {
            px(0.)
        } else {
            let mut s = scroll0;
            if cx_ - s > w {
                s = cx_ - w;
            }
            if cx_ < s {
                s = cx_;
            }
            s.min((line.width - w).max(px(0.))).max(px(0.))
        };
        let left = bounds.left() - scroll;
        let (selection, cursor) = if sel.is_empty() {
            (None, Some(fill(Bounds::new(point(left + cx_, bounds.top() + px(2.)), size_(px(1.5), bounds.size.height - px(4.))), t.accent)))
        } else {
            (
                Some(fill(
                    Bounds::from_corners(point(left + line.x_for_index(sel.start), bounds.top()), point(left + line.x_for_index(sel.end), bounds.bottom())),
                    t.selection,
                )),
                None,
            )
        };
        Prepaint { line: Some(line), cursor, selection, scroll }
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
        let line = p.line.take().unwrap();
        let scroll = p.scroll;
        // 裁剪到输入框内：长文本 / 长占位文字不再画出框外（光标宽 1.5px，右侧多留一点）
        let mask = ContentMask { bounds: Bounds::new(bounds.origin, size(bounds.size.width + px(2.), bounds.size.height)) };
        window.with_content_mask(Some(mask), |window| {
            if let Some(s) = p.selection.take() {
                window.paint_quad(s)
            }
            line.paint(point(bounds.left() - scroll, bounds.top()), window.line_height(), window, cx).ok();
            if focus.is_focused(window) {
                if let Some(c) = p.cursor.take() {
                    window.paint_quad(c);
                }
            }
        });
        self.input.update(cx, |i, _| {
            i.last_layout = Some(line);
            i.last_bounds = Some(bounds);
            i.scroll = scroll;
        });
    }
}

fn size_(w: Pixels, h: Pixels) -> Size<Pixels> {
    size(w, h)
}

impl Render for LineInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .key_context("LineInput")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::word_back))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::shift_submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, w, cx| {
                    w.focus(&this.focus);
                    let ix = this.index_for_mouse(e.position);
                    match e.click_count {
                        // 双击选词、三击全选（和系统文本框一致）
                        2 => {
                            this.selected = word_range_at(&this.content, ix);
                            this.reversed = false;
                            this.selecting = false;
                            cx.notify();
                        }
                        n if n >= 3 => {
                            this.selected = 0..this.content.len();
                            this.reversed = false;
                            this.selecting = false;
                            cx.notify();
                        }
                        _ => {
                            this.selecting = true;
                            if e.modifiers.shift {
                                this.select_to(ix, cx)
                            } else {
                                this.move_to(ix, cx)
                            }
                        }
                    }
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.selecting = false))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    let ix = this.index_for_mouse(e.position);
                    this.select_to(ix, cx);
                }
            }))
            .child(TextEl { input: cx.entity() })
    }
}

impl Focusable for LineInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    let c = Some("LineInput");
    cx.bind_keys([
        KeyBinding::new("backspace", IBackspace, c),
        KeyBinding::new("alt-backspace", IWordBack, c),
        KeyBinding::new("delete", IDelete, c),
        KeyBinding::new("left", ILeft, c),
        KeyBinding::new("right", IRight, c),
        KeyBinding::new("shift-left", ISelectLeft, c),
        KeyBinding::new("shift-right", ISelectRight, c),
        KeyBinding::new("alt-shift-left", ISelectWordLeft, c),
        KeyBinding::new("alt-shift-right", ISelectWordRight, c),
        KeyBinding::new("shift-home", ISelectHome, c),
        KeyBinding::new("shift-end", ISelectEnd, c),
        KeyBinding::new("cmd-shift-left", ISelectHome, c),
        KeyBinding::new("cmd-shift-right", ISelectEnd, c),
        KeyBinding::new("cmd-a", ISelectAll, c),
        KeyBinding::new("cmd-v", IPaste, c),
        KeyBinding::new("cmd-c", ICopy, c),
        KeyBinding::new("cmd-x", ICut, c),
        KeyBinding::new("home", IHome, c),
        KeyBinding::new("cmd-left", IHome, c),
        KeyBinding::new("end", IEnd, c),
        KeyBinding::new("cmd-right", IEnd, c),
        KeyBinding::new("enter", ISubmit, c),
        KeyBinding::new("shift-enter", IShiftSubmit, c),
        KeyBinding::new("escape", ICancel, c),
        KeyBinding::new("up", IUp, c),
        KeyBinding::new("down", IDown, c),
    ]);
}

#[cfg(test)]
mod tests {
    // 别用 `super::*`：会把 gpui 的 `test` 宏引进作用域，遮蔽内置的 `#[test]`
    use super::clamp_str_range;

    #[test]
    fn clamp_range_never_panics_and_stays_on_boundaries() {
        // 「你好」= 6 字节，每个汉字 3 字节
        let s = "你好";
        // 越界
        assert_eq!(clamp_str_range(s, 0..999), 0..6);
        assert_eq!(clamp_str_range(s, 99..999), 6..6);
        // 落在多字节字符中间 → 回退 / 前进到边界
        assert_eq!(clamp_str_range(s, 1..2), 0..3);
        assert_eq!(clamp_str_range(s, 4..5), 3..6);
        // 反转区间（start > end）会被压成空区间（故意构造反向区间，测的就是这种输入）
        #[allow(clippy::reversed_empty_ranges)]
        let r = clamp_str_range(s, 5..2);
        assert!(r.start <= r.end);
        // 夹完的区间一定可以安全切片
        let _ = &s[r];
    }

    #[test]
    fn clamp_range_handles_empty_content() {
        assert_eq!(clamp_str_range("", 0..0), 0..0);
        assert_eq!(clamp_str_range("", 3..7), 0..0);
    }

    use super::{next_word_end, prev_word_start, word_range_at};

    /// 双击选词：英文按单词，汉字按连续段（不是单字）。
    #[test]
    fn double_click_selects_word() {
        let s = "hello world 你好世界";
        assert_eq!(word_range_at(s, 1), 0..5);
        assert_eq!(word_range_at(s, 8), 6..11);
        // 点在词尾的空白上：选左边那个词
        assert_eq!(word_range_at(s, 11), 6..11);
        assert_eq!(word_range_at(s, 13), 12..24);
        assert_eq!(word_range_at(s, s.len()), 12..24);
        assert_eq!(word_range_at("", 0), 0..0);
        // 下划线和数字都是词的一部分
        assert_eq!(word_range_at("my_var2 = 1", 2), 0..7);
    }

    /// alt+shift+← / → 的词边界。
    #[test]
    fn keyboard_word_selection_bounds() {
        let s = "hello world";
        assert_eq!(prev_word_start(s, 11), 6);
        assert_eq!(prev_word_start(s, 6), 0);
        assert_eq!(prev_word_start(s, 0), 0);
        assert_eq!(next_word_end(s, 0), 5);
        assert_eq!(next_word_end(s, 5), 11);
        assert_eq!(next_word_end(s, 11), 11);
    }
}
