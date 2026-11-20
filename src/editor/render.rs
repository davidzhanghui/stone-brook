//! 渲染：虚拟列表只渲染可见块；每个文本槽一个 StyledText，
//! 光标 / 选区 / 查找高亮 / 语法高亮都折叠进 TextRun，绘制成本与文档长度无关。

use super::*;
use crate::convert::{bullet_glyph, ordered_label};
use crate::highlight::{highlight_cached, Tok};
use crate::model::{BlockKind, Marks, RichText};
use crate::theme::{theme, Theme};
use crate::ui::{icon, Tooltip};
use std::path::PathBuf;

/// 代码块可拖动高度的上下限（正文区，不含工具栏）
pub(crate) const CODE_MIN_H: f32 = 84.;
pub(crate) const CODE_MAX_H: f32 = 2000.;

pub fn content_w() -> f32 {
    if crate::settings::wide() {
        960.
    } else {
        760.
    }
}
/// 左侧装订线：悬停时依次放折叠三角、块类型图标、拖动手柄（各 22px，边缘略重叠）。
pub(super) const GUTTER: f32 = 64.;
const INDENT_W: f32 = 26.;
/// 供图片宽度计算复用。
pub fn indent_w() -> f32 {
    INDENT_W
}
pub fn body_size() -> f32 {
    crate::settings::font_size()
}
pub fn body_lh() -> f32 {
    (crate::settings::font_size() * 1.74).round()
}

const FONT: &str = "PingFang SC";
const FONT_LATIN: &str = ".SystemUIFont";
const MONO: &str = "Menlo";

const CALLOUT_EMOJIS: [&str; 8] = ["💡", "📌", "⚠️", "✅", "❗", "🔥", "📝", "🎯"];

#[derive(Clone)]
pub struct SlotSpec {
    pub size: f32,
    pub line_h: f32,
    pub weight: FontWeight,
    pub mono: bool,
    pub color: Hsla,
    pub strike: bool,
    pub placeholder: Option<SharedString>,
    pub lang: Option<String>,
    /// 不自动换行（代码块关闭换行时）：按内容固有宽度排成单行，由外层横向滚动
    pub nowrap: bool,
    /// 关闭换行时的横向滚动句柄：光标移出可视区时自动跟过去
    pub hscroll: Option<ScrollHandle>,
    /// 代码块折叠：要隐藏的模型字节区间（已排序、互不重叠）
    pub folds: Option<Rc<Vec<(usize, usize)>>>,
}

impl SlotSpec {
    pub fn body(t: &Theme) -> Self {
        SlotSpec {
            size: body_size(),
            line_h: body_lh(),
            weight: FontWeight::NORMAL,
            mono: false,
            color: t.text,
            strike: false,
            placeholder: None,
            lang: None,
            nowrap: false,
            hscroll: None,
            folds: None,
        }
    }
}

#[derive(Clone)]
pub struct DragBlock {
    pub ix: usize,
    pub label: String,
}

pub struct DragPreview(String);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px_3()
            .py_1()
            .rounded(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.accent)
            .shadow_md()
            .text_size(px(13.))
            .text_color(t.text)
            .opacity(0.92)
            .child(self.0.clone())
    }
}

fn spacing(kind: &BlockKind) -> (f32, f32) {
    match kind {
        BlockKind::Title => (56., 18.),
        BlockKind::Heading { level: 1 } => (22., 6.),
        BlockKind::Heading { level: 2 } => (18., 4.),
        BlockKind::Heading { .. } => (14., 2.),
        BlockKind::Code { .. } | BlockKind::Callout { .. } => (8., 8.),
        // 表格上方自带 16px 的列手柄行
        BlockKind::Table { .. } => (2., 8.),
        BlockKind::Columns { .. } | BlockKind::Image { .. } => (6., 6.),
        _ => (3., 3.),
    }
}

fn heading_size(level: u8) -> f32 {
    [26., 22., 19., 17., 16., 15.][(level.clamp(1, 6) - 1) as usize]
}

/// 块类型图标的配色（与斜杠菜单同一套色板）。
fn kind_icon_color(kind: &BlockKind) -> u32 {
    match kind {
        BlockKind::Title | BlockKind::Text | BlockKind::Heading { .. } | BlockKind::Quote | BlockKind::Html { .. } => 0x3b6cf6,
        BlockKind::Bullet | BlockKind::Ordered | BlockKind::Todo { .. } | BlockKind::Columns { .. } | BlockKind::Toc => 0x5b5bd6,
        BlockKind::Code { .. } | BlockKind::Embed { .. } => 0x2ea121,
        BlockKind::Callout { .. } | BlockKind::Divider | BlockKind::Image { .. } | BlockKind::Countdown { .. } => 0xf5a339,
        BlockKind::File { .. } | BlockKind::Video { .. } => 0x0ea5e9,
        BlockKind::Table { .. } | BlockKind::LinkCard { .. } => 0x14b8a6,
        BlockKind::Notes => 0x8f959e,
        BlockKind::Math => 0x1f2329,
        BlockKind::Diagram { .. } | BlockKind::Timeline { .. } => 0x8b5cf6,
        BlockKind::Button { .. } => 0x5b5bd6,
        BlockKind::Unknown { .. } => 0x8f959e,
    }
}

/// 块类型图标：鼠标悬停块时显示在左上角（菜单 / 拖动柄左边）。
fn kind_icon_name(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Title | BlockKind::Text | BlockKind::Heading { .. } => "type",
        BlockKind::Bullet => "list",
        BlockKind::Ordered => "list-ordered",
        BlockKind::Todo { .. } => "square-check",
        BlockKind::Quote => "quote",
        BlockKind::Code { .. } => "braces",
        BlockKind::Callout { .. } => "callout",
        BlockKind::Divider => "divider",
        BlockKind::Notes => "comment",
        BlockKind::Image { .. } => "image",
        BlockKind::Table { .. } => "table",
        BlockKind::Columns { .. } => "columns",
        BlockKind::File { .. } => "file",
        BlockKind::Toc => "outline",
        BlockKind::Countdown { .. } => "clock",
        BlockKind::Button { .. } => "square",
        BlockKind::Timeline { .. } => "timeline",
        BlockKind::Math => "sigma",
        BlockKind::Diagram { .. } => "workflow",
        BlockKind::Html { .. } => "code-xml",
        BlockKind::LinkCard { .. } => "link",
        BlockKind::Video { .. } => "video",
        BlockKind::Embed { .. } => "layout",
        BlockKind::Unknown { .. } => "file",
    }
}

/// 把富文本 + 各种叠加层（选区、输入法组字、查找、语法高亮）合成为 TextRun 序列。
#[allow(clippy::too_many_arguments)]
fn build_runs(
    rt: &RichText,
    spec: &SlotSpec,
    t: &Theme,
    sel: Option<Range<usize>>,
    marked: Option<Range<usize>>,
    finds: &[(Range<usize>, bool)],
    syntax: &[(Range<usize>, Tok)],
) -> Vec<TextRun> {
    let len = rt.text.len();
    let mut cuts: Vec<usize> = Vec::with_capacity(16);
    cuts.push(0);
    cuts.push(len);
    let mut pos = 0;
    for sp in &rt.spans {
        pos += sp.len;
        cuts.push(pos);
    }
    for r in sel.iter().chain(marked.iter()).chain(finds.iter().map(|(r, _)| r)).chain(syntax.iter().map(|(r, _)| r)) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs = Vec::with_capacity(cuts.len());
    let mut span_ix = 0;
    let mut span_end = rt.spans.first().map(|s| s.len).unwrap_or(len);
    let default_marks = Marks::default();
    let mut syn_ix = 0;
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        while a >= span_end && span_ix + 1 < rt.spans.len() {
            span_ix += 1;
            span_end += rt.spans[span_ix].len;
        }
        let m = rt.spans.get(span_ix).map(|s| &s.marks).unwrap_or(&default_marks);
        while syn_ix < syntax.len() && syntax[syn_ix].0.end <= a {
            syn_ix += 1;
        }
        let tok = syntax.get(syn_ix).filter(|(r, _)| r.start <= a && a < r.end).map(|(_, k)| *k);
        let in_range = |r: &Range<usize>| r.start <= a && a < r.end;

        let mono = spec.mono || m.code;
        let family = if mono {
            MONO
        } else if m.italic {
            FONT_LATIN
        } else {
            FONT
        };
        let mut f = font(family);
        f.weight = if m.bold {
            if spec.weight.0 >= FontWeight::SEMIBOLD.0 {
                FontWeight::BOLD
            } else {
                FontWeight::SEMIBOLD
            }
        } else {
            spec.weight
        };
        if m.italic {
            f.style = FontStyle::Italic;
        }
        let mut color = if m.math.is_some() {
            gpui::transparent_black()
        } else if m.link.is_some() {
            t.link
        } else if m.color > 0 {
            t.fg_palette[(m.color & 7) as usize]
        } else if m.code && !spec.mono {
            t.inline_code_fg
        } else {
            spec.color
        };
        if let Some(k) = tok {
            color = match k {
                Tok::Keyword => t.syntax.keyword,
                Tok::String => t.syntax.string,
                Tok::Comment => t.syntax.comment,
                Tok::Number => t.syntax.number,
                Tok::Function => t.syntax.function,
                Tok::Type => t.syntax.typ,
                Tok::Arrow => t.syntax.keyword,
            };
        }
        let mut bg = if m.math.is_some() {
            None
        } else if m.bg > 0 {
            Some(t.text_bg(m.bg))
        } else if m.code && !spec.mono {
            Some(t.inline_code_bg)
        } else {
            None
        };
        if let Some((_, cur)) = finds.iter().find(|(r, _)| in_range(r)) {
            bg = Some(if *cur { t.find_current } else { t.find_match });
        }
        if sel.as_ref().is_some_and(in_range) {
            bg = Some(t.selection);
        }
        let is_marked = marked.as_ref().is_some_and(in_range);
        let underline = (m.underline || m.link.is_some() || is_marked).then(|| UnderlineStyle {
            color: Some(if is_marked { t.accent } else { color }),
            thickness: px(1.),
            wavy: false,
        });
        let strikethrough = (m.strike || spec.strike).then(|| StrikethroughStyle { color: Some(color), thickness: px(1.) });
        runs.push(TextRun { len: b - a, font: f, color, background_color: bg, underline, strikethrough });
    }
    runs
}

/// 模型偏移 ⇄ 显示偏移。每个公式在模型里占 3 字节，显示时占 d_len 字节。
#[derive(Debug, Default)]
pub struct DisplayMap {
    /// 被替换掉的片段：模型区间 [m, m+mlen) 在显示上占 [d, d+dlen) 个字节
    pub segs: Vec<Seg>,
}

/// 一段「模型区间 → 显示区间」的替换（行内公式占位 / 折叠占位）。
#[derive(Debug)]
pub struct Seg {
    pub m: usize,
    pub mlen: usize,
    pub d: usize,
    pub dlen: usize,
    /// 是否行内公式（点击时打开公式编辑；否则是折叠占位，点击展开）
    pub math: bool,
}

impl DisplayMap {
    pub fn m2d(&self, o: usize) -> usize {
        let mut delta: isize = 0;
        for s in &self.segs {
            if o <= s.m {
                break;
            }
            if o < s.m + s.mlen {
                return s.d;
            }
            delta += s.dlen as isize - s.mlen as isize;
        }
        (o as isize + delta).max(0) as usize
    }
    /// 显示偏移 → 模型偏移；落在占位符内部时吸附到两端。
    pub fn d2m(&self, d: usize) -> usize {
        let mut delta: isize = 0;
        for s in &self.segs {
            if d < s.d {
                break;
            }
            if d < s.d + s.dlen {
                return if d - s.d > s.dlen / 2 { s.m + s.mlen } else { s.m };
            }
            delta += s.dlen as isize - s.mlen as isize;
        }
        (d as isize - delta).max(0) as usize
    }
    /// 显示偏移落在行内公式上时，返回公式的模型起点。
    pub fn formula_at(&self, d: usize) -> Option<usize> {
        self.segs.iter().find(|s| s.math && d >= s.d && d < s.d + s.dlen).map(|s| s.m)
    }
    /// 显示偏移落在折叠占位上时，返回折叠区间的模型起点。
    pub fn fold_at(&self, d: usize) -> Option<usize> {
        self.segs.iter().find(|s| !s.math && d >= s.d && d < s.d + s.dlen).map(|s| s.m)
    }
}

/// 构建显示用文本：已渲染的公式 → 不换行空格（上面再画图片）；渲染中 / 出错 → 显示源码。
#[allow(clippy::type_complexity)]
fn build_display(
    rt: &RichText,
    maths: &[(usize, String)],
    spec: &SlotSpec,
    nbsp_w: f32,
    dark: bool,
) -> (RichText, DisplayMap, Vec<(usize, PathBuf, f32, f32)>) {
    let mut out = RichText::default();
    let mut map = DisplayMap::default();
    let mut images = vec![];
    let mut pos = 0;
    for (start, tex) in maths {
        if *start > pos {
            let seg = rt.slice(pos..*start);
            out.insert_rich(out.len(), &seg);
        }
        let d_start = out.len();
        match crate::math::peek(tex, false, dark) {
            crate::math::MathState::Ready { path, width, height } => {
                // KaTeX 行内字号约 16.8px，按当前字号缩放；太高的公式压到 1.3 倍行高以内
                let mut s = spec.size / 15.;
                if height * s > spec.line_h * 1.3 {
                    s = spec.line_h * 1.3 / height;
                }
                let (w, h) = (width * s, height * s);
                let n = ((w + 2.) / nbsp_w.max(1.)).ceil().max(1.) as usize;
                let marks = Marks { math: Some(tex.clone()), ..Default::default() };
                out.insert(out.len(), &"\u{00A0}".repeat(n), marks);
                images.push((d_start, path, w, h));
            }
            state => {
                let err = matches!(state, crate::math::MathState::Error(_));
                let marks = Marks { code: true, color: if err { 1 } else { 7 }, ..Default::default() };
                out.insert(out.len(), &format!("${tex}$"), marks);
            }
        }
        map.segs.push(Seg { m: *start, mlen: 3, d: d_start, dlen: out.len() - d_start, math: true });
        pos = start + 3;
    }
    if pos < rt.len() {
        let seg = rt.slice(pos..rt.len());
        out.insert_rich(out.len(), &seg);
    }
    (out, map, images)
}

/// 折叠显示：被收起的行区间替换成「⋯」，建立模型 ⇄ 显示的映射。
fn build_folded(rt: &RichText, hides: &[(usize, usize)]) -> (RichText, DisplayMap) {
    let mut out = RichText::default();
    let mut map = DisplayMap { segs: vec![] };
    let mut pos = 0usize;
    for &(m, e) in hides {
        let m = m.min(rt.len());
        let e = e.min(rt.len()).max(m);
        if m > pos {
            let seg = rt.slice(pos..m);
            out.insert_rich(out.len(), &seg);
        }
        let d = out.len();
        const ELLIPSIS: &str = "⋯";
        out.insert(out.len(), ELLIPSIS, Marks { code: true, color: 7, ..Default::default() });
        map.segs.push(Seg { m, mlen: e.saturating_sub(m), d, dlen: ELLIPSIS.len(), math: false });
        pos = pos.max(e);
    }
    if pos < rt.len() {
        let seg = rt.slice(pos..rt.len());
        out.insert_rich(out.len(), &seg);
    }
    (out, map)
}

impl Editor {
    pub(super) fn slot_click(&self, b: usize, c: usize, cx: &mut Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static + use<> {
        let view = cx.entity().downgrade();
        move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            view.update(cx, |this, cx| this.slot_mouse_down(b, c, ev, window, cx)).ok();
        }
    }

    /// 渲染一个文本槽：StyledText + 一个透明画布（登记布局、绘制光标）。
    pub(super) fn render_slot(&self, b: usize, c: usize, rt: &RichText, spec: SlotSpec, focused: bool, t: &Theme) -> Div {
        let slot = (b, c);
        // 行内公式：把占位字符替换成撑宽的空格（或源码），建立 模型偏移 ⇄ 显示偏移 的映射
        let maths = rt.math_spans();
        let (disp, map, images) = if let Some(hides) = spec.folds.as_ref().filter(|h| !h.is_empty()) {
            let (d, m) = build_folded(rt, hides);
            (Some(d), Some(Rc::new(m)), vec![])
        } else if maths.is_empty() {
            (None, None, vec![])
        } else {
            let (d, m, im) = build_display(rt, &maths, &spec, self.caches.nbsp_w.get() * spec.size, t.dark);
            (Some(d), Some(Rc::new(m)), im)
        };
        let rt_disp: &RichText = disp.as_ref().unwrap_or(rt);
        let m2d = |o: usize| map.as_ref().map_or(o, |m| m.m2d(o));
        let sel = self.sel_in_slot(slot).map(|r| m2d(r.start)..m2d(r.end));
        let is_head = self.core.head.slot() == slot;
        let marked = if is_head { self.core.marked.clone().map(|r| m2d(r.start)..m2d(r.end)) } else { None };
        let finds: Vec<(Range<usize>, bool)> = self
            .find
            .as_ref()
            .map(|f| {
                f.matches
                    .iter()
                    .enumerate()
                    .filter(|(_, (p, _))| p.slot() == slot)
                    .map(|(i, (p, l))| (m2d(p.offset)..m2d(p.offset + l), i == f.current))
                    .collect()
            })
            .unwrap_or_default();
        let syntax = spec.lang.as_deref().map(|l| highlight_cached(l, &rt_disp.text)).unwrap_or_default();

        let (text, runs): (SharedString, Vec<TextRun>) = if rt.is_empty() {
            match &spec.placeholder {
                Some(ph) => {
                    let mut f = font(if spec.mono { MONO } else { FONT });
                    f.weight = spec.weight;
                    (ph.clone(), vec![TextRun { len: ph.len(), font: f, color: t.placeholder, background_color: None, underline: None, strikethrough: None }])
                }
                None => (SharedString::default(), vec![]),
            }
        } else {
            (rt_disp.text.clone().into(), build_runs(rt_disp, &spec, t, sel.clone(), marked, &finds, &syntax))
        };
        let styled = StyledText::new(text).with_runs(runs);
        let layout = styled.layout().clone();
        let frame = self.frame.clone();
        let has_sel = self.has_selection();
        let caret = is_head.then_some(m2d(self.core.head.offset.min(rt.len())));
        let show_caret = focused && is_head && !has_sel && self.cursor_visible && !self.sess.read_only;
        let sel_start = has_sel && self.sel().0.slot() == slot;
        let sel_start_offset = m2d(self.sel().0.offset.min(rt.len()));
        let caret_color = t.cursor;
        let caret_w = if spec.size > 20. { px(2.5) } else { px(2.) };
        // 光标只比字高一点，不占满整行（行距 1.74 倍，占满会显得空行里一根很高的柱子）
        let caret_h = (spec.size * 1.2).round();
        let line_h = spec.line_h;
        // 关闭换行的代码块：光标移动时把它带回横向可视区
        let hscroll = spec.hscroll.clone();
        let follow = focused && is_head && !has_sel && !self.sess.read_only;
        let caret_block = self.content.blocks.get(b).map(|x| x.id);

        div()
            .relative()
            .when(!spec.nowrap, |d| d.w_full())
            .min_w(px(0.))
            .when(spec.nowrap, |d| d.whitespace_nowrap())
            .text_size(px(spec.size))
            .line_height(px(spec.line_h))
            // 单元格 / 分栏自带对齐；普通块 align 为 0，继承 Block::align
            .when(rt.align == 1, |d| d.text_align(gpui::TextAlign::Center))
            .when(rt.align == 2, |d| d.text_align(gpui::TextAlign::Right))
            .cursor(CursorStyle::IBeam)
            .child(styled)
            .child(
                canvas(
                    move |_, window, _| {
                        let mut f = frame.borrow_mut();
                        f.layouts.insert(slot, layout.clone());
                        match &map {
                            Some(m) => {
                                f.maps.insert(slot, m.clone());
                            }
                            None => {
                                f.maps.remove(&slot);
                            }
                        }
                        let mut quad = None;
                        if let Some(o) = caret {
                            if let Some(p) = layout.position_for_index(o) {
                                let lh = layout.line_height();
                                let h = px(caret_h).min(lh);
                                let r = Bounds::new(point(p.x, p.y + (lh - h) / 2.), size(caret_w, h));
                                f.cursor = Some(r);
                                if show_caret {
                                    quad = Some(r);
                                }
                                // 只在光标位置变化时跟随：用户手动横向滚动不会被弹回
                                if follow {
                                    if let (Some(hs), Some(cid)) = (hscroll.as_ref(), caret_block) {
                                        if f.code_caret_last != Some((cid, o)) {
                                            f.code_caret_last = Some((cid, o));
                                            let vb = hs.bounds();
                                            if vb.size.width > px(0.) {
                                                let pad = px(16.);
                                                let mut off = hs.offset();
                                                if p.x < vb.left() + pad {
                                                    off.x += vb.left() + pad - p.x;
                                                } else if p.x > vb.right() - pad {
                                                    off.x -= p.x - (vb.right() - pad);
                                                }
                                                off.x = off.x.clamp(-hs.max_offset().width, px(0.));
                                                if off.x != hs.offset().x {
                                                    hs.set_offset(off);
                                                    window.refresh();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if sel_start {
                            f.sel_start = layout.position_for_index(sel_start_offset);
                        }
                        // 公式图片的位置
                        let imgs: Vec<(Bounds<Pixels>, PathBuf)> = images
                            .iter()
                            .filter_map(|(d, path, w, h)| {
                                let p = layout.position_for_index(*d)?;
                                Some((Bounds::new(point(p.x + px(1.), p.y + px((line_h - h) / 2.)), size(px(*w), px(*h))), path.clone()))
                            })
                            .collect();
                        (quad, imgs)
                    },
                    move |_, (quad, imgs), window, _| {
                        for (b, path) in imgs {
                            if let Some(img) = crate::canvas::paint::load_image(&path.to_string_lossy(), window) {
                                window.paint_image(b, Corners::default(), img, 0, false).ok();
                            }
                        }
                        if let Some(q) = quad {
                            window.paint_quad(fill(q, caret_color));
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    /// 登记「空块」（分割线 / 图片）的位置，用于拖选命中。
    pub(super) fn void_marker(&self, b: usize) -> impl IntoElement {
        let frame = self.frame.clone();
        canvas(
            move |bounds, _, _| {
                frame.borrow_mut().voids.insert(b, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    pub(super) fn render_block(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(blk) = self.content.blocks.get(ix).cloned() else {
            return div().into_any_element();
        };
        // 折叠 / 聚焦隐藏的块：零高度占位
        if self.is_hidden(ix) {
            return div().h(px(0.)).into_any_element();
        }
        let t = theme(cx).clone();
        if ix == 0 && self.sess.focus_root.is_some() {
            return self.render_focus_crumbs(&t, cx);
        }
        let focused = self.is_focused(window);
        let number = self.ordered_numbers().get(ix).copied().unwrap_or(1);
        let indent_px = blk.indent as f32 * INDENT_W;
        let (pt, pb) = spacing(&blk.kind);
        let is_head_block = self.core.head.block == ix;
        let is_image = matches!(blk.kind, BlockKind::Image { .. });
        let empty_here = focused && is_head_block && !self.has_selection() && blk.text.is_empty();
        let last = self.content.blocks.len() - 1;

        let mut row_lh = body_lh();
        let body: AnyElement = match &blk.kind {
            BlockKind::Title => {
                row_lh = 48.;
                let spec = SlotSpec { size: 34., line_h: 48., weight: FontWeight::BOLD, placeholder: Some("请输入标题".into()), ..SlotSpec::body(&t) };
                self.render_slot(ix, 0, &blk.text, spec, focused, &t).on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)).into_any_element()
            }
            BlockKind::Text => {
                let spec = SlotSpec { placeholder: empty_here.then(|| "输入「/」快速插入内容".into()), ..SlotSpec::body(&t) };
                self.render_slot(ix, 0, &blk.text, spec, focused, &t).on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)).into_any_element()
            }
            BlockKind::Heading { level } => {
                let size = heading_size(*level);
                row_lh = (size * 1.45).round();
                let spec = SlotSpec { size, line_h: row_lh, weight: FontWeight::SEMIBOLD, placeholder: Some(blk.kind.label().into()), ..SlotSpec::body(&t) };
                self.render_slot(ix, 0, &blk.text, spec, focused, &t).on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)).into_any_element()
            }
            BlockKind::Bullet | BlockKind::Ordered | BlockKind::Todo { .. } => self.render_block_bullet(ix, cx, &blk, focused, number, &t),
            BlockKind::Quote => {
                let spec = SlotSpec { color: t.text_secondary, ..SlotSpec::body(&t) };
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .child(div().w(px(3.)).flex_none().rounded(px(2.)).bg(t.quote_bar))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .pl(px(13.))
                            .child(self.render_slot(ix, 0, &blk.text, spec, focused, &t))
                            .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
                    )
                    .into_any_element()
            }
            BlockKind::Notes => self.render_block_notes(ix, cx, &blk, focused, &t),
            BlockKind::Callout { emoji, color } => self.render_block_callout(ix, cx, &blk, color, emoji, focused, &t),
            BlockKind::Code { lang, wrap, height } => self.render_code(ix, &blk, lang, *wrap, *height, focused, &t, cx),
            BlockKind::Divider => {
                let selected = self.block_selected(ix) && focused;
                div()
                    .relative()
                    .w_full()
                    .h(px(body_lh()))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .when(selected, |d| d.bg(t.accent_soft))
                    .child(div().w_full().h(px(1.)).bg(t.border_strong))
                    .child(self.void_marker(ix))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus);
                            this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                        }),
                    )
                    .into_any_element()
            }
            BlockKind::Image { path, .. } => self.render_image(ix, path, focused, &t, cx),
            BlockKind::Table { .. } => self.render_table(ix, &blk.kind, focused, &t, cx),
            BlockKind::Columns { cols } => self.render_columns(ix, cols, focused, &t, cx),
            _ => self.render_v2(ix, &blk, focused, &t, window, cx),
        };

        let menu_open = self.pop.block_menu == Some(ix);
        let label: String = {
            let s = blk.plain_text();
            let s: String = s.chars().take(24).collect();
            if s.trim().is_empty() {
                blk.kind.label().to_string()
            } else {
                s
            }
        };
        let is_table = matches!(blk.kind, BlockKind::Table { .. });
        // 表格的块手柄与第一行对齐，并让出左侧的行手柄条
        let handle_top = if is_table { pt + 16. + 7. } else { pt + (row_lh - 22.) / 2. };
        let handle = div()
            .absolute()
            .left(px(GUTTER + indent_px - 22. - if is_table { 16. } else { 0. }))
            .top(px(handle_top.max(0.)))
            .child(
                div()
                    .id(("grip", blk.id as usize))
                    .size(px(22.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .when(!menu_open, |d| d.invisible().when(!self.sess.read_only, |d| d.group_hover("blk", |s| s.visible())))
                    .when(menu_open, |d| d.bg(t.hover))
                    .hover(|s| s.bg(t.hover))
                    .child(icon("grip", 14., t.text_muted))
                    // 阅读模式：不能打开块菜单、不能拖动
                    .when(!self.sess.read_only, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.pop.block_menu = if this.pop.block_menu == Some(ix) { None } else { Some(ix) };
                            cx.notify();
                        }))
                        .on_drag(DragBlock { ix, label }, |d: &DragBlock, _, _, cx| cx.new(|_| DragPreview(d.label.clone())))
                    }),
            )
            .when(menu_open, |d| {
                d.child(
                    deferred(anchored().offset(point(px(0.), px(26.))).snap_to_window_with_margin(px(8.)).child(self.render_block_menu(ix, &blk.kind, &t, cx)))
                        .with_priority(2),
                )
            });
        // 块类型图标（飞书同款）：悬停时与拖动手柄一起出现在左上角，点击打开块菜单
        let kind_badge = (ix > 0).then(|| {
            div().absolute().left(px(GUTTER + indent_px - 43. - if is_table { 16. } else { 0. })).top(px(handle_top.max(0.))).child(
                div()
                    .id(("kind-ic", blk.id as usize))
                    .size(px(22.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(!menu_open, |d| d.invisible().when(!self.sess.read_only, |d| d.group_hover("blk", |s| s.visible())))
                    .when(menu_open, |d| d.bg(t.hover))
                    .hover(|s| s.bg(t.hover))
                    .child({
                        let col: gpui::Hsla = gpui::rgb(kind_icon_color(&blk.kind)).into();
                        let col = if t.dark { col.blend(gpui::hsla(0., 0., 1., 0.25)) } else { col };
                        // 文字类块用字母标签（飞书同款）：正文 T、标题 H1…H6，其余用图标
                        match &blk.kind {
                            BlockKind::Heading { level } => div()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(col)
                                .child(format!("H{}", level.min(&6)))
                                .into_any_element(),
                            BlockKind::Text | BlockKind::Title => {
                                div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(col).child("T").into_any_element()
                            }
                            k => icon(kind_icon_name(k), 14., col).into_any_element(),
                        }
                    })
                    // 阅读模式：不能打开块菜单
                    .when(!self.sess.read_only, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.pop.block_menu = if this.pop.block_menu == Some(ix) { None } else { Some(ix) };
                            cx.notify();
                        }))
                    }),
            )
        });

        let n_hidden = if blk.collapsed { crate::editor::fold::subtree(&self.content.blocks, ix).len() } else { 0 };
        // 折叠三角：有子项的块悬停显示，收起时常显（排在类型图标左边）
        let fold = (ix > 0 && !self.sess.read_only && self.has_children(ix)).then(|| {
            let collapsed = blk.collapsed;
            div().absolute().left(px(GUTTER + indent_px - 64. - if is_table { 16. } else { 0. })).top(px(handle_top.max(0.))).child(
                div()
                    .id(("fold", blk.id as usize))
                    .size(px(22.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(!collapsed, |d| d.invisible().group_hover("blk", |s| s.visible()))
                    .hover(|s| s.bg(t.hover))
                    .tooltip(move |_, cx| {
                        cx.new(|_| Tooltip(if collapsed { format!("展开 {n_hidden} 项（⌘.）") } else { "折叠（⌘.）".to_string() }.into())).into()
                    })
                    .child(icon(if collapsed { "chevron-right" } else { "chevron-down" }, 14., if collapsed { t.accent } else { t.text_muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_fold_at(ix, cx);
                        }),
                    ),
            )
        });
        let drop_color = t.drop_line;
        // 同步块：左侧橙色竖条 + 淡底色，组首块显示标签
        let sync_info = blk.sync.as_ref().map(|id| {
            let first = ix == 0 || self.content.blocks[ix - 1].sync.as_ref() != Some(id);
            let last_in = self.content.blocks.get(ix + 1).is_none_or(|b| b.sync.as_ref() != Some(id));
            (first, last_in)
        });
        let sync_col = gpui::rgb(0xf5a339);
        // 评论：右侧边距的气泡，打开该块评论时整块淡黄高亮
        let n_open = self.open_comments_on(ix);
        let n_cm = if n_open > 0 { n_open } else { self.comments_on(ix) };
        let cm_active = self.sess.comment_open == Some(ix) || self.sess.comment_input.as_ref().is_some_and(|c| c.block == ix);
        let cm_badge = (n_cm > 0).then(|| {
            let color = if n_open > 0 { t.accent } else { t.text_muted };
            div()
                .id(("cm-badge", blk.id as usize))
                .absolute()
                .right(px(14.))
                .top(px(handle_top.max(0.)))
                .h(px(22.))
                .px(px(6.))
                .rounded(px(11.))
                .flex()
                .items_center()
                .gap(px(3.))
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(color)
                .when(n_open > 0, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(if n_open > 0 { format!("{n_open} 条待处理评论") } else { "评论均已解决".into() }.into())).into())
                .child(icon("comment", 13., color))
                .child(n_cm.to_string())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_comment_open(ix, cx);
                    }),
                )
        });
        let frame_blocks = self.frame.clone();
        let avail = self.avail_w.clone();
        let block_recorder = canvas(
            move |bounds, window, _| {
                frame_blocks.borrow_mut().blocks.insert(ix, bounds);
                // 记下正文实际宽度（窗口窄时比 content_w 小），图片据此限宽；变化时补一帧
                let w = f32::from(bounds.size.width) - GUTTER * 2.;
                if (avail.get() - w).abs() > 0.5 {
                    avail.set(w);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let row = div()
            .id(("block", blk.id as usize))
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .group("blk")
                    .relative()
                    .w_full()
                    .max_w(px(content_w() + GUTTER * 2.))
                    .pl(px(GUTTER + indent_px))
                    .pr(px(GUTTER))
                    .pt(px(pt))
                    .pb(px(pb))
                    .when(ix > 0, |d| d.child(handle))
                    .children(kind_badge)
                    .children(fold)
                    .child(block_recorder)
                    .when(cm_active, |d| {
                        d.child(div().absolute().left(px(GUTTER - 8.)).right(px(GUTTER - 8.)).top_0().bottom_0().rounded(px(6.)).bg(gpui::hsla(
                            0.12,
                            0.95,
                            0.55,
                            if t.dark { 0.12 } else { 0.16 },
                        )))
                    })
                    .children(cm_badge)
                    .when_some(sync_info, |d, (first, last_in)| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(GUTTER - 10.))
                                .right(px(GUTTER - 10.))
                                .top(px(if first { 0. } else { -1. }))
                                .bottom(px(if last_in { 0. } else { -1. }))
                                .bg(Hsla::from(sync_col))
                                .opacity(0.06)
                                .when(first, |d| d.rounded_t(px(6.)))
                                .when(last_in, |d| d.rounded_b(px(6.))),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(GUTTER - 10.))
                                .top(px(if first { 2. } else { 0. }))
                                .bottom(px(if last_in { 2. } else { 0. }))
                                .w(px(3.))
                                .rounded(px(2.))
                                .bg(sync_col),
                        )
                        .when(first, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .right(px(GUTTER - 6.))
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .bg(sync_col)
                                    .text_color(gpui::white())
                                    .text_size(px(10.5))
                                    .line_height(px(16.))
                                    .child(crate::i18n::tr("同步块")),
                            )
                        })
                    })
                    .child(
                        // 块对齐：0 左 / 1 中 / 2 右（文本按 text_align 继承，图片按 items 对齐）
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .text_align(match blk.align {
                                1 => gpui::TextAlign::Center,
                                2 => gpui::TextAlign::Right,
                                _ => gpui::TextAlign::Left,
                            })
                            // 文本块靠 text_align 让文字本身移动；图片块没有文字对齐概念，
                            // 靠 flex 主轴把图推到中间 / 右边（图片外框已是 w_full）。
                            .when(blk.align == 1 && is_image, |d| d.items_center())
                            .when(blk.align == 2 && is_image, |d| d.items_end())
                            .child(body),
                    ),
            )
            .drag_over::<DragBlock>(move |s, _, _, _| s.border_t_2().border_color(drop_color))
            .on_drop(cx.listener(move |this, d: &DragBlock, _, cx| this.move_block_to(d.ix, ix, cx)));

        if ix == last {
            // 文末留白：点击空白处 → 光标落到文末（必要时补一个空段落）
            div()
                .w_full()
                .child(row)
                .child(div().w_full().h(px(280.)).cursor(CursorStyle::IBeam).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus);
                        let last = this.content.blocks.len() - 1;
                        let b = &this.content.blocks[last];
                        if !(b.kind == BlockKind::Text && b.text.is_empty()) && last > 0 {
                            this.checkpoint(EditKind::Other);
                            this.insert_blocks(last + 1, vec![crate::model::Block::text("")]);
                            this.changed(cx);
                            this.set_cursor(Pos::new(last + 1, 0, 0), false, cx);
                        } else {
                            let p = this.slot_end(last);
                            this.set_cursor(p, false, cx);
                        }
                    }),
                ))
                .into_any_element()
        } else {
            row.into_any_element()
        }
    }
    /// 从 `render_block` 拆出。
    fn render_block_notes(&self, ix: usize, cx: &mut Context<Self>, blk: &std::sync::Arc<Block>, focused: bool, t: &Theme) -> AnyElement {
        let spec = SlotSpec { color: t.text_secondary, ..SlotSpec::body(t) };
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(2.))
            .px(px(14.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(t.border)
            .bg(t.code_bg.opacity(0.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(t.text_muted)
                    .child(icon("presentation", 12., t.text_muted))
                    .child(crate::i18n::tr("演讲者备注 · 演示时不显示")),
            )
            .child(div().w_full().child(self.render_slot(ix, 0, &blk.text, spec, focused, t)).on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)))
            .into_any_element()
    }

    /// 从 `render_block` 拆出。
    fn render_block_bullet(&self, ix: usize, cx: &mut Context<Self>, blk: &std::sync::Arc<Block>, focused: bool, number: usize, t: &Theme) -> AnyElement {
        let checked = matches!(blk.kind, BlockKind::Todo { checked: true });
        let marker: AnyElement = match &blk.kind {
            BlockKind::Bullet => div()
                .text_size(px(if blk.indent.is_multiple_of(3) { 20. } else { 15. }))
                .text_color(t.accent)
                .child(bullet_glyph(blk.indent))
                .into_any_element(),
            BlockKind::Ordered => div()
                .relative()
                .child(
                    div()
                        .id(("ol-num", blk.id as usize))
                        // 悬停底色往左右各扩 2px，负外边距抵消，编号位置与原来一致
                        .px(px(2.))
                        .ml(px(-2.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .text_size(px(body_size()))
                        .text_color(t.accent)
                        .child(ordered_label(number, blk.indent))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_ordered_menu(ix, cx);
                            }),
                        ),
                )
                .children(self.render_ordered_menu(ix, t, cx))
                .into_any_element(),
            _ => div()
                .id(("todo", blk.id as usize))
                .size(px(16.))
                .rounded(px(3.))
                .border_1()
                .border_color(if checked { t.accent } else { t.border_strong })
                .when(checked, |d| d.bg(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.border_color(t.accent))
                .when(checked, |d| d.child(icon("check", 12., gpui::white())))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_todo_at(ix, cx);
                    }),
                )
                .into_any_element(),
        };
        let spec = SlotSpec { color: if checked { t.text_muted } else { t.text }, strike: checked, ..SlotSpec::body(t) };
        div()
            .flex()
            .flex_row()
            .w_full()
            .child(div().w(px(INDENT_W)).h(px(body_lh())).flex_none().flex().items_center().justify_start().child(marker))
            .child(self.render_slot(ix, 0, &blk.text, spec, focused, t).flex_1().on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)))
            .into_any_element()
    }

    /// 从 `render_block` 拆出。
    fn render_block_callout(&self, ix: usize, cx: &mut Context<Self>, blk: &Arc<Block>, color: &u8, emoji: &String, focused: bool, t: &Theme) -> AnyElement {
        let ci = (*color & 7) as usize;
        let next_emoji = CALLOUT_EMOJIS.iter().position(|e| e == emoji).map(|i| CALLOUT_EMOJIS[(i + 1) % CALLOUT_EMOJIS.len()]).unwrap_or(CALLOUT_EMOJIS[0]);
        div()
            .flex()
            .flex_row()
            .w_full()
            .gap(px(10.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(8.))
            .bg(t.callout_bg[ci])
            .border_1()
            .border_color(t.callout_border[ci])
            .child(
                div()
                    .id(("emoji", blk.id as usize))
                    .flex_none()
                    .h(px(body_lh()))
                    .text_size(px(17.))
                    .cursor_pointer()
                    .rounded(px(4.))
                    .hover(|s| s.bg(t.hover))
                    .child(emoji.clone())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.set_callout(ix, Some(next_emoji), None, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(self.render_slot(ix, 0, &blk.text, SlotSpec::body(t), focused, t))
                    .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
            )
            .into_any_element()
    }

    fn render_code(
        &self,
        ix: usize,
        blk: &crate::model::Block,
        lang: &str,
        wrap: bool,
        height: Option<f32>,
        focused: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 关闭换行时的横向滚动句柄（按块保存）
        let scroll = (!wrap)
            .then(|| self.caches.code_scroll.borrow_mut().entry(blk.id).or_insert_with(|| (ScrollHandle::new(), crate::ui::ScrollDrag::default())).clone());
        let mut spec = SlotSpec {
            size: 13.5,
            line_h: 22.,
            mono: true,
            lang: Some(lang.to_string()),
            nowrap: !wrap,
            hscroll: scroll.as_ref().map(|(h, _)| h.clone()),
            ..SlotSpec::body(t)
        };
        let code = blk.text.text.clone();
        // 折叠：按语言算区间，再取用户收起的那几条，合并成要隐藏的模型字节区间
        let fold_spans = self.code_fold_spans(blk.id, lang, &blk.text.text);
        let folded: std::collections::HashSet<usize> = self.sess.code_folds.get(&blk.id).cloned().unwrap_or_default();
        if !fold_spans.is_empty() {
            let starts = crate::editor::codefold::line_starts(&blk.text.text);
            let eol = |k: usize| if k + 1 < starts.len() { starts[k + 1] - 1 } else { blk.text.text.len() };
            let mut hides: Vec<(usize, usize)> =
                fold_spans.iter().filter(|f| folded.contains(&f.start)).map(|f| (eol(f.start), eol(f.end))).filter(|(a, b)| b > a).collect();
            hides.sort_by_key(|r| r.0);
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (a, b) in hides {
                if let Some(last) = merged.last_mut().filter(|l| a <= l.1) {
                    last.1 = last.1.max(b);
                } else {
                    merged.push((a, b));
                }
            }
            spec.folds = (!merged.is_empty()).then(|| Rc::new(merged));
        }
        let lang_open = self.pop.lang_menu == Some(ix);
        let lang_label = lang.to_string();
        // 飞书同款：默认无边框；鼠标所在块、或正在编辑的块（且此时鼠标不在别的代码块上）显示强调色边框。
        // 这样同一时刻最多只有一个代码块带边框。
        let hovered = self.sess.hover_code == Some(blk.id);
        let editing = focused && self.core.head.block == ix;
        let bordered = hovered || (editing && self.sess.hover_code.is_none());
        let code_id = blk.id;
        let lang_query = if lang_open { self.pop.lang_search.as_ref().map(|s| s.input.read(cx).text()).unwrap_or_default() } else { String::new() };
        let lang_items: Vec<&'static str> = crate::highlight::filter_langs(&lang_query);
        let lang_sel = self.pop.lang_sel.min(lang_items.len().saturating_sub(1));
        let lang_input = self.pop.lang_search.as_ref().map(|s| s.input.clone());
        div()
            .id(("code-blk", blk.id as usize))
            .group("code")
            .w_full()
            .rounded(px(8.))
            .bg(t.code_bg)
            .border_1()
            .border_color(if bordered { t.accent } else { gpui::transparent_black() })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    if this.sess.hover_code != Some(code_id) {
                        this.sess.hover_code = Some(code_id);
                        cx.notify();
                    }
                } else if this.sess.hover_code == Some(code_id) {
                    this.sess.hover_code = None;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .pt(px(8.))
                    .child(
                        div()
                            .relative()
                            .child(
                                div()
                                    .id(("lang", blk.id as usize))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_size(px(12.))
                                    .text_color(t.text_muted)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.hover))
                                    .child(lang_label)
                                    .child(icon("chevron-down", 12., t.text_muted))
                                    .on_click(cx.listener(move |this, _, window, cx| this.open_lang_menu(ix, window, cx))),
                            )
                            .when(lang_open, |d| {
                                d.child(
                                    deferred(
                                        anchored().offset(point(px(0.), px(26.))).snap_to_window_with_margin(px(8.)).child(
                                            div()
                                                .id("lang-menu")
                                                .occlude()
                                                .w(px(220.))
                                                .flex()
                                                .flex_col()
                                                .p_1()
                                                .bg(t.panel_bg)
                                                .border_1()
                                                .border_color(t.border)
                                                .rounded(px(8.))
                                                .shadow_lg()
                                                .text_size(px(13.))
                                                // 搜索框：输入即过滤（↑↓ 选择，回车确认，Esc 取消）
                                                .child(
                                                    div()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_1()
                                                        .px_1()
                                                        .py(px(3.))
                                                        .child(icon("search", 13., t.text_muted))
                                                        .when_some(lang_input, |d, input| d.child(input)),
                                                )
                                                .child(div().h(px(1.)).mx_1().my_1().bg(t.border))
                                                .child(
                                                    div()
                                                        .id("lang-list")
                                                        .max_h(px(240.))
                                                        .overflow_y_scroll()
                                                        .when(lang_items.is_empty(), |d| {
                                                            d.px_2().py_2().text_color(t.text_muted).child(crate::i18n::tr("没有匹配的语言"))
                                                        })
                                                        .children(lang_items.iter().enumerate().map(|(i, l)| {
                                                            let l = *l;
                                                            div()
                                                                .id(("lang-opt", i))
                                                                .px_2()
                                                                .py(px(5.))
                                                                .rounded(px(4.))
                                                                .cursor_pointer()
                                                                .when(i == lang_sel, |d| d.bg(t.hover))
                                                                .hover(|s| s.bg(t.hover))
                                                                .child(l)
                                                                .on_click(cx.listener(move |this, _, window, cx| this.pick_code_lang(ix, l, window, cx)))
                                                        })),
                                                )
                                                .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_lang_menu(window, cx))),
                                        ),
                                    )
                                    .with_priority(2),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            // 自动换行开关：关闭换行后常显（否则长行被横向裁切，用户找不到入口）
                            .child(
                                div()
                                    .id(("wrap", blk.id as usize))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_size(px(12.))
                                    .text_color(t.text_muted)
                                    .cursor_pointer()
                                    .when(wrap, |d| d.invisible().group_hover("code", |s| s.visible()))
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("wrap", 12., t.text_muted))
                                    .child(if wrap { crate::i18n::tr("取消自动换行") } else { crate::i18n::tr("自动换行") })
                                    // 阅读模式不能改文档
                                    .when(!self.sess.read_only, |d| d.on_click(cx.listener(move |this, _, _, cx| this.set_code_wrap(ix, !wrap, cx)))),
                            )
                            .child(
                                div()
                                    .id(("copy", blk.id as usize))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_size(px(12.))
                                    .text_color(t.text_muted)
                                    .cursor_pointer()
                                    .invisible()
                                    .group_hover("code", |s| s.visible())
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("copy", 12., t.text_muted))
                                    .child(crate::i18n::tr("复制"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                                        cx.emit(EditorEvent::ToastSuccess("复制成功".into()));
                                    })),
                            ),
                    ),
            )
            .child({
                // 高度：拖过下边缘就固定（存在块里、随文档保存），否则自适应；固定后正文区纵向滚动。
                // 内容变短（改动文字、窗口变宽导致换行减少）后固定高度比内容还高时按内容自适应。
                let content_h = self.caches.code_body_h.borrow().get(&blk.id).copied();
                let wanted = self.sess.code_height_drag.as_ref().filter(|d| d.block == ix).map(|d| d.height).or(height);
                let body_h = wanted.and_then(|h| match content_h {
                    Some(c) if c > 1. && h >= c - 0.5 => None,
                    _ => Some(h.clamp(CODE_MIN_H, CODE_MAX_H)),
                });
                let (vh, vd) =
                    self.caches.code_vscroll.borrow_mut().entry(blk.id).or_insert_with(|| (ScrollHandle::new(), crate::ui::ScrollDrag::default())).clone();
                let recorder = self.caches.code_body_h.clone();
                let body_id = blk.id;
                let row = div()
                    .relative()
                    .flex()
                    .flex_row()
                    .w_full()
                    // 记下正文的「内容高度」（不受固定高度影响），拖动时作为上限
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                recorder.borrow_mut().insert(body_id, f32::from(bounds.size.height));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    // 行号栏：数字不参与选中，也不随横向滚动移动
                    .child(self.code_gutter(ix, &blk.text.text, spec.size, spec.line_h, &fold_spans, &folded, t, cx))
                    .child(if wrap {
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .pl(px(8.))
                            .pr(px(16.))
                            .pt(px(4.))
                            .pb(px(12.))
                            .child(self.render_slot(ix, 0, &blk.text, spec, focused, t))
                            .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx))
                            .into_any_element()
                    } else {
                        // 关闭换行：按内容固有宽度排版，放进横向滚动容器（尾部一行自定义滚动条）
                        let (handle, drag) = scroll.clone().unwrap();
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .id(("code-scroll", blk.id as usize))
                                    .flex()
                                    .flex_row()
                                    .overflow_x_scroll()
                                    .restrict_scroll_to_axis()
                                    .track_scroll(&handle)
                                    .pl(px(8.))
                                    .pr(px(16.))
                                    .pt(px(4.))
                                    .pb(px(12.))
                                    .child(
                                        self.render_slot(ix, 0, &blk.text, spec, focused, t)
                                            .flex_none()
                                            .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
                                    ),
                            )
                            .child(crate::ui::hscrollbar(handle, drag, t.text_muted))
                            .into_any_element()
                    });
                div()
                    .relative()
                    .w_full()
                    .child(
                        div().id(("code-body", blk.id as usize)).w_full().when_some(body_h, |d, h| d.h(px(h)).overflow_y_scroll().track_scroll(&vh)).child(row),
                    )
                    .when_some(body_h, |d, _| d.child(crate::ui::scrollbar(vh.clone(), vd.clone(), t.text_muted)))
                    .child(
                        div()
                            .id(("code-resize", blk.id as usize))
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .w_full()
                            .h(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::ResizeUpDown)
                            // 手柄常驻（只占下边缘 8px，可拖动），拖动条本身悬停才显形
                            .child(
                                div()
                                    .w(px(32.))
                                    .h(px(3.))
                                    .rounded(px(2.))
                                    .bg(gpui::transparent_black())
                                    .group_hover("code", |s| s.bg(t.text_muted.opacity(0.4))),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    let start = body_h.or_else(|| this.caches.code_body_h.borrow().get(&body_id).copied()).unwrap_or(120.);
                                    let max_h = this
                                        .caches
                                        .code_body_h
                                        .borrow()
                                        .get(&body_id)
                                        .copied()
                                        .unwrap_or(super::render::CODE_MAX_H)
                                        .max(super::render::CODE_MIN_H);
                                    this.sess.code_height_drag =
                                        Some(super::CodeHeightDrag { block: ix, start_h: start, start_y: ev.position.y, height: start, max_h });
                                    cx.notify();
                                }),
                            ),
                    )
            })
            .into_any_element()
    }

    /// 代码块可折叠区间（按 (块 id, 文本哈希, 语言) 缓存）。
    fn code_fold_spans(&self, id: u64, lang: &str, text: &str) -> Rc<Vec<crate::editor::codefold::FoldSpan>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        lang.hash(&mut h);
        let key = h.finish();
        if let Some((hash, l, spans)) = self.caches.fold_spans.borrow().get(&id) {
            if *hash == key && l == lang {
                return spans.clone();
            }
        }
        let spans = Rc::new(crate::editor::codefold::spans(lang, text));
        self.caches.fold_spans.borrow_mut().insert(id, (key, lang.to_string(), spans.clone()));
        spans
    }

    /// 代码块行号栏（飞书样式）：位置取自文本布局，自动换行时也对准每个逻辑行的首行。
    /// 可折叠的行在行号左侧画三角，点击收起 / 展开。
    #[allow(clippy::too_many_arguments)]
    fn code_gutter(
        &self,
        ix: usize,
        code: &str,
        size: f32,
        line_h: f32,
        folds: &[crate::editor::codefold::FoldSpan],
        folded: &std::collections::HashSet<usize>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line_count = code.split('\n').count().max(1);
        let digits = line_count.to_string().len();
        let fold_col = if folds.is_empty() { 0. } else { 13. };
        // Menlo 的等宽字宽约为 0.6023 em
        let width = size * 0.6023 * digits as f32 + 18. + fold_col;
        let code = code.to_string();
        let frame = self.frame.clone();
        let color = t.text_muted.opacity(0.85);
        let chevron_color = t.text_muted.opacity(0.9);
        let view = cx.entity().downgrade();
        // 被收起的行：折叠起始行之后到结束行（含）
        let mut hidden = vec![false; line_count];
        for f in folds {
            if folded.contains(&f.start) {
                for k in (f.start + 1)..=f.end.min(line_count.saturating_sub(1)) {
                    hidden[k] = true;
                }
            }
        }
        let folds = folds.to_vec();
        let folded = folded.clone();
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, cx| {
                let f = frame.borrow();
                let layout = f.layouts.get(&(ix, 0)).cloned();
                let map = f.maps.get(&(ix, 0)).cloned();
                drop(f);
                let Some(layout) = layout else { return };
                let text_len = layout.len();
                let m2d = |o: usize| map.as_ref().map_or(o, |m| m.m2d(o));
                let mut off = 0usize;
                let mut toggles: Vec<(usize, Bounds<Pixels>)> = Vec::new();
                for k in 0..line_count {
                    let start_off = off;
                    off += code_line_len(&code, start_off) + 1;
                    if hidden.get(k).copied().unwrap_or(false) {
                        continue;
                    }
                    let Some(y) = layout.position_for_index(m2d(start_off).min(text_len)).map(|p| p.y) else { continue };
                    let s = (k + 1).to_string();
                    let run = TextRun { len: s.len(), font: font(MONO), color, background_color: None, underline: None, strikethrough: None };
                    let line = window.text_system().shape_line(s.into(), px(size - 1.), &[run], None);
                    line.paint(point(bounds.right() - px(10.) - line.width, y), px(line_h), window, cx).ok();
                    if folds.iter().any(|f| f.start == k) {
                        let open = !folded.contains(&k);
                        let g = if open { "▾" } else { "▸" };
                        let run =
                            TextRun { len: g.len(), font: font(MONO), color: chevron_color, background_color: None, underline: None, strikethrough: None };
                        let gl = window.text_system().shape_line(g.into(), px(size - 3.5), &[run], None);
                        gl.paint(point(bounds.left() + px(2.), y), px(line_h), window, cx).ok();
                        toggles.push((k, Bounds::new(point(bounds.left(), y), gpui::size(px(13.), px(line_h)))));
                    }
                }
                let view = view.clone();
                window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                        return;
                    }
                    if let Some((line, _)) = toggles.iter().find(|(_, r)| r.contains(&ev.position)) {
                        cx.stop_propagation();
                        view.update(cx, |this, cx| this.toggle_code_fold(ix, *line, cx)).ok();
                        window.refresh();
                    }
                });
            },
        )
        .w(px(width))
        .flex_none()
        .into_any_element()
    }

    /// 图片块：选中后四角出现缩放手柄、上方浮出工具栏；双击查看大图；
    /// 裁剪是独立模式：显示完整原图 + 遮罩 + 可拖动的裁剪框。
    fn render_image(&self, ix: usize, path: &str, focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let blk = &self.content.blocks[ix];
        let cropping = self.sess.image_crop.as_ref().filter(|c| c.block == ix);
        let selected = (focused && self.block_selected(ix)) || cropping.is_some();
        let max_w = self.image_max_w(ix);
        let crop = match &blk.kind {
            BlockKind::Image { crop, .. } => *crop,
            _ => None,
        };
        // 拖动缩放时用预览宽度
        let drag = self.sess.image_drag.as_ref().filter(|d| d.block == ix);
        let natural = self.image_natural(ix);
        let (dw, dh) = match natural {
            Some((nw, nh)) => {
                let mut k = blk.kind.clone();
                if let (Some(d), BlockKind::Image { width, .. }) = (drag, &mut k) {
                    *width = Some(d.width);
                }
                k.image_box((nw as u32, nh as u32), max_w).unwrap_or((nw.min(max_w), nh.min(max_w)))
            }
            None => (max_w.min(360.), 80.),
        };
        let frame2 = self.frame.clone();
        let recorder = canvas(
            move |bounds, _, _| {
                frame2.borrow_mut().images.insert(ix, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let (box_w, box_h, content): (f32, f32, AnyElement) = match (natural, cropping) {
            // 裁剪模式：完整原图，比例尽量与当前显示一致，但不超过正文宽度
            (Some((nw, nh)), Some(ce)) => {
                let full_w = (dw / crop.map_or(1., |c| c.2)).min(max_w);
                let full_h = full_w * nh / nw;
                (full_w, full_h, self.render_crop_editor(ix, path, ce.rect, full_w, full_h, t, cx))
            }
            (Some(_), None) => {
                // 外层框是显示尺寸，内层图片按 1/crop 放大并偏移
                let inner_w = dw / crop.map_or(1., |c| c.2);
                let inner_h = dh / crop.map_or(1., |c| c.3);
                let dx = -inner_w * crop.map_or(0., |c| c.0);
                let dy = -inner_h * crop.map_or(0., |c| c.1);
                (
                    dw,
                    dh,
                    div()
                        .relative()
                        .w(px(dw))
                        .h(px(dh))
                        .rounded(px(4.))
                        .overflow_hidden()
                        .child(img(PathBuf::from(path)).absolute().left(px(dx)).top(px(dy)).w(px(inner_w)).h(px(inner_h)))
                        .into_any_element(),
                )
            }
            (None, _) => (
                dw,
                dh,
                div()
                    .w(px(dw))
                    .h(px(dh))
                    .rounded(px(4.))
                    .bg(t.code_bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(t.text_muted)
                    .text_size(px(13.))
                    .child(icon("image", 16., t.text_muted))
                    .child(crate::i18n::tr("图片加载中…"))
                    .into_any_element(),
            ),
        };

        // 四角缩放手柄：只在选中且不在裁剪时出现
        let center = blk.align == 1;
        let handles = (selected && cropping.is_none() && natural.is_some()).then(|| {
            let corner = |id: usize, left: bool, top: bool| {
                let dir = (if left { -1. } else { 1. }) * if center { 2. } else { 1. };
                div()
                    .id(("img-rs", ix * 4 + id))
                    .absolute()
                    .when(left, |d| d.left(px(-6.)))
                    .when(!left, |d| d.left(px(box_w - 6.)))
                    .when(top, |d| d.top(px(-6.)))
                    .when(!top, |d| d.top(px(box_h - 6.)))
                    .size(px(12.))
                    .rounded_full()
                    .bg(gpui::white())
                    .border_2()
                    .border_color(t.accent)
                    .shadow_sm()
                    .cursor(if left == top { CursorStyle::ResizeUpLeftDownRight } else { CursorStyle::ResizeUpRightDownLeft })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            let (w, _) = this.image_display_box(ix).unwrap_or((240., 240.));
                            this.sess.image_drag = Some(super::ImageDrag { block: ix, start_w: w, start_px: ev.position, width: w, dir });
                            cx.notify();
                        }),
                    )
            };
            vec![corner(0, true, true), corner(1, false, true), corner(2, true, false), corner(3, false, false)]
        });
        // 缩放时显示实时尺寸
        let size_tag = drag.map(|_| {
            div()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .px(px(7.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(gpui::black().opacity(0.62))
                .text_color(gpui::white())
                .text_size(px(11.5))
                .child(format!("{:.0} × {:.0}", dw, dh))
        });

        let toolbar = selected.then(|| match cropping {
            Some(_) => self.render_crop_toolbar(ix, t, cx),
            None => self.render_image_toolbar(ix, dw, t, cx),
        });
        let toolbar_left = blk.align != 2;

        div()
            .relative()
            .flex()
            .w_full()
            .when(blk.align == 1, |d| d.justify_center())
            .when(blk.align == 2, |d| d.justify_end())
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .id(("img-box", ix))
                            .relative()
                            .w(px(box_w))
                            .h(px(box_h))
                            .child(content)
                            .child(recorder)
                            // 选中描边画在图片外侧，不挤占图片尺寸
                            .when(selected && cropping.is_none(), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(-2.))
                                        .left(px(-2.))
                                        .w(px(box_w + 4.))
                                        .h(px(box_h + 4.))
                                        .rounded(px(6.))
                                        .border_2()
                                        .border_color(t.accent),
                                )
                            })
                            .children(handles.into_iter().flatten())
                            .children(size_tag)
                            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                                if ev.click_count() == 2 && this.sess.image_crop.is_none() {
                                    this.open_image_preview(ix, cx);
                                }
                            })),
                    )
                    .when(cropping.is_none(), |d| d.child(div().w(px(dw.max(160.))).child(self.render_image_caption(ix, path, selected, t, cx))))
                    .children(
                        toolbar.map(|tb| div().absolute().top(px(-46.)).when(toolbar_left, |d| d.left_0()).when(!toolbar_left, |d| d.right_0()).child(tb)),
                    ),
            )
            .child(self.void_marker(ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    if this.sess.image_crop.as_ref().is_some_and(|c| c.block == ix) {
                        return;
                    }
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                }),
            )
            .into_any_element()
    }

    /// 裁剪模式的图片：完整原图 + 框外遮罩 + 三分线 + 8 个拖柄。
    #[allow(clippy::too_many_arguments)]
    fn render_crop_editor(&self, ix: usize, path: &str, rect: (f32, f32, f32, f32), w: f32, h: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use super::CropHandle as H;
        let (rx, ry, rw, rh) = (rect.0 * w, rect.1 * h, rect.2 * w, rect.3 * h);
        let mask =
            |l: f32, tp: f32, ww: f32, hh: f32| div().absolute().left(px(l)).top(px(tp)).w(px(ww.max(0.))).h(px(hh.max(0.))).bg(gpui::black().opacity(0.55));
        let line = |vertical: bool, at: f32| {
            let d = div().absolute().bg(gpui::white().opacity(0.45));
            if vertical {
                d.left(px(rx + at)).top(px(ry)).w(px(1.)).h(px(rh))
            } else {
                d.top(px(ry + at)).left(px(rx)).h(px(1.)).w(px(rw))
            }
        };
        let handle = |id: usize, hd: H, cx_: f32, cy_: f32| {
            let corner = matches!(hd, H::NE | H::NW | H::SE | H::SW);
            let cursor = match hd {
                H::N | H::S => CursorStyle::ResizeUpDown,
                H::E | H::W => CursorStyle::ResizeLeftRight,
                H::NW | H::SE => CursorStyle::ResizeUpLeftDownRight,
                _ => CursorStyle::ResizeUpRightDownLeft,
            };
            let (vw, vh) = match hd {
                H::N | H::S => (22., 5.),
                H::E | H::W => (5., 22.),
                _ => (12., 12.),
            };
            div()
                .id(("crop-h", ix * 16 + id))
                .absolute()
                .left(px(cx_ - 12.))
                .top(px(cy_ - 12.))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .cursor(cursor)
                .child(
                    div()
                        .w(px(vw))
                        .h(px(vh))
                        .bg(gpui::white())
                        .when(corner, |d| d.rounded_full().border_2().border_color(t.accent))
                        .when(!corner, |d| d.rounded(px(3.)))
                        .shadow_sm(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.crop_begin_drag(hd, ev.position, cx);
                    }),
                )
        };
        div()
            .relative()
            .w(px(w))
            .h(px(h))
            .child(img(PathBuf::from(path)).absolute().left_0().top_0().w(px(w)).h(px(h)))
            .child(mask(0., 0., w, ry))
            .child(mask(0., ry + rh, w, h - ry - rh))
            .child(mask(0., ry, rx, rh))
            .child(mask(rx + rw, ry, w - rx - rw, rh))
            .child(line(true, rw / 3.))
            .child(line(true, rw * 2. / 3.))
            .child(line(false, rh / 3.))
            .child(line(false, rh * 2. / 3.))
            .child(
                div()
                    .id(("crop-move", ix))
                    .absolute()
                    .left(px(rx))
                    .top(px(ry))
                    .w(px(rw))
                    .h(px(rh))
                    .border_1()
                    .border_color(gpui::white())
                    .cursor(CursorStyle::OpenHand)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.crop_begin_drag(H::Move, ev.position, cx);
                        }),
                    ),
            )
            .child(handle(0, H::NW, rx, ry))
            .child(handle(1, H::N, rx + rw / 2., ry))
            .child(handle(2, H::NE, rx + rw, ry))
            .child(handle(3, H::E, rx + rw, ry + rh / 2.))
            .child(handle(4, H::SE, rx + rw, ry + rh))
            .child(handle(5, H::S, rx + rw / 2., ry + rh))
            .child(handle(6, H::SW, rx, ry + rh))
            .child(handle(7, H::W, rx, ry + rh / 2.))
            .into_any_element()
    }

    /// 裁剪模式工具栏：取消 / 还原 / 完成。
    fn render_crop_toolbar(&self, ix: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let _ = ix;
        let tbtn = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(12.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(13.))
                .when(primary, |d| d.bg(t.accent).text_color(gpui::white()).hover(|s| s.opacity(0.9)))
                .when(!primary, |d| d.text_color(t.text).hover(|s| s.bg(t.hover)))
                .child(crate::i18n::tr(label))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(8.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .occlude()
            .child(
                div()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(icon("crop", 14., t.text_muted))
                    .child(crate::i18n::tr("拖动边角调整 · ↵ 完成 · Esc 取消")),
            )
            .child(tbtn("crop-full", "还原", false).on_click(cx.listener(|this, _, _, cx| this.crop_reset_full(cx))))
            .child(tbtn("crop-cancel", "取消", false).on_click(cx.listener(|this, _, _, cx| this.crop_cancel(cx))))
            .child(tbtn("crop-done", "完成", true).on_click(cx.listener(|this, _, _, cx| this.crop_commit(cx))))
            .into_any_element()
    }

    /// 图片选中后的浮动工具栏：缩放 / 裁剪 / 对齐 / 查看大图 / 描述 / 评论 / 删除。
    fn render_image_toolbar(&self, ix: usize, dw: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let blk = &self.content.blocks[ix];
        let align = blk.align;
        let (has_crop, has_width) = match &blk.kind {
            BlockKind::Image { crop, width, .. } => (crop.is_some(), width.is_some()),
            _ => (false, false),
        };
        let btn = |id: (&'static str, usize), ic: &'static str, tip: &'static str, on: bool| {
            let color = if on { t.accent } else { t.text_secondary };
            div()
                .id(id)
                .size(px(28.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
                .child(icon(ic, 16., color))
        };
        let sep = || div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(t.border);
        let n_comment = self.comments_on(ix);
        div()
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
            .child(btn(("im-al", ix), "img-left", "左对齐", align == 0).on_click(cx.listener(move |this, _, _, cx| this.set_align(ix, 0, cx))))
            .child(btn(("im-ac", ix), "img-center", "居中", align == 1).on_click(cx.listener(move |this, _, _, cx| this.set_align(ix, 1, cx))))
            .child(btn(("im-ar", ix), "img-right", "右对齐", align == 2).on_click(cx.listener(move |this, _, _, cx| this.set_align(ix, 2, cx))))
            .child(sep())
            .child(btn(("im-small", ix), "minus", "缩小", false).on_click(cx.listener(move |this, _, _, cx| {
                let (w, _) = this.image_display_box(ix).unwrap_or((400., 400.));
                this.set_image_width(ix, Some(((w * 0.85).max(60.)).round()), cx);
            })))
            .child(div().min_w(px(34.)).text_center().text_size(px(11.5)).text_color(t.text_muted).child(format!("{dw:.0}")))
            .child(btn(("im-big", ix), "plus", "放大", false).on_click(cx.listener(move |this, _, _, cx| {
                let (w, _) = this.image_display_box(ix).unwrap_or((400., 400.));
                this.set_image_width(ix, Some(((w * 1.18).min(2400.)).round()), cx);
            })))
            .when(has_width, |d| {
                d.child(btn(("im-fit", ix), "size-reset", "恢复默认大小", false).on_click(cx.listener(move |this, _, _, cx| {
                    this.set_image_width(ix, None, cx);
                })))
            })
            .child(sep())
            .child(btn(("im-crop", ix), "crop", "裁剪", false).on_click(cx.listener(move |this, _, _, cx| this.start_crop(ix, cx))))
            .when(has_crop, |d| {
                d.child(btn(("im-reset", ix), "rotate", "撤销裁剪", false).on_click(cx.listener(move |this, _, _, cx| {
                    this.reset_image_crop(ix, cx);
                })))
            })
            .child(
                btn(("im-view", ix), "image-view", "查看大图（双击图片）", false).on_click(cx.listener(move |this, _, _, cx| this.open_image_preview(ix, cx))),
            )
            .child(btn(("im-alt", ix), "pencil", "编辑图片描述", false).on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.edit_image_alt(ix, ev.position(), window, cx);
            })))
            .child(sep())
            .child(
                div()
                    .id(("im-cm", ix))
                    .h(px(28.))
                    .px(px(6.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .text_color(if n_comment > 0 { t.accent } else { t.text_secondary })
                    .hover(|s| s.bg(t.hover))
                    .tooltip(move |_, cx| cx.new(|_| Tooltip(if n_comment > 0 { "查看评论" } else { "添加评论" }.into())).into())
                    .child(icon("comment", 16., if n_comment > 0 { t.accent } else { t.text_secondary }))
                    .when(n_comment > 0, |d| d.child(div().text_size(px(11.)).child(n_comment.to_string())))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            // 已有评论：打开列表（列表头部有「写评论」）；没有评论才直接开输入框。
                            if this.comments_on(ix) > 0 {
                                this.open_comment_panel(ix, cx);
                            } else {
                                this.start_comment(ix, ev.position, window, cx);
                            }
                        }),
                    ),
            )
            .child(btn(("im-del", ix), "trash", "删除图片", false).on_click(cx.listener(move |this, _, _, cx| this.delete_block_at(ix, cx))))
            .into_any_element()
    }

    fn render_find(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let f = self.find.as_ref()?;
        let total = f.matches.len();
        let label = if total == 0 { "无结果".to_string() } else { format!("{}/{}", f.current + 1, total) };
        let nav = |id: &'static str, ic: &'static str, d: i32, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 14., t.text_secondary))
                .on_click(cx.listener(move |this, _, _, cx| this.find_step(d, cx)))
        };
        Some(
            div()
                .absolute()
                .top(px(10.))
                .right(px(20.))
                .occlude()
                .w(px(340.))
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py(px(5.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .rounded(px(8.))
                .shadow_lg()
                .text_size(px(13.))
                .child(icon("search", 14., t.text_muted))
                .child(div().flex_1().line_height(px(22.)).child(f.input.clone()))
                .child(div().text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr_owned(&label)))
                .child(nav("find-prev", "chevron-up", -1, cx))
                .child(nav("find-next", "chevron-down", 1, cx))
                .child(
                    div()
                        .id("find-close")
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(icon("x", 14., t.text_secondary))
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                ),
        )
    }

    fn render_perf(&mut self, t: &Theme) -> impl IntoElement {
        let (med, p95, max) = self.perf_stats();
        let blocks = self.content.blocks.len();
        let chars = self.char_count();
        let good = t.fg_palette[4];
        let warn = t.fg_palette[2];
        let color = if p95 < 8.3 { good } else { warn };
        div()
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .occlude()
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .font_family(MONO)
            .text_size(px(11.))
            .text_color(t.text_secondary)
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().text_color(color).child(format!("● 帧 中位 {med:.2}ms")))
                    .child(format!("P95 {p95:.2}ms"))
                    .child(format!("最大 {max:.1}ms")),
            )
            .child(format!("{} 块 · {} 字 · 采样 {}", fmt_num(blocks), fmt_num(chars), self.caches.frame_times.len()))
    }

    fn render_link_edit(&self, at: Point<Pixels>, t: &Theme) -> Option<impl IntoElement> {
        let le = self.pop.link_edit.as_ref()?;
        Some(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .occlude()
                        .w(px(360.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(8.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(8.))
                        .shadow_lg()
                        .text_size(px(13.))
                        .child(icon("link", 14., t.text_muted))
                        .child(div().flex_1().line_height(px(22.)).child(le.input.clone()))
                        .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("↵ 确认 · 留空移除"))),
                ),
            )
            .with_priority(4),
        )
    }
}

/// 从 `off` 开始的一行在字节上的长度（不含换行符）。
fn code_line_len(code: &str, off: usize) -> usize {
    let rest = &code[off.min(code.len())..];
    rest.find('\n').unwrap_or(rest.len())
}

pub fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let start = Instant::now();
        self.caches.render_start = Some(start);
        self.poll_url_paste(cx);
        // 不换行空格的宽度只和字体有关，量一次就够
        if !self.caches.nbsp_measured {
            self.caches.nbsp_measured = true;
            let sz = 15.;
            let run =
                TextRun { len: "\u{00A0}".len() * 10, font: font(FONT), color: gpui::black(), background_color: None, underline: None, strikethrough: None };
            let w = f32::from(window.text_system().shape_line("\u{00A0}".repeat(10).into(), px(sz), &[run], None).width);
            if w > 0. {
                self.caches.nbsp_w.set(w / 10. / sz);
            }
        }
        crate::math::pump(cx);
        if self.sess.read_only {
            // 兜底：阅读模式下被回滚的操作可能把光标留在已撤回的位置
            self.core.anchor = self.clamp(self.core.anchor);
            self.core.head = self.clamp(self.core.head);
        }
        let t = theme(cx).clone();
        let focused = self.is_focused(window);
        // 光标只在聚焦且窗口在前台时闪烁（否则不再定时重绘）；截图回归时用 STONEBROOK_NOBLINK 让光标常亮
        self.blink_on = focused && window.is_window_active() && !no_blink();
        let cm_block = self.sess.comment_open.or(self.sess.comment_input.as_ref().map(|c| c.block));
        let (prev_cursor, prev_sel, cm_anchor) = {
            let mut f = self.frame.borrow_mut();
            let cm_anchor = cm_block.and_then(|b| f.blocks.get(&b).copied());
            let pc = f.cursor.take();
            let ps = f.sel_start.take();
            f.layouts.clear();
            f.voids.clear();
            f.diagrams.clear();
            f.html_bounds.clear();
            f.videos.clear();
            f.images.clear();
            f.blocks.clear();
            f.comment_input_bounds = None;
            f.comment_panel_bounds = None;
            f.maps.clear();
            (pc, ps, cm_anchor)
        };
        let entity = cx.entity();
        let focus = self.focus.clone();
        let frame = self.frame.clone();
        let overlay_follow =
            self.pop.slash.is_some() || self.pop.mention.is_some() || (self.has_selection() && !self.core.selecting) || self.pop.link_edit.is_some();
        // HTML 交互模式：本帧绘制后把原生 WebView 叠加到预览区域
        let menus_open = self.pop.slash.is_some()
            || self.pop.block_menu.is_some()
            || self.pop.lang_menu.is_some()
            || self.pop.ordered_menu.is_some()
            || self.pop.prop_edit.is_some()
            || self.pop.button_edit.is_some()
            || self.pop.countdown_edit.is_some()
            || self.pop.tl_menu.is_some()
            || self.find.is_some();
        let dark = t.dark;
        // 全屏文件预览优先：内容区叠加原生 WebView
        let full_live = self
            .sess
            .file_full
            .and_then(|id| self.content.blocks.iter().find(|b| b.id == id))
            .and_then(|b| super::attach::live_src(&b.kind, dark).map(|s| (b.id, s)));
        let live_html: Option<(u64, crate::webview::LiveSrc)> = full_live.or_else(|| {
            self.sess.html_live.and_then(|id| {
                self.content.blocks.iter().find(|b| b.id == id).and_then(|b| match &b.kind {
                    BlockKind::Html { file, network, .. } if !file.is_empty() => {
                        Some((id, crate::webview::LiveSrc::File { path: file.into(), network: *network }))
                    }
                    BlockKind::File { .. } => super::attach::live_src(&b.kind, dark).map(|s| (id, s)),
                    BlockKind::Html { scripts, .. } => {
                        Some((id, crate::webview::LiveSrc::Source { page: crate::webview::sandbox_html(&b.text.text, *scripts, dark), scripts: *scripts }))
                    }
                    _ => None,
                })
            })
        });
        if self.sess.file_full.is_some() && !self.content.blocks.iter().any(|b| Some(b.id) == self.sess.file_full) {
            self.sess.file_full = None;
        }
        if self.sess.html_live.is_some() && live_html.is_none() {
            self.sess.html_live = None;
            crate::webview::hide_live();
        }

        let list_el = list(self.list.clone(), cx.processor(|this, ix: usize, window, cx| this.render_block(ix, window, cx))).size_full();

        let mut root = div()
            .key_context("Editor")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(t.bg)
            .text_color(t.text)
            .font_family(FONT)
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::focus_block))
            // 阅读模式：只保留移动、选择、复制、查找；会改文档的动作不注册，按键直接落空
            .when(!self.sess.read_only, |d| {
                d.on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::delete_word_back))
                    .on_action(cx.listener(Self::delete_line_back))
                    .on_action(cx.listener(Self::enter))
                    .on_action(cx.listener(Self::soft_enter))
                    .on_action(cx.listener(Self::tab))
                    .on_action(cx.listener(Self::shift_tab))
                    .on_action(cx.listener(Self::cut))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::paste_plain))
                    .on_action(cx.listener(Self::undo))
                    .on_action(cx.listener(Self::redo))
                    .on_action(cx.listener(Self::bold))
                    .on_action(cx.listener(Self::italic))
                    .on_action(cx.listener(Self::underline))
                    .on_action(cx.listener(Self::strike))
                    .on_action(cx.listener(Self::inline_code))
                    .on_action(cx.listener(Self::insert_link))
                    .on_action(cx.listener(Self::set_text_kind))
                    .on_action(cx.listener(Self::set_h1))
                    .on_action(cx.listener(Self::set_h2))
                    .on_action(cx.listener(Self::set_h3))
                    .on_action(cx.listener(Self::set_h4))
                    .on_action(cx.listener(Self::set_bullet))
                    .on_action(cx.listener(Self::set_ordered))
                    .on_action(cx.listener(Self::set_todo))
                    .on_action(cx.listener(Self::set_quote))
                    .on_action(cx.listener(Self::set_code))
                    .on_action(cx.listener(Self::toggle_todo))
                    .on_action(cx.listener(Self::duplicate_block))
                    .on_action(cx.listener(Self::move_block_up))
                    .on_action(cx.listener(Self::move_block_down))
                    .on_action(cx.listener(Self::insert_image))
                    .on_action(cx.listener(Self::insert_timestamp_action))
                    .on_action(cx.listener(Self::toggle_fold))
            })
            .on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| window.show_character_palette()))
            .on_mouse_move(cx.listener(Self::root_mouse_move))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // 点击交互中的 HTML 以外的区域：退出交互，把键盘焦点还给编辑器
                    if this.sess.file_full.is_some() {
                        return;
                    }
                    let Some(id) = this.sess.html_live else { return };
                    let inside = this.frame.borrow().html_bounds.get(&id).map(|b| b.contains(&ev.position));
                    if inside == Some(false) {
                        this.sess.html_live = None;
                        crate::webview::hide_live();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(Self::root_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::root_mouse_up))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.insert_files(paths.paths().to_vec(), cx)))
            .child(list_el)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        if focused {
                            window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                        }
                        // 浮层依赖上一帧的光标坐标；坐标变化时再刷新一帧
                        let f = frame.borrow();
                        let mut redraw = overlay_follow && (f.cursor != prev_cursor || f.sel_start != prev_sel);
                        // 评论浮层跟随被评论块（滚动时块位置会变）
                        redraw |= cm_block.is_some_and(|b| f.blocks.get(&b).copied() != cm_anchor);
                        match (&live_html, live_html.as_ref().and_then(|(id, _)| f.html_bounds.get(id))) {
                            (Some((id, src)), Some(b)) if !menus_open => {
                                crate::webview::show_live(window, *id, src, *b);
                            }
                            _ => crate::webview::hide_live_unless_external(),
                        }
                        let visible: std::collections::HashSet<u64> = f.videos.keys().copied().collect();
                        crate::media::pause_except(&visible);
                        drop(f);
                        let ms = start.elapsed().as_secs_f32() * 1000.;
                        let again = entity.update(cx, |e, _| {
                            if e.caches.frame_times.len() >= 240 {
                                e.caches.frame_times.pop_front();
                            }
                            e.caches.frame_times.push_back(ms);
                            e.finish_reveal()
                        });
                        redraw |= again;
                        if redraw {
                            // 绘制阶段内直接 refresh 会被本帧收尾清掉，改为在下一帧开始时请求重绘
                            window.on_next_frame(|window, _| window.refresh());
                        }
                    },
                )
                .absolute()
                .size_full(),
            );

        if let Some(c) = prev_cursor.filter(|_| self.pop.mention.is_some()) {
            root = root.child(self.render_mention(point(c.left() - px(4.), c.bottom() + px(6.)), &t, cx));
        }
        if let Some(c) = prev_cursor.filter(|_| self.pop.slash.is_some()) {
            let at = point(c.left() - px(4.), c.bottom() + px(6.));
            self.slash_at = at;
            root = root.child(self.render_slash(at, c, &t, cx));
        }
        if self.has_selection() && !self.core.selecting && focused && self.pop.link_edit.is_none() && !self.sess.read_only {
            if let Some(p) = prev_sel {
                if !self.selected_slots().is_empty() {
                    root = root.child(self.render_toolbar(point(p.x - px(8.), p.y - px(46.)), &t, cx));
                }
            }
        } else if self.pop.tb_menu.is_some() && !self.has_selection() {
            self.pop.tb_menu = None;
        }
        if let Some(p) = prev_sel.or(prev_cursor.map(|c| c.origin)) {
            if let Some(le) = self.render_link_edit(point(p.x, p.y + px(30.)), &t) {
                root = root.child(le);
            }
        }
        if let Some(fb) = self.render_find(&t, cx) {
            root = root.child(fb);
        }
        if let Some(pe) = self.render_prop_edit(&t) {
            root = root.child(pe);
        }
        if let Some(cd) = self.render_countdown_dialog(&t, window, cx) {
            root = root.child(cd);
        }
        if let Some(cp) = self.render_comment_popover(cm_anchor, &t, cx) {
            root = root.child(cp);
        }
        if let Some(tp) = self.render_table_picker(window, &t, cx) {
            root = root.child(tp);
        }
        if let Some(c) = prev_cursor {
            if let Some(up) = self.render_url_paste(point(c.left() + px(6.), c.bottom() + px(6.)), &t, cx) {
                root = root.child(up);
            }
        }
        if self.pop.sync_menu {
            let at = prev_cursor.map(|c| point(c.left() - px(4.), c.bottom() + px(6.))).unwrap_or(point(px(300.), px(200.)));
            root = root.child(self.render_sync_menu(at, &t, cx));
        }
        if let Some(pv) = self.render_image_preview(window, cx) {
            root = root.child(pv);
        }
        if let Some(pv) = self.render_diagram_preview(window, &t, cx) {
            root = root.child(pv);
        }
        if let Some(vf) = self.render_video_full(&t, window, cx) {
            root = root.child(vf);
        }
        if let Some(ff) = self.render_file_full(&t, window, cx) {
            root = root.child(ff);
        }
        if self.show_perf {
            root = root.child(self.render_perf(&t));
        }
        root
    }
}

/// 截图回归（tools/harness）时关闭光标闪烁，同一脚本两次截图逐像素一致。
fn no_blink() -> bool {
    static NO_BLINK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_BLINK.get_or_init(|| std::env::var_os("STONEBROOK_NOBLINK").is_some())
}
