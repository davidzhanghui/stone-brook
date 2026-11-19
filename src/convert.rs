//! 格式转换：Markdown / HTML 的导入导出。
//! - 从网页复制的内容以 HTML 形式进入剪贴板，经 `html_to_blocks` 还原为块；
//! - 复制时同时写出 HTML + Markdown 纯文本，粘贴到其他编辑器也能保留格式。

use crate::model::{Block, BlockKind, Marks, RichText};
use std::path::Path;

/// 计算有序列表编号：同一缩进层级上连续的有序列表项递增。
pub fn ordered_numbers(blocks: &[impl std::ops::Deref<Target = Block>]) -> Vec<usize> {
    let mut out = vec![0; blocks.len()];
    // counters[indent] = 当前层级的计数
    let mut counters: Vec<usize> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let ind = b.indent as usize;
        if counters.len() < ind + 1 {
            counters.resize(ind + 1, 0);
        }
        if b.kind == BlockKind::Ordered {
            counters.truncate(ind + 1);
            counters[ind] = match b.num {
                // 指定编号：从这一项起重新计数
                Some(n) => n.max(1) as usize,
                None => counters[ind] + 1,
            };
            out[i] = counters[ind];
        } else if b.kind.is_list() {
            counters.truncate(ind + 1);
            counters[ind] = 0;
        } else {
            counters.truncate(ind);
        }
    }
    out
}

pub fn ordered_label(n: usize, indent: u8) -> String {
    match indent % 3 {
        0 => format!("{n}."),
        1 => {
            let mut s = String::new();
            let mut n = n;
            while n > 0 {
                n -= 1;
                s.insert(0, (b'a' + (n % 26) as u8) as char);
                n /= 26;
            }
            format!("{s}.")
        }
        _ => format!("{}.", roman(n)),
    }
}

fn roman(mut n: usize) -> String {
    let table =
        [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

pub fn bullet_glyph(indent: u8) -> &'static str {
    match indent % 3 {
        0 => "•",
        1 => "◦",
        _ => "▪",
    }
}

// ───────────────────────────── Markdown 导出 ─────────────────────────────

fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '*' | '_' | '`' | '[' | ']' | '\\' | '~') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn rich_to_md(r: &RichText) -> String {
    let mut out = String::new();
    for (range, m) in r.runs() {
        let raw = &r.text[range];
        if let Some(tex) = &m.math {
            out.push_str(&format!("${tex}$"));
            continue;
        }
        if m.code {
            out.push('`');
            out.push_str(raw);
            out.push('`');
            continue;
        }
        let mut s = md_escape(raw);
        // 整段都是空白（如加粗的空格）：没有可包的内容，原样输出（否则下面首尾空白重叠会下溢）
        if s.trim().is_empty() {
            out.push_str(&s);
            continue;
        }
        // 标记不能包住首尾空白，否则 Markdown 不识别
        let lead = s.len() - s.trim_start().len();
        let trail = s.len() - s.trim_end().len();
        let (pre, rest) = s.split_at(lead);
        let (core, post) = rest.split_at(rest.len() - trail);
        let mut core = core.to_string();
        if !core.is_empty() {
            if m.bold {
                core = format!("**{core}**");
            }
            if m.italic {
                core = format!("*{core}*");
            }
            if m.strike {
                core = format!("~~{core}~~");
            }
            if m.underline {
                core = format!("<u>{core}</u>");
            }
            if let Some(link) = &m.link {
                core = format!("[{core}]({link})");
            }
        }
        s = format!("{pre}{core}{post}");
        out.push_str(&s);
    }
    out
}

pub fn blocks_to_md(blocks: &[impl std::ops::Deref<Target = Block>]) -> String {
    let nums = ordered_numbers(blocks);
    let mut out = String::new();
    let mut prev_list = false;
    for (i, b) in blocks.iter().enumerate() {
        let pad = "    ".repeat(b.indent as usize);
        let text = rich_to_md(&b.text);
        let is_list = b.kind.is_list();
        if i > 0 && !(is_list && prev_list) {
            out.push('\n');
        }
        prev_list = is_list;
        match &b.kind {
            BlockKind::Title => out.push_str(&format!("# {text}\n")),
            BlockKind::Heading { level } => out.push_str(&format!("{} {text}\n", "#".repeat((*level as usize + 1).min(6)))),
            BlockKind::Text => {
                out.push_str(&pad);
                // Markdown 没有块级对齐，非左对齐时用内联 HTML 保留
                match b.align {
                    1 => out.push_str(&format!("<p align=\"center\">{}</p>\n", rich_to_md(&b.text).replace('\n', "<br>"))),
                    2 => out.push_str(&format!("<p align=\"right\">{}</p>\n", rich_to_md(&b.text).replace('\n', "<br>"))),
                    _ => {
                        out.push_str(&text.replace('\n', "  \n"));
                        out.push('\n');
                    }
                }
            }
            BlockKind::Bullet => out.push_str(&format!("{pad}- {}\n", text.replace('\n', "  \n"))),
            BlockKind::Ordered => out.push_str(&format!("{pad}{}. {}\n", nums[i], text)),
            BlockKind::Todo { checked } => out.push_str(&format!("{pad}- [{}] {text}\n", if *checked { "x" } else { " " })),
            BlockKind::Quote => {
                for line in text.split('\n') {
                    out.push_str(&format!("> {line}\n"));
                }
            }
            // 演讲者备注：沿用 Marp 的 HTML 注释写法，渲染 Markdown 时不显示
            BlockKind::Notes => out.push_str(&format!("<!-- 备注：{} -->\n", b.text.text.replace("-->", "—>"))),
            BlockKind::Callout { emoji, .. } => {
                for (k, line) in text.split('\n').enumerate() {
                    if k == 0 {
                        out.push_str(&format!("> {emoji} {line}\n"));
                    } else {
                        out.push_str(&format!("> {line}\n"));
                    }
                }
            }
            BlockKind::Code { lang, .. } => {
                let l = if lang == "Plain Text" { "" } else { lang.as_str() };
                out.push_str(&format!("```{}\n{}\n```\n", l.to_lowercase(), b.text.text));
            }
            BlockKind::Divider => out.push_str("---\n"),
            BlockKind::Image { path, alt, width, crop } => {
                // 设了宽度 / 对齐 / 裁剪的图片，Markdown 语法表达不了，降级成内联 HTML（裁剪同样保留）
                let styled = width.is_some() || crop.is_some() || b.align != 0;
                if !styled {
                    out.push_str(&format!("![{}]({path})\n", md_escape(alt)));
                } else {
                    out.push_str(&image_html(path, path, alt, *width, *crop, b.align));
                    out.push('\n');
                }
            }
            BlockKind::Table { rows, cols, cells, .. } => {
                for r in 0..*rows {
                    let row: Vec<String> = (0..*cols).map(|c| rich_to_md(&cells[r * cols + c]).replace('\n', "<br>").replace('|', "\\|")).collect();
                    out.push_str(&format!("| {} |\n", row.join(" | ")));
                    if r == 0 {
                        out.push_str(&format!("|{}\n", " --- |".repeat(*cols)));
                    }
                }
            }
            BlockKind::Columns { cols } => {
                for (k, c) in cols.iter().enumerate() {
                    if k > 0 {
                        out.push('\n');
                    }
                    out.push_str(&rich_to_md(c));
                    out.push('\n');
                }
            }
            BlockKind::File { path, name, .. } => out.push_str(&format!("[📎 {}](<file://{}>)\n", md_escape(name), path)),
            BlockKind::Toc => out.push_str("[TOC]\n"),
            BlockKind::Countdown { target, .. } => {
                out.push_str(&format!("⏳ 倒计时：{}\n", fmt_ts(*target)));
            }
            BlockKind::Button { action, url, .. } => {
                if action == "open_url" && !url.is_empty() {
                    out.push_str(&format!("[{}]({url})\n", if text.is_empty() { url.clone() } else { text.clone() }));
                } else {
                    out.push_str(&format!("{}\n", text));
                }
            }
            BlockKind::Timeline { items, .. } => {
                for it in items {
                    let mut line = String::from("-");
                    if !it.time.is_empty() {
                        line.push_str(&format!(" `{}`", it.time.text));
                    }
                    if !it.title.is_empty() {
                        line.push_str(&format!(" **{}**", rich_to_md(&it.title).replace('\n', " ")));
                    }
                    if !it.detail.is_empty() {
                        line.push_str(&format!(" {}", rich_to_md(&it.detail).replace('\n', "  \n    ")));
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            BlockKind::Math => out.push_str(&format!("$$\n{}\n$$\n", b.text.text)),
            BlockKind::Diagram { .. } => out.push_str(&format!("```mermaid\n{}\n```\n", b.text.text)),
            BlockKind::Html { .. } => out.push_str(&format!("```html\n{}\n```\n", b.text.text)),
            BlockKind::LinkCard { url } => out.push_str(&format!("<{url}>\n")),
            BlockKind::Video { path, name, .. } => out.push_str(&format!("[🎬 {}](<file://{}>)\n", md_escape(name), path)),
            BlockKind::Embed { kind, id } => out.push_str(&crate::objects::to_markdown(kind, id)),
            BlockKind::Unknown { .. } => {}
        }
    }
    out
}

pub fn fmt_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

// ───────────────────────────── HTML 导出 ─────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const FG_HEX: [&str; 8] = ["", "#D83931", "#DE7802", "#DC9B04", "#2EA121", "#245BDB", "#6425D0", "#8F959E"];
/// 0..7 浅色背景，8..15 深色背景（颜色面板两行）
const BG_HEX: [&str; 16] = [
    "", "#FDDDEF", "#FEEAD2", "#FFF5B8", "#D9F5D6", "#E1EAFF", "#ECE2FE", "#EFF0F1", "#DEE0E3", "#BBBFC4", "#F76964", "#FFA53D", "#FFE928", "#62D256",
    "#94B4FA", "#C2A6F8",
];

/// 单元格对齐对应的 HTML style 属性（左上对齐为空）。
fn align_attr(align: u8, valign: u8) -> String {
    let h = match align {
        1 => "text-align:center;",
        2 => "text-align:right;",
        _ => "",
    };
    let v = match valign {
        1 => "vertical-align:middle;",
        2 => "vertical-align:bottom;",
        _ => "",
    };
    if h.is_empty() && v.is_empty() {
        String::new()
    } else {
        format!(" style=\"{h}{v}\"")
    }
}

pub fn rich_to_html(r: &RichText) -> String {
    let mut out = String::new();
    for (range, m) in r.runs() {
        if let Some(tex) = &m.math {
            match crate::math::cached_path(tex, false) {
                Some(p) => out.push_str(&format!("<img src=\"{}\" style=\"zoom:0.5;vertical-align:middle\">", img_src(&p))),
                None => out.push_str(&format!("\\({}\\)", html_escape(tex))),
            }
            continue;
        }
        let mut s = html_escape(&r.text[range]).replace('\n', "<br>");
        if m.code {
            s = format!("<code>{s}</code>");
        }
        if m.bold {
            s = format!("<strong>{s}</strong>");
        }
        if m.italic {
            s = format!("<em>{s}</em>");
        }
        if m.underline {
            s = format!("<u>{s}</u>");
        }
        if m.strike {
            s = format!("<s>{s}</s>");
        }
        let mut style = String::new();
        if m.color > 0 {
            style.push_str(&format!("color:{};", FG_HEX[m.color as usize & 7]));
        }
        if m.bg > 0 {
            style.push_str(&format!("background-color:{};", BG_HEX[m.bg as usize & 15]));
        }
        if !style.is_empty() {
            s = format!("<span style=\"{style}\">{s}</span>");
        }
        if let Some(link) = &m.link {
            s = format!("<a href=\"{}\">{s}</a>", html_escape(link));
        }
        out.push_str(&s);
    }
    out
}

pub fn blocks_to_html(blocks: &[impl std::ops::Deref<Target = Block>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < blocks.len() {
        let b = &blocks[i];
        if b.kind.is_list() {
            // 把连续的列表项组织成嵌套的 ul/ol
            let start = i;
            while i < blocks.len() && blocks[i].kind.is_list() {
                i += 1;
            }
            out.push_str(&list_to_html(&blocks[start..i]));
            continue;
        }
        let t = rich_to_html(&b.text);
        match &b.kind {
            BlockKind::Title => out.push_str(&format!("<h1>{t}</h1>")),
            BlockKind::Heading { level } => {
                let l = (*level + 1).min(6);
                let a = match b.align {
                    1 => " style=\"text-align:center\"",
                    2 => " style=\"text-align:right\"",
                    _ => "",
                };
                out.push_str(&format!("<h{l} id=\"h-{i}\"{a}>{t}</h{l}>"))
            }
            BlockKind::Text => {
                let a = match b.align {
                    1 => " style=\"text-align:center\"",
                    2 => " style=\"text-align:right\"",
                    _ => "",
                };
                out.push_str(&format!("<p{a}>{t}</p>"))
            }
            BlockKind::Quote => out.push_str(&format!("<blockquote>{t}</blockquote>")),
            // reveal.js 约定的备注写法；网页 / PDF 导出里不显示
            BlockKind::Notes => out.push_str(&format!("<aside class=\"notes\" style=\"display:none\">{t}</aside>")),
            BlockKind::Callout { emoji, .. } => {
                out.push_str(&format!("<blockquote style=\"background:#F0F4FF;border-left:4px solid #3B6CF6;padding:8px 12px\">{emoji} {t}</blockquote>"))
            }
            BlockKind::Code { lang, .. } => {
                out.push_str(&format!("<pre><code class=\"language-{}\">{}</code></pre>", lang.to_lowercase(), html_escape(&b.text.text)))
            }
            BlockKind::Divider => out.push_str("<hr>"),
            BlockKind::Image { path, alt, width, crop } => {
                let src = img_src(std::path::Path::new(path));
                out.push_str(&image_html(&src, path, alt, *width, *crop, b.align));
            }
            BlockKind::Table { rows, cols, cells, header_row, header_col, merges, .. } => {
                out.push_str("<table border=\"1\" style=\"border-collapse:collapse\">");
                for r in 0..*rows {
                    out.push_str("<tr>");
                    for c in 0..*cols {
                        let (orow, ocol, rs, cs) = crate::model::merge_origin(merges, r, c);
                        if (orow, ocol) != (r, c) {
                            continue;
                        }
                        let head = (r == 0 && *header_row) || (c == 0 && *header_col);
                        let tag = if head { "th" } else { "td" };
                        let mut attrs = String::new();
                        if cs > 1 {
                            attrs.push_str(&format!(" colspan=\"{cs}\""));
                        }
                        if rs > 1 {
                            attrs.push_str(&format!(" rowspan=\"{rs}\""));
                        }
                        let cell = &cells[r * cols + c];
                        attrs.push_str(&align_attr(cell.align, cell.valign));
                        out.push_str(&format!("<{tag}{attrs}>{}</{tag}>", rich_to_html(cell)));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
            BlockKind::Columns { cols } => {
                out.push_str("<table style=\"width:100%\"><tr>");
                for c in cols {
                    let ta = match c.align {
                        1 => ";text-align:center",
                        2 => ";text-align:right",
                        _ => "",
                    };
                    out.push_str(&format!("<td style=\"vertical-align:top{ta}\">{}</td>", rich_to_html(c)));
                }
                out.push_str("</tr></table>");
            }
            BlockKind::File { path, name, size, .. } => out.push_str(&format!(
                "<p class=\"file\">📎 <a href=\"file://{}\">{}</a> <span style=\"color:#8f959e\">{}</span></p>",
                html_escape(path),
                html_escape(name),
                human_size(*size)
            )),
            BlockKind::Toc => {
                out.push_str("<nav class=\"toc\"><ul>");
                for (k, h) in blocks.iter().enumerate() {
                    if let BlockKind::Heading { level } = h.kind {
                        if level <= 3 {
                            out.push_str(&format!(
                                "<li style=\"margin-left:{}px\"><a href=\"#h-{k}\">{}</a></li>",
                                (level - 1) * 16,
                                html_escape(&h.text.text)
                            ));
                        }
                    }
                }
                out.push_str("</ul></nav>");
            }
            BlockKind::Countdown { target, .. } => out.push_str(&format!("<p>⏳ 倒计时：{}</p>", fmt_ts(*target))),
            BlockKind::Button { action, url, style } => {
                let href = if action == "open_url" { html_escape(url) } else { "#".into() };
                let (_, bg, fg) = crate::model::BUTTON_COLORS[crate::model::button_color(*style)];
                out.push_str(&format!(
                    "<p><a href=\"{href}\" style=\"display:inline-block;padding:2px 8px;border-radius:4px;background:#{bg:06x};color:#{fg:06x};text-decoration:none\">{t}</a></p>"
                ));
            }
            BlockKind::Timeline { items, .. } => {
                out.push_str("<ul style=\"list-style:none;border-left:2px solid #dee0e3;padding-left:16px\">");
                for (k, it) in items.iter().enumerate() {
                    let (bg, dot) = crate::model::TIMELINE_COLORS[k % crate::model::TIMELINE_COLORS.len()];
                    out.push_str(&format!(
                        "<li style=\"margin:8px 0\"><span style=\"color:#{dot:06x}\">●</span> <small style=\"color:#8f959e\">{}</small><div style=\"background:#{bg:06x};border-radius:6px;padding:8px 12px;margin-top:4px\"><b>{}</b><br><span style=\"color:#646a73\">{}</span></div></li>",
                        html_escape(&it.time.text),
                        rich_to_html(&it.title),
                        rich_to_html(&it.detail)
                    ));
                }
                out.push_str("</ul>");
            }
            BlockKind::Math => match crate::math::cached_path(&b.text.text, true) {
                Some(p) => out.push_str(&format!("<p style=\"text-align:center\"><img src=\"{}\" style=\"zoom:0.5\"></p>", img_src(&p))),
                None => out.push_str(&format!("<pre>$$ {} $$</pre>", html_escape(&b.text.text))),
            },
            BlockKind::Diagram { offsets, .. } => {
                out.push_str("<div class=\"diagram\">");
                out.push_str(&crate::diagram::source_to_svg(&b.text.text, offsets));
                out.push_str("</div>");
            }
            BlockKind::Html { .. } => out.push_str(&format!(
                "<iframe sandbox style=\"width:100%;border:1px solid #dee0e3;border-radius:6px;min-height:200px\" srcdoc=\"{}\"></iframe>",
                html_escape(&b.text.text)
            )),
            BlockKind::LinkCard { url } => {
                let (title, desc) = crate::linkcard::cached_text(url);
                out.push_str(&format!(
                    "<p><a href=\"{0}\" style=\"display:block;border:1px solid #dee0e3;border-radius:8px;padding:10px 14px;text-decoration:none;color:inherit\"><b>{1}</b><br><span style=\"color:#646a73\">{2}</span><br><span style=\"color:#8f959e;font-size:12px\">{0}</span></a></p>",
                    html_escape(url),
                    html_escape(&title),
                    html_escape(&desc)
                ));
            }
            BlockKind::Video { path, .. } => out.push_str(&format!("<video controls style=\"max-width:100%\" src=\"file://{}\"></video>", html_escape(path))),
            BlockKind::Embed { kind, id } => out.push_str(&crate::objects::to_html(kind, id)),
            _ => {}
        }
        i += 1;
    }
    out
}

pub fn human_size(n: u64) -> String {
    let n = n as f64;
    if n < 1024. {
        format!("{n:.0} B")
    } else if n < 1024. * 1024. {
        format!("{:.1} KB", n / 1024.)
    } else if n < 1024. * 1024. * 1024. {
        format!("{:.1} MB", n / 1024. / 1024.)
    } else {
        format!("{:.2} GB", n / 1024. / 1024. / 1024.)
    }
}

/// 图片转为 data URI，保证导出的 HTML 是自包含的。
pub fn img_src(path: &std::path::Path) -> String {
    use base64::Engine;
    let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "image/jpeg",
    };
    match std::fs::read(path) {
        Ok(bytes) => format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
        Err(_) => format!("file://{}", path.display()),
    }
}

fn list_to_html(items: &[impl std::ops::Deref<Target = Block>]) -> String {
    let mut out = String::new();
    let mut stack: Vec<&'static str> = Vec::new();
    for b in items {
        let tag = if b.kind == BlockKind::Ordered { "ol" } else { "ul" };
        let depth = b.indent as usize + 1;
        while stack.len() > depth {
            out.push_str(&format!("</li></{}>", stack.pop().unwrap()));
        }
        if stack.len() == depth && stack.last() != Some(&tag) {
            out.push_str(&format!("</li></{}>", stack.pop().unwrap()));
        }
        if stack.len() == depth {
            out.push_str("</li>");
        }
        while stack.len() < depth {
            out.push_str(&format!("<{tag}>"));
            stack.push(tag);
        }
        let body = rich_to_html(&b.text);
        match b.kind {
            BlockKind::Todo { checked } => out.push_str(&format!("<li><input type=\"checkbox\"{}> {body}", if checked { " checked" } else { "" })),
            _ => out.push_str(&format!("<li>{body}")),
        }
    }
    while let Some(t) = stack.pop() {
        out.push_str(&format!("</li></{t}>"));
    }
    out
}

pub fn full_html_page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>{}</title>\
<style>body{{max-width:820px;margin:48px auto;padding:0 24px;font:16px/1.75 -apple-system,'PingFang SC',sans-serif;color:#1f2329}}\
pre{{background:#f5f6f7;padding:12px 16px;border-radius:8px;overflow:auto}}code{{font-family:Menlo,monospace}}\
blockquote{{margin:8px 0;padding:2px 14px;border-left:3px solid #bbbfc4;color:#646a73}}\
table{{border-collapse:collapse}}td,th{{border:1px solid #dee0e3;padding:6px 10px}}th{{background:#f5f6f7}}img{{max-width:100%}}</style>\
</head><body>{}</body></html>",
        html_escape(title),
        body
    )
}

// ───────────────────────────── Markdown 导入 ─────────────────────────────

/// 粗略判断一段纯文本是不是 Markdown。
pub fn looks_like_markdown(s: &str) -> bool {
    let mut score = 0;
    for line in s.lines().take(200) {
        let t = line.trim_start();
        if t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("### ")
            || t.starts_with("```")
            || t.starts_with("- [")
            || t.starts_with("> ")
            || (t.starts_with('|') && t.ends_with('|'))
        {
            score += 2;
        } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("1. ") {
            score += 1;
        }
        if t.contains("**") || t.contains("](") {
            score += 1;
        }
    }
    score >= 2
}

pub fn md_to_blocks(md: &str) -> Vec<Block> {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_MATH;
    let mut b = Builder::default();
    let mut lists: Vec<bool> = Vec::new(); // true = ordered
    let mut in_code = false;
    let mut in_image = false;
    let mut table: Option<(Vec<Vec<RichText>>, bool)> = None;
    let mut quote = 0usize;

    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if table.is_none() {
                        let kind = if quote > 0 { BlockKind::Quote } else { BlockKind::Text };
                        b.open_if_needed(kind, lists.len().saturating_sub(1) as u8, !lists.is_empty());
                    }
                }
                Tag::Heading { level, .. } => {
                    let l = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    b.open(BlockKind::Heading { level: l }, 0);
                }
                Tag::BlockQuote(_) => quote += 1,
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => normalize_lang(&l),
                        CodeBlockKind::Indented => "Plain Text".into(),
                    };
                    b.open(BlockKind::Code { lang, wrap: true }, 0);
                    in_code = true;
                }
                Tag::List(start) => {
                    b.flush();
                    lists.push(start.is_some());
                }
                Tag::Item => {
                    let ordered = lists.last().copied().unwrap_or(false);
                    let kind = if ordered { BlockKind::Ordered } else { BlockKind::Bullet };
                    b.open(kind, lists.len().saturating_sub(1) as u8);
                }
                Tag::Emphasis => b.marks.italic = true,
                Tag::Strong => b.marks.bold = true,
                Tag::Strikethrough => b.marks.strike = true,
                Tag::Link { dest_url, .. } => b.marks.link = Some(dest_url.to_string()),
                Tag::Image { dest_url, .. } => {
                    b.flush();
                    b.blocks.push(Block::new(BlockKind::image(dest_url.to_string()), RichText::default()));
                    in_image = true;
                }
                Tag::Table(_) => {
                    b.flush();
                    table = Some((Vec::new(), false));
                }
                Tag::TableHead | Tag::TableRow => {
                    if let Some((rows, _)) = &mut table {
                        rows.push(Vec::new());
                    }
                }
                Tag::TableCell => {
                    if let Some((rows, _)) = &mut table {
                        if let Some(r) = rows.last_mut() {
                            r.push(RichText::default());
                        }
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) => {
                    if table.is_none() {
                        b.flush();
                    }
                }
                TagEnd::Item => b.flush(),
                TagEnd::List(_) => {
                    lists.pop();
                }
                TagEnd::BlockQuote(_) => quote = quote.saturating_sub(1),
                TagEnd::CodeBlock => {
                    if let Some(cur) = &mut b.cur {
                        let t = cur.text.text.trim_end_matches('\n').to_string();
                        cur.text = RichText::plain(t);
                        if matches!(&cur.kind, BlockKind::Code { lang, .. } if lang.eq_ignore_ascii_case("mermaid")) {
                            cur.kind = BlockKind::Diagram { view: 0, offsets: Default::default() };
                        }
                    }
                    b.flush();
                    in_code = false;
                }
                TagEnd::Emphasis => b.marks.italic = false,
                TagEnd::Strong => b.marks.bold = false,
                TagEnd::Strikethrough => b.marks.strike = false,
                TagEnd::Link => b.marks.link = None,
                TagEnd::Image => in_image = false,
                TagEnd::Table => {
                    if let Some((rows, _)) = table.take() {
                        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
                        let mut cells = Vec::new();
                        for mut r in rows.clone() {
                            r.resize(cols, RichText::default());
                            cells.extend(r);
                        }
                        b.blocks.push(Block::new(
                            BlockKind::Table { rows: rows.len().max(1), cols, cells, header_row: true, header_col: false, widths: None, merges: Vec::new() },
                            RichText::default(),
                        ));
                    }
                }
                _ => {}
            },
            Event::Text(t) | Event::Code(t) if in_image => {
                let _ = t;
            }
            Event::Text(t) => {
                if let Some((rows, _)) = &mut table {
                    if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                        cell.insert(cell.len(), &t, b.marks.clone());
                    }
                } else if in_code {
                    b.push_raw(&t);
                } else {
                    b.push_text(&t, quote > 0);
                }
            }
            Event::Code(t) => {
                let mut m = b.marks.clone();
                m.code = true;
                if let Some((rows, _)) = &mut table {
                    if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                        cell.insert(cell.len(), &t, m);
                    }
                } else {
                    b.push_with(&t, m, quote > 0);
                }
            }
            Event::DisplayMath(t) => {
                b.flush();
                b.blocks.push(Block::new(BlockKind::Math, RichText::plain(t.trim().to_string())));
            }
            Event::InlineMath(t) => {
                if b.cur.is_none() {
                    b.open(if quote > 0 { BlockKind::Quote } else { BlockKind::Text }, 0);
                }
                let cur = b.cur.as_mut().unwrap();
                let len = cur.text.len();
                cur.text.insert_math(len, &t);
            }
            Event::SoftBreak | Event::HardBreak => {
                if table.is_none() {
                    b.push_text("\n", quote > 0);
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                let h = h.trim();
                if h.eq_ignore_ascii_case("<u>") {
                    b.marks.underline = true;
                } else if h.eq_ignore_ascii_case("</u>") {
                    b.marks.underline = false;
                } else if h.eq_ignore_ascii_case("<br>") || h.eq_ignore_ascii_case("<br/>") {
                    b.push_text("\n", quote > 0);
                }
            }
            Event::Rule => {
                b.flush();
                b.blocks.push(Block::new(BlockKind::Divider, RichText::default()));
            }
            Event::TaskListMarker(checked) => {
                if let Some(cur) = &mut b.cur {
                    cur.kind = BlockKind::Todo { checked };
                }
            }
            _ => {}
        }
    }
    b.flush();
    // 引用块首行是 emoji 时还原为高亮块
    for blk in &mut b.blocks {
        if blk.kind == BlockKind::Quote {
            if let Some(first) = blk.text.text.chars().next() {
                if is_emoji(first) {
                    let n = first.len_utf8();
                    let skip = if blk.text.text[n..].starts_with(' ') { n + 1 } else { n };
                    blk.text.delete(0..skip);
                    blk.kind = BlockKind::Callout { emoji: first.to_string(), color: 0 };
                }
            }
        }
    }
    b.blocks
}

fn is_emoji(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u)
}

pub fn normalize_lang(l: &str) -> String {
    let l = l.trim().to_lowercase();
    let name = match l.as_str() {
        "" | "text" | "plain" | "plaintext" => "Plain Text",
        "rs" | "rust" => "Rust",
        "py" | "python" => "Python",
        "js" | "javascript" | "jsx" => "JavaScript",
        "ts" | "typescript" | "tsx" => "TypeScript",
        "go" | "golang" => "Go",
        "java" => "Java",
        "c" | "h" => "C",
        "cpp" | "c++" | "cc" => "C++",
        "sh" | "bash" | "shell" | "zsh" => "Shell",
        "sql" => "SQL",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "html" | "xml" => "HTML",
        "css" => "CSS",
        "md" | "markdown" => "Markdown",
        "mermaid" => "mermaid",
        "kotlin" | "kt" => "Kotlin",
        "swift" => "Swift",
        _ => return l,
    };
    name.to_string()
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    cur: Option<Block>,
    marks: Marks,
}

impl Builder {
    fn open(&mut self, kind: BlockKind, indent: u8) {
        self.flush();
        let mut blk = Block::new(kind, RichText::default());
        blk.indent = indent;
        self.cur = Some(blk);
    }

    /// 列表项里的第一个段落复用列表项本身。
    fn open_if_needed(&mut self, kind: BlockKind, indent: u8, in_list: bool) {
        if let Some(cur) = &self.cur {
            if cur.text.is_empty() {
                return;
            }
        }
        let kind = if in_list && kind == BlockKind::Text { BlockKind::Text } else { kind };
        self.open(kind, if in_list { indent } else { 0 });
    }

    fn flush(&mut self) {
        if let Some(b) = self.cur.take() {
            self.blocks.push(b);
        }
    }

    fn push_raw(&mut self, s: &str) {
        if let Some(cur) = &mut self.cur {
            let len = cur.text.len();
            cur.text.insert(len, s, Marks::default());
        }
    }

    fn push_text(&mut self, s: &str, quote: bool) {
        let m = self.marks.clone();
        self.push_with(s, m, quote);
    }

    fn push_with(&mut self, s: &str, m: Marks, quote: bool) {
        if self.cur.is_none() {
            self.open(if quote { BlockKind::Quote } else { BlockKind::Text }, 0);
        }
        let cur = self.cur.as_mut().unwrap();
        let len = cur.text.len();
        cur.text.insert(len, s, m);
    }
}

/// 纯文本 → 块：每行一个段落。
pub fn plain_to_blocks(s: &str) -> Vec<Block> {
    s.replace("\r\n", "\n").split('\n').map(Block::text).collect()
}

// ───────────────────────────── HTML 导入 ─────────────────────────────

pub fn html_to_blocks(html: &str, assets_dir: &Path) -> Vec<Block> {
    let doc = scraper::Html::parse_document(html);
    let mut w = HtmlWalker { b: Builder::default(), lists: Vec::new(), quote: 0, assets_dir, callout: false };
    w.walk(doc.root_element(), &Marks::default());
    w.b.flush();
    // 去掉首尾空段落
    let mut blocks = w.b.blocks;
    while blocks.first().is_some_and(|b| b.kind == BlockKind::Text && b.text.text.trim().is_empty()) {
        blocks.remove(0);
    }
    while blocks.last().is_some_and(|b| b.kind == BlockKind::Text && b.text.text.trim().is_empty()) {
        blocks.pop();
    }
    for b in &mut blocks {
        trim_rich(&mut b.text);
    }
    blocks
}

fn trim_rich(r: &mut RichText) {
    let start = r.text.len() - r.text.trim_start_matches([' ', '\n']).len();
    let end = r.text.trim_end_matches([' ', '\n']).len();
    if end < r.text.len() {
        r.delete(end..r.text.len());
    }
    if start > 0 && start <= r.text.len() {
        r.delete(0..start);
    }
}

struct HtmlWalker<'a> {
    b: Builder,
    lists: Vec<bool>,
    quote: usize,
    assets_dir: &'a Path,
    /// 正在飞书高亮块里：里层的 div 并进同一个高亮块（多行），不再各自成块
    callout: bool,
}

/// 飞书高亮块的 `data-emoji-id` → 表情（常见的几十个；没收录的用 💡）。
fn feishu_emoji(id: &str) -> &'static str {
    match id {
        "chestnut" => "🌰",
        "bulb" | "light_bulb" | "idea" => "💡",
        "warning" | "warn" => "⚠️",
        "fire" => "🔥",
        "pushpin" | "pin" => "📌",
        "white_check_mark" | "check" | "done" => "✅",
        "exclamation" | "heavy_exclamation_mark" => "❗",
        "question" => "❓",
        "memo" | "pencil" => "📝",
        "dart" | "target" => "🎯",
        "star" => "⭐",
        "heart" | "red_heart" => "❤️",
        "rocket" => "🚀",
        "tada" | "party" => "🎉",
        "thumbsup" | "+1" => "👍",
        "eyes" => "👀",
        "bell" => "🔔",
        "book" | "books" => "📖",
        "calendar" | "date" => "📅",
        "smile" | "smiley" => "😄",
        "no_entry" | "prohibited" => "🚫",
        "information_source" | "info" => "ℹ️",
        _ => "💡",
    }
}

/// 颜色最接近的调色板序号（1 红 … 7 灰），用于飞书高亮块的底色 / 边框色。
fn nearest_palette(rgb: (u8, u8, u8)) -> u8 {
    const HUES: [(u8, u8, u8); 7] =
        [(0xd8, 0x39, 0x31), (0xde, 0x78, 0x02), (0xdc, 0x9b, 0x04), (0x2e, 0xa1, 0x21), (0x24, 0x5b, 0xdb), (0x64, 0x25, 0xd0), (0x8f, 0x95, 0x9e)];
    // 浅色底：先按亮度拉开，再比色相（直接比 RGB 时浅色都离灰色最近）
    let norm = |c: (u8, u8, u8)| {
        let (r, g, b) = (c.0 as f32, c.1 as f32, c.2 as f32);
        let mx = r.max(g).max(b).max(1.);
        let mn = r.min(g).min(b);
        let sat = (mx - mn) / mx;
        ((r - mn) / (mx - mn + 1.), (g - mn) / (mx - mn + 1.), (b - mn) / (mx - mn + 1.), sat)
    };
    let a = norm(rgb);
    if a.3 < 0.08 {
        return 7;
    }
    (0..6)
        .min_by(|&i, &j| {
            let d = |k: usize| {
                let h = norm(HUES[k]);
                (a.0 - h.0).powi(2) + (a.1 - h.1).powi(2) + (a.2 - h.2).powi(2)
            };
            d(i).total_cmp(&d(j))
        })
        .map(|i| i as u8 + 1)
        .unwrap_or(0)
}

fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().to_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => Some(((v >> 16) as u8, (v >> 8) as u8, v as u8)),
            3 => Some((((v >> 8) & 0xf) as u8 * 17, ((v >> 4) & 0xf) as u8 * 17, (v & 0xf) as u8 * 17)),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.trim_end_matches(')');
    let parts: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.len() >= 3 {
        if parts.len() == 4 && parts[3] < 0.05 {
            return None;
        }
        return Some((parts[0] as u8, parts[1] as u8, parts[2] as u8));
    }
    None
}

fn nearest(rgb: (u8, u8, u8), table: &[&str]) -> u8 {
    let mut best = (u32::MAX, 0u8);
    for (i, hex) in table.iter().enumerate().skip(1) {
        let Some(c) = parse_color(hex) else { continue };
        let d = (rgb.0 as i32 - c.0 as i32).pow(2) + (rgb.1 as i32 - c.1 as i32).pow(2) + (rgb.2 as i32 - c.2 as i32).pow(2);
        if (d as u32) < best.0 {
            best = (d as u32, i as u8);
        }
    }
    best.1
}

fn is_neutral_text(c: (u8, u8, u8)) -> bool {
    // 黑 / 深灰 视为默认颜色
    let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
    (r - g).abs() < 16 && (g - b).abs() < 16 && r < 110
}

fn is_white(c: (u8, u8, u8)) -> bool {
    c.0 > 245 && c.1 > 245 && c.2 > 245
}

fn apply_style(style: &str, m: &mut Marks) {
    for decl in style.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let (k, v) = (k.trim().to_lowercase(), v.trim().to_lowercase());
        match k.as_str() {
            "font-weight" => m.bold = v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => m.italic = v.contains("italic"),
            "text-decoration" | "text-decoration-line" => {
                if v.contains("underline") {
                    m.underline = true;
                }
                if v.contains("line-through") {
                    m.strike = true;
                }
            }
            "color" => {
                if let Some(c) = parse_color(&v) {
                    m.color = if is_neutral_text(c) { 0 } else { nearest(c, &FG_HEX) };
                }
            }
            "background-color" | "background" => {
                if let Some(c) = parse_color(&v) {
                    if !is_white(c) {
                        m.bg = nearest(c, &BG_HEX);
                    }
                }
            }
            _ => {}
        }
    }
}

impl<'a> HtmlWalker<'a> {
    fn walk(&mut self, el: scraper::ElementRef, marks: &Marks) {
        use scraper::node::Node;
        for child in el.children() {
            match child.value() {
                Node::Text(t) => {
                    let raw: &str = t;
                    let pre = self.b.cur.as_ref().is_some_and(|c| matches!(c.kind, BlockKind::Code { .. }));
                    if pre {
                        self.b.push_raw(raw);
                        continue;
                    }
                    let collapsed = collapse_ws(raw);
                    if collapsed.trim().is_empty() && self.b.cur.as_ref().is_none_or(|c| c.text.is_empty()) {
                        continue;
                    }
                    self.b.push_with(&collapsed, marks.clone(), self.quote > 0);
                }
                Node::Element(_) => {
                    if let Some(ce) = scraper::ElementRef::wrap(child) {
                        self.element(ce, marks);
                    }
                }
                _ => {}
            }
        }
    }

    fn list_indent(&self) -> u8 {
        self.lists.len().saturating_sub(1) as u8
    }

    fn element(&mut self, el: scraper::ElementRef, marks: &Marks) {
        let v = el.value();
        let name = v.name().to_ascii_lowercase();
        let class = v.attr("class").unwrap_or("").to_lowercase();
        let mut m = marks.clone();
        if let Some(style) = v.attr("style") {
            apply_style(style, &mut m);
        }
        match name.as_str() {
            "head" | "style" | "script" | "meta" | "title" | "noscript" | "svg" | "button" => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<u8>().unwrap_or(1);
                self.b.open(BlockKind::Heading { level }, 0);
                m.bold = false;
                self.walk(el, &m);
                self.b.flush();
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "figure" | "figcaption" => {
                let looks_heading = class.contains("heading-h");
                if self.callout {
                    // 高亮块里层：同一块里另起一行，不新建块；里层容器的底色不是文字底色
                    m.bg = 0;
                    if !has_block_child(el) && self.b.cur.as_ref().is_some_and(|c| !c.text.is_empty()) && el.text().any(|t| !t.trim().is_empty()) {
                        self.b.push_raw("\n");
                    }
                    self.walk(el, &m);
                } else if class.contains("callout") {
                    // 飞书：zoneType-calloutBlock > callout-container[data-emoji-id] > callout-block[style] > 正文 div
                    let all = || std::iter::once(el).chain(el.descendants().filter_map(scraper::ElementRef::wrap));
                    let emoji = all().find_map(|e| e.value().attr("data-emoji-id")).map(feishu_emoji).unwrap_or("💡");
                    let style_color = |prop: &str| {
                        all().filter_map(|e| e.value().attr("style")).find_map(|s| {
                            s.split(';')
                                .find_map(|kv| kv.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(prop)).and_then(|(_, v)| parse_color(v)))
                        })
                    };
                    let color = style_color("border-color").or_else(|| style_color("background-color")).map(nearest_palette).unwrap_or(0);
                    self.b.flush();
                    self.b.open(BlockKind::Callout { emoji: emoji.into(), color }, 0);
                    self.callout = true;
                    m.bg = 0;
                    self.walk(el, &m);
                    self.callout = false;
                    self.b.flush();
                } else if looks_heading {
                    let level = class.split("heading-h").nth(1).and_then(|s| s.chars().next()).and_then(|c| c.to_digit(10)).unwrap_or(1) as u8;
                    self.b.open(BlockKind::Heading { level }, 0);
                    self.walk(el, &m);
                    self.b.flush();
                } else if has_block_child(el) {
                    self.b.flush();
                    self.walk(el, &m);
                    self.b.flush();
                } else {
                    let in_li = self.b.cur.as_ref().is_some_and(|c| c.kind.is_list() && c.text.is_empty());
                    if !in_li {
                        let kind = if self.quote > 0 { BlockKind::Quote } else { BlockKind::Text };
                        self.b.open(kind, if self.lists.is_empty() { 0 } else { self.list_indent() });
                    }
                    self.walk(el, &m);
                    self.b.flush();
                }
            }
            "ul" | "ol" => {
                self.b.flush();
                self.lists.push(name == "ol");
                self.walk(el, &m);
                self.lists.pop();
                self.b.flush();
            }
            "li" => {
                let ordered = self.lists.last().copied().unwrap_or(false);
                let checkbox =
                    el.descendants().filter_map(scraper::ElementRef::wrap).find(|e| e.value().name() == "input" && e.value().attr("type") == Some("checkbox"));
                let kind = if let Some(cb) = checkbox {
                    BlockKind::Todo { checked: cb.value().attr("checked").is_some() }
                } else if let Some(state @ ("done" | "undone")) = v.attr("data-list") {
                    // 飞书待办：<li data-list="done|undone">
                    BlockKind::Todo { checked: state == "done" }
                } else if class.contains("todo") || class.contains("task") {
                    BlockKind::Todo { checked: class.contains("done") || class.contains("checked") }
                } else if ordered {
                    BlockKind::Ordered
                } else {
                    BlockKind::Bullet
                };
                self.b.open(kind, self.list_indent());
                self.walk(el, &m);
                self.b.flush();
            }
            "blockquote" => {
                self.b.flush();
                self.quote += 1;
                if !has_block_child(el) {
                    self.b.open(BlockKind::Quote, 0);
                }
                self.walk(el, &m);
                self.b.flush();
                self.quote -= 1;
            }
            "pre" => {
                let all = || std::iter::once(el).chain(el.descendants().filter_map(scraper::ElementRef::wrap));
                // 飞书：data-lark-language；其他站点：data-lang / data-language / class="language-xx"
                let lang = all()
                    .find_map(|e| ["data-lark-language", "data-lang", "data-language"].iter().find_map(|k| e.value().attr(k).map(str::to_string)))
                    .or_else(|| {
                        all()
                            .filter_map(|e| e.value().attr("class").map(|c| c.to_string()))
                            .find_map(|c| c.split_whitespace().find_map(|p| p.strip_prefix("language-").map(|s| s.to_string())))
                    })
                    .map(|l| normalize_lang(&l))
                    .unwrap_or_else(|| "Plain Text".into());
                let text: String = el.text().collect();
                self.b.open(BlockKind::Code { lang, wrap: true }, 0);
                self.b.push_raw(text.trim_end_matches('\n'));
                self.b.flush();
            }
            "hr" => {
                self.b.flush();
                self.b.blocks.push(Block::new(BlockKind::Divider, RichText::default()));
            }
            "br" => {
                if self.b.cur.is_some() {
                    self.b.push_raw("\n");
                } else {
                    self.b.blocks.push(Block::text(""));
                }
            }
            "img" => {
                if let Some(src) = v.attr("src") {
                    if let Some(path) = save_image_src(src, self.assets_dir) {
                        self.b.flush();
                        self.b.blocks.push(Block::new(BlockKind::image(path), RichText::default()));
                    }
                }
            }
            "table" => {
                self.b.flush();
                let mut rows: Vec<Vec<RichText>> = Vec::new();
                for tr in el.descendants().filter_map(scraper::ElementRef::wrap).filter(|e| e.value().name() == "tr") {
                    let mut row = Vec::new();
                    for cell in tr.children().filter_map(scraper::ElementRef::wrap).filter(|e| matches!(e.value().name(), "td" | "th")) {
                        let mut sub = HtmlWalker { b: Builder::default(), lists: vec![], quote: 0, assets_dir: self.assets_dir, callout: false };
                        let mut cm = Marks::default();
                        if let Some(style) = cell.value().attr("style") {
                            apply_style(style, &mut cm);
                        }
                        cm.bg = 0;
                        sub.walk(cell, &cm);
                        sub.b.flush();
                        let mut rt = RichText::default();
                        for (k, blk) in sub.b.blocks.iter().enumerate() {
                            if k > 0 {
                                rt.insert(rt.len(), "\n", Marks::default());
                            }
                            rt.append(&blk.text);
                        }
                        trim_rich(&mut rt);
                        row.push(rt);
                    }
                    if !row.is_empty() {
                        rows.push(row);
                    }
                }
                if !rows.is_empty() {
                    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
                    let n = rows.len();
                    let mut cells = Vec::new();
                    for mut r in rows {
                        r.resize(cols, RichText::default());
                        cells.extend(r);
                    }
                    self.b.blocks.push(Block::new(
                        BlockKind::Table { rows: n, cols, cells, header_row: true, header_col: false, widths: None, merges: Vec::new() },
                        RichText::default(),
                    ));
                }
            }
            "input" => {}
            _ => {
                // 行内元素
                match name.as_str() {
                    "b" | "strong" => m.bold = true,
                    "i" | "em" => m.italic = true,
                    "u" | "ins" => m.underline = true,
                    "s" | "del" | "strike" => m.strike = true,
                    "mark" => m.bg = 3,
                    "code" | "kbd" => m.code = true,
                    "a" => {
                        if let Some(href) = v.attr("href") {
                            if !href.starts_with('#') && !href.starts_with("javascript:") {
                                m.link = Some(href.to_string());
                            }
                        }
                    }
                    _ => {}
                }
                self.walk(el, &m);
            }
        }
    }
}

fn has_block_child(el: scraper::ElementRef) -> bool {
    el.children().filter_map(scraper::ElementRef::wrap).any(|c| {
        matches!(
            c.value().name(),
            "p" | "div"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "li"
                | "pre"
                | "table"
                | "blockquote"
                | "hr"
                | "section"
                | "article"
                | "figure"
                | "img"
        )
    })
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() && ch != '\u{a0}' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(if ch == '\u{a0}' { ' ' } else { ch });
            last_space = false;
        }
    }
    out
}

/// 把 HTML 里的图片落地到本地 assets 目录，返回本地路径。仅处理 data: 与本地文件。
fn save_image_src(src: &str, assets_dir: &Path) -> Option<String> {
    use base64::Engine;
    if let Some(rest) = src.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let ext = if meta.contains("png") {
            "png"
        } else if meta.contains("gif") {
            "gif"
        } else {
            "jpg"
        };
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
        return save_image_bytes(&bytes, ext, assets_dir);
    }
    // 网络图片：先保留地址，粘贴后由编辑器在后台下载到本地（D3）
    if src.starts_with("http://") || src.starts_with("https://") {
        return Some(src.to_string());
    }
    if let Some(rest) = src.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    let path = src.strip_prefix("file://").unwrap_or(src);
    if Path::new(path).is_file() {
        return Some(path.to_string());
    }
    None
}

pub fn save_image_bytes(bytes: &[u8], ext: &str, assets_dir: &Path) -> Option<String> {
    let path = assets_dir.join(format!("{}.{ext}", crate::store::new_uuid()));
    std::fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// 图片导出为 HTML 片段（HTML / PDF 导出与 Markdown 的内联降级共用）。
///
/// 显示宽度规则与编辑器一致：用户设定的宽度，否则原图宽（>1200px 折半）× 裁剪比例。
/// 裁剪用「外层固定窗口 + overflow:hidden，内层按 1/crop 放大并偏移」表达。
/// `src` 是写进 HTML 的地址（可能是 data URI），`path` 是本地文件，用来读原图尺寸。
fn image_html(src: &str, path: &str, alt: &str, width: Option<f32>, crop: Option<(f32, f32, f32, f32)>, align: u8) -> String {
    let align_css = match align {
        1 => "center",
        2 => "right",
        _ => "left",
    };
    let alt_attr = if alt.is_empty() { String::new() } else { format!(" alt=\"{}\"", html_escape(alt)) };
    let natural = image::image_dimensions(path).ok().map(|(w, h)| (w.max(1) as f32, h.max(1) as f32));
    let disp_w = |cw: f32| match (width, natural) {
        (Some(w), _) => Some(w),
        (None, Some((nw, _))) => Some(if nw > 1200. { nw * 0.5 } else { nw } * cw),
        (None, None) => None,
    };
    match crop {
        Some((cx, cy, cw, ch)) => {
            let dw = disp_w(cw).unwrap_or(600. * cw);
            let iw = dw / cw;
            // 内层高度按原图宽高比走；窗口高度 = 内层高度 × 裁剪高度比例
            let ih = match natural {
                Some((nw, nh)) => iw * nh / nw,
                None => iw * ch / cw,
            };
            format!(
                "<p style=\"text-align:{align_css}\"><span style=\"display:inline-block;overflow:hidden;width:{dw:.0}px;height:{:.0}px\">\
                 <img src=\"{src}\"{alt_attr} style=\"display:block;max-width:none;width:{iw:.0}px;margin-left:{:.0}px;margin-top:{:.0}px\"></span></p>",
                ih * ch,
                -iw * cx,
                -ih * cy
            )
        }
        None => {
            let style = width.map(|w| format!(" style=\"width:{w:.0}px\"")).unwrap_or_default();
            format!("<p style=\"text-align:{align_css}\"><img src=\"{src}\"{alt_attr}{style}></p>")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_roundtrip_basic() {
        let md = "# 标题\n\n正文 **粗体** 和 `代码`\n\n- a\n    - b\n1. one\n2. two\n- [x] done\n\n> 引用\n\n```rust\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n---\n";
        let blocks = md_to_blocks(md);
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.label()).collect();
        assert_eq!(kinds, vec!["一级标题", "正文", "无序列表", "无序列表", "有序列表", "有序列表", "待办", "引用", "代码块", "表格", "分割线"]);
        assert_eq!(blocks[3].indent, 1);
        assert!(blocks[1].text.spans.iter().any(|s| s.marks.bold));
        let back = blocks_to_md(&blocks.iter().collect::<Vec<_>>());
        assert!(back.contains("**粗体**"));
        assert!(back.contains("| 1 | 2 |"));
    }

    #[test]
    fn html_import_web_like() {
        let html = r#"<html><body><h1>大标题</h1><p>普通<span style="font-weight:bold">加粗</span><span style="color: rgb(216, 57, 49)">红字</span></p>
        <ul><li>一<ul><li>二</li></ul></li></ul><ol><li>甲</li></ol>
        <pre><code class="language-python">print(1)</code></pre>
        <blockquote>名言</blockquote><table><tr><td>a</td><td>b</td></tr></table><hr></body></html>"#;
        let blocks = html_to_blocks(html, Path::new("/tmp"));
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.label()).collect();
        assert_eq!(kinds, vec!["一级标题", "正文", "无序列表", "无序列表", "有序列表", "代码块", "引用", "表格", "分割线"]);
        assert_eq!(blocks[3].indent, 1);
        let p = &blocks[1].text;
        assert!(p.spans.iter().any(|s| s.marks.bold));
        assert!(p.spans.iter().any(|s| s.marks.color == 1));
        assert_eq!(blocks[5].kind, BlockKind::Code { lang: "Python".into(), wrap: true });
    }

    #[test]
    fn image_export_keeps_style() {
        // 未设样式：保持纯 Markdown 语法
        let plain = Block::new(BlockKind::image("/tmp/a.png"), RichText::plain(""));
        let md = blocks_to_md(&[&plain]);
        assert_eq!(md.trim(), "![](/tmp/a.png)");

        // 设了宽度 / 对齐 / 描述：降级为内联 HTML，信息不丢
        let mut styled = Block::new(BlockKind::image("/tmp/a.png"), RichText::plain(""));
        styled.align = 1;
        if let BlockKind::Image { alt, width, .. } = &mut styled.kind {
            *alt = "示意图".into();
            *width = Some(320.);
        }
        let md = blocks_to_md(&[&styled]);
        assert!(md.contains("text-align:center"), "{md}");
        assert!(md.contains("alt=\"示意图\""), "{md}");
        assert!(md.contains("width:320px"), "{md}");

        // HTML 导出：裁剪用 outer overflow + inner 放大偏移表达
        let mut cropped = Block::new(BlockKind::image("/tmp/not-exist.png"), RichText::plain(""));
        if let BlockKind::Image { alt, width, crop, .. } = &mut cropped.kind {
            *alt = "裁过".into();
            *width = Some(320.);
            *crop = Some((0.1, 0.2, 0.5, 0.6));
        }
        let html = blocks_to_html(&[&cropped]);
        assert!(html.contains("overflow:hidden"), "{html}");
        assert!(html.contains("width:320px"), "{html}");
        // 内层宽度 = 320 / 0.5
        assert!(html.contains("width:640px"), "{html}");
    }

    #[test]
    fn cropped_image_window_follows_aspect_ratio() {
        // 800×400 的图裁左上 1/4，显示宽 200：内层 400×200，窗口应为 200×100
        let dir = std::env::temp_dir().join(format!("stonebrook-crop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wide.png");
        image::RgbImage::new(800, 400).save(&path).unwrap();
        let mut b = Block::new(BlockKind::image(path.to_string_lossy().to_string()), RichText::plain(""));
        if let BlockKind::Image { width, crop, .. } = &mut b.kind {
            *width = Some(200.);
            *crop = Some((0., 0., 0.5, 0.5));
        }
        let html = blocks_to_html(&[&b]);
        assert!(html.contains("width:200px;height:100px"), "{html}");
        // Markdown 降级同样带上裁剪窗口
        let md = blocks_to_md(&[&b]);
        assert!(md.contains("overflow:hidden") && md.contains("height:100px"), "{md}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn image_box_respects_crop_and_width() {
        let mut b = Block::new(BlockKind::image("/tmp/a.png"), RichText::plain(""));
        // 无裁剪、无宽度：显示原始像素
        assert_eq!(b.kind.image_box((512, 512), 800.), Some((512., 512.)));
        // 宽度优先，高度按裁剪后的宽高比
        if let BlockKind::Image { width, crop, .. } = &mut b.kind {
            *width = Some(200.);
            *crop = Some((0., 0., 0.5, 0.25));
        }
        assert_eq!(b.kind.image_box((512, 512), 800.), Some((200., 100.)));
        // 超过可用宽度时收敛到 max_w
        if let BlockKind::Image { width, .. } = &mut b.kind {
            *width = None;
        }
        if let BlockKind::Image { crop, .. } = &mut b.kind {
            *crop = Some((0., 0., 1., 1.));
        }
        assert_eq!(b.kind.image_box((512, 512), 300.), Some((300., 300.)));
    }

    #[test]
    fn numbering() {
        let mut v = vec![Block::new(BlockKind::Ordered, RichText::plain("a")); 3];
        v[1].indent = 1;
        let refs: Vec<&Block> = v.iter().collect();
        assert_eq!(ordered_numbers(&refs), vec![1, 1, 2]);
        assert_eq!(ordered_label(2, 1), "b.");
        assert_eq!(ordered_label(4, 2), "iv.");
    }

    #[test]
    fn feishu_paste_todos_callouts_and_code() {
        // 飞书网页复制出来的 HTML（结构取自 quiet.md 的粘贴测试）
        let html = "<ul class=\"list-done1\"><li data-list=\"done\"><div>完成了</div></li><li data-list=\"undone\"><div>还没做</div></li></ul>\
            <div class=\"zoneType-calloutBlock\"><div class=\"callout-container\" data-emoji-id=\"chestnut\">\
            <div class=\"callout-block\" style=\"background-color:rgb(255,245,235);border-color:rgb(254,212,164);\"><div>第一行</div><div>第二行</div></div></div></div>\
            <pre data-lark-language=\"Python\"><code>print(1)</code></pre>";
        let bs = html_to_blocks(html, std::path::Path::new("/tmp"));
        let kinds: Vec<_> = bs.iter().map(|b| (b.kind.clone(), b.text.text.clone())).collect();
        assert_eq!(kinds[0], (BlockKind::Todo { checked: true }, "完成了".to_string()));
        assert_eq!(kinds[1], (BlockKind::Todo { checked: false }, "还没做".to_string()));
        assert_eq!(kinds[2], (BlockKind::Callout { emoji: "🌰".into(), color: 2 }, "第一行\n第二行".to_string()), "{kinds:#?}");
        assert!(matches!(&kinds[3].0, BlockKind::Code { lang, .. } if lang.eq_ignore_ascii_case("python")), "{kinds:#?}");
        assert_eq!(bs.len(), 4, "{kinds:#?}");
        assert!(bs[2].text.runs().all(|(_, m)| m.bg == 0), "高亮块底色不应变成文字底色");
    }
}
