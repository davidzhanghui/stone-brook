//! 「视频或文件」块：
//! - 三种视图：预览视图（视频播放器 / 文档首屏）、卡片视图、文本视图，悬停时上方浮出工具栏切换；
//! - 插入后先占位显示「上传中 N%」，后台把文件拷进 assets；
//! - 文档预览：Markdown / 文本（代码按语言高亮）/ HTML 渲染成网页快照，点击后换成可滚动的原生 WebView；
//!   PDF / Office / iWork / 图片用系统 Quick Look 生成首页缩略图，Office / iWork 点击后换成原生 QLPreviewView 翻页查看；
//! - 音频：播放条 + 波形（audio.rs）；压缩包：包内目录树（archive.rs）；webm：WebView 里的播放页；
//! - 视频预览居中显示，悬停时右下角出现圆点，拖动调整大小；
//! - 放大预览 / 点击卡片、文本视图：视频全屏播放，其他文件打开应用内全屏预览浮层。

use super::render::{body_lh, content_w};
use super::*;
use crate::convert::human_size;
use crate::model::{Block, BlockKind, FILE_VIEW_CARD, FILE_VIEW_PREVIEW, FILE_VIEW_TEXT};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 文档预览区最大高度（超出部分在交互模式下滚动查看）
const PREVIEW_MAX_H: f32 = 460.;
/// 视频预览默认最大高度（竖屏视频不至于占满一屏）
const VIDEO_MAX_H: f32 = 440.;
const VIDEO_MIN_W: f32 = 160.;
const CARD_W: f32 = 400.;
/// 上传进度（千分比）：完成 / 失败
const UPLOAD_DONE: u32 = 1000;
const UPLOAD_FAILED: u32 = u32::MAX;

/// 文件能怎么预览。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Preview {
    Video,
    /// AVPlayer 不支持、WebKit 能播的视频（webm）：WebView 播放页
    WebVideo,
    Audio,
    Markdown,
    Text,
    Html,
    Pdf,
    /// Office / iWork：Quick Look 缩略图，交互时换成 QLPreviewView
    Office,
    /// 图片（heic / svg 等）：Quick Look 缩略图
    Image,
    Archive,
    None,
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

/// 图片文件（「视频或文件」插入图片时默认卡片视图）。
pub fn is_image_name(name: &str) -> bool {
    matches!(ext_of(name).as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tif" | "tiff")
}

pub fn preview_kind(name: &str) -> Preview {
    let ext = ext_of(name);
    match ext.as_str() {
        e if crate::media::VIDEO_EXTS.contains(&e) => Preview::Video,
        "webm" => Preview::WebVideo,
        "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" | "oga" | "aif" | "aiff" | "caf" => Preview::Audio,
        "zip" | "rar" | "7z" | "tar" | "tgz" | "gz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "iso" | "jar" => Preview::Archive,
        "md" | "markdown" => Preview::Markdown,
        "txt" | "log" | "csv" | "tsv" | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "conf" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go"
        | "java" | "kt" | "c" | "h" | "cpp" | "hpp" | "cs" | "rb" | "php" | "swift" | "sh" | "bash" | "zsh" | "sql" | "css" | "scss" | "less" | "vue"
        | "lua" | "pl" | "r" | "scala" | "dart" | "m" | "mm" | "diff" | "patch" | "properties" | "gradle" | "bat" | "ps1" | "proto" | "graphql" | "tex"
        | "env" => Preview::Text,
        "html" | "htm" => Preview::Html,
        "pdf" => Preview::Pdf,
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "key" | "pages" | "numbers" | "rtf" => Preview::Office,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "tif" | "tiff" | "bmp" => Preview::Image,
        _ => Preview::None,
    }
}

pub(crate) enum Glyph {
    Svg(&'static str),
    Text(&'static str),
}

/// 文件类型图标的颜色与图形（彩色文档底 + 白色图形）。
pub(crate) fn file_style(name: &str) -> (u32, Glyph) {
    match ext_of(name).as_str() {
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" => (0x3370ff, Glyph::Svg("fi-video")),
        "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" | "oga" | "aif" | "aiff" | "caf" => (0x7f3bf5, Glyph::Svg("fi-music")),
        "pdf" => (0xf54a45, Glyph::Text("PDF")),
        "doc" | "docx" | "pages" | "rtf" => (0x3370ff, Glyph::Text("W")),
        "xls" | "xlsx" | "csv" | "tsv" | "numbers" => (0x2ea121, Glyph::Text("X")),
        "ppt" | "pptx" | "key" => (0xff8800, Glyph::Text("P")),
        "zip" | "rar" | "7z" | "gz" | "tgz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "tar" | "iso" | "jar" | "dmg" => (0xf5a700, Glyph::Svg("fi-zip")),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tif" | "tiff" => (0x14b8a6, Glyph::Svg("fi-image")),
        "txt" | "log" => (0x3370ff, Glyph::Svg("fi-lines")),
        _ if matches!(preview_kind(name), Preview::Markdown | Preview::Text | Preview::Html) => (0x3370ff, Glyph::Svg("fi-code")),
        _ => (0x8f959e, Glyph::Text("?")),
    }
}

/// 「无法预览」插画（闭眼的文件夹 + 缠绕彩带），FOLDER 按明暗主题替换。
const NO_PREVIEW_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="196" viewBox="20 20 200 196">
<path d="M150 120 C 205 128, 222 160, 196 186" fill="none" stroke="#0d2a6e" stroke-width="14" stroke-linecap="round"/>
<path d="M128 74 C 126 30, 196 22, 202 70 L 202 112 L 136 112 Z" fill="#0d2a6e"/>
<path d="M146 46 C 132 62, 126 84, 118 98 L 132 100 L 136 112 L 150 112 C 152 96, 150 70, 146 46 Z" fill="#ffffff" stroke="#0d2a6e" stroke-width="2" stroke-linejoin="round"/>
<path d="M134 70 q5 5 11 1" fill="none" stroke="#0d2a6e" stroke-width="2" stroke-linecap="round"/>
<path d="M34 128 C 22 140, 30 156, 56 166" fill="none" stroke="#0d2a6e" stroke-width="14" stroke-linecap="round"/>
<path d="M50 96 a8 8 0 0 1 8 -8 h46 l14 16 h82 a8 8 0 0 1 8 8 v94 a8 8 0 0 1 -8 8 H58 a8 8 0 0 1 -8 -8 Z" fill="FOLDER"/>
<circle cx="66" cy="100" r="4" fill="none" stroke="#8f959e" stroke-width="1.6"/>
<circle cx="80" cy="100" r="4" fill="none" stroke="#8f959e" stroke-width="1.6"/>
<path d="M98 150 q30 28 60 0" fill="none" stroke="#0d2a6e" stroke-width="2" stroke-linecap="round"/>
<path d="M128 164 v10 M108 160 l-6 8 M148 160 l6 8 M98 150 l-7 -8 M158 150 l7 -8" stroke="#0d2a6e" stroke-width="2" stroke-linecap="round"/>
<path d="M30 140 C 32 156, 46 164, 64 170" fill="none" stroke="#3370ff" stroke-width="16" stroke-linecap="round"/>
<path d="M38 174 C 46 184, 62 190, 84 194" fill="none" stroke="#0d2a6e" stroke-width="2" stroke-linecap="round"/>
<path d="M150 206 H184 C 204 206, 212 196, 210 182" fill="none" stroke="#00d6b9" stroke-width="14" stroke-linecap="round"/>
</svg>"##;

thread_local! {
    static NO_PREVIEW_IMG: [Arc<Image>; 2] = ["#dee0e3", "#4a4d52"]
        .map(|c| Arc::new(Image::from_bytes(ImageFormat::Svg, NO_PREVIEW_SVG.replace("FOLDER", c).into_bytes())));
}

fn no_preview_img(dark: bool) -> Arc<Image> {
    NO_PREVIEW_IMG.with(|a| a[dark as usize].clone())
}

/// 文件类型图标（size 为外框边长）。
pub fn file_icon(name: &str, size: f32) -> Div {
    let (col, glyph) = file_style(name);
    let col: Hsla = rgb(col).into();
    let inner = match glyph {
        Glyph::Svg(n) => icon(n, size * 0.5, gpui::white()).into_any_element(),
        Glyph::Text(s) => div()
            .text_size(px(size * if s.len() > 1 { 0.26 } else { 0.4 }))
            .line_height(px(size * 0.4))
            .font_weight(FontWeight::BOLD)
            .text_color(gpui::white())
            .child(s)
            .into_any_element(),
    };
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(icon("file-shape", size, col).absolute().top_0().left_0())
        .child(div().absolute().top(px(size * 0.1)).left(px(size * 0.04)).size_full().flex().items_center().justify_center().child(inner))
}

// ───────────── 文档预览页 / 缩略图 ─────────────

fn file_stamp(path: &str) -> String {
    std::fs::metadata(path).ok().map(|m| format!("{}-{:?}", m.len(), m.modified().ok())).unwrap_or_default()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 文本文件解码：有 BOM 按 BOM（UTF-8 / UTF-16），合法 UTF-8 直接用，否则按 GB18030（兼容 GBK，中文 txt 常见）。
fn decode_text(bytes: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::GB18030.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// 语法高亮最多处理的行数（再多的行按纯文本输出，避免超大日志卡住）
const HL_MAX_LINES: usize = 5000;

/// 文本 / 代码文件：带行号，按扩展名（认不出时按首行，如 #!/bin/bash）选语法高亮（代码文件预览）。
fn code_html(name: &str, src: &str, dark: bool) -> String {
    use crate::highlight::{highlight, lang_for_ext, lang_for_shebang, tok_css};
    let lang = lang_for_ext(&ext_of(name))
        .filter(|l| *l != "generic")
        .or_else(|| lang_for_shebang(src.lines().next().unwrap_or("")))
        .or_else(|| lang_for_ext(&ext_of(name)));
    // 超长文件只高亮前 HL_MAX_LINES 行
    let hl_end = src.match_indices('\n').nth(HL_MAX_LINES).map_or(src.len(), |(i, _)| i);
    let toks = lang.map(|l| highlight(l, &src[..hl_end])).unwrap_or_default();
    let mut ti = 0;
    let mut out = String::with_capacity(src.len() * 2);
    out.push_str("<div class=\"code\">");
    let mut start = 0;
    for (i, line) in src.split_inclusive('\n').enumerate() {
        let (ls, le) = (start, start + line.trim_end_matches(['\n', '\r']).len());
        start += line.len();
        let mut body = String::new();
        let mut pos = ls;
        while ti < toks.len() && toks[ti].0.start < le {
            let (r, t) = (&toks[ti].0, toks[ti].1);
            let (s, e) = (r.start.max(ls), r.end.min(le));
            if s > pos {
                body.push_str(&esc(&src[pos..s]));
            }
            if e > s {
                body.push_str(&format!("<span style=\"color:{}\">{}</span>", tok_css(t, dark), esc(&src[s..e])));
                pos = e;
            }
            if r.end <= le {
                ti += 1;
            } else {
                break; // 跨行记号（块注释）：下一行继续
            }
        }
        if pos < le {
            body.push_str(&esc(&src[pos..le]));
        }
        out.push_str(&format!("<div class=\"ln\"><span class=\"no\">{}</span><span class=\"tx\">{}</span></div>", i + 1, body));
    }
    out.push_str("</div>");
    out
}

/// Markdown / 文本生成一张带样式的预览网页（缓存在 cache 目录）；HTML 文件直接用原文件。
fn preview_page(path: &str, name: &str, kind: Preview, dark: bool) -> Option<PathBuf> {
    if kind == Preview::Html {
        return Some(PathBuf::from(path));
    }
    if !matches!(kind, Preview::Markdown | Preview::Text) {
        return None;
    }
    let key = crate::webview::hash_key(&[path, &file_stamp(path), if dark { "dark" } else { "light" }, "filepv4"]);
    let out = crate::webview::cache_dir().join(format!("filepv-{key}.html"));
    if out.exists() {
        return Some(out);
    }
    let src = decode_text(&std::fs::read(path).ok()?);
    let body = if kind == Preview::Markdown {
        let blocks = crate::convert::md_to_blocks(&src);
        crate::convert::blocks_to_html(&blocks.iter().collect::<Vec<&Block>>())
    } else {
        code_html(name, &src, dark)
    };
    let (fg, bg, muted, code_bg, border, link) = if dark {
        ("#e3e4e6", "#1f1f1f", "#a6a6a6", "#2b2b2b", "#3a3a3a", "#4c88ff")
    } else {
        ("#1f2329", "#ffffff", "#646a73", "#f5f6f7", "#dee0e3", "#3370ff")
    };
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
html,body{{margin:0;background:{bg};color:{fg}}}\
body{{padding:14px 18px;font:14px/1.7 -apple-system,'PingFang SC',sans-serif;word-wrap:break-word}}\
h1{{font-size:22px;margin:4px 0 12px}}h2{{font-size:18px;margin:18px 0 8px}}h3{{font-size:16px;margin:14px 0 6px}}h4,h5,h6{{font-size:14px}}\
p{{margin:6px 0}}ul,ol{{padding-left:22px;margin:6px 0}}a{{color:{link};text-decoration:none}}\
blockquote{{margin:6px 0;padding-left:12px;border-left:3px solid {border};color:{muted}}}\
code{{background:{code_bg};border-radius:4px;padding:1px 4px;font:12.5px/1.6 Menlo,monospace}}\
pre{{background:{code_bg};border-radius:6px;padding:10px 12px;overflow:auto;font:12.5px/1.6 Menlo,monospace}}pre code{{background:none;padding:0}}\
.code{{font:12.5px/1.65 Menlo,monospace;margin-left:-8px}}.ln{{display:flex}}\
.no{{flex:none;width:34px;padding-right:12px;text-align:right;color:{muted};opacity:.6;user-select:none}}.tx{{flex:1;min-width:0;white-space:pre-wrap;word-break:break-all}}\
table{{border-collapse:collapse;margin:8px 0}}td,th{{border:1px solid {border};padding:4px 8px}}img{{max-width:100%}}hr{{border:none;border-top:1px solid {border}}}\
</style></head><body>{body}</body></html>"
    );
    std::fs::write(&out, html).ok()?;
    Some(out)
}

/// 静态整页快照的最大高度（再高超出 GPU 纹理上限）
pub(crate) const STATIC_MAX_H: f32 = 3600.;

/// 其他模块（电子表格附件）的全屏静态预览内容。
pub(crate) enum StaticPreview {
    /// 直接显示的图片（原图 / Quick Look 首页）
    Image(PathBuf),
    /// 网页快照（Markdown / 文本 / HTML 渲染后的整页长图）：(图片, 宽, 高)
    Snap(PathBuf, f32, f32),
    Pending,
    Unsupported,
}

/// 生成文件的静态预览：图片原图；Markdown / 代码 / 文本 / HTML 渲染成整页快照；PDF / Office 用 Quick Look 首页。
pub(crate) fn static_preview(path: &str, name: &str, width: f32, dark: bool, cx: &mut App) -> StaticPreview {
    if !Path::new(path).exists() {
        return StaticPreview::Unsupported;
    }
    if matches!(ext_of(name).as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") {
        return StaticPreview::Image(PathBuf::from(path));
    }
    match preview_kind(name) {
        k @ (Preview::Markdown | Preview::Text | Preview::Html) => {
            match preview_page(path, name, k, dark).map(|p| crate::webview::request_file(&p, false, width, STATIC_MAX_H, cx)) {
                Some(crate::webview::SnapState::Ready { path, width, height }) => StaticPreview::Snap(path, width, height),
                Some(crate::webview::SnapState::Pending) => StaticPreview::Pending,
                _ => StaticPreview::Unsupported,
            }
        }
        Preview::Pdf | Preview::Office | Preview::Image => match ql_thumb(path, cx) {
            Thumb::Ready(p) => StaticPreview::Image(p),
            Thumb::Pending => StaticPreview::Pending,
            Thumb::Failed => StaticPreview::Unsupported,
        },
        _ => StaticPreview::Unsupported,
    }
}

/// 交互模式下叠加的原生 WebView 加载什么。
pub fn live_src(kind: &BlockKind, dark: bool) -> Option<crate::webview::LiveSrc> {
    let BlockKind::File { path, name, .. } = kind else { return None };
    file_live_src(path, name, dark)
}

/// 文件的交互预览源（原生 WebView / QLPreviewView），文档与电子表格附件共用。
pub(crate) fn file_live_src(path: &str, name: &str, dark: bool) -> Option<crate::webview::LiveSrc> {
    if !Path::new(path).exists() {
        return None;
    }
    match preview_kind(name) {
        k @ (Preview::Markdown | Preview::Text | Preview::Html) => {
            preview_page(path, name, k, dark).map(|p| crate::webview::LiveSrc::File { path: p, network: false })
        }
        // PDF 用 WebKit 自带的阅读器（以文件 URL 加载）
        Preview::Pdf => Some(crate::webview::LiveSrc::File { path: path.into(), network: true }),
        Preview::Office => Some(crate::webview::LiveSrc::QuickLook { path: path.into() }),
        Preview::WebVideo => video_page(path).map(|(page, root)| crate::webview::LiveSrc::Media { page, root }),
        _ => None,
    }
}

/// 可交互（叠加原生视图）的预览类型
fn is_live_kind(kind: Preview) -> bool {
    matches!(kind, Preview::Markdown | Preview::Text | Preview::Html | Preview::Pdf | Preview::Office | Preview::WebVideo)
}

/// webm 播放页：黑底 + 原生 video 控件，自动播放。返回 (页面, 允许读取的根目录)。
fn video_page(path: &str) -> Option<(PathBuf, PathBuf)> {
    // 页面与视频放进同一个独立目录，WebView 只获得这个目录的读权限
    // （原来取缓存目录与 assets 的共同上级，等于把整个数据目录、包括数据库都开放给了页面）。
    // 视频以硬链接放进来：不占额外空间；跨卷等硬链接失败时退回拷贝。
    let root = crate::webview::cache_dir().join(format!("webm-{}", crate::webview::hash_key(&[path, "webm3"])));
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "webm".into());
    let video = root.join(format!("video.{ext}"));
    let page = root.join("index.html");
    if !video.exists() {
        std::fs::create_dir_all(&root).ok()?;
        if std::fs::hard_link(path, &video).is_err() {
            std::fs::copy(path, &video).ok()?;
        }
    }
    if !page.exists() {
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}\
video{{display:block;width:100%;height:100%;object-fit:contain;outline:none}}\
#m{{display:none;position:fixed;inset:0;align-items:center;justify-content:center;padding:24px;text-align:center;\
color:#c9cdd4;font:14px/1.7 -apple-system,'PingFang SC',sans-serif}}</style></head>\
<body><video src=\"video.{ext}\" controls autoplay playsinline></video>\
<div id=\"m\">无法在应用内播放这个 webm 文件<br>系统 WebView 可能不支持它的视频编码（如 VP9），请用其他应用打开</div>\
<script>var v=document.querySelector('video'),m=document.getElementById('m');\
function fail(){{v.style.display='none';m.style.display='flex'}}v.addEventListener('error',fail);\
setTimeout(function(){{if(v.readyState<2)fail()}},4000);</script></body></html>"
        );
        std::fs::write(&page, html).ok()?;
    }
    Some((page, root))
}

#[derive(Clone)]
enum Thumb {
    Pending,
    Ready(PathBuf),
    Failed,
}

thread_local! {
    static THUMBS: RefCell<crate::lru::Lru<String, Thumb>> = RefCell::new(crate::lru::Lru::new(256));
}

/// 系统 Quick Look 生成的首页缩略图：已缓存直接返回，否则后台生成。
fn ql_thumb(path: &str, cx: &mut App) -> Thumb {
    let key = crate::webview::hash_key(&[path, &file_stamp(path), "ql"]);
    if let Some(t) = THUMBS.with(|m| m.borrow().get(&key).cloned()) {
        return t;
    }
    let dir = crate::webview::cache_dir().join(format!("ql-{key}"));
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let png = dir.join(format!("{name}.png"));
    if png.exists() {
        THUMBS.with(|m| m.borrow_mut().insert(key, Thumb::Ready(png.clone())));
        return Thumb::Ready(png);
    }
    THUMBS.with(|m| m.borrow_mut().insert(key.clone(), Thumb::Pending));
    let src = path.to_string();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let out = png.clone();
        let ok = cx
            .background_executor()
            .spawn(async move {
                std::fs::create_dir_all(&dir).ok();
                std::process::Command::new("qlmanage")
                    .args(["-t", "-s", "1200", "-o"])
                    .arg(&dir)
                    .arg(&src)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok()
                    && out.exists()
            })
            .await;
        THUMBS.with(|m| m.borrow_mut().insert(key, if ok { Thumb::Ready(png) } else { Thumb::Failed }));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    Thumb::Pending
}

/// 后台拷贝文件，进度写入千分比。
fn copy_progress(src: &Path, dst: &Path, prog: &AtomicU32) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let total = std::fs::metadata(src)?.len().max(1);
    let mut r = std::fs::File::open(src)?;
    let mut w = std::fs::File::create(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        done += n as u64;
        prog.store((done * 999 / total) as u32, Ordering::Relaxed);
    }
    w.flush()
}

impl Editor {
    /// 开始「上传」：块已插入（path 指向 assets 里的目标位置），后台拷贝期间显示进度占位。
    pub(super) fn start_upload(&mut self, src: PathBuf, dst: String, cx: &mut Context<Self>) {
        let prog = Arc::new(AtomicU32::new(0));
        self.uploads.insert(dst.clone(), prog.clone());
        let (p2, d2) = (prog.clone(), PathBuf::from(&dst));
        cx.background_executor()
            .spawn(async move {
                let ok = copy_progress(&src, &d2, &p2).is_ok();
                p2.store(if ok { UPLOAD_DONE } else { UPLOAD_FAILED }, Ordering::Relaxed);
            })
            .detach();
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(Duration::from_millis(80)).await;
            let p = prog.load(Ordering::Relaxed);
            let done = p >= UPLOAD_DONE;
            let alive = this
                .update(cx, |this, cx| {
                    if done {
                        this.uploads.remove(&dst);
                        if p == UPLOAD_FAILED {
                            cx.emit(EditorEvent::Toast("文件上传失败".into()));
                        }
                    }
                    cx.notify();
                })
                .is_ok();
            if done || !alive {
                break;
            }
        })
        .detach();
    }

    /// 鼠标进出文件块 / 工具栏：离开后稍等再收起，留出移到工具栏上的时间。
    fn attach_hover(&mut self, id: u64, on: bool, cx: &mut Context<Self>) {
        self.file_hover_epoch += 1;
        if on {
            if self.sess.file_hover != Some(id) {
                self.sess.file_hover = Some(id);
                cx.notify();
            }
            return;
        }
        let epoch = self.file_hover_epoch;
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            this.update(cx, |this, cx| {
                if this.file_hover_epoch == epoch && this.sess.file_hover == Some(id) {
                    this.sess.file_hover = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn set_file_view(&mut self, ix: usize, v: u8, cx: &mut Context<Self>) {
        let id = self.content.blocks[ix].id;
        if self.sess.html_live == Some(id) {
            self.sess.html_live = None;
            crate::webview::hide_live();
        }
        self.update_kind(
            ix,
            |k| {
                if let BlockKind::File { view, .. } | BlockKind::Video { view, .. } = k {
                    *view = v;
                }
            },
            cx,
        );
    }

    /// 测试脚本：切换文件块视图。
    #[cfg(feature = "dev")]
    pub fn test_file_view(&mut self, ix: usize, v: u8, cx: &mut Context<Self>) {
        if self.attach_path(ix).is_some() {
            self.set_file_view(ix, v, cx);
        }
    }

    /// 测试脚本：放大预览指定块。
    #[cfg(feature = "dev")]
    pub fn test_open_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.content.blocks.len() {
            self.open_attachment(ix, cx);
        }
    }

    /// 放大预览：视频进入全屏播放，其他文件打开应用内全屏预览。
    pub(super) fn open_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        let blk = &self.content.blocks[ix];
        match &blk.kind {
            BlockKind::Video { path, .. } => {
                let id = blk.id;
                crate::media::pause_others(id);
                crate::media::with(id, path, |p| p.play());
                self.sess.video_full = Some(id);
                cx.notify();
            }
            BlockKind::File { .. } => {
                if self.sess.html_live.take().is_some() {
                    crate::webview::hide_live();
                }
                self.sess.file_hover = None;
                self.sess.img_views.remove(&(blk.id | super::imgview::FULL_KEY));
                self.sess.file_full = Some(blk.id);
                cx.notify();
            }
            _ => {}
        }
    }

    fn close_file_full(&mut self, cx: &mut Context<Self>) {
        if self.sess.file_full.take().is_some() {
            crate::webview::hide_live();
            cx.notify();
        }
    }

    /// 测试脚本：打开文件块的全屏预览。
    #[cfg(feature = "dev")]
    pub fn test_file_full(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.attach_path(ix).is_some() {
            self.open_attachment(ix, cx);
        }
    }

    /// 全屏预览浮层：顶栏「× 退出 | 图标 文件名 / 大小 | 下载」，下方铺满预览内容。
    /// Markdown / 文本 / HTML / PDF / webm 由原生 WebView、Office / iWork 由 QLPreviewView 叠加在内容区（见 render.rs）；
    /// 图片直接显示原图，音频显示播放条，压缩包显示目录树，不支持的类型给出「用其他应用打开」。
    pub(super) fn render_file_full(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.sess.file_full?;
        let ix = self.content.blocks.iter().position(|b| b.id == id)?;
        let BlockKind::File { path, name, size, .. } = &self.content.blocks[ix].kind else { return None };
        let (path, name, size) = (path.clone(), name.clone(), *size);
        let kind = preview_kind(&name);
        let exists = Path::new(&path).exists();
        let is_image = matches!(ext_of(&name).as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp");
        let vp = window.viewport_size();
        let viewer = exists && super::imgview::viewable(&name) && self.image_size(&path).is_some();
        let live = exists && is_live_kind(kind);
        let frame = self.frame.clone();
        let msg =
            |s: &'static str| div().size_full().flex().items_center().justify_center().text_size(px(13.)).text_color(t.text_muted).child(s).into_any_element();
        let content: AnyElement = if !exists {
            msg("文件不存在")
        } else if live {
            canvas(
                move |b, _, _| {
                    frame.borrow_mut().html_bounds.insert(id, b);
                },
                |_, _, _, _| {},
            )
            .size_full()
            .into_any_element()
        } else if viewer {
            let (w, h) = (f32::from(vp.width), f32::from(vp.height) - 57.);
            self.render_img_viewer(ix, id, &path, w, h, true, t, cx)
        } else if is_image {
            div().size_full().p(px(24.)).child(img(PathBuf::from(&path)).size_full().object_fit(ObjectFit::Contain)).into_any_element()
        } else if kind == Preview::Audio {
            let w = (f32::from(vp.width) - 48.).min(640.);
            let player = self.render_audio(ix, id, &path, &name, w, false, t, cx);
            div().size_full().flex().items_center().justify_center().child(player).into_any_element()
        } else if kind == Preview::Archive {
            let tree = self.render_archive(ix, id, &path, &name, size, None, false, t, cx);
            div()
                .size_full()
                .flex()
                .justify_center()
                .py(px(16.))
                .child(div().w(px(860.)).max_w_full().h_full().rounded(px(8.)).border_1().border_color(t.border).bg(t.bg).overflow_hidden().child(tree))
                .into_any_element()
        } else if matches!(kind, Preview::Image | Preview::Office) {
            match ql_thumb(&path, cx) {
                Thumb::Ready(p) => div().size_full().p(px(24.)).child(img(p).size_full().object_fit(ObjectFit::Contain)).into_any_element(),
                Thumb::Pending => msg("加载中…"),
                Thumb::Failed => self.render_full_unsupported(ix, &name, size, t, cx),
            }
        } else {
            self.render_full_unsupported(ix, &name, size, t, cx)
        };
        let head_btn = |eid: &'static str, ic: &'static str, label: &'static str| {
            div()
                .id((eid, id as usize))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded(px(6.))
                .text_size(px(13.))
                .text_color(t.text)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 14., t.text_secondary))
                .child(crate::i18n::tr(label))
        };
        // 盖住整个窗口（侧栏、大纲也在下面）
        let panel = div()
            .id("file-full")
            .w(vp.width)
            .h(vp.height)
            .occlude()
            .flex()
            .flex_col()
            .bg(t.bg)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    // 左侧让出透明标题栏的红绿灯
                    .pl(px(80.))
                    .pr(px(16.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(head_btn("ff-close", "x", "退出").on_click(cx.listener(|this, _, _, cx| this.close_file_full(cx))))
                    .child(file_icon(&name, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(14.)).text_color(t.text).whitespace_nowrap().overflow_hidden().text_ellipsis().child(name.clone()))
                            .child(div().text_size(px(11.5)).text_color(t.text_muted).child(human_size(size))),
                    )
                    .child(
                        head_btn("ff-dl", "save-as", "下载")
                            .border_1()
                            .border_color(t.border)
                            .on_click(cx.listener(move |this, _, _, cx| this.download_attachment(ix, cx))),
                    ),
            )
            .child(div().flex_1().min_h(px(0.)).bg(if live { t.bg } else { t.code_bg }).child(content));
        Some(deferred(anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(panel)).with_priority(10).into_any_element())
    }

    /// 全屏预览中无法预览的文件：插画 + 说明 +「用其他应用打开」。
    fn render_full_unsupported(&self, ix: usize, name: &str, size: u64, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(div().h(px(300.)).w(px(420.)).child(self.render_no_preview(name, Some(size), t)))
            .child(
                div()
                    .id("ff-ext")
                    .mt(px(16.))
                    .h(px(32.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .bg(t.accent)
                    .text_size(px(13.))
                    .text_color(gpui::white())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child(crate::i18n::tr("用其他应用打开"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some((path, _)) = this.attach_path(ix) {
                            cx.open_with_system(Path::new(&path));
                        }
                    })),
            )
            .into_any_element()
    }

    fn attach_path(&self, ix: usize) -> Option<(String, String)> {
        match &self.content.blocks.get(ix)?.kind {
            BlockKind::File { path, name, .. } | BlockKind::Video { path, name, .. } => Some((path.clone(), name.clone())),
            _ => None,
        }
    }

    /// 另存为：拷贝到用户选择的位置。
    pub(super) fn download_attachment(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((path, name)) = self.attach_path(ix) else { return };
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(dst))) = rx.await {
                let ok = std::fs::copy(&path, &dst).is_ok();
                this.update(cx, |_, cx| {
                    cx.emit(EditorEvent::Toast(if ok { format!("已另存为 {}", dst.display()) } else { "另存为失败".into() }));
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn render_attachment(&mut self, ix: usize, blk: &Block, focused: bool, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (path, name, view, is_video) = match &blk.kind {
            BlockKind::File { path, name, view, .. } => (path.clone(), name.clone(), *view, false),
            BlockKind::Video { path, name, view, .. } => (path.clone(), name.clone(), *view, true),
            _ => return div().into_any_element(),
        };
        let id = blk.id;
        let selected = focused && self.block_selected(ix);
        let kind = if is_video { Preview::Video } else { preview_kind(&name) };
        let uploading = self.uploads.get(&path).map(|p| p.load(Ordering::Relaxed).min(999));
        if self.sess.html_live == Some(id) && self.core.head.block != ix {
            self.sess.html_live = None;
            cx.notify();
        }
        let body: AnyElement = match (uploading, view) {
            (Some(p), _) => self.render_upload(&name, p, is_video, t),
            (None, FILE_VIEW_TEXT) => self.render_file_text(ix, id, &name, selected, t, cx),
            (None, FILE_VIEW_CARD) => self.render_file_card(ix, id, &path, &name, blk, is_video, selected, t, cx),
            (None, _) if is_video => self.render_video(ix, blk, &path, &name, selected, t, window, cx),
            (None, _) => self.render_file_preview(ix, blk, &path, &name, kind, selected, t, cx),
        };
        let sole = self.core.head.block == ix && self.core.anchor.block == ix;
        let show_tb = uploading.is_none() && (self.sess.file_hover == Some(id) || (self.sess.file_hover.is_none() && selected && sole));
        let center = view == FILE_VIEW_PREVIEW && uploading.is_none();
        let video_pv = center && is_video;
        let toolbar = show_tb.then(|| self.render_attach_toolbar(ix, id, view, t, cx));
        div()
            .relative()
            .w_full()
            .flex()
            .when(video_pv, |d| d.justify_center())
            .child(
                div()
                    .id(("attach", id as usize))
                    .relative()
                    .when(!video_pv, |d| d.w_full())
                    .child(body)
                    .children(toolbar.map(|tb| div().absolute().top(px(-44.)).left_0().when(center, |d| d.right_0().flex().justify_center()).child(tb)))
                    .child(self.void_marker(ix))
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| this.attach_hover(id, *h, cx))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                }),
            )
            .into_any_element()
    }

    /// 悬停工具栏：放大预览 / 另存为 | 文本 / 卡片 / 预览视图 | 复制路径 / 在访达中显示。
    fn render_attach_toolbar(&self, ix: usize, id: u64, view: u8, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let btn = |eid: &'static str, ic: &'static str, tip: &'static str, on: bool| {
            let color = if on { t.accent } else { t.text_secondary };
            div()
                .id((eid, id as usize))
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
        div()
            .id(("att-tb", id as usize))
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
            .on_hover(cx.listener(move |this, h: &bool, _, cx| this.attach_hover(id, *h, cx)))
            .child(btn("att-open", "maximize", "放大预览", false).on_click(cx.listener(move |this, _, _, cx| this.open_attachment(ix, cx))))
            .child(btn("att-dl", "save-as", "另存为", false).on_click(cx.listener(move |this, _, _, cx| this.download_attachment(ix, cx))))
            .child(sep())
            .child(
                btn("att-vt", "view-text", "文本视图", view == FILE_VIEW_TEXT)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_file_view(ix, FILE_VIEW_TEXT, cx))),
            )
            .child(
                btn("att-vc", "view-card", "卡片视图", view == FILE_VIEW_CARD)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_file_view(ix, FILE_VIEW_CARD, cx))),
            )
            .child(
                btn("att-vp", "view-preview", "预览视图", view == FILE_VIEW_PREVIEW)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_file_view(ix, FILE_VIEW_PREVIEW, cx))),
            )
            .child(sep())
            .child(btn("att-copy", "link", "复制文件路径", false).on_click(cx.listener(move |this, _, _, cx| {
                if let Some((path, _)) = this.attach_path(ix) {
                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                    cx.emit(EditorEvent::Toast("已复制文件路径".into()));
                }
            })))
            .child(btn("att-reveal", "folder", "在访达中显示", false).on_click(cx.listener(move |this, _, _, cx| {
                if let Some((path, _)) = this.attach_path(ix) {
                    cx.reveal_path(Path::new(&path));
                }
            })))
            .into_any_element()
    }

    /// 上传占位：灰底 +「上传中 N%」+ 进度条；文件另带底部文件名栏。
    fn render_upload(&self, name: &str, permille: u32, video: bool, t: &Theme) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(
                div()
                    .h(px(200.))
                    .bg(t.code_bg)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .child(div().text_size(px(12.5)).text_color(t.text_muted).child(format!("上传中 {}%", permille / 10)))
                    .child(
                        div()
                            .w(px(120.))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(t.border)
                            .child(div().h_full().w(relative(permille as f32 / 1000.)).rounded(px(2.)).bg(t.accent)),
                    ),
            )
            .when(!video, |d| d.child(self.file_footer(name, None, t)))
            .into_any_element()
    }

    pub(super) fn file_footer(&self, name: &str, size: Option<u64>, t: &Theme) -> Div {
        div()
            .h(px(34.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_t_1()
            .border_color(t.border)
            .bg(t.sidebar_bg)
            .text_size(px(12.5))
            .text_color(t.text_secondary)
            .child(file_icon(name, 16.))
            .child(div().flex_1().min_w(px(0.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.to_string()))
            .children(size.map(|s| div().text_color(t.text_muted).child(human_size(s))))
    }

    /// 无法预览占位：插画 +「.ext 文件类型无法预览」+「文件名 · 大小」。
    pub(super) fn render_no_preview(&self, name: &str, size: Option<u64>, t: &Theme) -> AnyElement {
        let ext = ext_of(name);
        let title = if ext.is_empty() {
            div().child(crate::i18n::tr("该文件类型无法预览"))
        } else {
            div().flex().child(div().font_weight(FontWeight::SEMIBOLD).child(format!(".{ext}"))).child(crate::i18n::tr(" 文件类型无法预览"))
        };
        let info = match size {
            Some(s) => format!("{name} · {}", human_size(s)),
            None => name.to_string(),
        };
        div()
            .size_full()
            .px(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(img(no_preview_img(t.dark)).w(px(150.)).h(px(147.)).flex_none())
            .child(title.mt(px(22.)).text_size(px(14.)).text_color(t.text_secondary))
            .child(div().mt(px(8.)).max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).text_color(t.text_muted).child(info))
            .into_any_element()
    }

    /// 文本视图：一行蓝色文件名，点击全屏预览。
    fn render_file_text(&self, ix: usize, id: u64, name: &str, selected: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .child(
                div()
                    .id(("att-text", id as usize))
                    .h(px(body_lh()))
                    .px(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.accent_soft))
                    .hover(|s| s.bg(t.hover))
                    .child(file_icon(name, 17.))
                    .child(div().text_size(px(15.)).text_color(t.accent).child(name.to_string()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                            if !ev.modifiers.shift {
                                this.open_attachment(ix, cx);
                            }
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 卡片视图：文件图标 + 文件名 / 大小，右侧播放（视频）或预览按钮。
    #[allow(clippy::too_many_arguments)]
    fn render_file_card(
        &self,
        ix: usize,
        id: u64,
        path: &str,
        name: &str,
        blk: &Block,
        is_video: bool,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let size = match blk.kind {
            BlockKind::File { size, .. } => size,
            _ => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        };
        div()
            .id(("att-card", id as usize))
            .group("att-card")
            .w(px(CARD_W))
            .max_w_full()
            .h(px(64.))
            .px(px(14.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .bg(t.bg)
            .cursor_pointer()
            .hover(|s| s.border_color(t.accent))
            .child(file_icon(name, 40.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(t.text).whitespace_nowrap().overflow_hidden().text_ellipsis().child(name.to_string()))
                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(human_size(size))),
            )
            .child(
                div()
                    .id(("att-card-act", id as usize))
                    .size(px(30.))
                    .flex_none()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(t.hover))
                    .tooltip(move |_, cx| cx.new(|_| Tooltip(if is_video { "播放" } else { "预览" }.into())).into())
                    .child(icon(if is_video { "play-fill" } else { "eye" }, 16., t.text_secondary)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                    if !ev.modifiers.shift {
                        this.open_attachment(ix, cx);
                    }
                }),
            )
            .into_any_element()
    }

    /// 文档预览视图：首屏快照 + 底部文件名栏；点击快照后切换为可滚动的原生 WebView。
    #[allow(clippy::too_many_arguments)]
    fn render_file_preview(
        &mut self,
        ix: usize,
        blk: &Block,
        path: &str,
        name: &str,
        kind: Preview,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = blk.id;
        let width = content_w() - blk.indent as f32 * 26. - 2.;
        if super::imgview::viewable(name) && Path::new(path).exists() && self.image_size(path).is_some() {
            return self.render_image_file_preview(ix, blk, path, name, width, selected, t, cx);
        }
        let live = self.sess.html_live == Some(id);
        let can_live = is_live_kind(kind);
        let thumb = matches!(kind, Preview::Pdf | Preview::Office | Preview::Image);
        let size = match blk.kind {
            BlockKind::File { size, .. } => Some(size),
            _ => None,
        };
        let exists = Path::new(path).exists();
        match kind {
            Preview::Audio => return self.render_audio(ix, id, path, name, width, selected, t, cx),
            Preview::Archive if exists => return self.render_archive(ix, id, path, name, size.unwrap_or(0), Some(PREVIEW_MAX_H), selected, t, cx),
            Preview::WebVideo if exists => return self.render_web_video(ix, id, name, width, live, selected, t, cx),
            _ => {}
        }
        // (快照图片, 高度)；Err(Some(文案)) = 加载中 / 文件不存在，Err(None) = 无法预览
        let shot: Result<(PathBuf, f32), Option<&'static str>> = if !Path::new(path).exists() {
            Err(Some("文件不存在"))
        } else if kind == Preview::None {
            Err(None)
        } else if thumb {
            match ql_thumb(path, cx) {
                Thumb::Ready(p) => Ok((p, PREVIEW_MAX_H)),
                Thumb::Pending => Err(Some("加载中…")),
                Thumb::Failed => Err(None),
            }
        } else {
            match preview_page(path, name, kind, t.dark).map(|p| crate::webview::request_file(&p, false, width, PREVIEW_MAX_H, cx)) {
                Some(crate::webview::SnapState::Ready { path, height, .. }) => Ok((path, height)),
                Some(crate::webview::SnapState::Pending) => Err(Some("加载中…")),
                _ => Err(None),
            }
        };
        let unsupported = matches!(shot, Err(None));
        let can_live = can_live && !unsupported;
        let h = match &shot {
            Ok((_, h)) => *h,
            Err(Some(_)) => 240.,
            Err(None) => 360.,
        };
        let picture: AnyElement = match shot {
            Ok((p, _)) if thumb => img(p).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            Ok((p, h)) => img(p).w(px(width)).h(px(h)).into_any_element(),
            Err(Some(msg)) => {
                div().size_full().flex().items_center().justify_center().text_size(px(13.)).text_color(t.text_muted).child(msg).into_any_element()
            }
            Err(None) => self.render_no_preview(name, size, t),
        };
        let frame = self.frame.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(if selected || live { t.accent } else { t.border })
            .overflow_hidden()
            .child(
                div()
                    .id(("att-pv", id as usize))
                    .relative()
                    .w_full()
                    .h(px(h))
                    .overflow_hidden()
                    .bg(if thumb || unsupported { t.code_bg } else { t.bg })
                    .child(picture)
                    .when(live, |d| {
                        d.child(
                            canvas(
                                move |b, _, _| {
                                    frame.borrow_mut().html_bounds.insert(id, b);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                            if ev.click_count >= 2 && !can_live {
                                this.open_attachment(ix, cx);
                            } else if can_live && this.sess.html_live != Some(id) {
                                this.sess.html_live = Some(id);
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(self.file_footer(name, size, t))
            .into_any_element()
    }

    /// webm 预览视图：黑底封面 + 播放键，点击后叠加 WebView 播放页（自动播放）。
    #[allow(clippy::too_many_arguments)]
    fn render_web_video(&mut self, ix: usize, id: u64, name: &str, width: f32, live: bool, selected: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let h = (width * 9. / 16.).min(VIDEO_MAX_H);
        let frame = self.frame.clone();
        div()
            .id(("att-webm", id as usize))
            .group("webm")
            .relative()
            .w_full()
            .h(px(h))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(gpui::black())
            .border_1()
            .border_color(if selected || live { t.accent } else { gpui::black() })
            .cursor_pointer()
            .child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    div()
                        .size(px(48.))
                        .rounded_full()
                        .bg(gpui::hsla(0., 0., 1., 0.18))
                        .group_hover("webm", |s| s.bg(gpui::hsla(0., 0., 1., 0.28)))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().pl(px(3.)).child(icon("play-fill", 22., gpui::white()))),
                ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .left(px(8.))
                    .max_w(px((width - 16.).max(40.)))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(gpui::hsla(0., 0., 0., 0.45))
                    .text_size(px(11.5))
                    .text_color(gpui::white())
                    .child(icon("fi-video", 12., gpui::white()))
                    .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.to_string())),
            )
            .when(live, |d| {
                d.child(
                    canvas(
                        move |b, _, _| {
                            frame.borrow_mut().html_bounds.insert(id, b);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                    if this.sess.html_live != Some(id) {
                        this.sess.html_live = Some(id);
                        cx.notify();
                    }
                }),
            )
            .into_any_element()
    }

    /// 图片文件的预览视图：灰底查看区（缩放 / 旋转工具栏）+ 底部文件名栏。
    #[allow(clippy::too_many_arguments)]
    fn render_image_file_preview(
        &mut self,
        ix: usize,
        blk: &Block,
        path: &str,
        name: &str,
        width: f32,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let h = (width * 0.5).clamp(240., PREVIEW_MAX_H);
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .overflow_hidden()
            .child(self.render_img_viewer(ix, blk.id, path, width, h, false, t, cx))
            .child(self.file_footer(name, None, t))
            .into_any_element()
    }

    // ───────────── 视频预览 ─────────────

    /// 点击播放器时选中所在块（音频 / 视频播放器的 `on_press`）。
    pub(super) fn block_press(&self, ix: usize, cx: &mut Context<Self>) -> Press {
        let this = cx.entity().downgrade();
        Rc::new(move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| {
                window.focus(&this.focus);
                this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
            })
            .ok();
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_video(
        &mut self,
        ix: usize,
        blk: &Block,
        path: &str,
        name: &str,
        selected: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = blk.id;
        let meta = super::blocks::video_meta(path, window);
        let (vw, vh) = meta.as_ref().map(|m| (m.1 as f32, m.2 as f32)).unwrap_or((1280., 720.));
        let maxw = content_w() - blk.indent as f32 * 26.;
        let set_w = match &blk.kind {
            BlockKind::Video { width, .. } => *width,
            _ => None,
        };
        let drag_w = self.sess.image_drag.as_ref().filter(|d| d.block == ix).map(|d| d.width);
        let w = match drag_w.or(set_w) {
            Some(w) => w,
            None => {
                let w = if vw > 1200. { vw / 2. } else { vw };
                // 默认高度不超过 VIDEO_MAX_H
                w.min(VIDEO_MAX_H * vw / vh.max(1.))
            }
        }
        .min(maxw)
        .max(VIDEO_MIN_W);
        let h = w * vh / vw.max(1.);
        let this = cx.entity().downgrade();
        let opts = PlayerOpts {
            on_press: Some(self.block_press(ix, cx)),
            on_full: Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| {
                    this.sess.video_full = Some(id);
                    cx.notify();
                })
                .ok();
            })),
        };
        let frame = self.frame.clone();
        // 右下角圆点：拖动等比缩放（视频居中显示，两侧同时变化）
        let dragging = drag_w.is_some();
        let handle = div()
            .id(("v-rs", id as usize))
            .absolute()
            .left(px(w - 7.))
            .top(px(h - 7.))
            .size(px(14.))
            .rounded_full()
            .bg(gpui::white())
            .border_2()
            .border_color(t.accent)
            .shadow_sm()
            .cursor(CursorStyle::ResizeUpLeftDownRight)
            .when(!selected && !dragging, |d| d.invisible().group_hover("video", |s| s.visible()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.sess.image_drag = Some(super::ImageDrag { block: ix, start_w: w, start_px: ev.position, width: w, dir: 2. });
                    cx.notify();
                }),
            );
        let size_tag = dragging.then(|| {
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
                .child(format!("{:.0} × {:.0}", w, h))
        });
        video_player(id, path, name, w, h, &opts, t, window)
            .children(size_tag)
            .when(selected, |d| {
                d.child(div().absolute().top(px(-2.)).left(px(-2.)).w(px(w + 4.)).h(px(h + 4.)).rounded(px(9.)).border_2().border_color(t.accent))
            })
            .child(handle)
            .child(
                canvas(
                    move |b, _, _| {
                        frame.borrow_mut().videos.insert(id, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }
}

// ───────────── 播放器（不依赖编辑器，文档与电子表格附件预览共用） ─────────────

/// 点击播放器时的回调（文档里用来选中块）
pub(crate) type Press = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

#[derive(Default)]
pub(crate) struct PlayerOpts {
    pub on_press: Option<Press>,
    /// 有则显示「全屏」按钮
    pub on_full: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

thread_local! {
    /// 播放器（进度条 / 波形）上一帧的位置：点击跳转时换算比例
    static PLAYER_BOUNDS: RefCell<HashMap<u64, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// 记录播放器位置，并告诉 media 这个播放器本帧可见（不可见的会被自动暂停）。
pub(crate) fn bounds_probe(id: u64) -> impl IntoElement {
    canvas(
        move |b, _, _| {
            PLAYER_BOUNDS.with(|m| m.borrow_mut().insert(id, b));
            crate::media::mark_visible(id);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// 只记录位置（全屏浮层的进度条用，不影响可见性判断）。
pub(crate) fn bounds_only(key: u64) -> impl IntoElement {
    canvas(
        move |b, _, _| {
            PLAYER_BOUNDS.with(|m| m.borrow_mut().insert(key, b));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

pub(crate) fn player_bounds(id: u64) -> Option<Bounds<Pixels>> {
    PLAYER_BOUNDS.with(|m| m.borrow().get(&id).copied())
}

/// 视频播放器（w × h）：封面 / 实时帧、居中播放键与时长、悬停文件名、底部控制条（进度、倍速、静音、全屏）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn video_player(id: u64, path: &str, name: &str, w: f32, h: f32, opts: &PlayerOpts, t: &Theme, window: &mut Window) -> Stateful<Div> {
    let meta = super::blocks::video_meta(path, window);
    let dur = meta.as_ref().map_or(0., |m| m.3);
    let started = crate::media::exists(id);
    let path_s = path.to_string();
    let (playing, pos, total, frame_buf, muted, rate) = if started {
        crate::media::with(id, path, |p| (p.playing, p.position(), p.duration(), p.frame(), p.muted, p.rate)).unwrap_or((false, 0., dur, None, false, 1.))
    } else {
        (false, 0., dur, None, false, 1.)
    };
    if playing {
        window.request_animation_frame();
    }
    let picture: AnyElement = match (&frame_buf, &meta) {
        (Some(buf), _) => surface(buf.clone()).size_full().into_any_element(),
        (None, Some(m)) => img(PathBuf::from(&m.0)).size_full().object_fit(ObjectFit::Cover).into_any_element(),
        (None, None) => div()
            .size_full()
            .bg(gpui::black())
            .flex()
            .items_center()
            .justify_center()
            .text_color(gpui::white())
            .child(crate::i18n::tr("无法读取视频"))
            .into_any_element(),
    };
    let p1 = path_s.clone();
    let press = opts.on_press.clone();
    let toggle = move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
        cx.stop_propagation();
        if let Some(f) = &press {
            f(ev, window, cx);
        }
        crate::media::pause_others(id);
        crate::media::with(id, &p1, |p| p.toggle());
        window.refresh();
    };
    let ctrl_btn = |eid: &'static str, label: String| {
        div()
            .id((eid, id as usize))
            .px_2()
            .py_1()
            .rounded(px(4.))
            .text_size(px(12.))
            .text_color(gpui::white())
            .cursor_pointer()
            .hover(|s| s.bg(gpui::hsla(0., 0., 1., 0.15)))
            .child(crate::i18n::tr_owned(&label))
    };
    let frac = if total > 0. { (pos / total).clamp(0., 1.) as f32 } else { 0. };
    let idle = !started || !playing && pos < 0.05;
    let (p2, p3, p4, p5) = (path_s.clone(), path_s.clone(), path_s.clone(), path_s.clone());
    let on_full = opts.on_full.clone();
    let controls = div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .px_2()
        .pb_1()
        .pt_4()
        .bg(gpui::linear_gradient(180., gpui::linear_color_stop(gpui::hsla(0., 0., 0., 0.), 0.), gpui::linear_color_stop(gpui::hsla(0., 0., 0., 0.6), 1.)))
        .when(playing, |d| d.invisible().group_hover("video", |s| s.visible()))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            // 进度条：点击跳转
            div()
                .id(("v-seek", id as usize))
                .relative()
                .h(px(14.))
                .w_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .child(
                    div()
                        .relative()
                        .h(px(4.))
                        .w_full()
                        .rounded(px(2.))
                        .bg(gpui::hsla(0., 0., 1., 0.3))
                        .child(div().absolute().left_0().top_0().bottom_0().w(relative(frac)).rounded(px(2.)).bg(t.accent)),
                )
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(b) = player_bounds(id) {
                        let f = (f32::from(ev.position.x - b.left()) / f32::from(b.size.width)).clamp(0., 1.);
                        crate::media::with(id, &p2, |p| {
                            let d = p.duration();
                            p.seek(d * f as f64);
                        });
                    }
                    window.refresh();
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(ctrl_btn("v-play", if playing { "❚❚".into() } else { "▶".into() }).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    crate::media::pause_others(id);
                    crate::media::with(id, &p3, |p| p.toggle());
                    window.refresh();
                }))
                .child(div().text_size(px(12.)).text_color(gpui::white()).whitespace_nowrap().child(format!(
                    "{} / {}",
                    crate::media::fmt_time(pos),
                    crate::media::fmt_time(total)
                )))
                .child(div().flex_1())
                .child(ctrl_btn("v-mute", if muted { "🔇".into() } else { "🔊".into() }).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    crate::media::with(id, &p4, |p| p.toggle_mute());
                    window.refresh();
                }))
                .child(ctrl_btn("v-rate", format!("{rate}x")).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    crate::media::with(id, &p5, |p| {
                        let i = crate::media::RATES.iter().position(|r| (*r - p.rate).abs() < 0.01).unwrap_or(1);
                        let r = crate::media::RATES[(i + 1) % crate::media::RATES.len()];
                        p.set_rate(r);
                    });
                    window.refresh();
                }))
                .when_some(on_full, |d, f| {
                    d.child(ctrl_btn("v-full", "⛶".into()).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        f(window, cx);
                    }))
                }),
        );
    // 未播放：居中播放按钮 + 右下角时长
    let idle_layer = idle.then(|| {
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(
                div().size(px(44.)).rounded_full().bg(gpui::hsla(0., 0., 0., 0.45)).flex().items_center().justify_center().child(div().pl(px(3.)).child(icon(
                    "play-fill",
                    20.,
                    gpui::white(),
                ))),
            ))
            .when(total > 0., |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(8.))
                        .bottom(px(8.))
                        .px(px(5.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(gpui::hsla(0., 0., 0., 0.5))
                        .text_size(px(11.))
                        .text_color(gpui::white())
                        .child(crate::media::fmt_time(total)),
                )
            })
    });
    // 悬停时左上角显示文件名
    let name_tag = div()
        .absolute()
        .top(px(8.))
        .left(px(8.))
        .max_w(px((w - 16.).max(40.)))
        .invisible()
        .group_hover("video", |s| s.visible())
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(gpui::hsla(0., 0., 0., 0.45))
        .text_size(px(11.5))
        .text_color(gpui::white())
        .child(icon("fi-video", 12., gpui::white()))
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
        );
    div()
        .id(("video", id as usize))
        .group("video")
        .relative()
        .w(px(w))
        .h(px(h))
        .child(
            div()
                .size_full()
                .relative()
                .rounded(px(8.))
                .overflow_hidden()
                .bg(gpui::black())
                .child(picture)
                .children(idle_layer)
                .child(name_tag)
                .when(started, |d| d.child(controls))
                .child(bounds_probe(id)),
        )
        .on_mouse_down(MouseButton::Left, toggle)
}
