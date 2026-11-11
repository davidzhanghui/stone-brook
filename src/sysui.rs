//! 不依赖 GPUI 窗口的系统级提示：系统通知、启动失败时的原生对话框。
//!
//! 通过 `osascript` 实现。所有拼进 AppleScript 字符串字面量的文本都必须经过 `quote`，
//! 否则内容里的 `\` 或 `"` 会让脚本语法错误（或被注入额外语句）。

use std::process::Command;

/// 转成 AppleScript 字符串字面量（含两侧引号）。
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // AppleScript 字符串支持 \n 转义；保持脚本本身是单行
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 发一条 macOS 系统通知（后台线程执行，不阻塞调用方）。脚本测试模式下不发。
pub fn notify(title: &str, body: &str) {
    if std::env::var_os("STONEBROOK_SCRIPT").is_some() {
        return;
    }
    let script = format!("display notification {} with title {}", quote(body), quote(title));
    std::thread::spawn(move || {
        Command::new("osascript").arg("-e").arg(script).output().ok();
    });
}

/// 启动阶段的致命错误：弹原生对话框（此时还没有窗口），可选「打开数据目录」。阻塞直到用户关闭。
pub fn fatal_dialog(title: &str, message: &str, data_dir: &std::path::Path) {
    let script =
        format!("display alert {} message {} as critical buttons {{\"打开数据目录\", \"退出\"}} default button \"退出\"", quote(title), quote(message));
    let out = Command::new("osascript").arg("-e").arg(script).output();
    if let Ok(o) = out {
        if String::from_utf8_lossy(&o.stdout).contains("打开数据目录") {
            Command::new("open").arg(data_dir).spawn().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_escapes_backslash_and_quote() {
        assert_eq!(quote(r#"a"b\"#), r#""a\"b\\""#);
        assert_eq!(quote("x\ny"), "\"x\\ny\"");
    }
}
