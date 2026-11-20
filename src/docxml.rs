//! 飞书 / Lark DocxXML 的导入与导出（格式参考 quiet.md 的 docs/FORMAT.md 与 all-blocks.xml）。
//!
//! DocxXML 是一串顶层元素（`<title>`、`<p>`、`<h1>`…`<h9>`、`<ul>/<ol>/<li>`、`<checkbox>`、`<blockquote>`、
//! `<callout>`、`<pre><code>`、`<hr/>`、`<grid>/<column>`、`<table>`、`<img>`、`<bookmark>`…），可以互相嵌套。
//! 石渠的文档是扁平的块序列，嵌套用 `indent` 表示：
//!
//! - 导入：段落 / 列表项 / 待办里嵌套的块，缩进 +1；引用、高亮块、分栏、表格单元格里的多个块合成多行文本；
//! - 导出：按缩进还原嵌套，连续的同级列表项放进同一个 `<ul>` / `<ol>`；石渠独有的块降级为可读的段落。

use crate::model::*;
use std::fmt::Write;

// ───────────────────────────── 颜色 ─────────────────────────────

/// 文字色 / 背景色名，顺序与 `theme::PALETTE_NAMES` 的 1..7 对应。
const COLOR_NAMES: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

fn color_index(name: &str) -> Option<u8> {
    COLOR_NAMES.iter().position(|c| *c == name).map(|i| i as u8 + 1)
}

/// 文字色：`red` → 1。
fn text_color(name: &str) -> u8 {
    color_index(name.trim_start_matches("light-")).unwrap_or(0)
}

/// 行内背景色：`light-red` → 1（浅色），`red` → 9（深色，见 `Theme::text_bg`）。
fn background(name: &str) -> u8 {
    match name.strip_prefix("light-") {
        Some(n) => color_index(n).unwrap_or(0),
        None => color_index(name).map(|i| i + 8).unwrap_or(0),
    }
}

fn color_name(i: u8) -> Option<&'static str> {
    COLOR_NAMES.get((i as usize).checked_sub(1)?).copied()
}

fn background_name(i: u8) -> Option<String> {
    if i >= 8 {
        color_name(i - 8).map(str::to_string)
    } else {
        color_name(i).map(|n| format!("light-{n}"))
    }
}

// ───────────────────────────── 导入 ─────────────────────────────

type Node<'a, 'i> = roxmltree::Node<'a, 'i>;

/// 块级元素：出现在段落 / 列表项里时是它的子块，而不是行内内容。
fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "ul"
            | "ol"
            | "li"
            | "checkbox"
            | "blockquote"
            | "callout"
            | "pre"
            | "hr"
            | "grid"
            | "column"
            | "table"
            | "img"
            | "bookmark"
            | "figure"
            | "whiteboard"
            | "sheet"
            | "title"
    ) || heading_level(tag).is_some()
}

fn heading_level(tag: &str) -> Option<u8> {
    let n: u8 = tag.strip_prefix('h')?.parse().ok()?;
    (1..=9).contains(&n).then_some(n.min(6))
}

/// 读入 DocxXML。第一块总是标题（文件里没有 `<title>` 时为空标题）。
pub fn import(xml: &str) -> Result<Vec<Block>, String> {
    let body = xml.trim_start_matches('\u{feff}').trim();
    let body = match body.strip_prefix("<?xml") {
        Some(rest) => rest.split_once("?>").map(|(_, r)| r).unwrap_or(rest),
        None => body,
    };
    // 顶层是一串元素，没有根：包一层再解析
    let wrapped = format!("<docxml>{body}</docxml>");
    let doc = roxmltree::Document::parse(&wrapped).map_err(|e| format!("不是有效的 DocxXML：{e}"))?;
    let mut out = Vec::new();
    for n in doc.root_element().children().filter(Node::is_element) {
        if n.tag_name().name() == "title" {
            out.insert(0, Block::new(BlockKind::Title, inline_of(n)));
        } else {
            block(n, 0, &mut out);
        }
    }
    if out.first().is_none_or(|b| b.kind != BlockKind::Title) {
        out.insert(0, Block::new(BlockKind::Title, RichText::default()));
    }
    Ok(out)
}

fn align_of(n: Node) -> u8 {
    match n.attribute("align") {
        Some("center") => 1,
        Some("right") => 2,
        _ => 0,
    }
}

fn push(out: &mut Vec<Block>, kind: BlockKind, text: RichText, indent: u8, align: u8) {
    let mut b = Block::new(kind, text);
    b.indent = indent.min(8);
    b.align = align;
    out.push(b);
}

/// 子元素里的块（跳过行内内容），缩进为 `indent`。
fn nested(n: Node, indent: u8, out: &mut Vec<Block>) {
    for c in n.children().filter(|c| c.is_element() && is_block_tag(c.tag_name().name())) {
        block(c, indent, out);
    }
}

fn block(n: Node, indent: u8, out: &mut Vec<Block>) {
    let tag = n.tag_name().name();
    match tag {
        "p" => {
            push(out, BlockKind::Text, inline_of(n), indent, align_of(n));
            nested(n, indent + 1, out);
        }
        "ul" | "ol" => {
            for li in n.children().filter(|c| c.is_element() && c.tag_name().name() == "li") {
                let kind = if tag == "ol" { BlockKind::Ordered } else { BlockKind::Bullet };
                push(out, kind, inline_of(li), indent, align_of(li));
                if tag == "ol" {
                    out.last_mut().unwrap().num = li.attribute("seq").and_then(|s| s.parse().ok());
                }
                // 空列表项后面的 <p></p> 占位（见 FORMAT.md）不算子块
                let mut kids = li.children().filter(|c| c.is_element() && is_block_tag(c.tag_name().name())).peekable();
                if out.last().is_some_and(|b| b.text.is_empty())
                    && kids.peek().is_some_and(|c| c.tag_name().name() == "p" && c.text().is_none() && !c.has_children())
                {
                    kids.next();
                }
                for c in kids {
                    block(c, indent + 1, out);
                }
            }
        }
        "li" => {
            push(out, BlockKind::Bullet, inline_of(n), indent, align_of(n));
            nested(n, indent + 1, out);
        }
        "checkbox" => {
            push(out, BlockKind::Todo { checked: n.attribute("done") == Some("true") }, inline_of(n), indent, align_of(n));
            nested(n, indent + 1, out);
        }
        "blockquote" => push(out, BlockKind::Quote, lines_of(n), indent, 0),
        "callout" => {
            let emoji = n.attribute("emoji").filter(|e| !e.is_empty()).unwrap_or("💡").to_string();
            let color = n.attribute("background-color").map(text_color).unwrap_or(0);
            push(out, BlockKind::Callout { emoji, color }, lines_of(n), indent, 0);
        }
        "pre" => {
            let lang = n.attribute("lang").filter(|l| *l != "plaintext").unwrap_or("").to_string();
            let code: String = n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect();
            let kind = match lang.as_str() {
                "mermaid" => BlockKind::Diagram { view: 0, offsets: Default::default() },
                _ => BlockKind::Code { lang, wrap: true, height: None },
            };
            push(out, kind, RichText::plain(code), indent, 0);
        }
        "hr" => push(out, BlockKind::Divider, RichText::default(), indent, 0),
        "grid" => {
            let cols: Vec<RichText> = n.children().filter(|c| c.is_element() && c.tag_name().name() == "column").map(lines_of).collect();
            if !cols.is_empty() {
                push(out, BlockKind::Columns { cols }, RichText::default(), indent, 0);
            }
        }
        "table" => {
            if let Some(b) = table(n) {
                out.push(b);
            }
        }
        "img" => match (n.attribute("path"), n.attribute("href")) {
            (Some(path), _) => {
                let mut b = Block::new(BlockKind::image(path), RichText::default());
                if let BlockKind::Image { alt, width, .. } = &mut b.kind {
                    *alt = n.attribute("caption").unwrap_or_default().to_string();
                    *width = n.attribute("width").and_then(|w| w.parse().ok());
                }
                b.align = align_of(n);
                out.push(b);
            }
            // 云端图片（只有链接）：保留为一个可点的链接
            (None, Some(href)) => {
                push(out, BlockKind::Text, RichText::styled_link(n.attribute("caption").filter(|c| !c.is_empty()).unwrap_or("图片"), href), indent, 0)
            }
            _ => {}
        },
        "bookmark" => {
            if let Some(href) = n.attribute("href") {
                push(out, BlockKind::LinkCard { url: href.to_string() }, RichText::default(), indent, 0);
            }
        }
        "figure" => {
            if let Some(src) = n.children().find(|c| c.tag_name().name() == "source") {
                if let Some(path) = src.attribute("path") {
                    let name = src.attribute("name").map(str::to_string).unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
                    push(out, BlockKind::File { path: path.to_string(), name, size: 0, view: FILE_VIEW_CARD }, RichText::default(), indent, 0);
                }
            }
        }
        _ => {
            if let Some(level) = heading_level(tag) {
                push(out, BlockKind::Heading { level }, inline_of(n), 0, align_of(n));
                nested(n, indent + 1, out);
                return;
            }
            // 不认识的元素（白板、电子表格、@人……）：有文字就留下文字，否则跳过
            let t = lines_of(n);
            if !t.is_empty() {
                push(out, BlockKind::Text, t, indent, 0);
            }
        }
    }
}

/// 容器里的全部内容合成一段多行文本（引用、高亮块、分栏、单元格）。
fn lines_of(n: Node) -> RichText {
    let mut out = RichText::default();
    let mut first = true;
    let mut add = |t: RichText, out: &mut RichText| {
        if !first {
            out.insert(out.len(), "\n", Marks::default());
        }
        first = false;
        out.append(&t);
    };
    let has_blocks = n.children().any(|c| c.is_element() && is_block_tag(c.tag_name().name()));
    if !has_blocks {
        return inline_of(n);
    }
    let mut blocks = Vec::new();
    nested(n, 0, &mut blocks);
    for b in blocks {
        let prefix = match b.kind {
            BlockKind::Bullet => "• ",
            BlockKind::Todo { checked: true } => "☑ ",
            BlockKind::Todo { .. } => "☐ ",
            _ => "",
        };
        let mut t = RichText::plain(format!("{}{prefix}", "  ".repeat(b.indent as usize)));
        t.append(&b.text);
        if !matches!(b.kind, BlockKind::Divider) {
            add(t, &mut out);
        }
    }
    out
}

/// 只取行内内容（文字、样式、链接、公式、换行），跳过嵌套的块。
fn inline_of(n: Node) -> RichText {
    let mut out = RichText::default();
    inline(n, &Marks::default(), &mut out);
    out
}

fn inline(n: Node, marks: &Marks, out: &mut RichText) {
    for c in n.children() {
        if c.is_text() {
            let t = c.text().unwrap_or_default();
            // 源码排版产生的空白（换行 + 缩进）不是内容
            if t.contains('\n') && t.trim().is_empty() {
                continue;
            }
            let t = if c.next_sibling().is_some_and(|s| s.is_element() && is_block_tag(s.tag_name().name())) { t.trim_end() } else { t };
            out.insert(out.len(), t, marks.clone());
            continue;
        }
        if !c.is_element() {
            continue;
        }
        let tag = c.tag_name().name();
        if is_block_tag(tag) {
            continue;
        }
        let mut m = marks.clone();
        match tag {
            "b" | "strong" => m.bold = true,
            "em" | "i" => m.italic = true,
            "u" => m.underline = true,
            "del" | "s" | "strike" => m.strike = true,
            "code" => m.code = true,
            "a" => m.link = c.attribute("href").map(str::to_string),
            "span" => {
                if let Some(v) = c.attribute("text-color") {
                    m.color = text_color(v);
                }
                if let Some(v) = c.attribute("background-color") {
                    m.bg = background(v);
                }
            }
            "br" => {
                out.insert(out.len(), "\n", marks.clone());
                continue;
            }
            "latex" | "equation" => {
                let tex: String = c.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect();
                out.insert_math(out.len(), tex.trim());
                continue;
            }
            // @人、日期提醒等云端对象：本地没有对应物，保留其中的文字
            _ => {}
        }
        inline(c, &m, out);
    }
}

fn row(taken: &mut Vec<Vec<bool>>, r: usize) -> &mut Vec<bool> {
    if taken.len() <= r {
        taken.resize(r + 1, vec![]);
    }
    &mut taken[r]
}

fn table(n: Node) -> Option<Block> {
    let rows: Vec<Node> = n.descendants().filter(|d| d.tag_name().name() == "tr").collect();
    if rows.is_empty() {
        return None;
    }
    // 先按 colspan / rowspan 把每个单元格落到网格上
    let mut placed: Vec<(usize, usize, usize, usize, RichText, bool)> = vec![];
    let mut taken: Vec<Vec<bool>> = vec![];
    let mut cols = 0;
    for (r, tr) in rows.iter().enumerate() {
        let mut c = 0;
        for td in tr.children().filter(|x| matches!(x.tag_name().name(), "td" | "th")) {
            while row(&mut taken, r).get(c).copied().unwrap_or(false) {
                c += 1;
            }
            let cs = td.attribute("colspan").and_then(|v| v.parse().ok()).unwrap_or(1usize).clamp(1, 64);
            let rs = td.attribute("rowspan").and_then(|v| v.parse().ok()).unwrap_or(1usize).clamp(1, 256);
            for rr in r..r + rs {
                let line = row(&mut taken, rr);
                if line.len() < c + cs {
                    line.resize(c + cs, false);
                }
                line[c..c + cs].iter_mut().for_each(|x| *x = true);
            }
            let mut text = lines_of(td);
            text.valign = match td.attribute("vertical-align") {
                Some("middle") => 1,
                Some("bottom") => 2,
                _ => 0,
            };
            placed.push((r, c, rs, cs, text, td.tag_name().name() == "th"));
            c += cs;
            cols = cols.max(c);
        }
    }
    let nrows = taken.len().max(rows.len());
    let cols = cols.max(1);
    let mut cells = vec![RichText::default(); nrows * cols];
    let mut merges = vec![];
    for (r, c, rs, cs, text, _) in &placed {
        cells[r * cols + c] = text.clone();
        if *rs > 1 || *cs > 1 {
            merges.push(CellMerge { row: *r, col: *c, row_span: (*rs).min(nrows - r), col_span: (*cs).min(cols - c) });
        }
    }
    let header_row = n.children().any(|c| c.tag_name().name() == "thead") || placed.iter().filter(|p| p.0 == 0).all(|p| p.5);
    let widths: Vec<f32> = n.descendants().filter(|d| d.tag_name().name() == "col").filter_map(|c| c.attribute("width")?.parse().ok()).collect();
    let widths = (widths.len() == cols && widths.iter().all(|w: &f32| *w > 0.)).then(|| {
        let mean = widths.iter().sum::<f32>() / cols as f32;
        widths.iter().map(|w| w / mean).collect()
    });
    let mut b = Block::new(BlockKind::Table { rows: nrows, cols, cells, header_row, header_col: false, widths, merges }, RichText::default());
    b.fixup();
    Some(b)
}

// ───────────────────────────── 导出 ─────────────────────────────

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// 行内内容：样式区间 → `<b>` / `<em>` / `<u>` / `<del>` / `<code>` / `<span>` / `<a>` / `<latex>`，换行写成 `<br/>`。
fn inline_xml(t: &RichText) -> String {
    let mut out = String::new();
    for (range, m) in t.runs() {
        if let Some(tex) = &m.math {
            let _ = write!(out, "<latex>{}</latex>", esc(tex));
            continue;
        }
        let mut s = esc(&t.text[range]).replace('\n', "<br/>");
        if m.code {
            s = format!("<code>{s}</code>");
        }
        if m.bold {
            s = format!("<b>{s}</b>");
        }
        if m.italic {
            s = format!("<em>{s}</em>");
        }
        if m.underline {
            s = format!("<u>{s}</u>");
        }
        if m.strike {
            s = format!("<del>{s}</del>");
        }
        let fg = color_name(m.color);
        let bg = background_name(m.bg);
        if fg.is_some() || bg.is_some() {
            let mut attrs = String::new();
            if let Some(c) = fg {
                let _ = write!(attrs, " text-color=\"{c}\"");
            }
            if let Some(c) = bg {
                let _ = write!(attrs, " background-color=\"{c}\"");
            }
            s = format!("<span{attrs}>{s}</span>");
        }
        if let Some(href) = &m.link {
            s = format!("<a href=\"{}\">{s}</a>", esc(href));
        }
        out.push_str(&s);
    }
    out
}

/// 多行文本写成多个 `<p>`（引用、高亮块、分栏、单元格里用）。
fn paragraphs(t: &RichText, pad: &str, out: &mut String) {
    if t.is_empty() {
        let _ = writeln!(out, "{pad}<p></p>");
        return;
    }
    let mut start = 0;
    for (i, _) in t.text.match_indices('\n').chain(std::iter::once((t.len(), ""))) {
        let _ = writeln!(out, "{pad}<p>{}</p>", inline_xml(&t.slice(start..i)));
        start = i + 1;
    }
}

fn align_attr(a: u8) -> &'static str {
    match a {
        1 => " align=\"center\"",
        2 => " align=\"right\"",
        _ => "",
    }
}

/// 导出整篇文档。
pub fn export<B: std::borrow::Borrow<Block>>(blocks: &[B]) -> String {
    let mut out = String::new();
    let blocks: Vec<&Block> = blocks.iter().map(|b| b.borrow()).collect();
    let title = blocks.first().copied().filter(|b| b.kind == BlockKind::Title);
    let _ = writeln!(out, "<title>{}</title>", title.map(|b| inline_xml(&b.text)).unwrap_or_default());
    let rest: Vec<&Block> = blocks.iter().skip(usize::from(title.is_some())).copied().collect();
    emit(&rest, 0, &mut out);
    out
}

/// 能装子块的元素：段落、列表项、待办、标题。其余块后面更深缩进的块按同级输出。
fn nests(k: &BlockKind) -> bool {
    matches!(k, BlockKind::Text | BlockKind::Bullet | BlockKind::Ordered | BlockKind::Todo { .. } | BlockKind::Heading { .. })
}

/// 输出一段块（第一块的缩进为这一层）；`depth` 为 XML 嵌套层数（只影响排版空格）。
fn emit(blocks: &[&Block], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let mut i = 0;
    while i < blocks.len() {
        let b = blocks[i];
        // 子块：紧跟其后、缩进更深的块
        let end = |from: usize| {
            let mut j = from + 1;
            while j < blocks.len() && blocks[j].indent > blocks[from].indent {
                j += 1;
            }
            j
        };
        let j = end(i);
        let kids = &blocks[i + 1..j];
        if matches!(b.kind, BlockKind::Bullet | BlockKind::Ordered) {
            // 连续的同级同类列表项放进同一个 <ul> / <ol>
            let tag = if b.kind == BlockKind::Ordered { "ol" } else { "ul" };
            let _ = writeln!(out, "{pad}<{tag}>");
            let mut k = i;
            while k < blocks.len() && blocks[k].kind == b.kind && blocks[k].indent == b.indent {
                let e = end(k);
                let it = blocks[k];
                let seq = it.num.filter(|_| tag == "ol").map(|n| format!(" seq=\"{n}\"")).unwrap_or_default();
                let sub = &blocks[k + 1..e];
                let body = if it.text.is_empty() && !sub.is_empty() { "<p></p>".to_string() } else { inline_xml(&it.text) };
                if sub.is_empty() {
                    let _ = writeln!(out, "{pad}  <li{seq}{}>{body}</li>", align_attr(it.align));
                } else {
                    let _ = writeln!(out, "{pad}  <li{seq}{}>{body}", align_attr(it.align));
                    emit(sub, depth + 2, out);
                    let _ = writeln!(out, "{pad}  </li>");
                }
                k = e;
            }
            let _ = writeln!(out, "{pad}</{tag}>");
            i = k;
            continue;
        }
        let open_close = |tag: &str, attrs: &str, out: &mut String| {
            if kids.is_empty() || !nests(&b.kind) {
                let _ = writeln!(out, "{pad}<{tag}{attrs}>{}</{tag}>", inline_xml(&b.text));
            } else {
                let _ = writeln!(out, "{pad}<{tag}{attrs}>{}", inline_xml(&b.text));
                emit(kids, depth + 1, out);
                let _ = writeln!(out, "{pad}</{tag}>");
            }
        };
        match &b.kind {
            BlockKind::Title | BlockKind::Text => open_close("p", align_attr(b.align), out),
            BlockKind::Heading { level } => open_close(&format!("h{level}"), align_attr(b.align), out),
            BlockKind::Todo { checked } => open_close("checkbox", &format!(" done=\"{checked}\""), out),
            BlockKind::Quote | BlockKind::Notes => {
                let _ = writeln!(out, "{pad}<blockquote>");
                paragraphs(&b.text, &format!("{pad}  "), out);
                let _ = writeln!(out, "{pad}</blockquote>");
            }
            BlockKind::Callout { emoji, color } => {
                let name = color_name(*color).unwrap_or("blue");
                let _ = writeln!(out, "{pad}<callout emoji=\"{}\" background-color=\"light-{name}\" border-color=\"{name}\">", esc(emoji));
                paragraphs(&b.text, &format!("{pad}  "), out);
                let _ = writeln!(out, "{pad}</callout>");
            }
            BlockKind::Code { lang, .. } => {
                let lang = if lang.is_empty() { "plaintext" } else { lang };
                let _ = writeln!(out, "{pad}<pre lang=\"{}\"><code>{}</code></pre>", esc(lang), esc(&b.text.text));
            }
            BlockKind::Diagram { .. } => {
                let _ = writeln!(out, "{pad}<pre lang=\"mermaid\"><code>{}</code></pre>", esc(&b.text.text));
            }
            BlockKind::Html { .. } => {
                let _ = writeln!(out, "{pad}<pre lang=\"html\"><code>{}</code></pre>", esc(&b.text.text));
            }
            BlockKind::Math => {
                let _ = writeln!(out, "{pad}<p><latex>{}</latex></p>", esc(&b.text.text));
            }
            BlockKind::Divider => {
                let _ = writeln!(out, "{pad}<hr/>");
            }
            BlockKind::Image { path, alt, width, .. } => {
                let mut attrs = format!(" path=\"{}\"", esc(path));
                if let Some(w) = width {
                    let _ = write!(attrs, " width=\"{}\"", w.round() as i64);
                }
                if !alt.is_empty() {
                    let _ = write!(attrs, " caption=\"{}\"", esc(alt));
                }
                let _ = writeln!(out, "{pad}<img{attrs}{}/>", align_attr(b.align));
            }
            BlockKind::LinkCard { url } => {
                let _ = writeln!(out, "{pad}<bookmark href=\"{}\"/>", esc(url));
            }
            BlockKind::File { path, name, .. } | BlockKind::Video { path, name, .. } => {
                let _ = writeln!(out, "{pad}<figure><source path=\"{}\" name=\"{}\"/></figure>", esc(path), esc(name));
            }
            BlockKind::Columns { cols } => {
                let _ = writeln!(out, "{pad}<grid>");
                let ratio = 1. / cols.len().max(1) as f32;
                for c in cols {
                    let _ = writeln!(out, "{pad}  <column width-ratio=\"{ratio:.3}\">");
                    paragraphs(c, &format!("{pad}    "), out);
                    let _ = writeln!(out, "{pad}  </column>");
                }
                let _ = writeln!(out, "{pad}</grid>");
            }
            BlockKind::Table { rows, cols, cells, header_row, widths, merges, .. } => {
                table_xml(*rows, *cols, cells, *header_row, widths.as_deref(), merges, &pad, out)
            }
            BlockKind::Button { url, .. } => {
                let _ = writeln!(out, "{pad}<p><a href=\"{}\">{}</a></p>", esc(url), esc(&b.text.text));
            }
            BlockKind::Timeline { items, .. } => {
                for it in items {
                    let line =
                        [&it.time.text, &it.title.text, &it.detail.text].iter().filter(|s| !s.is_empty()).map(|s| esc(s)).collect::<Vec<_>>().join(" · ");
                    let _ = writeln!(out, "{pad}<p>{line}</p>");
                }
            }
            // 没有对应物的块（目录、倒计时、画板 / 电子表格 / 多维表格、不认识的块）：留一行说明
            k => {
                if !matches!(k, BlockKind::Toc) {
                    let _ = writeln!(out, "{pad}<p>[{}]</p>", k.label());
                }
            }
        }
        // 装不下子块的块：子块按同级输出
        if !nests(&b.kind) && !kids.is_empty() {
            emit(kids, depth, out);
        }
        i = j;
    }
}

#[allow(clippy::too_many_arguments)]
fn table_xml(rows: usize, cols: usize, cells: &[RichText], header_row: bool, widths: Option<&[f32]>, merges: &[CellMerge], pad: &str, out: &mut String) {
    let _ = writeln!(out, "{pad}<table>");
    if let Some(w) = widths.filter(|w| w.len() == cols) {
        let total: f32 = w.iter().sum::<f32>().max(0.01);
        let cols_xml: String = w.iter().map(|v| format!("<col width=\"{}\"/>", (v / total * 100. * cols as f32).round() as i64)).collect();
        let _ = writeln!(out, "{pad}  <colgroup>{cols_xml}</colgroup>");
    }
    let covered =
        |r: usize, c: usize| merges.iter().any(|m| r >= m.row && r < m.row + m.row_span && c >= m.col && c < m.col + m.col_span && !(r == m.row && c == m.col));
    for r in 0..rows {
        let head = header_row && r == 0;
        if head {
            let _ = writeln!(out, "{pad}  <thead>");
        } else if r == usize::from(header_row) {
            let _ = writeln!(out, "{pad}  <tbody>");
        }
        let mut tr = String::new();
        for c in 0..cols {
            if covered(r, c) {
                continue;
            }
            let tag = if head { "th" } else { "td" };
            let mut attrs = String::new();
            if let Some(m) = merges.iter().find(|m| m.row == r && m.col == c) {
                if m.col_span > 1 {
                    let _ = write!(attrs, " colspan=\"{}\"", m.col_span);
                }
                if m.row_span > 1 {
                    let _ = write!(attrs, " rowspan=\"{}\"", m.row_span);
                }
            }
            let cell = &cells[r * cols + c];
            match cell.valign {
                1 => attrs.push_str(" vertical-align=\"middle\""),
                2 => attrs.push_str(" vertical-align=\"bottom\""),
                _ => {}
            }
            // 单元格里的块写在一行内（见 FORMAT.md）
            let mut ps = String::new();
            paragraphs(cell, "", &mut ps);
            let _ = write!(tr, "<{tag}{attrs}>{}</{tag}>", ps.replace('\n', ""));
        }
        let _ = writeln!(out, "{pad}    <tr>{tr}</tr>");
        if head {
            let _ = writeln!(out, "{pad}  </thead>");
        }
    }
    if rows > usize::from(header_row) {
        let _ = writeln!(out, "{pad}  </tbody>");
    }
    let _ = writeln!(out, "{pad}</table>");
}

#[cfg(test)]
mod tests {
    use super::{export, import};
    use crate::model::*;

    /// quiet.md 的「全部块」样例：每种块都能读进来，结构和样式对得上。
    const QUIET_MD_FIXTURE: &str = include_str!("../tests/fixtures/quiet-md-all-blocks.xml");

    fn kinds(bs: &[Block]) -> Vec<String> {
        bs.iter().map(|b| format!("{}{}", "  ".repeat(b.indent as usize), b.kind.label())).collect()
    }

    #[test]
    fn imports_the_quiet_md_fixture() {
        let bs = import(QUIET_MD_FIXTURE).unwrap();
        assert_eq!(bs[0].kind, BlockKind::Title);
        assert_eq!(bs[0].text.text, "全部 块 类型");
        let find = |s: &str| bs.iter().find(|b| b.text.text.contains(s)).unwrap_or_else(|| panic!("找不到「{s}」：{:#?}", kinds(&bs)));
        // 行内样式与颜色
        let p = find("普通");
        let marks_of = |t: &str| p.text.runs().find(|(r, _)| &p.text.text[r.clone()] == t).map(|(_, m)| m.clone()).unwrap_or_else(|| panic!("{t}"));
        assert!(marks_of("加粗").bold && marks_of("斜体").italic && marks_of("下划线").underline && marks_of("删除线").strike && marks_of("code").code);
        assert_eq!((marks_of("彩色").color, marks_of("彩色").bg), (1, 3));
        assert_eq!(marks_of("链接").link.as_deref(), Some("https://a.com/?x=1&y=2"));
        assert!(p.text.math_spans().iter().any(|(_, t)| t == "E=mc^2"));
        assert!(p.text.text.contains('\n'), "<br/> 变成换行");
        // 嵌套
        assert_eq!(find("缩进的子段落").indent, 1);
        assert_eq!((find("一.一.一").kind.clone(), find("一.一.一").indent), (BlockKind::Bullet, 2));
        let exact = |s: &str| bs.iter().find(|b| b.text.text == s).unwrap_or_else(|| panic!("找不到「{s}」"));
        assert_eq!((exact("三").kind.clone(), exact("三").num), (BlockKind::Ordered, Some(3)));
        assert_eq!((exact("四.一").kind.clone(), exact("四.一").indent), (BlockKind::Ordered, 1));
        assert_eq!(find("子任务").kind, BlockKind::Todo { checked: false });
        assert_eq!(find("子任务").indent, 1);
        assert_eq!(find("完成").kind, BlockKind::Todo { checked: true });
        assert_eq!(find("空项下的段落").indent, 1);
        // 容器
        assert_eq!(find("引用一").kind, BlockKind::Quote);
        assert!(find("引用一").text.text.contains("引用二"));
        assert_eq!(find("提示").kind, BlockKind::Callout { emoji: "💡".into(), color: 2 });
        assert_eq!(find("fn main").kind, BlockKind::Code { lang: "rust".into(), wrap: true, height: None });
        assert!(find("fn main").text.text.contains("<hi> & 'q'"));
        assert!(matches!(&find("七").kind, BlockKind::Heading { level: 6 }));
        assert_eq!(find("三级").align, 1);
        assert!(bs.iter().any(|b| matches!(&b.kind, BlockKind::Columns { cols } if cols.len() == 2 && cols[1].text.contains("右引"))));
        assert!(bs.iter().any(|b| b.kind == BlockKind::Divider));
        assert!(bs.iter().any(|b| matches!(&b.kind, BlockKind::LinkCard { url } if url == "https://example.com")));
        // 表格：3×3，左下合并 2×2，标题行
        let t = bs.iter().find(|b| matches!(b.kind, BlockKind::Table { .. })).unwrap();
        let BlockKind::Table { rows, cols, cells, merges, header_row, widths, .. } = &t.kind else { unreachable!() };
        assert_eq!((*rows, *cols, *header_row), (4, 3, true));
        assert_eq!(merges, &vec![CellMerge { row: 1, col: 0, row_span: 2, col_span: 2 }]);
        assert!(cells[3].text.contains("合并") && cells[3].text.contains("嵌套"));
        assert_eq!(cells[3].valign, 1);
        assert!(cells[5].text.contains("a = 1\nb = 2"));
        assert!(widths.is_some());
        assert_eq!(bs.last().unwrap().text.text, "结尾");
    }

    #[test]
    fn export_then_import_keeps_the_structure() {
        let bs = import(QUIET_MD_FIXTURE).unwrap();
        let xml = export(&bs);
        let back = import(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
        assert_eq!(kinds(&back), kinds(&bs), "\n{xml}");
        for (a, b) in bs.iter().zip(&back) {
            assert_eq!(a.text.text, b.text.text, "{}\n{xml}", a.kind.label());
            assert_eq!(a.num, b.num);
            assert_eq!(a.align, b.align);
        }
        let (t1, t2) =
            (bs.iter().find(|b| matches!(b.kind, BlockKind::Table { .. })).unwrap(), back.iter().find(|b| matches!(b.kind, BlockKind::Table { .. })).unwrap());
        assert_eq!(t1.kind, t2.kind);
    }

    #[test]
    fn rejects_broken_xml() {
        assert!(import("<p>没闭合").is_err());
    }
}
