//! 代码块折叠：按语言算出可折叠的行区间。
//!
//! 行号从 0 起。`FoldSpan.end` 那一行会和中间的行一起被收起（和飞书一致：
//! 收起后起始行末尾显示「⋯」，结束行的换行保留，用于和下一行分隔）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FoldSpan {
    /// 起始行（保留显示）
    pub start: usize,
    /// 结束行（随折叠一起隐藏）
    pub end: usize,
}

/// 每行的起始字节偏移。
pub(crate) fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

/// 字节偏移所在的行号（0 起）。
pub(crate) fn line_of_offset(text: &str, off: usize) -> usize {
    line_of(&line_starts(text), off)
}

fn line_of(starts: &[usize], off: usize) -> usize {
    match starts.binary_search(&off) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// 可折叠区间（按下标升序、同一起始行只保留最大的一个）。
pub(crate) fn spans(lang: &str, text: &str) -> Vec<FoldSpan> {
    let l = lang.to_ascii_lowercase();
    let mut v = match l.as_str() {
        "" | "plain text" | "text" | "markdown" | "md" => vec![],
        "python" | "py" => python(text),
        "html" | "xml" | "vue" | "svelte" | "svg" => html(text),
        _ => brackets(lang, text),
    };
    v.extend(comments(lang, text));
    v.retain(|s| s.end > s.start);
    v.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    v.dedup_by_key(|s| s.start);
    v
}

/// 大括号 / 方括号 / 圆括号配对（跳过字符串和注释里的括号）。
fn brackets(lang: &str, text: &str) -> Vec<FoldSpan> {
    use crate::highlight::{highlight, Tok};
    let starts = line_starts(text);
    let toks = highlight(lang, text);
    let mut ti = 0usize;
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut out = Vec::new();
    for (i, b) in text.bytes().enumerate() {
        while ti < toks.len() && toks[ti].0.end <= i {
            ti += 1;
        }
        if toks.get(ti).is_some_and(|(r, t)| r.contains(&i) && matches!(t, Tok::String | Tok::Comment)) {
            continue;
        }
        match b {
            b'{' | b'[' | b'(' => stack.push((b, i)),
            b'}' | b']' | b')' => {
                let want = match b {
                    b'}' => b'{',
                    b']' => b'[',
                    _ => b'(',
                };
                if let Some(pos) = stack.iter().rposition(|&(o, _)| o == want) {
                    let (_, open) = stack[pos];
                    stack.truncate(pos);
                    let (s, e) = (line_of(&starts, open), line_of(&starts, i));
                    if e > s {
                        out.push(FoldSpan { start: s, end: e });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// 多行注释块（`/* … */`、`"""…"""` 等，取决于语言的高亮实现）。
fn comments(lang: &str, text: &str) -> Vec<FoldSpan> {
    use crate::highlight::{highlight, Tok};
    let starts = line_starts(text);
    highlight(lang, text)
        .into_iter()
        .filter(|(_, t)| matches!(t, Tok::Comment))
        .filter_map(|(r, _)| {
            let s = line_of(&starts, r.start);
            let e = line_of(&starts, r.end.saturating_sub(1).max(r.start));
            (e > s).then_some(FoldSpan { start: s, end: e })
        })
        .collect()
}

/// Python：以 `:` 结尾的行，按缩进收起整个块。
fn python(text: &str) -> Vec<FoldSpan> {
    let lines: Vec<&str> = text.split('\n').collect();
    let indent = |s: &str| s.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
    let mut out = Vec::new();
    for i in 0..lines.len() {
        let t = lines[i].trim_end();
        let code = match t.find('#') {
            Some(p) => &t[..p],
            None => t,
        };
        if !code.trim_end().ends_with(':') {
            continue;
        }
        let base = indent(lines[i]);
        let mut j = i + 1;
        let mut last = i;
        while j < lines.len() {
            let l = lines[j];
            if l.trim().is_empty() {
                j += 1;
                continue;
            }
            if indent(l) <= base {
                break;
            }
            last = j;
            j += 1;
        }
        if last > i {
            out.push(FoldSpan { start: i, end: last });
        }
    }
    out
}

/// HTML / XML：标签配对，跨多行的整块可折叠。
fn html(text: &str) -> Vec<FoldSpan> {
    const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
    let starts = line_starts(text);
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if !rest.starts_with('<') {
            i += 1;
            continue;
        }
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(e) => {
                    i += e + 3;
                    continue;
                }
                None => break,
            }
        }
        let Some(e) = rest.find('>') else { break };
        let tag_start = i;
        let inner = rest[1..e].trim();
        i += e + 1;
        if inner.is_empty() || inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        let closing = inner.starts_with('/');
        let self_close = inner.ends_with('/');
        let name: String = inner.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("").to_ascii_lowercase();
        if name.is_empty() || VOID.contains(&name.as_str()) {
            continue;
        }
        if closing {
            if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                let (_, open) = stack[pos].clone();
                stack.truncate(pos);
                let (s, e) = (line_of(&starts, open), line_of(&starts, tag_start));
                if e > s {
                    out.push(FoldSpan { start: s, end: e });
                }
            }
        } else if !self_close {
            stack.push((name, tag_start));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(lang: &str, text: &str) -> Vec<(usize, usize)> {
        spans(lang, text).into_iter().map(|s| (s.start, s.end)).collect()
    }

    #[test]
    fn json_braces_and_arrays() {
        let text = "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"c\": [\n    1\n  ]\n}";
        let v = get("JSON", text);
        assert!(v.contains(&(0, 7)), "{v:?}");
        assert!(v.contains(&(1, 3)), "{v:?}");
        assert!(v.contains(&(4, 6)), "{v:?}");
    }

    #[test]
    fn braces_inside_strings_ignored() {
        let text = "const a = \"{\";\nfunction f() {\n  return 1;\n}";
        let v = get("JavaScript", text);
        assert!(v.contains(&(1, 3)), "{v:?}");
        // 字符串里的 { 不会和后面的 } 配上产生 (0,3)
        assert!(!v.contains(&(0, 3)), "{v:?}");
    }

    #[test]
    fn python_indentation_blocks() {
        let text = "def f(x):\n    if x:\n        return 1\n    return 0\nprint(1)";
        let v = get("Python", text);
        assert!(v.contains(&(0, 3)), "{v:?}");
        assert!(v.contains(&(1, 2)), "{v:?}");
    }

    #[test]
    fn html_tag_blocks() {
        let text = "<div>\n  <p>\n    hi\n  </p>\n</div>\n<span>x</span>";
        let v = get("HTML", text);
        assert!(v.contains(&(0, 4)), "{v:?}");
        assert!(v.contains(&(1, 3)), "{v:?}");
    }

    #[test]
    fn comment_blocks_fold() {
        let text = "/*\n * 注释\n */\ncode();";
        let v = get("JavaScript", text);
        assert!(v.contains(&(0, 2)), "{v:?}");
    }

    #[test]
    fn plain_text_not_folded() {
        assert!(get("Plain Text", "a {\nb\n}").is_empty());
    }
}
