//! 界面语言（思路来自 quiet.md 的 settings::text / tr）：简体中文为源语言，英文为翻译。
//!
//! - 源码里照常写中文；需要翻译的界面文字经 `tr("中文")`，英文模式下查 `assets/i18n/en.tsv`，查不到就原样显示中文；
//! - 用户写的内容（标题、正文、附件名）不参与翻译；
//! - 偏好存在键值表 `lang`（`en` / 其他），切换后刷新所有窗口和应用菜单。
//!
//! 覆盖范围：主窗口（侧边栏、顶栏、菜单、命令面板、设置、快捷键面板、斜杠菜单、浮动工具栏、块菜单、
//! 编号 / 链接 / 阅读模式）。画板、电子表格、多维表格的全屏编辑器暂未翻译。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

static ENGLISH: AtomicBool = AtomicBool::new(false);

static EN: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    include_str!("../assets/i18n/en.tsv").lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect()
});

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

/// 静态界面文字：英文模式下返回译文。
pub fn tr(zh: &'static str) -> &'static str {
    tr_in(english(), zh)
}

/// 按指定语言查表（不读进程级开关，测试用它，避免与并行测试互相干扰）。
fn tr_in(english: bool, zh: &'static str) -> &'static str {
    if !english {
        return zh;
    }
    EN.get(zh).copied().unwrap_or(zh)
}

/// 运行时拼出来的文字（如提示消息）：整句在表里才翻译，否则原样。
pub fn tr_owned(zh: &str) -> String {
    if !english() {
        return zh.to_string();
    }
    EN.get(zh).map(|s| s.to_string()).unwrap_or_else(|| zh.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_well_formed() {
        let src = include_str!("../assets/i18n/en.tsv");
        let mut seen = std::collections::HashSet::new();
        for (n, l) in src.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#')) {
            let (zh, en) = l.split_once('\t').unwrap_or_else(|| panic!("第 {} 行没有制表符：{l}", n + 1));
            assert!(!zh.is_empty() && !en.trim().is_empty(), "第 {} 行有空栏", n + 1);
            assert!(!en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "第 {} 行的译文里还有中文：{en}", n + 1);
            assert!(seen.insert(zh), "第 {} 行重复：{zh}", n + 1);
        }
    }

    #[test]
    fn falls_back_to_chinese() {
        // 不改进程级开关：其他并行测试里的 tr() 会读到它
        assert_eq!(tr_in(true, "新建文档"), "New document");
        assert_eq!(tr_in(true, "一句表里没有的话"), "一句表里没有的话");
        assert_eq!(tr_in(false, "新建文档"), "新建文档");
    }
}
