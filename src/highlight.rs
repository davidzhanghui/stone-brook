//! 轻量代码高亮：一个通用的单遍扫描器，按语言切换关键字表与注释语法。
//! 文档代码块（每帧，带缓存）、文本编辑框、附件里的代码文件预览（HTML）共用这一套。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    /// 连线箭头（`-->`、`->>`、`<|--` 等，Mermaid / PlantUML）
    Arrow,
}

pub const LANGS: &[&str] = &[
    "Plain Text",
    "Rust",
    "Python",
    "JavaScript",
    "TypeScript",
    "Go",
    "Java",
    "C",
    "C++",
    "Shell",
    "SQL",
    "JSON",
    "YAML",
    "HTML",
    "CSS",
    "Markdown",
    "Kotlin",
    "Swift",
    "Mermaid",
    "LaTeX",
];

/// 语言菜单的搜索：按名称与常见别名（py / js / ts / rs…）过滤，大小写不敏感。
pub fn filter_langs(query: &str) -> Vec<&'static str> {
    LANGS.iter().copied().filter(|l| lang_matches(l, query)).collect()
}

fn lang_matches(lang: &str, query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    let name = lang.to_ascii_lowercase();
    if name.contains(&q) {
        return true;
    }
    let aliases = match name.as_str() {
        "plain text" => "text txt plain",
        "rust" => "rs",
        "python" => "py",
        "javascript" => "js jsx",
        "typescript" => "ts tsx",
        "shell" => "sh bash zsh",
        "markdown" => "md",
        "yaml" => "yml",
        "html" => "xml",
        "c++" => "cpp",
        "kotlin" => "kt",
        "latex" => "tex",
        "mermaid" => "flowchart",
        _ => "",
    };
    aliases.split_whitespace().any(|a| a.starts_with(&q))
}

struct Syntax {
    keywords: &'static [&'static str],
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
}

fn syntax_for(lang: &str) -> Option<Syntax> {
    let l = lang.to_ascii_lowercase();
    let c_like_comments = (&["//"][..], Some(("/*", "*/")));
    let (keywords, (line_comment, block_comment)): (&[&str], (&[&str], Option<(&str, &str)>)) = match l.as_str() {
        "rust" | "rs" => (
            &[
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod", "match", "if", "else", "for", "while", "loop", "return", "self",
                "Self", "crate", "super", "as", "in", "where", "async", "await", "move", "ref", "const", "static", "type", "dyn", "unsafe", "true", "false",
                "break", "continue",
            ],
            c_like_comments,
        ),
        "python" | "py" => (
            &[
                "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while", "return", "in", "not", "and", "or", "is", "None", "True",
                "False", "with", "try", "except", "finally", "raise", "lambda", "yield", "async", "await", "pass", "break", "continue", "global", "self",
            ],
            (&["#"][..], None),
        ),
        "javascript" | "js" | "typescript" | "ts" | "jsx" | "tsx" => (
            &[
                "function",
                "const",
                "let",
                "var",
                "if",
                "else",
                "for",
                "while",
                "return",
                "class",
                "extends",
                "new",
                "this",
                "import",
                "from",
                "export",
                "default",
                "async",
                "await",
                "try",
                "catch",
                "finally",
                "throw",
                "typeof",
                "instanceof",
                "in",
                "of",
                "true",
                "false",
                "null",
                "undefined",
                "interface",
                "type",
                "enum",
                "implements",
                "public",
                "private",
                "readonly",
                "switch",
                "case",
                "break",
            ],
            c_like_comments,
        ),
        "go" | "golang" => (
            &[
                "func",
                "package",
                "import",
                "var",
                "const",
                "type",
                "struct",
                "interface",
                "map",
                "chan",
                "go",
                "defer",
                "if",
                "else",
                "for",
                "range",
                "return",
                "switch",
                "case",
                "default",
                "select",
                "nil",
                "true",
                "false",
                "break",
                "continue",
            ],
            c_like_comments,
        ),
        "java" | "kotlin" | "kt" | "swift" | "c" | "c++" | "cpp" | "h" => (
            &[
                "class",
                "public",
                "private",
                "protected",
                "static",
                "final",
                "void",
                "int",
                "long",
                "double",
                "float",
                "char",
                "bool",
                "boolean",
                "return",
                "if",
                "else",
                "for",
                "while",
                "new",
                "this",
                "import",
                "package",
                "extends",
                "implements",
                "interface",
                "try",
                "catch",
                "throw",
                "throws",
                "null",
                "true",
                "false",
                "const",
                "struct",
                "include",
                "namespace",
                "template",
                "auto",
                "fun",
                "val",
                "var",
                "func",
                "let",
                "nil",
                "override",
                "switch",
                "case",
                "break",
            ],
            c_like_comments,
        ),
        "shell" | "bash" | "sh" | "zsh" => (
            &[
                "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "case", "esac", "function", "return", "export", "local", "echo", "cd", "in",
                "source",
            ],
            (&["#"][..], None),
        ),
        "sql" => (
            &[
                "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "join", "left", "right",
                "inner", "on", "group", "by", "order", "limit", "and", "or", "not", "null", "as", "primary", "key", "index", "SELECT", "FROM", "WHERE",
                "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "JOIN", "LEFT", "ON", "GROUP", "BY", "ORDER", "LIMIT", "AND", "OR",
                "NOT", "NULL", "AS",
            ],
            (&["--"][..], Some(("/*", "*/"))),
        ),
        "json" => (&["true", "false", "null"], (&[][..], None)),
        "yaml" | "yml" => (&["true", "false", "null"], (&["#"][..], None)),
        "css" => (&["important"], (&[][..], Some(("/*", "*/")))),
        "mermaid" => (
            &[
                "flowchart",
                "graph",
                "subgraph",
                "end",
                "direction",
                "sequenceDiagram",
                "participant",
                "actor",
                "loop",
                "alt",
                "else",
                "opt",
                "par",
                "and",
                "note",
                "Note",
                "over",
                "classDiagram",
                "class",
                "stateDiagram",
                "state",
                "erDiagram",
                "gantt",
                "section",
                "title",
                "dateFormat",
                "pie",
                "mindmap",
                "timeline",
                "classDef",
                "style",
                "linkStyle",
                "activate",
                "deactivate",
                "autonumber",
                "TD",
                "TB",
                "LR",
                "RL",
                "BT",
                "of",
                "left",
                "right",
                "as",
                "critical",
                "break",
                "rect",
            ],
            (&["%%"][..], None),
        ),
        "plantuml" | "puml" => (
            &[
                "startuml",
                "enduml",
                "startmindmap",
                "endmindmap",
                "startwbs",
                "endwbs",
                "actor",
                "participant",
                "boundary",
                "control",
                "entity",
                "database",
                "collections",
                "queue",
                "as",
                "activate",
                "deactivate",
                "destroy",
                "autonumber",
                "alt",
                "else",
                "opt",
                "loop",
                "par",
                "break",
                "critical",
                "group",
                "end",
                "note",
                "over",
                "of",
                "left",
                "right",
                "top",
                "bottom",
                "ref",
                "title",
                "start",
                "stop",
                "if",
                "then",
                "elseif",
                "endif",
                "while",
                "endwhile",
                "repeat",
                "is",
                "not",
                "fork",
                "again",
                "split",
                "partition",
                "class",
                "abstract",
                "interface",
                "enum",
                "package",
                "namespace",
                "usecase",
                "rectangle",
                "component",
                "node",
                "cloud",
                "frame",
                "folder",
                "state",
                "skinparam",
                "direction",
                "to",
                "hide",
                "show",
                "return",
                "box",
                "detach",
                "kill",
            ],
            (&["'"][..], Some(("/'", "'/"))),
        ),
        "latex" | "tex" => (&[], (&["%"][..], None)),
        "html" | "xml" => (&[], (&[][..], Some(("<!--", "-->")))),
        // 认不出的代码文件：只标字符串 / 数字 / 常见注释
        "generic" => (&[], (&["#", "//"][..], Some(("/*", "*/")))),
        _ => return None,
    };
    Some(Syntax { keywords, line_comment, block_comment })
}

pub fn highlight(lang: &str, text: &str) -> Vec<(Range<usize>, Tok)> {
    let l = lang.to_ascii_lowercase();
    match l.as_str() {
        "markdown" | "md" => return highlight_markdown(text),
        "html" | "xml" => return highlight_markup(text),
        "css" => return highlight_css(text),
        "yaml" | "yml" => return highlight_yaml(text),
        "latex" | "tex" => return highlight_latex(text),
        _ => {}
    }
    let Some(syn) = syntax_for(lang) else {
        return vec![];
    };
    scan_syntax(&l, syn, text)
}

/// 通用单遍扫描器：注释 / 字符串 / 数字 / 关键字 / 函数名 / 类型名。
fn scan_syntax(l: &str, syn: Syntax, text: &str) -> Vec<(Range<usize>, Tok)> {
    let arrows = matches!(l, "mermaid" | "plantuml" | "puml");
    // PlantUML 的单引号是注释，不是字符串
    let quote_comment = matches!(l, "plantuml" | "puml");
    let keywords: HashSet<&str> = syn.keywords.iter().copied().collect();
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some(lc) = syn.line_comment.iter().find(|lc| rest.starts_with(**lc)) {
            let _ = lc;
            let end = rest.find('\n').map(|e| i + e).unwrap_or(text.len());
            out.push((i..end, Tok::Comment));
            i = end;
            continue;
        }
        if let Some((open, close)) = syn.block_comment {
            if let Some(after) = rest.strip_prefix(open) {
                let end = after.find(close).map(|e| i + open.len() + e + close.len()).unwrap_or(text.len());
                out.push((i..end, Tok::Comment));
                i = end;
                continue;
            }
        }
        let b = bytes[i];
        if arrows && matches!(b, b'-' | b'.' | b'=' | b'<' | b'>' | b'|' | b'*' | b'o' | b'x' | b'}' | b'{') {
            if let Some(n) = arrow_len(rest) {
                out.push((i..i + n, Tok::Arrow));
                i += n;
                continue;
            }
        }
        if b == b'"' || (b == b'\'' && !quote_comment) || b == b'`' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b && bytes[j] != b'\n' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(bytes.len());
            out.push((i..end, Tok::String));
            i = end;
            continue;
        }
        if b.is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_') {
                j += 1;
            }
            out.push((i..j, Tok::Number));
            i = j;
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &text[i..j];
            if keywords.contains(word) {
                out.push((i..j, Tok::Keyword));
            } else if arrows {
                // 图表代码里的节点名 / 别名不着色
            } else if bytes.get(j) == Some(&b'(') || bytes.get(j) == Some(&b'!') {
                out.push((i..j, Tok::Function));
            } else if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && word.len() > 1 {
                out.push((i..j, Tok::Type));
            }
            i = j;
            continue;
        }
        // 跳过一个完整字符（可能是多字节）
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// Markdown 没有关键字表，按行结构上色：标题 / 引用 / 列表 / 分隔线 / 围栏代码，
/// 以及行内的强调标记、行内代码和链接。
fn highlight_markdown(text: &str) -> Vec<(Range<usize>, Tok)> {
    let mut out = Vec::new();
    let mut i = 0;
    // 围栏内的代码不参与行内扫描，只等结束围栏
    let mut fence: Option<u8> = None;
    while i < text.len() {
        let line_end = text[i..].find('\n').map_or(text.len(), |e| i + e);
        let line = &text[i..line_end];
        let trimmed = line.trim_start_matches([' ', '\t']);
        let start = i + (line.len() - trimmed.len());
        let next = if line_end < text.len() { line_end + 1 } else { line_end };

        let fence_char = (*b"`~").into_iter().find(|c| trimmed.as_bytes().starts_with(&[*c, *c, *c]));
        if let Some(c) = fence {
            if fence_char == Some(c) {
                let n = trimmed.bytes().take_while(|b| *b == c).count();
                out.push((start..start + n, Tok::Comment));
                fence = None;
            }
            i = next;
            continue;
        }
        if let Some(c) = fence_char {
            let n = trimmed.bytes().take_while(|b| *b == c).count();
            out.push((start..start + n, Tok::Comment));
            // 围栏后的语言名
            let tail = &trimmed[n..];
            let lang = tail.trim();
            if !lang.is_empty() {
                let ls = start + n + (tail.len() - tail.trim_start().len());
                out.push((ls..ls + lang.len(), Tok::Type));
            }
            fence = Some(c);
            i = next;
            continue;
        }

        let rest = &text[start..line_end];
        let mut p = start;
        if rest.starts_with('#') {
            let n = rest.bytes().take_while(|b| *b == b'#').count();
            // 「#标题」不算标题，必须有空格或行尾
            if n <= 6 && (rest.len() == n || rest.as_bytes().get(n) == Some(&b' ')) {
                out.push((start..start + n, Tok::Keyword));
                p = start + n;
            }
        } else if rest.starts_with('>') {
            out.push((start..start + 1, Tok::Keyword));
            p = start + 1;
        } else if is_hr(rest) {
            out.push((start..line_end, Tok::Keyword));
            i = next;
            continue;
        } else if rest.len() >= 2 && matches!(rest.as_bytes()[0], b'-' | b'*' | b'+') && rest.as_bytes()[1] == b' ' {
            out.push((start..start + 1, Tok::Keyword));
            p = start + 1;
        } else if let Some(n) = ordered_marker(rest) {
            out.push((start..start + n, Tok::Keyword));
            p = start + n;
        }
        md_inline(text, p, line_end, &mut out);
        i = next;
    }
    out
}

/// 分隔线：至少三个相同的 `-`、`*` 或 `_`。
fn is_hr(s: &str) -> bool {
    let t = s.trim_end();
    let b = t.as_bytes();
    b.len() >= 3 && matches!(b[0], b'-' | b'*' | b'_') && b.iter().all(|c| *c == b[0])
}

/// 有序列表标记（`1. ` / `1) `）的字节长度。
fn ordered_marker(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 || n > 9 {
        return None;
    }
    if matches!(b.get(n), Some(&b'.') | Some(&b')')) && b.get(n + 1) == Some(&b' ') {
        Some(n + 1)
    } else {
        None
    }
}

/// 行内：反引号代码、强调标记、链接 / 图片（URL 用字符串色）。
fn md_inline(text: &str, mut i: usize, end: usize, out: &mut Vec<(Range<usize>, Tok)>) {
    let b = text.as_bytes();
    while i < end {
        match b[i] {
            b'`' => {
                let n = b[i..end].iter().take_while(|c| **c == b'`').count();
                match md_find_run(b, i + n, end, b'`', n) {
                    Some(e) => {
                        out.push((i..e, Tok::String));
                        i = e;
                    }
                    None => i += n,
                }
            }
            b'*' | b'_' => {
                // snake_case 里的下划线不是强调
                let word = i > 0 && b[i - 1].is_ascii_alphanumeric() && b.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric());
                if b[i] == b'*' || !word {
                    out.push((i..i + 1, Tok::Keyword));
                }
                i += 1;
            }
            b'[' => {
                if let Some(close) = text[i + 1..end].find(']').map(|p| i + 1 + p) {
                    if b.get(close + 1) == Some(&b'(') {
                        if let Some(rp) = text[close + 2..end].find(')').map(|p| close + 2 + p) {
                            out.push((i..i + 1, Tok::Keyword));
                            out.push((close..close + 2, Tok::Keyword));
                            if rp > close + 2 {
                                out.push((close + 2..rp, Tok::String));
                            }
                            out.push((rp..rp + 1, Tok::Keyword));
                            i = rp + 1;
                            continue;
                        }
                    }
                }
                i += 1;
            }
            b'\\' if i + 1 < end => {
                i += 1 + text[i + 1..end].chars().next().map_or(0, |c| c.len_utf8());
            }
            _ => i += text[i..end].chars().next().map_or(1, |c| c.len_utf8()),
        }
    }
}

/// 从 `from` 起找长度恰为 `n` 的 `ch` 连续串，返回串后的位置。
fn md_find_run(b: &[u8], mut i: usize, end: usize, ch: u8, n: usize) -> Option<usize> {
    while i < end {
        if b[i] == ch {
            let m = b[i..end].iter().take_while(|c| **c == ch).count();
            if m == n {
                return Some(i + m);
            }
            i += m;
        } else {
            i += 1;
        }
    }
    None
}

/// HTML / XML：注释、标签名、属性名、属性值。
fn highlight_markup(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if text[i..].starts_with("<!--") {
            let end = text[i + 4..].find("-->").map(|e| i + 4 + e + 3).unwrap_or(text.len());
            out.push((i..end, Tok::Comment));
            i = end;
            continue;
        }
        if b[i] != b'<' {
            i += text[i..].chars().next().map_or(1, |c| c.len_utf8());
            continue;
        }
        let mut j = i + 1;
        if b.get(j) == Some(&b'/') {
            j += 1;
        }
        if b.get(j) == Some(&b'!') {
            // <!DOCTYPE html>
            let end = text[j..].find('>').map(|e| j + e + 1).unwrap_or(text.len());
            out.push((i..end, Tok::Comment));
            i = end;
            continue;
        }
        let name_start = j;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b':' | b'_')) {
            j += 1;
        }
        if j == name_start {
            i += 1;
            continue;
        }
        out.push((i..j, Tok::Keyword));
        // 标签内的属性：名字 → 类型色，引号里的值 → 字符串色
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() || b[j] == b'>' {
                break;
            }
            if b[j] == b'/' {
                j += 1;
                continue;
            }
            if b[j] == b'"' || b[j] == b'\'' {
                let q = b[j];
                let mut k = j + 1;
                while k < b.len() && b[k] != q {
                    k += 1;
                }
                let end = (k + 1).min(b.len());
                out.push((j..end, Tok::String));
                j = end;
                continue;
            }
            let a = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/' | b'"' | b'\'') {
                j += 1;
            }
            if j == a {
                j += 1; // 「=」等标点
            } else {
                out.push((a..j, Tok::Type));
            }
        }
        if b.get(j) == Some(&b'>') {
            j += 1;
        }
        i = j;
    }
    out
}

/// CSS：注释、选择器、属性名、字符串、数字（含颜色值）。
fn highlight_css(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        if text[i..].starts_with("/*") {
            let end = text[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(text.len());
            out.push((i..end, Tok::Comment));
            i = end;
            continue;
        }
        match b[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = (depth - 1).max(0);
                i += 1;
            }
            b'"' | b'\'' => {
                let q = b[i];
                let mut j = i + 1;
                while j < b.len() && b[j] != q && b[j] != b'\n' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                out.push((i..end, Tok::String));
                i = end;
            }
            b'@' => {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                    j += 1;
                }
                out.push((i..j, Tok::Keyword));
                i = j;
            }
            b'#' if b.get(i + 1).is_some_and(|c| c.is_ascii_hexdigit()) => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_hexdigit() {
                    j += 1;
                }
                // #fff / #ff0000 是颜色，其余是 id 选择器
                if matches!(j - i - 1, 3 | 4 | 6 | 8) && b.get(j).is_none_or(|c| !c.is_ascii_alphanumeric()) {
                    out.push((i..j, Tok::Number));
                    i = j;
                } else {
                    let mut k = i + 1;
                    while k < b.len() && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'-' | b'_')) {
                        k += 1;
                    }
                    out.push((i..k, Tok::Type));
                    i = k;
                }
            }
            b'.' if b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-' || *c == b'_') => {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b'_')) {
                    j += 1;
                }
                out.push((i..j, Tok::Type));
                i = j;
            }
            c if c.is_ascii_alphabetic() || c == b'-' || c == b'_' => {
                let a = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'-' | b'_')) {
                    i += 1;
                }
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) == Some(&b':') && depth > 0 {
                    out.push((a..i, Tok::Keyword)); // 属性名
                } else if depth == 0 {
                    out.push((a..i, Tok::Type)); // 选择器
                }
            }
            c if c.is_ascii_digit() => {
                let a = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                out.push((a..i, Tok::Number));
            }
            _ => i += text[i..].chars().next().map_or(1, |c| c.len_utf8()),
        }
    }
    out
}

/// YAML：在通用扫描（注释 / 字符串 / 数字 / true false null）之上补充键名与列表标记。
fn highlight_yaml(text: &str) -> Vec<(Range<usize>, Tok)> {
    let mut out = scan_syntax("yaml", syntax_for("yaml").expect("yaml 语法"), text);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let line_end = text[i..].find('\n').map_or(text.len(), |e| i + e);
        let next = if line_end < text.len() { line_end + 1 } else { line_end };
        let line = &text[i..line_end];
        let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
        let mut p = i + lead;
        if text[p..line_end].starts_with("- ") {
            out.push((p..p + 1, Tok::Keyword));
            p += 1;
        }
        let rest = &text[p..line_end];
        if let Some(cp) = rest.find(':') {
            let key = rest[..cp].trim_end();
            let plain = !key.is_empty()
                && !key.starts_with('"')
                && !key.starts_with('\'')
                && key.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'$' | b'/'));
            if plain {
                out.push((p..p + key.len(), Tok::Type));
            }
        }
        i = next;
    }
    out.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
    // 与已有 token 重叠的键名（位于字符串 / 注释里）优先保留已有 token
    let mut kept: Vec<(Range<usize>, Tok)> = Vec::with_capacity(out.len());
    for (r, k) in out {
        if kept.last().is_none_or(|(last, _)| last.end <= r.start) {
            kept.push((r, k));
        }
    }
    kept
}

/// LaTeX：反斜杠命令、`%` 注释、数字。
fn highlight_latex(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let end = text[i..].find('\n').map_or(text.len(), |e| i + e);
                out.push((i..end, Tok::Comment));
                i = end;
            }
            b'\\' => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j > i + 1 {
                    out.push((i..j, Tok::Keyword));
                    i = j;
                } else {
                    // \% \\ \{ 之类的转义字符
                    i += 2.min(b.len() - i);
                }
            }
            c if c.is_ascii_digit() => {
                let a = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                out.push((a..i, Tok::Number));
            }
            _ => i += text[i..].chars().next().map_or(1, |c| c.len_utf8()),
        }
    }
    out
}

type Tokens = Rc<Vec<(Range<usize>, Tok)>>;

thread_local! {
    /// (语言, 文本哈希, 文本长度) → 高亮结果。代码块每帧都要高亮，文字不变就直接复用。
    static CACHE: RefCell<HashMap<(String, u64, usize), Tokens>> = RefCell::new(HashMap::new());
}

const CACHE_CAP: usize = 512;

/// 带缓存的 `highlight`（渲染路径用）。
pub fn highlight_cached(lang: &str, text: &str) -> Tokens {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    let key = (lang.to_string(), h.finish(), text.len());
    CACHE.with(|c| {
        if let Some(t) = c.borrow().get(&key) {
            return t.clone();
        }
        let t: Tokens = Rc::new(highlight(lang, text));
        let mut c = c.borrow_mut();
        if c.len() >= CACHE_CAP {
            c.clear();
        }
        c.insert(key, t.clone());
        t
    })
}

/// 文件扩展名 → 高亮语言（附件代码预览用）。纯文本类返回 None，其余认不出的按通用规则。
pub fn lang_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "txt" | "log" | "csv" | "tsv" | "env" | "md" | "markdown" => return None,
        "rs" => "rust",
        "py" | "pyw" => "python",
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "vue" => "javascript",
        "go" => "go",
        "java" | "kt" | "kts" | "scala" | "dart" | "gradle" | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" | "m" | "mm" | "cs" => "java",
        "sh" | "bash" | "zsh" | "fish" => "shell",
        "sql" => "sql",
        "json" | "jsonc" => "json",
        "yaml" | "yml" | "toml" | "ini" | "conf" | "properties" => "yaml",
        "css" | "scss" | "less" => "css",
        "html" | "htm" | "xml" | "svg" | "plist" => "html",
        "mmd" | "mermaid" => "mermaid",
        "puml" | "plantuml" => "plantuml",
        "tex" => "latex",
        _ => "generic",
    })
}

/// 首行是 shebang 时猜语言（`#!/bin/bash`、`#!/usr/bin/env python3`）。
pub fn lang_for_shebang(first: &str) -> Option<&'static str> {
    let f = first.strip_prefix("#!")?;
    Some(if f.contains("python") {
        "python"
    } else if f.contains("node") {
        "javascript"
    } else if f.contains("sh") {
        "shell"
    } else {
        "generic"
    })
}

/// 记号的 CSS 颜色（HTML 预览用，与编辑器主题一致）。
pub fn tok_css(t: Tok, dark: bool) -> &'static str {
    match (t, dark) {
        (Tok::Keyword | Tok::Arrow, false) => "#a626a4",
        (Tok::String, false) => "#50a14f",
        (Tok::Comment, false) => "#a0a1a7",
        (Tok::Number, false) => "#986801",
        (Tok::Function, false) => "#4078f2",
        (Tok::Type, false) => "#c18401",
        (Tok::Keyword | Tok::Arrow, true) => "#c678dd",
        (Tok::String, true) => "#98c379",
        (Tok::Comment, true) => "#7f848e",
        (Tok::Number, true) => "#d19a66",
        (Tok::Function, true) => "#61afef",
        (Tok::Type, true) => "#e5c07b",
    }
}

/// 从开头起的一段连线箭头长度：由 `- . = < > | * o x { }` 组成、至少含两个 `-`/`.`/`=` 或一个 `>`，
/// 且 `o` / `x` / `{ }` 只能出现在两端（避免把单词里的字母当箭头）。
fn arrow_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let n = b.iter().take_while(|c| matches!(c, b'-' | b'.' | b'=' | b'<' | b'>' | b'|' | b'*' | b'o' | b'x' | b'}' | b'{')).count();
    if n < 2 {
        return None;
    }
    let seg = &b[..n];
    let body = seg.iter().filter(|c| matches!(c, b'-' | b'.' | b'=')).count();
    if body == 0 || (body < 2 && !seg.contains(&b'>')) {
        return None;
    }
    // 字母只允许在两端：中间出现说明是普通单词
    let inner = &seg[1..n - 1];
    if inner.iter().any(|c| matches!(c, b'o' | b'x')) {
        return None;
    }
    if b.get(n).is_some_and(|c| c.is_ascii_alphanumeric()) && matches!(seg[n - 1], b'o' | b'x') {
        return None;
    }
    if seg[0] == b'.' && body < 2 {
        return None;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_tokens() {
        let toks = highlight("rust", "fn main() { let s = \"hi\"; // 注释\n}");
        assert!(toks.iter().any(|(_, t)| *t == Tok::Keyword));
        assert!(toks.iter().any(|(_, t)| *t == Tok::String));
        assert!(toks.iter().any(|(_, t)| *t == Tok::Comment));
        assert!(toks.iter().any(|(_, t)| *t == Tok::Function));
    }

    #[test]
    fn cached_matches_uncached() {
        let src = "fn a() {}\n/* x */";
        assert_eq!(*highlight_cached("rust", src), highlight("rust", src));
        assert!(Rc::ptr_eq(&highlight_cached("rust", src), &highlight_cached("rust", src)));
    }

    fn tok_of<'a>(src: &'a str, toks: &[(Range<usize>, Tok)], t: Tok) -> Vec<&'a str> {
        toks.iter().filter(|(_, k)| *k == t).map(|(r, _)| &src[r.clone()]).collect()
    }

    #[test]
    fn markdown_structure() {
        let src = "# 标题\n\n- 列表 `code` **粗**\n> 引用\n\n---\n";
        let toks = highlight("Markdown", src);
        assert_eq!(tok_of(src, &toks, Tok::Keyword).first(), Some(&"#"));
        assert!(tok_of(src, &toks, Tok::String).contains(&"`code`"));
        assert!(tok_of(src, &toks, Tok::Keyword).contains(&"-"));
        assert!(tok_of(src, &toks, Tok::Keyword).contains(&">"));
        assert!(tok_of(src, &toks, Tok::Keyword).contains(&"---"));
        // 标题文字本身不着色
        assert!(!tok_of(src, &toks, Tok::Keyword).contains(&"标题"));
    }

    #[test]
    fn markdown_fence_and_link() {
        let src = "```rust\nfn main() {}\n```\n\n[链接](https://example.com) snake_case\n";
        let toks = highlight("markdown", src);
        assert_eq!(tok_of(src, &toks, Tok::Comment).len(), 2);
        assert!(tok_of(src, &toks, Tok::Type).contains(&"rust"));
        assert!(tok_of(src, &toks, Tok::String).contains(&"https://example.com"));
        // snake_case 的下划线不当强调
        assert!(!tok_of(src, &toks, Tok::Keyword).contains(&"_"));
        // 围栏内的代码不再按行内规则上色
        assert!(!tok_of(src, &toks, Tok::String).contains(&"{}"));
    }

    /// 每种语言的典型样例都必须能着色；同时检查 token 的范围合法且按顺序不重叠。
    #[test]
    fn every_language_highlights() {
        let cases: &[(&str, &str)] = &[
            ("Plain Text", "hello world 你好 123"),
            ("Rust", "fn main() { let x: u32 = 42; println!(\"hi\"); } // 注释"),
            ("Python", "def f(name): return \"hi\"  # 注释"),
            ("JavaScript", "const x = 1; function f() { return null; } // 注释"),
            ("TypeScript", "interface User { id: number; name: string }"),
            ("Go", "func main() { var x int = 1; fmt.Println(\"go\") }"),
            ("Java", "class A { public void f() { int x = 1; } } // 注释"),
            ("C", "#include <stdio.h>\nint main() { return 0; } /* 注释 */"),
            ("C++", "class A { public: int x = 1; }; // 注释"),
            ("Shell", "export FOO=1 && echo \"hi\"  # 注释"),
            ("SQL", "SELECT id FROM users WHERE id = 1; -- 注释"),
            ("JSON", "{\"name\": \"demo\", \"ok\": true, \"n\": 42}"),
            ("YAML", "name: demo  # 注释\nflag: true"),
            ("HTML", "<!-- 注释 --><div class=\"box\">hi</div>"),
            ("CSS", "/* 注释 */ .box { color: #ff0000; width: 10px; }"),
            ("Markdown", "# Title **bold** [link](https://example.com) `code`"),
            ("Kotlin", "fun main() { val x = 1 } // 注释"),
            ("Swift", "func f() { let x = 1 } // 注释"),
            ("Mermaid", "flowchart LR\n  A[开始] --> B{判断}"),
            ("LaTeX", "% 注释\n\\frac{1}{2} + x^2 = \\sqrt{4}"),
        ];
        for (lang, src) in cases {
            let toks = highlight(lang, src);
            if *lang == "Plain Text" {
                assert!(toks.is_empty(), "Plain Text 不应着色：{toks:?}");
                continue;
            }
            assert!(!toks.is_empty(), "{lang} 没有任何高亮");
            let mut last = 0;
            for (r, _) in &toks {
                assert!(r.start < r.end && r.end <= src.len(), "{lang} token 越界：{r:?}（{src:?}）");
                assert!(r.start >= last, "{lang} token 重叠或乱序：{r:?}（{src:?}）");
                last = r.end;
            }
        }
        // 语言列表新增而忘了补样例时，这里会失败
        for lang in LANGS {
            assert!(cases.iter().any(|(l, _)| l == lang), "缺少 {lang} 的测试样例");
        }
    }

    #[test]
    fn markup_css_yaml_latex_tokens() {
        let html = "<div class=\"box\">hi</div><!-- c -->";
        let toks = highlight("HTML", html);
        assert!(tok_of(html, &toks, Tok::Keyword).contains(&"<div"));
        assert!(tok_of(html, &toks, Tok::Type).contains(&"class"));
        assert!(tok_of(html, &toks, Tok::String).contains(&"\"box\""));
        assert_eq!(tok_of(html, &toks, Tok::Comment), ["<!-- c -->"]);

        let css = ".box { color: #ff0000; } /* c */";
        let toks = highlight("CSS", css);
        assert!(tok_of(css, &toks, Tok::Type).contains(&".box"));
        assert!(tok_of(css, &toks, Tok::Keyword).contains(&"color"));
        assert!(tok_of(css, &toks, Tok::Number).contains(&"#ff0000"));

        let yaml = "name: demo\nflag: true  # c";
        let toks = highlight("YAML", yaml);
        assert!(tok_of(yaml, &toks, Tok::Type).contains(&"name"));
        assert!(tok_of(yaml, &toks, Tok::Keyword).contains(&"true"));
        assert_eq!(tok_of(yaml, &toks, Tok::Comment), ["# c"]);

        let tex = "% c\n\\frac{1}{2} + 3";
        let toks = highlight("LaTeX", tex);
        assert_eq!(tok_of(tex, &toks, Tok::Comment), ["% c"]);
        assert!(tok_of(tex, &toks, Tok::Keyword).contains(&"\\frac"));
        assert!(tok_of(tex, &toks, Tok::Number).contains(&"3"));
    }

    #[test]
    fn diagram_arrows() {
        let src = "A --> B\nB ->> C: hi\nfoo-bar";
        let toks = highlight("mermaid", src);
        let arrows: Vec<&str> = toks.iter().filter(|(_, t)| *t == Tok::Arrow).map(|(r, _)| &src[r.clone()]).collect();
        assert_eq!(arrows, ["-->", "->>"]);
        let p = "@startuml\n' 注释\nUser -> Gateway: 提交\n@enduml";
        let toks = highlight("plantuml", p);
        assert!(toks.iter().any(|(r, t)| *t == Tok::Comment && p[r.clone()].contains("注释")));
        assert!(toks.iter().any(|(r, t)| *t == Tok::Keyword && &p[r.clone()] == "startuml"));
    }
}
