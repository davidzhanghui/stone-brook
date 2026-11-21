//! 界面小部件：内嵌 SVG 图标集、Tooltip、Logo。

use crate::theme::theme;
use gpui::*;
use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

/// 线性图标（24×24，描边风格），路径数据内嵌在二进制里，无需外部资源文件。
fn icon_body(name: &str) -> Option<&'static str> {
    Some(match name {
        "check" => r##"<path d="M20 6 9 17l-5-5"/>"##,
        "x" => r##"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"##,
        "paintbrush" => {
            r##"<path d="m14.622 17.897-10.68-2.913"/><path d="M18.376 2.622a1 1 0 1 1 3.002 3.002L17.36 9.643a.5.5 0 0 0 0 .707l.944.944a2.41 2.41 0 0 1 0 3.408l-.944.944a.5.5 0 0 1-.707 0L8.354 7.348a.5.5 0 0 1 0-.707l.944-.944a2.41 2.41 0 0 1 3.408 0l.944.944a.5.5 0 0 0 .707 0z"/><path d="M9 8c-1.804 2.71-3.97 3.46-6.583 3.948a.507.507 0 0 0-.302.819l7.32 8.883a1 1 0 0 0 1.185.204C12.735 20.405 16 16.792 16 15"/>"##
        }
        "plus" => r##"<path d="M5 12h14"/><path d="M12 5v14"/>"##,
        "search" => r##"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"##,
        "grip" => {
            r##"<circle cx="9" cy="5" r="1.2" fill="#000"/><circle cx="9" cy="12" r="1.2" fill="#000"/><circle cx="9" cy="19" r="1.2" fill="#000"/><circle cx="15" cy="5" r="1.2" fill="#000"/><circle cx="15" cy="12" r="1.2" fill="#000"/><circle cx="15" cy="19" r="1.2" fill="#000"/>"##
        }
        "chevron-down" => r##"<path d="m6 9 6 6 6-6"/>"##,
        "chevron-up" => r##"<path d="m18 15-6-6-6 6"/>"##,
        "chevron-right" => r##"<path d="m9 18 6-6-6-6"/>"##,
        "copy" => r##"<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"##,
        "wrap" => {
            r##"<line x1="3" x2="21" y1="6" y2="6"/><path d="M3 12h15a3 3 0 1 1 0 6h-4"/><polyline points="16 16 14 18 16 20"/><line x1="3" x2="10" y1="18" y2="18"/>"##
        }
        "image" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/>"##,
        "arrow-up" => r##"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"##,
        "arrow-down" => r##"<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>"##,
        "trash" => r##"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"##,
        "link" => {
            r##"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"##
        }
        "bold" => r##"<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/>"##,
        "italic" => r##"<line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/>"##,
        "underline" => r##"<path d="M6 4v6a6 6 0 0 0 12 0V4"/><line x1="4" x2="20" y1="20" y2="20"/>"##,
        "strike" => r##"<path d="M16 4H9a3 3 0 0 0-2.83 4"/><path d="M14 12a4 4 0 0 1 0 8H6"/><line x1="4" x2="20" y1="12" y2="12"/>"##,
        "code" => r##"<polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>"##,
        "palette" => {
            r##"<circle cx="13.5" cy="6.5" r="1" fill="#000"/><circle cx="17.5" cy="10.5" r="1" fill="#000"/><circle cx="8.5" cy="7.5" r="1" fill="#000"/><circle cx="6.5" cy="12.5" r="1" fill="#000"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.93 0 1.65-.75 1.65-1.69 0-.44-.18-.84-.44-1.13-.29-.29-.44-.65-.44-1.13a1.64 1.64 0 0 1 1.67-1.67h2c3.05 0 5.55-2.5 5.55-5.55C21.97 6.01 17.46 2 12 2z"/>"##
        }
        "eraser" => {
            r##"<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>"##
        }
        "file" => {
            r##"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"##
        }
        "sun" => {
            r##"<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>"##
        }
        "moon" => r##"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"##,
        "book-open" => r##"<path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>"##,
        "monitor" => r##"<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>"##,
        "activity" => r##"<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>"##,
        "outline" => r##"<path d="M3 6h18"/><path d="M8 12h13"/><path d="M13 18h8"/>"##,
        "more" => r##"<circle cx="12" cy="12" r="1.3" fill="#000"/><circle cx="19" cy="12" r="1.3" fill="#000"/><circle cx="5" cy="12" r="1.3" fill="#000"/>"##,
        "sidebar" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>"##,
        "download" => r##"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>"##,
        "upload" => r##"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m17 8-5-5-5 5"/><path d="M12 3v12"/>"##,
        "info" => r##"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"##,
        "zap" => {
            r##"<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>"##
        }
        "restore" => r##"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"##,
        "focus" => {
            r##"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"##
        }
        "clock" => r##"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"##,
        "lock" => r##"<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"##,
        "feather" => {
            r##"<path d="M12.67 19a2 2 0 0 0 1.416-.588l6.154-6.172a6 6 0 0 0-8.49-8.49L5.586 9.914A2 2 0 0 0 5 11.328V18a1 1 0 0 0 1 1z"/><path d="M16 8 2 22"/><path d="M17.5 15H9"/>"##
        }
        "type" => r##"<polyline points="4 7 4 4 20 4 20 7"/><line x1="9" x2="15" y1="20" y2="20"/><line x1="12" x2="12" y1="4" y2="20"/>"##,
        "list-ordered" => {
            r##"<path d="M10 12h11"/><path d="M10 18h11"/><path d="M10 6h11"/><path d="M4 10h2"/><path d="M4 6h1v4"/><path d="M6 18H4c0-1 2-2 2-3s-1-1.5-2-1"/>"##
        }
        "list" => r##"<path d="M3 12h.01"/><path d="M3 18h.01"/><path d="M3 6h.01"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M8 6h13"/>"##,
        "braces" => {
            r##"<path d="M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5c0 1.1.9 2 2 2h1"/><path d="M16 21h1a2 2 0 0 0 2-2v-5c0-1.1.9-2 2-2a2 2 0 0 1-2-2V5a2 2 0 0 0-2-2h-1"/>"##
        }
        "quote" => {
            r##"<path d="M16 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z"/><path d="M5 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z"/>"##
        }
        "divider" => {
            r##"<path d="M3 12h18"/><path d="M3 6h4"/><path d="M10 6h4"/><path d="M17 6h4"/><path d="M3 18h4"/><path d="M10 18h4"/><path d="M17 18h4"/>"##
        }
        "square-check" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="m9 12 2 2 4-4"/>"##,
        "paperclip" => r##"<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>"##,
        "table" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 3v18"/>"##,
        "columns" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M12 3v18"/>"##,
        "callout" => r##"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M7 8h10"/><path d="M7 12h6"/>"##,
        "sync" => {
            r##"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>"##
        }
        "sigma" => r##"<path d="M18 7V5a1 1 0 0 0-1-1H6.5a.5.5 0 0 0-.4.8l4.5 6a2 2 0 0 1 0 2.4l-4.5 6a.5.5 0 0 0 .4.8H17a1 1 0 0 0 1-1v-2"/>"##,
        "template" => {
            r##"<rect width="18" height="7" x="3" y="3" rx="1"/><rect width="9" height="7" x="3" y="14" rx="1"/><rect width="5" height="7" x="16" y="14" rx="1"/>"##
        }
        "sheet" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 3v18"/><path d="M15 3v18"/>"##,
        "pen-tool" => {
            r##"<path d="M15.707 21.293a1 1 0 0 1-1.414 0l-1.586-1.586a1 1 0 0 1 0-1.414l5.586-5.586a1 1 0 0 1 1.414 0l1.586 1.586a1 1 0 0 1 0 1.414z"/><path d="m18 13-1.375-6.874a1 1 0 0 0-.746-.776L3.235 2.028a1 1 0 0 0-1.207 1.207L5.35 15.879a1 1 0 0 0 .776.746L13 18"/><path d="m2.3 2.3 7.286 7.286"/><circle cx="11" cy="11" r="2"/>"##
        }
        "mindmap" => {
            r##"<rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3"/><path d="M12 12V8"/>"##
        }
        "workflow" => {
            r##"<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>"##
        }
        "shapes" => {
            r##"<path d="M8.3 10a.7.7 0 0 1-.626-1.079L11.4 3a.7.7 0 0 1 1.198-.043L16.3 8.9a.7.7 0 0 1-.572 1.1Z"/><rect x="3" y="14" width="7" height="7" rx="1"/><circle cx="17.5" cy="17.5" r="3.5"/>"##
        }
        "globe" => r##"<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>"##,
        "hourglass" => {
            r##"<path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>"##
        }
        "users" => {
            r##"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>"##
        }
        "target" => r##"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/>"##,
        "book" => r##"<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"/>"##,
        "bug" => {
            r##"<path d="m8 2 1.88 1.88"/><path d="M14.12 3.88 16 2"/><path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1"/><path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"/><path d="M12 20v-9"/><path d="M6.53 9C4.6 8.8 3 7.1 3 5"/><path d="M6 13H2"/><path d="M3 21c0-2.1 1.7-3.9 3.8-4"/><path d="M20.97 5c0 2.1-1.6 3.8-3.5 4"/><path d="M22 13h-4"/><path d="M17.2 17c2.1.1 3.8 1.9 3.8 4"/>"##
        }
        "layout-template" => {
            r##"<rect width="18" height="7" x="3" y="3" rx="1"/><rect width="9" height="7" x="3" y="14" rx="1"/><rect width="5" height="7" x="16" y="14" rx="1"/>"##
        }
        "calendar" => r##"<path d="M8 2v4"/><path d="M16 2v4"/><rect width="18" height="18" x="3" y="4" rx="2"/><path d="M3 10h18"/>"##,
        "code-xml" => r##"<path d="m18 16 4-4-4-4"/><path d="m6 8-4 4 4 4"/><path d="m14.5 4-5 16"/>"##,
        "sitemap" => {
            r##"<rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3"/><path d="M12 12V8"/>"##
        }
        "square-code" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="m10 9-3 3 3 3"/><path d="m14 15 3-3-3-3"/>"##,
        "square-type" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 8h8"/><path d="M12 8v8"/>"##,
        "list-tree" => {
            r##"<path d="M21 12h-8"/><path d="M21 6H8"/><path d="M21 18h-8"/><path d="M3 6v4c0 1.1.9 2 2 2h3"/><path d="M3 10v6c0 1.1.9 2 2 2h3"/>"##
        }
        "timeline" => r##"<path d="M12 3v6"/><circle cx="12" cy="12" r="3"/><path d="M12 15v6"/>"##,
        "video" => {
            r##"<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>"##
        }
        "external" => r##"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"##,
        "chart-pie" => {
            r##"<path d="M21 12c.552 0 1.005-.449.95-.998a10 10 0 0 0-8.953-8.951c-.55-.055-.998.398-.998.95v8a1 1 0 0 0 1 1z"/><path d="M21.21 15.89A10 10 0 1 1 8 2.83"/>"##
        }
        "gantt" => r##"<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M7 7h8"/><path d="M11 12h8"/><path d="M9 17h5"/>"##,
        "database" => r##"<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/>"##,
        "command" => r##"<path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/>"##,
        "settings" => {
            r##"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"##
        }
        "hand" => {
            r##"<path d="M18 11V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2"/><path d="M14 10V4a2 2 0 0 0-2-2a2 2 0 0 0-2 2v2"/><path d="M10 10.5V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2v8"/><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15"/>"##
        }
        "pointer" => {
            r##"<path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"/>"##
        }
        "square" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/>"##,
        "circle" => r##"<circle cx="12" cy="12" r="10"/>"##,
        "diamond" => {
            r##"<path d="M2.7 10.3a2.41 2.41 0 0 0 0 3.41l7.59 7.59a2.41 2.41 0 0 0 3.41 0l7.59-7.59a2.41 2.41 0 0 0 0-3.41l-7.59-7.59a2.41 2.41 0 0 0-3.41 0Z"/>"##
        }
        "arrow-right" => r##"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"##,
        "minus" => r##"<path d="M5 12h14"/>"##,
        "pencil" => {
            r##"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/>"##
        }
        "sticky" => r##"<path d="M16 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V8Z"/><path d="M15 3v4a2 2 0 0 0 2 2h4"/>"##,
        "eraser2" => r##"<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/>"##,
        "group" => {
            r##"<path d="M3 7V5c0-1.1.9-2 2-2h2"/><path d="M17 3h2c1.1 0 2 .9 2 2v2"/><path d="M21 17v2c0 1.1-.9 2-2 2h-2"/><path d="M7 21H5c-1.1 0-2-.9-2-2v-2"/><rect width="7" height="5" x="7" y="7" rx="1"/><rect width="7" height="5" x="10" y="12" rx="1"/>"##
        }
        "bring-front" => {
            r##"<rect x="8" y="8" width="8" height="8" rx="2"/><path d="M4 10a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2"/><path d="M14 20a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2v-4a2 2 0 0 0-2-2"/>"##
        }
        "align-left" => r##"<rect width="6" height="16" x="4" y="2" rx="2"/><rect width="6" height="9" x="14" y="9" rx="2"/><path d="M22 22H2"/>"##,
        "zoom-in" => r##"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/><path d="M11 8v6"/><path d="M8 11h6"/>"##,
        "zoom-out" => r##"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/><path d="M8 11h6"/>"##,
        "rotate-ccw" => r##"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"##,
        "fit-page" => {
            r##"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/><rect x="8" y="8" width="8" height="8" rx="1"/>"##
        }
        "actual-size" => {
            r##"<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M8 10v5"/><path d="M16 10v5"/><path d="M12 10.5v.01"/><path d="M12 14.5v.01"/>"##
        }
        "maximize" => r##"<path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/>"##,
        "layout" => {
            r##"<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>"##
        }
        "presentation" => r##"<path d="M2 3h20"/><path d="M21 3v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V3"/><path d="m7 21 5-5 5 5"/>"##,
        "flame" => {
            r##"<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>"##
        }
        "crop" => r##"<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>"##,
        "align-center" => r##"<path d="M17 4H3"/><path d="M21 9H3"/><path d="M17 14H3"/><path d="M21 19H3"/>"##,
        "align-right" => r##"<path d="M21 4H3"/><path d="M21 9H11"/><path d="M21 14H3"/><path d="M21 19H11"/>"##,
        "comment" => r##"<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>"##,
        "expand" => {
            r##"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"##
        }
        "rotate" => r##"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"##,
        "circle-dot" => r##"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3" fill="#000"/>"##,
        "scissors" => {
            r##"<circle cx="6" cy="6" r="3"/><path d="M8.12 8.12 12 12"/><path d="M20 4 8.12 15.88"/><circle cx="6" cy="18" r="3"/><path d="M14.8 14.8 20 20"/>"##
        }
        // 表格操作
        "row-above" => r##"<rect x="3" y="11" width="18" height="10" rx="2"/><path d="M12 2v6"/><path d="M9 5h6"/>"##,
        "row-below" => r##"<rect x="3" y="3" width="18" height="10" rx="2"/><path d="M12 16v6"/><path d="M9 19h6"/>"##,
        "col-left" => r##"<rect x="11" y="3" width="10" height="18" rx="2"/><path d="M2 12h6"/><path d="M5 9v6"/>"##,
        "col-right" => r##"<rect x="3" y="3" width="10" height="18" rx="2"/><path d="M16 12h6"/><path d="M19 9v6"/>"##,
        "row-del" => r##"<rect x="3" y="4" width="18" height="7" rx="1.5"/><path d="m9 15 6 6"/><path d="m15 15-6 6"/>"##,
        "col-del" => r##"<rect x="4" y="3" width="7" height="18" rx="1.5"/><path d="m15 9 6 6"/><path d="m21 9-6 6"/>"##,
        "header-row" => r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 3h18v6H3z" fill="#000"/><path d="M9 9v12"/><path d="M15 9v12"/>"##,
        "header-col" => r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 3h6v18H3z" fill="#000"/><path d="M9 9h12"/><path d="M9 15h12"/>"##,
        "cells-merge" => {
            r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 21v-6"/><path d="M12 9V3"/><path d="M3 15h18"/><path d="M3 9h18"/><path d="m8 12 2 0"/><path d="m14 12 2 0"/>"##
        }
        "cells-split" => r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 3v18"/><path d="M3 12h18"/>"##,
        "cols-even" => {
            r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/><path d="M15 3v18"/><path d="m5 12 2 0"/><path d="m17 12 2 0"/>"##
        }
        "img-left" => r##"<rect x="3" y="6" width="10" height="12" rx="1.5"/><path d="M16 8h5"/><path d="M16 12h5"/><path d="M16 16h5"/>"##,
        "img-center" => r##"<rect x="7" y="7" width="10" height="10" rx="1.5"/><path d="M3 3h18"/><path d="M3 21h18"/>"##,
        "img-right" => r##"<rect x="11" y="6" width="10" height="12" rx="1.5"/><path d="M3 8h5"/><path d="M3 12h5"/><path d="M3 16h5"/>"##,
        "size-reset" => r##"<path d="M21 9V3h-6"/><path d="M3 15v6h6"/><rect x="8" y="8" width="8" height="8" rx="1"/>"##,
        // 画板
        "section" => r##"<rect x="3" y="7" width="18" height="14" rx="2" stroke-dasharray="3 2"/><path d="M3 4h7"/>"##,
        "conn-straight" => r##"<path d="M4 20 20 4"/><path d="M13 4h7v7"/>"##,
        "conn-elbow" => r##"<path d="M4 20V10h16"/><path d="m16 6 4 4-4 4"/>"##,
        "conn-curve" => r##"<path d="M4 20C4 10 14 14 20 4"/><path d="M14 4h6v6"/>"##,
        "conn-round-elbow" => r##"<path d="M4 20v-5a5 5 0 0 1 5-5h11"/><path d="m16 6 4 4-4 4"/>"##,
        "line-jump" => r##"<path d="M3 15h5a4 4 0 0 1 8 0h5"/><path d="M12 5v14" opacity=".45"/>"##,
        "conn-line" => r##"<path d="M4 20 20 4"/>"##,
        "grid" => r##"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 3v18"/><path d="M15 3v18"/>"##,
        "map" => {
            r##"<path d="M14.1 4.6 9.9 2.5a2 2 0 0 0-1.8 0L3.7 4.7A1.2 1.2 0 0 0 3 5.8v14a1 1 0 0 0 1.4.9l3.7-1.9a2 2 0 0 1 1.8 0l4.2 2.1a2 2 0 0 0 1.8 0l4.4-2.2a1.2 1.2 0 0 0 .7-1.1v-14a1 1 0 0 0-1.4-.9l-3.7 1.9a2 2 0 0 1-1.8 0z"/><path d="M15 5v16"/><path d="M9 3v16"/>"##
        }
        "unlock" => r##"<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>"##,
        "redo" => r##"<path d="M21 12a9 9 0 1 1-9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>"##,
        "text-left" => r##"<path d="M15 12H3"/><path d="M17 18H3"/><path d="M21 6H3"/>"##,
        "text-center" => r##"<path d="M17 12H7"/><path d="M19 18H5"/><path d="M21 6H3"/>"##,
        "text-right" => r##"<path d="M21 12H9"/><path d="M21 18H7"/><path d="M21 6H3"/>"##,
        "fill" => {
            r##"<path d="m19 11-8-8-8.6 8.6a2 2 0 0 0 0 2.8l5.2 5.2c.8.8 2 .8 2.8 0L19 11Z"/><path d="m5 2 5 5"/><path d="M2 13h15"/><path d="M22 20a2 2 0 1 1-4 0c0-1.6 1.7-2.4 2-4 .3 1.6 2 2.4 2 4Z"/>"##
        }
        "highlighter" => r##"<path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4"/>"##,
        "align-v-top" => r##"<rect x="5" y="6" width="5" height="12" rx="1"/><rect x="14" y="6" width="5" height="7" rx="1"/><path d="M3 3h18"/>"##,
        "align-v-mid" => {
            r##"<rect x="5" y="5" width="5" height="14" rx="1"/><rect x="14" y="8" width="5" height="8" rx="1"/><path d="M3 12h2"/><path d="M10 12h4"/><path d="M19 12h2"/>"##
        }
        "align-v-bottom" => r##"<rect x="5" y="6" width="5" height="12" rx="1"/><rect x="14" y="11" width="5" height="7" rx="1"/><path d="M3 21h18"/>"##,
        "align-h-left" => r##"<rect x="6" y="5" width="12" height="5" rx="1"/><rect x="6" y="14" width="7" height="5" rx="1"/><path d="M3 3v18"/>"##,
        "align-h-mid" => {
            r##"<rect x="5" y="5" width="14" height="5" rx="1"/><rect x="8" y="14" width="8" height="5" rx="1"/><path d="M12 3v2"/><path d="M12 10v4"/><path d="M12 19v2"/>"##
        }
        "align-h-right" => r##"<rect x="6" y="5" width="12" height="5" rx="1"/><rect x="11" y="14" width="7" height="5" rx="1"/><path d="M21 3v18"/>"##,
        "same-size" => r##"<rect x="3" y="5" width="7" height="14" rx="1"/><rect x="14" y="5" width="7" height="14" rx="1"/>"##,
        "image-view" => r##"<path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/>"##,
        "ban" => r##"<circle cx="12" cy="12" r="10" /><path d="m4.9 4.9 14.2 14.2" />"##,
        "strikethrough" => r##"<path d="M16 4H9a3 3 0 0 0-2.83 4" /><path d="M14 12a4 4 0 0 1 0 8H6" /><line x1="4" x2="20" y1="12" y2="12" />"##,
        "arrow-up-to-line" => r##"<path d="M5 3h14" /><path d="m18 13-6-6-6 6" /><path d="M12 7v14" />"##,
        "fold-vertical" => {
            r##"<path d="M12 22v-6" /><path d="M12 8V2" /><path d="M4 12H2" /><path d="M10 12H8" /><path d="M16 12h-2" /><path d="M22 12h-2" /><path d="m15 19-3-3-3 3" /><path d="m15 5-3 3-3-3" />"##
        }
        "arrow-down-to-line" => r##"<path d="M12 17V3" /><path d="m6 11 6 6 6-6" /><path d="M19 21H5" />"##,
        "flip-horizontal-2" => {
            r##"<path d="m3 7 5 5-5 5V7" /><path d="m21 7-5 5 5 5V7" /><path d="M12 20v2" /><path d="M12 14v2" /><path d="M12 8v2" /><path d="M12 2v2" />"##
        }
        "flip-vertical-2" => {
            r##"<path d="m17 3-5 5-5-5h10" /><path d="m17 21-5-5-5 5h10" /><path d="M4 12H2" /><path d="M10 12H8" /><path d="M16 12h-2" /><path d="M22 12h-2" />"##
        }
        "layers" => {
            r##"<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z" /><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65" /><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65" />"##
        }
        "smile-plus" => {
            r##"<path d="M22 11v1a10 10 0 1 1-9-10" /><path d="M8 14s1.5 2 4 2 4-2 4-2" /><line x1="9" x2="9.01" y1="9" y2="9" /><line x1="15" x2="15.01" y1="9" y2="9" /><path d="M16 5h6" /><path d="M19 2v6" />"##
        }
        "clipboard-paste" => {
            r##"<path d="M15 2H9a1 1 0 0 0-1 1v2c0 .6.4 1 1 1h6c.6 0 1-.4 1-1V3c0-.6-.4-1-1-1Z" /><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2M16 4h2a2 2 0 0 1 2 2v2M11 14h10" /><path d="m17 10 4 4-4 4" />"##
        }
        "image-down" => {
            r##"<path d="M10.3 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v10l-3.1-3.1a2 2 0 0 0-2.814.014L6 21" /><path d="m14 19 3 3v-5.5" /><path d="m17 22 3-3" /><circle cx="9" cy="9" r="2" />"##
        }
        "f-text" => r##"<path d="M17 6.1H3"/><path d="M21 12.1H3"/><path d="M15.1 18H3"/>"##,
        "f-number" => {
            r##"<line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/>"##
        }
        "f-select" => r##"<circle cx="12" cy="12" r="10"/><path d="m16 10-4 4-4-4"/>"##,
        "f-multi" => r##"<path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/>"##,
        "f-person" => r##"<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>"##,
        "f-phone" => {
            r##"<path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"/>"##
        }
        "f-mail" => r##"<rect width="20" height="16" x="2" y="4" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>"##,
        "f-progress" => r##"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"##,
        "f-currency" => r##"<circle cx="12" cy="12" r="10"/><path d="M16 8h-6a2 2 0 1 0 0 4h4a2 2 0 1 1 0 4H8"/><path d="M12 18V6"/>"##,
        "f-star" => r##"<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>"##,
        "f-heart" => {
            r##"<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>"##
        }
        "f-thumb" => {
            r##"<path d="M7 10v12"/><path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2h0a3.13 3.13 0 0 1 3 3.88Z"/>"##
        }
        "f-fire" => {
            r##"<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>"##
        }
        "f-star-on" => r##"<polygon fill="#000" points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>"##,
        "f-heart-on" => {
            r##"<path fill="#000" d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>"##
        }
        "f-thumb-on" => {
            r##"<path fill="#000" d="M7 10v12"/><path fill="#000" d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2h0a3.13 3.13 0 0 1 3 3.88Z"/>"##
        }
        "f-fire-on" => {
            r##"<path fill="#000" d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>"##
        }
        "f-stage" => r##"<path d="m6 17 5-5-5-5"/><path d="m13 17 5-5-5-5"/>"##,
        "f-link" => r##"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"##,
        "f-duplex" => r##"<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>"##,
        "f-button" => r##"<rect x="3" y="7" width="18" height="10" rx="3"/><path d="M8 12h8"/>"##,
        "chart-bar" => r##"<path d="M3 3v18h18"/><path d="M8 17v-5"/><path d="M13 17V8"/><path d="M18 17v-9"/>"##,
        "chart-line" => r##"<path d="M3 3v18h18"/><path d="m7 15 4-5 4 3 5-6"/>"##,
        "dashboard" => {
            r##"<rect x="3" y="3" width="7" height="9" rx="1"/><rect x="14" y="3" width="7" height="5" rx="1"/><rect x="14" y="12" width="7" height="9" rx="1"/><rect x="3" y="16" width="7" height="5" rx="1"/>"##
        }
        "f-history" => r##"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>"##,
        "v-grid" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 9v12"/>"##,
        "v-kanban" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 7v7"/><path d="M12 7v4"/><path d="M16 7v9"/>"##,
        "v-gallery" => {
            r##"<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>"##
        }
        "v-form" => {
            r##"<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/>"##
        }
        "filter" => r##"<polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3"/>"##,
        "sort" => r##"<path d="m21 16-4 4-4-4"/><path d="M17 20V4"/><path d="m3 8 4-4 4 4"/><path d="M7 4v16"/>"##,
        "rows" => r##"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/>"##,
        "eye" => {
            r##"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"##
        }
        "eye-off" => {
            r##"<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>"##
        }
        "expand-2" => r##"<path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/>"##,
        "group-by" => r##"<path d="M3 5h18"/><path d="M7 10h14"/><path d="M7 15h14"/><path d="M3 20h18"/><path d="M3 10v5"/>"##,
        "base" => r##"<rect width="18" height="18" x="3" y="3" rx="3"/><path d="M3 9h18"/><path d="M9 9v12"/><circle cx="15.5" cy="15" r="2.5"/>"##,
        "chevron-left" => r##"<path d="m15 18-6-6 6-6"/>"##,
        "table-props" => r##"<path d="M12 3v18"/><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/>"##,
        "upload-file" => r##"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" x2="12" y1="3" y2="15"/>"##,
        "grip-v" => {
            r##"<circle cx="9" cy="5" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="19" r="1"/>"##
        }
        "sparkles" => {
            r##"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/>"##
        }
        "triangle" => r##"<path d="M13.73 4a2 2 0 0 0-3.46 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/>"##,
        "smartphone" => r##"<rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/>"##,
        // 「视频或文件」：三种视图（工具栏）与文件类型图标
        "view-text" => r##"<path d="M3 12h2.5"/><path d="M9 12h12"/>"##,
        "view-card" => r##"<path d="M3 4h18"/><rect x="3" y="8" width="18" height="8" rx="1.5"/><path d="M3 20h18"/>"##,
        "view-preview" => r##"<rect x="3" y="3" width="18" height="13" rx="2"/><path d="M3 20.5h18"/>"##,
        "save-as" => {
            r##"<path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"/><path d="M7 3v4a1 1 0 0 0 1 1h7"/>"##
        }
        "folder" => r##"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"##,
        "play-fill" => r##"<path d="M7 4.5v15l12-7.5z" fill="#000"/>"##,
        "pause-fill" => {
            r##"<rect x="6" y="4.5" width="4" height="15" rx="1" fill="#000" stroke="none"/><rect x="14" y="4.5" width="4" height="15" rx="1" fill="#000" stroke="none"/>"##
        }
        "folder-fill" => {
            r##"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="#000" stroke="none"/>"##
        }
        "file-shape" => r##"<path d="M6.5 1.5h8.5l6 6v13a2 2 0 0 1-2 2h-12.5a2 2 0 0 1-2-2v-17a2 2 0 0 1 2-2z" fill="#000" stroke-width="1"/>"##,
        "fi-video" => {
            r##"<rect x="4" y="8" width="11" height="9" rx="1.5" fill="#000" stroke="none"/><path d="M15.5 11.5 20 9v7l-4.5-2.5z" fill="#000" stroke="none"/>"##
        }
        "fi-code" => r##"<path d="m9 8-4 4.5L9 17"/><path d="m15 8 4 4.5-4 4.5"/>"##,
        "fi-lines" => r##"<path d="M7 9h10"/><path d="M7 13h10"/><path d="M7 17h6"/>"##,
        "fi-music" => r##"<path d="M10 17V7l8-2v10"/><circle cx="8" cy="17" r="2" fill="#000"/><circle cx="16" cy="15" r="2" fill="#000"/>"##,
        "fi-zip" => r##"<path d="M12 4v2"/><path d="M12 8v2"/><path d="M12 12v2"/><rect x="10" y="15" width="4" height="4" rx="1"/>"##,
        "fi-image" => r##"<circle cx="9" cy="10" r="1.6" fill="#000"/><path d="m5 18 4.5-4.5 3 3 2.5-2.5 4 4"/>"##,
        "lightbulb" => {
            r##"<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"/><path d="M9 18h6"/><path d="M10 22h4"/>"##
        }
        _ => return None,
    })
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let name = path.trim_start_matches("icons/").trim_end_matches(".svg");
        // 画板图标库：icons/lib/<名字>.svg
        let body = match name.strip_prefix("lib/") {
            Some(n) => return Ok(crate::board::iconlib::svg(n, None).map(|s| Cow::Owned(s.into_bytes()))),
            None => icon_body(name).or_else(|| crate::sheet::icons::body(name)).or_else(|| crate::sketch::icons::body(name)),
        };
        Ok(body.map(|body| {
            Cow::Owned(
                format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
                )
                .into_bytes(),
            )
        }))
    }

    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg().path(format!("icons/{name}.svg")).size(px(size)).flex_none().text_color(color)
}

static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../assets/logo-256.png").to_vec())));

pub fn logo(size: f32) -> Img {
    img(LOGO.clone()).size(px(size)).flex_none()
}

pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let (bg, fg) = if t.dark { (rgb(0xe3e4e6), rgb(0x1f2329)) } else { (rgb(0x1f2329), rgb(0xffffff)) };
        div().mt_1().px_2().py(px(3.)).rounded(px(5.)).bg(bg).text_color(fg).text_size(px(12.)).font_family("PingFang SC").child(translate_tip(&self.0))
    }
}

/// 提示文字：整句在翻译表里就整句翻；否则把第一个空格前的部分当名称翻（「粗体 ⌘B」→「Bold ⌘B」）。
fn translate_tip(s: &str) -> String {
    if !crate::i18n::english() {
        return s.to_string();
    }
    let whole = crate::i18n::tr_owned(s);
    if whole != s {
        return whole;
    }
    match s.split_once(' ') {
        Some((head, rest)) => format!("{} {rest}", crate::i18n::tr_owned(head)),
        None => whole,
    }
}

/// 一个方形图标按钮。
pub fn icon_button(id: impl Into<ElementId>, name: &'static str, tip: &'static str, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(icon(name, 16., t.text_secondary))
        .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
}

/// 拖动滚动条时记下的 (按下时鼠标 y, 按下时滚动距离)。
pub type ScrollDrag = std::rc::Rc<std::cell::Cell<Option<(Pixels, Pixels)>>>;

/// 叠加在滚动容器右侧的竖直滚动条（弹出菜单：灰色圆角拇指，内容放得下时不显示）。
/// 放在滚动容器之后、同一个 `relative` 父元素里；拖动拇指滚动，点轨道空白处跳到该位置。
pub fn scrollbar(handle: ScrollHandle, drag: ScrollDrag, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let max = handle.max_offset().height;
            if max <= px(0.5) || view.size.height <= px(20.) {
                return;
            }
            let inset = px(4.);
            let track_h = view.size.height - inset * 2.;
            let thumb_h = (track_h * (view.size.height / (view.size.height + max))).max(px(28.));
            let scrolled = -handle.offset().y;
            let thumb_top = view.top() + inset + (track_h - thumb_h) * (scrolled / max);
            let track = Bounds::new(point(view.right() - px(10.), view.top()), size(px(10.), view.size.height));
            let thumb = Bounds::new(point(view.right() - px(8.), thumb_top), size(px(5.), thumb_h));
            let active = drag.get().is_some();
            window.paint_quad(fill(thumb, color.opacity(if active { 0.75 } else { 0.5 })).corner_radii(px(2.5)));
            let per_px = max / (track_h - thumb_h).max(px(1.));
            let (d, h) = (drag.clone(), handle.clone());
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left || !track.contains(&ev.position) {
                    return;
                }
                if !thumb.contains(&ev.position) {
                    // 点在轨道上：让拇指中心跳到点击处
                    let y = ((ev.position.y - track.top() - inset - thumb_h / 2.) * per_px).clamp(px(0.), max);
                    h.set_offset(point(h.offset().x, -y));
                }
                d.set(Some((ev.position.y, -h.offset().y)));
                cx.stop_propagation();
                window.refresh();
            });
            let (d, h) = (drag.clone(), handle.clone());
            window.on_mouse_event(move |ev: &MouseMoveEvent, _, window, cx| {
                let Some((y0, s0)) = d.get() else { return };
                if !ev.dragging() {
                    d.set(None);
                    window.refresh();
                    return;
                }
                let y = (s0 + (ev.position.y - y0) * per_px).clamp(px(0.), max);
                h.set_offset(point(h.offset().x, -y));
                cx.stop_propagation();
                window.refresh();
            });
            let d = drag.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, window, cx| {
                if d.take().is_some() {
                    cx.stop_propagation();
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// 叠加在滚动容器底部的水平滚动条（代码块关闭自动换行时使用）；拖动拇指或点轨道滚动。
/// `viewport` 是纵向滚动容器：代码块同时横向 + 纵向滚动时，滚动条要贴着「可视区底部」而不是内容底部。
pub fn hscrollbar(handle: ScrollHandle, viewport: ScrollHandle, drag: ScrollDrag, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let bottom = viewport.bounds().bottom();
            let max = handle.max_offset().width;
            if max <= px(0.5) || view.size.width <= px(20.) {
                return;
            }
            let inset = px(4.);
            let track_w = view.size.width - inset * 2.;
            let thumb_w = (track_w * (view.size.width / (view.size.width + max))).max(px(28.));
            let scrolled = -handle.offset().x;
            let thumb_left = view.left() + inset + (track_w - thumb_w) * (scrolled / max);
            // 往上让 4px，给代码块底部的拖拽手柄留出空间
            let track = Bounds::new(point(view.left(), bottom - px(14.)), size(view.size.width, px(10.)));
            let thumb = Bounds::new(point(thumb_left, bottom - px(12.)), size(thumb_w, px(5.)));
            let active = drag.get().is_some();
            window.paint_quad(fill(thumb, color.opacity(if active { 0.75 } else { 0.5 })).corner_radii(px(2.5)));
            let per_px = max / (track_w - thumb_w).max(px(1.));
            let (d, h) = (drag.clone(), handle.clone());
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left || !track.contains(&ev.position) {
                    return;
                }
                if !thumb.contains(&ev.position) {
                    // 点在轨道上：让拇指中心跳到点击处
                    let x = ((ev.position.x - track.left() - inset - thumb_w / 2.) * per_px).clamp(px(0.), max);
                    h.set_offset(point(-x, h.offset().y));
                }
                d.set(Some((ev.position.x, -h.offset().x)));
                cx.stop_propagation();
                window.refresh();
            });
            let (d, h) = (drag.clone(), handle.clone());
            window.on_mouse_event(move |ev: &MouseMoveEvent, _, window, cx| {
                let Some((x0, s0)) = d.get() else { return };
                if !ev.dragging() {
                    d.set(None);
                    window.refresh();
                    return;
                }
                let x = (s0 + (ev.position.x - x0) * per_px).clamp(px(0.), max);
                h.set_offset(point(-x, h.offset().y));
                cx.stop_propagation();
                window.refresh();
            });
            let d = drag.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, window, cx| {
                if d.take().is_some() {
                    cx.stop_propagation();
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
