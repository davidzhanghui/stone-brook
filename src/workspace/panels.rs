//! 工作区的几个全局面板：设置（⌘,）、命令面板（⇧⌘P）、演示模式（F5）、PDF 导出、写作统计。

use super::*;
use crate::line_input::{InputEvent, LineInput};
use crate::model::Block;
use chrono::{Datelike, Duration as CDuration, Local, NaiveDate};
use std::sync::Arc;

type CmdFn = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);

/// 命令面板条目：(名称, 快捷键, 搜索关键词, 执行)
const COMMANDS: &[(&str, &str, &str, CmdFn)] = &[
    ("新建文档", "⌘N", "new xinjian wendang", |_, w, cx| w.dispatch_action(Box::new(NewDoc), cx)),
    ("新建子文档", "⇧⌘N", "sub zi wendang", |_, w, cx| w.dispatch_action(Box::new(NewSubDoc), cx)),
    ("从模板新建…", "", "template moban", |ws, _, cx| ws.open_templates(true, cx)),
    ("另存为模板", "", "save template moban lingcun", |ws, _, cx| ws.save_as_template(cx)),
    ("快速打开 / 全文搜索", "⌘P", "open search sousuo dakai", |_, w, cx| w.dispatch_action(Box::new(QuickOpen), cx)),
    ("演示模式", "F5", "present slides yanshi huandeng ppt", |ws, w, cx| ws.present(&Present, w, cx)),
    ("文档图谱", "", "graph tupu link network guanxi", |ws, w, cx| ws.open_graph(w, cx)),
    ("切换阅读 / 编辑模式", "⌥⌘R", "read only reading yuedu bianji zhidu", |_, w, cx| w.dispatch_action(Box::new(ToggleReadMode), cx)),
    ("以思维导图查看", "", "mindmap siwei daotu", |ws, w, cx| ws.open_doc_mindmap(w, cx)),
    ("导出为 PDF", "", "pdf export daochu", |ws, w, cx| ws.export_pdf(&ExportPdf, w, cx)),
    ("导出为 Markdown", "⇧⌘E", "markdown md export daochu", |_, w, cx| w.dispatch_action(Box::new(ExportMarkdown), cx)),
    ("导出完整备份（文档 + 图片 + 附件）", "", "backup beifen export daochu quanbu", |_, w, cx| w.dispatch_action(Box::new(ExportBackup), cx)),
    ("从备份恢复…", "", "restore backup huifu beifen", |_, w, cx| w.dispatch_action(Box::new(RestoreBackup), cx)),
    ("导出为 HTML", "", "html export daochu", |_, w, cx| w.dispatch_action(Box::new(ExportHtml), cx)),
    ("导出为飞书 DocxXML", "", "feishu lark docxml xml export daochu", |_, w, cx| w.dispatch_action(Box::new(ExportDocxXml), cx)),
    ("导入 Markdown…", "⌘O", "import daoru markdown feishu docxml xml", |_, w, cx| w.dispatch_action(Box::new(ImportMarkdown), cx)),
    ("切换深色 / 浅色主题", "⇧⌘D", "theme dark light zhuti shense", |_, w, cx| w.dispatch_action(Box::new(CycleTheme), cx)),
    ("专注模式", "⇧⌘F", "focus zhuanzhu", |_, w, cx| w.dispatch_action(Box::new(FocusMode), cx)),
    ("显示 / 隐藏侧边栏", "⌘\\", "sidebar cebianlan", |_, w, cx| w.dispatch_action(Box::new(ToggleSidebar), cx)),
    ("显示 / 隐藏大纲", "⇧⌘O", "outline dagang", |_, w, cx| w.dispatch_action(Box::new(ToggleOutline), cx)),
    ("设置", "⌘,", "settings shezhi preferences", |ws, w, cx| ws.open_settings(&OpenSettings, w, cx)),
    ("快捷键", "⌘/", "shortcuts keys keymap kuaijiejian anjian", |_, w, cx| w.dispatch_action(Box::new(ShowShortcuts), cx)),
    ("性能面板", "⌥⌘P", "perf xingneng", |_, w, cx| w.dispatch_action(Box::new(TogglePerf), cx)),
    ("打开欢迎文档", "", "welcome huanying", |_, w, cx| w.dispatch_action(Box::new(OpenWelcome), cx)),
    ("把当前文档移到回收站", "", "trash delete shanchu huishouzhan", |_, w, cx| w.dispatch_action(Box::new(TrashDoc), cx)),
    ("关于石渠", "", "about guanyu", |_, w, cx| w.dispatch_action(Box::new(About), cx)),
];

/// 命令面板里显示的按键：命令对应的动作被用户改过键时显示新按键。
fn command_hint(name: &str, default: &str, cx: &App) -> String {
    let action = match name {
        "新建文档" => "stonebrook::NewDoc",
        "新建子文档" => "stonebrook::NewSubDoc",
        "快速打开 / 全文搜索" => "stonebrook::QuickOpen",
        "演示模式" => "stonebrook::Present",
        "切换阅读 / 编辑模式" => "stonebrook::ToggleReadMode",
        "导出为 Markdown" => "stonebrook::ExportMarkdown",
        "导入 Markdown…" => "stonebrook::ImportMarkdown",
        "切换深色 / 浅色主题" => "stonebrook::CycleTheme",
        "专注模式" => "stonebrook::FocusMode",
        "显示 / 隐藏侧边栏" => "stonebrook::ToggleSidebar",
        "显示 / 隐藏大纲" => "stonebrook::ToggleOutline",
        "设置" => "stonebrook::OpenSettings",
        "快捷键" => "stonebrook::ShowShortcuts",
        "性能面板" => "stonebrook::TogglePerf",
        _ => return default.to_string(),
    };
    crate::shortcuts::hint(action, cx)
}

fn cmd_matches(q: &str) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, (name, _, kw, _))| {
            q.is_empty()
                || name.to_lowercase().contains(&q)
                || crate::i18n::tr(name).to_lowercase().contains(&q)
                || kw.contains(&q)
                || kw.replace(' ', "").contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

impl Workspace {
    // ───────────── 写作统计 ─────────────

    /// 最近 n 天的每日输入字数（今天在最后）。
    fn daily_stats(&self, days: i64) -> Vec<(NaiveDate, usize)> {
        let today = Local::now().date_naive();
        (0..days)
            .rev()
            .map(|d| {
                let day = today - CDuration::days(d);
                let n = self.store.get_kv(&format!("stats:{}", day.format("%Y-%m-%d"))).and_then(|v| v.parse().ok()).unwrap_or(0);
                (day, n)
            })
            .collect()
    }

    fn render_heatmap(&self, t: &Theme) -> impl IntoElement {
        let (stats, total, active, streak) = self.writing_summary(20);
        let max = stats.iter().map(|s| s.1).max().unwrap_or(0).max(1);
        let today = Local::now().date_naive();
        let weeks = 20i64;
        let level = |n: usize| -> Hsla {
            if n == 0 {
                return t.hover;
            }
            let r = n as f32 / max as f32;
            let a = if r > 0.75 {
                1.0
            } else if r > 0.5 {
                0.75
            } else if r > 0.25 {
                0.5
            } else {
                0.28
            };
            t.accent.opacity(a)
        };
        let mut cols = div().flex().flex_row().gap(px(4.));
        for w in 0..weeks {
            let mut col = div().flex().flex_col().gap(px(4.));
            for d in 0..7 {
                let i = (w * 7 + d) as usize;
                let cell = stats.get(i).copied();
                col = col.child(div().size(px(13.)).rounded(px(3.)).bg(match cell {
                    Some((day, n)) if day <= today => level(n),
                    _ => gpui::transparent_black(),
                }));
            }
            cols = cols.child(col);
        }
        let legend = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pt(px(6.))
            .child(div().text_size(px(11.5)).text_color(t.text_muted).child(format!("近 {weeks} 周 · {active} 天活跃 · 共 {total} 字 · 连续 {streak} 天")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.))
                    .text_color(t.text_muted)
                    .child(crate::i18n::tr("少"))
                    .children(
                        [0.0f32, 0.28, 0.5, 0.75, 1.0]
                            .into_iter()
                            .map(|a| div().size(px(10.)).rounded(px(2.)).bg(if a == 0.0 { t.hover } else { t.accent.opacity(a) })),
                    )
                    .child(crate::i18n::tr("多")),
            );
        div().flex().flex_col().gap_2().child(cols).child(legend)
    }

    /// 统计近 `weeks` 周的每日输入数据、总字数、活跃天数和当前连续天数。
    fn writing_summary(&self, weeks: i64) -> (Vec<(NaiveDate, usize)>, usize, usize, usize) {
        let today = Local::now().date_naive();
        let pad = today.weekday().num_days_from_monday() as i64;
        let stats = self.daily_stats(weeks * 7 + pad - 6);
        let total: usize = stats.iter().map(|s| s.1).sum();
        let active = stats.iter().filter(|s| s.1 > 0).count();
        let mut streak = 0;
        for s in stats.iter().rev() {
            if s.1 > 0 {
                streak += 1;
            } else if s.0 != today {
                break;
            }
        }
        (stats, total, active, streak)
    }

    // ───────────── 设置与更新弹窗 ─────────────

    /// 打开设置（已打开则保持）。
    pub fn show_settings(&mut self, cx: &mut Context<Self>) {
        if !self.settings_open {
            self.editor.update(cx, |e, cx| e.save_now(cx));
            self.settings_open = true;
            self.more_menu = false;
        }
        cx.notify();
    }

    /// 打开设置并直接切到指定分页。
    pub(super) fn show_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_tab = tab;
        self.show_settings(cx);
    }

    /// 重启并安装已下载的新版本。
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let crate::updater::State::Ready { app, .. } = crate::updater::state() else { return };
        self.update_dialog_open = false;
        self.editor.update(cx, |e, _| e.save_now_queued());
        crate::flush_all();
        match crate::updater::schedule_install(&app, true) {
            Ok(()) => cx.quit(),
            Err(e) => self.show_toast_for(e.to_string(), 8000, cx),
        }
    }

    /// 授权「退出应用时安装」：关闭更新弹窗并保留待装包，等用户主动退出时静默替换。
    fn arm_update_on_quit(&mut self, version: String, cx: &mut Context<Self>) {
        crate::updater::arm_install_on_quit(true);
        self.update_dialog_open = false;
        self.show_toast_for(format!("已安排在退出石渠时安装 v{version}"), 4500, cx);
        cx.notify();
    }

    /// 跳过当前版本并清理暂存更新包。
    fn skip_pending_update(&mut self, version: String, cx: &mut Context<Self>) {
        crate::updater::skip_version(&self.store, &version);
        self.update_dialog_open = false;
        self.show_toast_for(format!("已跳过版本 v{version}"), 3500, cx);
        cx.notify();
    }

    /// Muse / Sparkle 2 风格的新版本就绪决策弹窗：
    /// 后台下载校验完毕后才出现，提供「跳过这个版本 / 退出应用时安装 / 安装并重启应用」三态分流，
    /// 以及就地勾选「以后自动下载并安装更新」。
    pub(super) fn render_update_dialog(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let crate::updater::State::Ready { release, .. } = crate::updater::state() else {
            return div().into_any_element();
        };
        let cur = crate::updater::current_version();
        let version = release.version.clone();
        let notes = release.notes.trim().to_string();
        let page = release.page.clone();
        let auto_install = crate::updater::auto_install_enabled(&self.store);
        let armed = crate::updater::is_install_on_quit_armed();
        let v_skip = version.clone();
        let v_quit = version.clone();
        let page_link = page.clone();

        div()
            .absolute()
            .inset_0()
            .bg(t.overlay)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.update_dialog_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("update-dialog")
                    .w(px(500.))
                    .p(px(24.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(14.))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    // 头部：应用图标 + 结论 + 一句话说明 + 关闭按钮
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(16.))
                            .child(div().size(px(52.)).flex_none().rounded(px(12.)).overflow_hidden().shadow_sm().child(logo(52.)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(t.text)
                                            .child(crate::i18n::tr("新版本的石渠可以安装了")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(px(20.))
                                            .text_color(t.text_secondary)
                                            .child(format!("石渠 {version} 已下载完毕并可以使用（当前 {cur}）。要立刻安装并重启石渠吗？")),
                                    ),
                            )
                            .child(
                                div()
                                    .id("upd-dlg-close")
                                    .size(px(26.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("x", 14., t.text_muted))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.update_dialog_open = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // 可折叠更新日志 + GitHub 发布页外链
                    .when(!notes.is_empty() || !page.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(if !notes.is_empty() {
                                            div()
                                                .id("upd-dlg-notes-toggle")
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(4.))
                                                .cursor_pointer()
                                                .text_size(px(12.5))
                                                .text_color(t.text_secondary)
                                                .hover(|s| s.text_color(t.text))
                                                .child(crate::i18n::tr("查看本次更新内容"))
                                                .child(icon(if self.update_notes_open { "chevron-up" } else { "chevron-down" }, 13., t.text_muted))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.update_notes_open = !this.update_notes_open;
                                                    cx.notify();
                                                }))
                                                .into_any_element()
                                        } else {
                                            div().into_any_element()
                                        })
                                        .when(!page.is_empty(), |row| {
                                            row.child(
                                                div()
                                                    .id("upd-dlg-page")
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap(px(4.))
                                                    .cursor_pointer()
                                                    .text_size(px(12.5))
                                                    .text_color(t.accent)
                                                    .hover(|s| s.opacity(0.8))
                                                    .child(crate::i18n::tr("查看发布说明"))
                                                    .child(icon("external", 12., t.accent))
                                                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&page_link))),
                                            )
                                        }),
                                )
                                .when(self.update_notes_open && !notes.is_empty(), |box_d| {
                                    box_d.child(
                                        div()
                                            .id("upd-dlg-notes")
                                            .max_h(px(160.))
                                            .overflow_y_scroll()
                                            .p(px(12.))
                                            .rounded(px(8.))
                                            .bg(t.sidebar_bg)
                                            .border_1()
                                            .border_color(t.border)
                                            .flex()
                                            .flex_col()
                                            .gap(px(4.))
                                            .text_size(px(12.5))
                                            .line_height(px(19.))
                                            .text_color(t.text_secondary)
                                            .children(notes.lines().map(|line| div().child(line.to_string()))),
                                    )
                                }),
                        )
                    })
                    // 就地偏好勾选框：以后自动下载并安装更新
                    .child(
                        div()
                            .id("upd-dlg-auto")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                crate::updater::set_auto_install(&this.store, !auto_install);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(16.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(if auto_install { t.accent } else { t.border_strong })
                                    .bg(if auto_install { t.accent } else { t.panel_bg })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(auto_install, |d| d.child(icon("check", 11., gpui::white()))),
                            )
                            .child(div().text_size(px(13.)).text_color(t.text).child(crate::i18n::tr("以后自动下载并安装更新"))),
                    )
                    // 底部三态操作按钮：左跳过、中退出时安装、右安装并重启
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .pt(px(4.))
                            .child(
                                div()
                                    .id("upd-dlg-skip")
                                    .px(px(14.))
                                    .py(px(7.))
                                    .rounded(px(8.))
                                    .bg(t.hover)
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .text_color(t.text_secondary)
                                    .hover(|s| s.text_color(t.text))
                                    .child(crate::i18n::tr("跳过这个版本"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.skip_pending_update(v_skip.clone(), cx))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id("upd-dlg-quit")
                                            .px(px(14.))
                                            .py(px(7.))
                                            .rounded(px(8.))
                                            .border_1()
                                            .border_color(if armed { t.accent } else { t.border_strong })
                                            .bg(if armed { t.accent_soft } else { t.panel_bg })
                                            .cursor_pointer()
                                            .text_size(px(13.))
                                            .text_color(if armed { t.accent } else { t.text })
                                            .hover(|s| s.bg(t.hover))
                                            .child(crate::i18n::tr(if armed { "已安排退出时安装" } else { "退出应用时安装" }))
                                            .on_click(cx.listener(move |this, _, _, cx| this.arm_update_on_quit(v_quit.clone(), cx))),
                                    )
                                    .child(
                                        div()
                                            .id("upd-dlg-install")
                                            .px(px(16.))
                                            .py(px(7.))
                                            .rounded(px(8.))
                                            .bg(t.accent)
                                            .cursor_pointer()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(gpui::white())
                                            .hover(|s| s.opacity(0.92))
                                            .child(crate::i18n::tr("安装并重启应用"))
                                            .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// 设置里的「关于与更新」分页。
    fn render_update(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::updater::State;
        let btn = |id: &'static str, label: String, primary: bool| {
            div()
                .id(id)
                .px_3()
                .py(px(6.))
                .rounded(px(7.))
                .border_1()
                .border_color(if primary { t.accent } else { t.border_strong })
                .when(primary, |d| d.bg(t.accent).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.opacity(0.92)))
                .when(!primary, |d| d.bg(t.panel_bg).text_color(t.text).hover(|s| s.bg(t.hover)))
                .cursor_pointer()
                .text_size(px(12.5))
                .flex_shrink_0()
                .child(label)
        };
        let cur = crate::updater::current_version();
        let st = crate::updater::state();
        let armed = crate::updater::is_install_on_quit_armed();
        let mut actions: Vec<gpui::Stateful<gpui::Div>> = Vec::new();
        let mut dl_ratio: Option<f32> = None;
        let status = match &st {
            State::Idle => {
                actions.push(btn("upd-check", crate::i18n::tr("检查更新").into(), true).on_click(cx.listener(|_, _, _, cx| {
                    crate::updater::run(true, true);
                    cx.notify();
                })));
                format!("当前已是版本 {cur}")
            }
            State::Checking => "正在检查新版本…".into(),
            State::UpToDate => {
                actions.push(btn("upd-check", crate::i18n::tr("重新检查").into(), false).on_click(cx.listener(|_, _, _, cx| {
                    crate::updater::run(true, true);
                    cx.notify();
                })));
                format!("当前版本 {cur} 已是最新")
            }
            State::Available(r) => {
                let size = if r.size > 0 { format!("，{}", crate::convert::human_size(r.size)) } else { String::new() };
                let rel = r.clone();
                let version = r.version.clone();
                actions.push(btn("upd-skip", crate::i18n::tr("跳过此版本").into(), false).on_click(cx.listener(move |this, _, _, cx| {
                    this.skip_pending_update(version.clone(), cx);
                })));
                actions.push(btn("upd-dl", crate::i18n::tr("下载更新").into(), true).on_click(cx.listener(move |_, _, _, cx| {
                    crate::updater::start_download(rel.clone());
                    cx.notify();
                })));
                format!("发现新版本 {}（当前 {cur}{size}）", r.version)
            }
            State::Downloading(r) => {
                let (got, total) = crate::updater::progress();
                if total > 0 {
                    dl_ratio = Some((got as f32 / total as f32).clamp(0., 1.));
                    format!("正在下载 {}（{}% · {} / {}）", r.version, (got * 100) / total, crate::convert::human_size(got), crate::convert::human_size(total))
                } else {
                    format!("正在下载 {}（{}）", r.version, crate::convert::human_size(got))
                }
            }
            State::Ready { release, .. } => {
                let v_skip = release.version.clone();
                let v_quit = release.version.clone();
                actions.push(btn("upd-skip", crate::i18n::tr("跳过此版本").into(), false).on_click(cx.listener(move |this, _, _, cx| {
                    this.skip_pending_update(v_skip.clone(), cx);
                })));
                actions.push(btn("upd-quit", crate::i18n::tr(if armed { "已安排退出时安装" } else { "退出应用时安装" }).into(), false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.arm_update_on_quit(v_quit.clone(), cx);
                    },
                )));
                actions.push(btn("upd-install", crate::i18n::tr("安装并重启应用").into(), true).on_click(cx.listener(|this, _, _, cx| {
                    this.install_update(cx);
                })));
                format!("新版本 {} 已下载完毕并可以使用", release.version)
            }
            State::Failed(e) => {
                actions.push(btn("upd-check", crate::i18n::tr("重试").into(), false).on_click(cx.listener(|_, _, _, cx| {
                    crate::updater::run(true, true);
                    cx.notify();
                })));
                e.clone()
            }
        };
        let auto = crate::updater::auto_enabled(&self.store);
        let auto_install = crate::updater::auto_install_enabled(&self.store);
        let (notes, page) = match &st {
            State::Available(r) | State::Downloading(r) | State::Ready { release: r, .. } => (r.notes.trim().to_string(), r.page.clone()),
            _ => (String::new(), String::new()),
        };
        let seg_pair = |id_on: &'static str, id_off: &'static str, val: bool, on_toggle: fn(&mut Workspace, bool)| {
            div().flex().flex_row().p(px(3.)).gap(px(2.)).rounded(px(8.)).bg(t.hover).children([(id_on, "开启", true), (id_off, "关闭", false)].map(
                |(id, label, v)| {
                    let active = val == v;
                    div()
                        .id(id)
                        .px_3()
                        .py(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .when(active, |d| d.bg(t.panel_bg).shadow_sm().text_color(t.accent).font_weight(FontWeight::MEDIUM))
                        .when(!active, |d| d.text_color(t.text_secondary).hover(|s| s.text_color(t.text)))
                        .child(crate::i18n::tr(label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            on_toggle(this, v);
                            cx.notify();
                        }))
                },
            ))
        };

        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            // 顶部应用身份 + 更新操作卡片
            .child(
                div()
                    .p(px(16.))
                    .rounded(px(10.))
                    .bg(t.sidebar_bg)
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(14.))
                            .child(div().size(px(44.)).flex_none().rounded(px(10.)).overflow_hidden().shadow_sm().child(logo(44.)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.text).child(crate::store::APP_NAME))
                                            .child(
                                                div()
                                                    .px(px(7.))
                                                    .py(px(1.))
                                                    .rounded(px(99.))
                                                    .bg(t.accent_soft)
                                                    .text_size(px(11.5))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(t.accent)
                                                    .child(format!("v{cur}")),
                                            )
                                            .child(
                                                div()
                                                    .px(px(7.))
                                                    .py(px(1.))
                                                    .rounded(px(99.))
                                                    .bg(t.hover)
                                                    .text_size(px(11.))
                                                    .text_color(t.text_muted)
                                                    .child(crate::updater::arch_label()),
                                            ),
                                    )
                                    .child(div().text_size(px(12.5)).text_color(t.text_secondary).child(status)),
                            ),
                    )
                    // 下载进度条
                    .when_some(dl_ratio, |d, ratio| {
                        d.child(
                            div()
                                .w_full()
                                .h(px(6.))
                                .rounded_full()
                                .bg(t.hover)
                                .overflow_hidden()
                                .child(div().h_full().w(relative(ratio)).rounded_full().bg(t.accent)),
                        )
                    })
                    .when(!actions.is_empty(), |d| d.child(div().flex().flex_row().flex_wrap().items_center().gap(px(8.)).children(actions))),
            )
            // 可滚动更新日志 + GitHub 发布页外链
            .when(!notes.is_empty() || !page.is_empty(), |d| {
                let page_url = page.clone();
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child(crate::i18n::tr("更新说明")))
                                .when(!page.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .id("upd-set-page")
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(4.))
                                            .cursor_pointer()
                                            .text_size(px(12.))
                                            .text_color(t.accent)
                                            .hover(|s| s.opacity(0.8))
                                            .child(crate::i18n::tr("查看发布说明"))
                                            .child(icon("external", 12., t.accent))
                                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&page_url))),
                                    )
                                }),
                        )
                        .when(!notes.is_empty(), |box_d| {
                            box_d.child(
                                div()
                                    .id("upd-set-notes")
                                    .max_h(px(160.))
                                    .overflow_y_scroll()
                                    .p(px(12.))
                                    .rounded(px(10.))
                                    .bg(t.sidebar_bg)
                                    .border_1()
                                    .border_color(t.border)
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .text_size(px(12.5))
                                    .line_height(px(19.))
                                    .text_color(t.text_secondary)
                                    .children(notes.lines().map(|line| div().child(line.to_string()))),
                            )
                        }),
                )
            })
            // 更新策略分组卡片
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child(crate::i18n::tr("更新策略")))
                    .child(
                        div()
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(16.))
                                    .px(px(14.))
                                    .py(px(12.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .gap(px(2.))
                                            .child(div().text_size(px(13.5)).text_color(t.text).child(crate::i18n::tr("自动检查与下载")))
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(t.text_muted)
                                                    .child(crate::i18n::tr("每 6 小时后台检查一次，有新版本静默下载（需允许联网）")),
                                            ),
                                    )
                                    .child(seg_pair("upd-auto-on", "upd-auto-off", auto, |this, v| crate::updater::set_auto(&this.store, v))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(16.))
                                    .px(px(14.))
                                    .py(px(12.))
                                    .border_t_1()
                                    .border_color(t.border)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .gap(px(2.))
                                            .child(div().text_size(px(13.5)).text_color(t.text).child(crate::i18n::tr("以后自动下载并安装更新")))
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(t.text_muted)
                                                    .child(crate::i18n::tr("开启后下载完成不弹窗打扰，退出应用时静默安装；关闭则弹窗询问")),
                                            ),
                                    )
                                    .child(seg_pair("upd-autoinst-on", "upd-autoinst-off", auto_install, |this, v| {
                                        crate::updater::set_auto_install(&this.store, v)
                                    })),
                            ),
                    ),
            )
    }

    pub fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        self.settings_open = !self.settings_open;
        self.more_menu = false;
        cx.notify();
    }

    fn set_language(&mut self, english: bool, _: &mut Window, cx: &mut Context<Self>) {
        crate::i18n::set_english(english);
        self.store.set_kv("lang", if english { "en" } else { "zh" });
        crate::refresh_menus(cx);
        cx.refresh_windows();
        cx.notify();
    }

    fn set_css_theme(&mut self, sel: String, window: &mut Window, cx: &mut Context<Self>) {
        self.store.set_kv("css_theme", &sel);
        self.css_theme = sel;
        self.apply_theme(window, cx);
        cx.notify();
    }

    /// 在默认文本编辑器里打开主题文件。
    fn open_theme_file(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if std::process::Command::new("open").arg("-t").arg(path).spawn().is_err() {
            cx.reveal_path(path);
        }
    }

    /// 写一份新主题（内容为 `src`，空时用写满全部变量的模板），选中并打开。
    fn create_css_theme(&mut self, name: &str, src: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let dir = crate::theme_css::themes_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.show_toast(format!("无法创建主题文件夹：{e}"), cx);
            return;
        }
        let file = (1..)
            .map(|i| if i == 1 { format!("{name}.css") } else { format!("{name} {i}.css") })
            .find(|f| !dir.join(f).exists())
            .unwrap_or_else(|| format!("{name}.css"));
        let shown = file.trim_end_matches(".css").to_string();
        let src = match src {
            // 复制内置主题：改个名字，免得两份同名
            Some(s) => match crate::theme_css::theme_name(&s) {
                Some(old) => s.replacen(&format!("\"{old}\""), &format!("\"{shown}\""), 1),
                None => s,
            },
            None => crate::theme_css::template(&shown),
        };
        if let Err(e) = std::fs::write(dir.join(&file), src) {
            self.show_toast(format!("无法写入主题文件：{e}"), cx);
            return;
        }
        self.set_css_theme(format!("user:{file}"), window, cx);
        self.open_theme_file(&dir.join(&file), cx);
        self.show_toast(format!("已创建主题「{shown}」，保存文件后自动生效"), cx);
    }

    /// 编辑当前主题：用户主题直接打开；内置主题先复制一份再打开（内置的不能改）。
    fn edit_css_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = crate::theme_css::user_path(&self.css_theme) {
            self.open_theme_file(&path, cx);
            return;
        }
        let builtin = crate::theme_css::source(&self.css_theme).filter(|s| !s.is_empty());
        let name = crate::theme_css::list().into_iter().find(|(v, _)| *v == self.css_theme).map(|(_, n)| n).unwrap_or_default();
        match builtin {
            Some(src) => self.create_css_theme(&format!("{name} 副本"), Some(src), window, cx),
            None => self.create_css_theme("我的主题", None, window, cx),
        }
    }

    fn set_theme_mode(&mut self, mode: ThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        self.store.set_kv(
            "theme",
            match mode {
                ThemeMode::Light => "light",
                ThemeMode::Dark => "dark",
                ThemeMode::System => "system",
            },
        );
        self.apply_theme(window, cx);
    }

    pub(super) fn render_settings(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let seg = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .px_3()
                .py(px(4.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |d| d.bg(t.panel_bg).shadow_sm().text_color(t.accent).font_weight(FontWeight::MEDIUM))
                .when(!on, |d| d.text_color(t.text_secondary).hover(|s| s.text_color(t.text)))
                .child(crate::i18n::tr(label))
        };
        let seg_group = || div().flex().flex_row().p(px(3.)).gap(px(2.)).rounded(px(8.)).bg(t.hover);
        let card_row = |label: &'static str, desc: &'static str, top_border: bool| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .px(px(14.))
                .py(px(12.))
                .when(top_border, |d| d.border_t_1().border_color(t.border))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(2.))
                        .child(div().text_size(px(13.5)).text_color(t.text).child(crate::i18n::tr(label)))
                        .child(div().text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr(desc))),
                )
        };
        let section = |title: &'static str, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child(crate::i18n::tr(title)))
                .child(body)
        };
        let kpi = |label: &'static str, value: String, hint: &'static str| {
            div()
                .flex_1()
                .min_w(px(0.))
                .p(px(14.))
                .rounded(px(10.))
                .bg(t.sidebar_bg)
                .border_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(11.5)).text_color(t.text_muted).child(crate::i18n::tr(label)))
                .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(t.text).child(value))
                .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr(hint)))
        };

        let tab = self.settings_tab;
        let has_update = matches!(crate::updater::state(), crate::updater::State::Ready { .. });
        let cur_ver = crate::updater::current_version();

        let nav_items: [(SettingsTab, &'static str, &'static str); 5] = [
            (SettingsTab::Appearance, "palette", "外观与排版"),
            (SettingsTab::General, "feather", "通用与隐私"),
            (SettingsTab::Stats, "activity", "写作统计"),
            (SettingsTab::Data, "database", "数据与存储"),
            (SettingsTab::Updates, "download", "关于与更新"),
        ];
        let (tab_title, tab_sub) = match tab {
            SettingsTab::Appearance => ("外观与排版", "主题模式、CSS 配色、界面语言与阅读版式"),
            SettingsTab::General => ("通用与隐私", "评论署名与网络权限控制"),
            SettingsTab::Stats => ("写作统计", "记录最近 20 周的本地写作活跃度"),
            SettingsTab::Data => ("数据与存储", "所有文档与资源均保存在本机"),
            SettingsTab::Updates => ("关于与更新", "版本信息、发布说明与自动更新策略"),
        };

        let content: AnyElement = match tab {
            SettingsTab::Appearance => {
                let mode = self.theme_mode;
                let fs = crate::settings::font_size() as u32;
                let wide = crate::settings::wide();
                let themes = crate::theme_css::list();
                let current = self.css_theme.clone();
                let problems = self.theme_problems.clone();
                let mode_card = |id: &'static str, ic: &'static str, label: &'static str, on: bool, target: ThemeMode| {
                    div()
                        .id(id)
                        .flex_1()
                        .min_w(px(0.))
                        .py(px(10.))
                        .px(px(12.))
                        .rounded(px(9.))
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .bg(if on { t.accent_soft } else { t.panel_bg })
                        .cursor_pointer()
                        .hover(|s| s.border_color(if on { t.accent } else { t.border_strong }))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap(px(7.))
                        .child(icon(ic, 15., if on { t.accent } else { t.text_secondary }))
                        .child(
                            div()
                                .text_size(px(13.))
                                .when(on, |d| d.font_weight(FontWeight::MEDIUM).text_color(t.accent))
                                .when(!on, |d| d.text_color(t.text))
                                .child(crate::i18n::tr(label)),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| this.set_theme_mode(target, w, cx)))
                };
                let css_btn = |id: &'static str, ic: &'static str, label: &'static str| {
                    div()
                        .id(id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(7.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(t.text_secondary)
                        .hover(|s| s.bg(t.hover).text_color(t.text))
                        .child(icon(ic, 12.5, t.text_muted))
                        .child(crate::i18n::tr(label))
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(section(
                        "主题模式",
                        div()
                            .p(px(12.))
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(8.))
                                    .child(mode_card("th-l", "sun", "浅色", mode == ThemeMode::Light, ThemeMode::Light))
                                    .child(mode_card("th-d", "moon", "深色", mode == ThemeMode::Dark, ThemeMode::Dark))
                                    .child(mode_card("th-s", "monitor", "跟随系统", mode == ThemeMode::System, ThemeMode::System)),
                            )
                            .child(div().text_size(px(11.5)).text_color(t.text_muted).child(crate::i18n::tr("深色模式下所有图表、公式都会自动适配")))
                            .into_any_element(),
                    ))
                    .child(section(
                        "配色方案",
                        div()
                            .p(px(14.))
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(div().flex().flex_row().flex_wrap().gap(px(8.)).children(themes.into_iter().enumerate().map(|(k, (sel, name))| {
                                let on = sel == current;
                                let mut preview = if t.dark { Theme::dark() } else { Theme::light() };
                                if let Some(src) = crate::theme_css::source(&sel) {
                                    if !src.is_empty() {
                                        crate::theme_css::apply(&mut preview, &src);
                                    }
                                }
                                div()
                                    .id(("css-theme", k))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(7.))
                                    .px(px(11.))
                                    .py(px(6.))
                                    .rounded(px(8.))
                                    .bg(if on { t.accent_soft } else { t.panel_bg })
                                    .border_1()
                                    .border_color(if on { t.accent } else { t.border })
                                    .cursor_pointer()
                                    .hover(|s| s.border_color(if on { t.accent } else { t.border_strong }))
                                    // 双色圆点预览：主题背景色 + 强调色
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .child(div().size(px(12.)).rounded_full().bg(preview.bg).border_1().border_color(t.border_strong))
                                            .child(div().ml(px(-4.)).size(px(12.)).rounded_full().bg(preview.accent).border_1().border_color(t.panel_bg)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .when(on, |d| d.font_weight(FontWeight::MEDIUM).text_color(t.accent))
                                            .when(!on, |d| d.text_color(t.text))
                                            .child(crate::i18n::tr_owned(&name)),
                                    )
                                    .on_click(cx.listener(move |this, _, w, cx| this.set_css_theme(sel.clone(), w, cx)))
                            })))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.))
                                    .pt(px(2.))
                                    .child(
                                        css_btn("css-new", "plus", "新建主题")
                                            .on_click(cx.listener(|this, _, w, cx| this.create_css_theme("我的主题", None, w, cx))),
                                    )
                                    .child(css_btn("css-edit", "code", "编辑 CSS").on_click(cx.listener(|this, _, w, cx| this.edit_css_theme(w, cx))))
                                    .child(css_btn("css-dir", "external", "打开主题文件夹").on_click(cx.listener(|_, _, _, cx| {
                                        let dir = crate::theme_css::themes_dir();
                                        let _ = std::fs::create_dir_all(&dir);
                                        cx.reveal_path(&dir);
                                    }))),
                            )
                            .child(
                                div().text_size(px(11.5)).text_color(t.text_muted).child(crate::i18n::tr("用 CSS 变量改界面与文档的颜色，保存文件后自动生效")),
                            )
                            .when(!problems.is_empty(), |d| {
                                d.child(
                                    div()
                                        .p_2()
                                        .rounded(px(6.))
                                        .bg(t.panel_bg)
                                        .border_1()
                                        .border_color(t.border)
                                        .text_size(px(12.))
                                        .text_color(t.text_secondary)
                                        .child(div().text_color(t.text).font_weight(FontWeight::MEDIUM).child(format!("未生效（{}）", problems.len())))
                                        .children(problems.into_iter().take(12).map(|p| div().child(p))),
                                )
                            })
                            .into_any_element(),
                    ))
                    .child(section(
                        "界面与排版",
                        div()
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .child(
                                card_row("界面语言", "菜单、面板与提示的语言；文档内容不变", false).child(
                                    seg_group()
                                        .child(
                                            seg("lang-zh", "中文", !crate::i18n::english())
                                                .on_click(cx.listener(|this, _, w, cx| this.set_language(false, w, cx))),
                                        )
                                        .child(
                                            seg("lang-en", "English", crate::i18n::english())
                                                .on_click(cx.listener(|this, _, w, cx| this.set_language(true, w, cx))),
                                        ),
                                ),
                            )
                            .child(card_row("正文字号", "影响所有文档的正文与列表", true).child(seg_group().children(
                                crate::settings::FONT_SIZES.iter().enumerate().map(|(i, &v)| {
                                    seg(["fs14", "fs15", "fs16", "fs17"][i], ["14", "15", "16", "17"][i], fs == v).on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            crate::settings::set_font_size(&this.store, v);
                                            this.editor.update(cx, |e, cx| e.relayout(cx));
                                            cx.notify();
                                        },
                                    ))
                                }),
                            )))
                            .child(
                                card_row("页面宽度", "宽版适合表格、图表较多的文档", true).child(
                                    seg_group()
                                        .child(seg("w-n", "标准", !wide).on_click(cx.listener(|this, _, _, cx| {
                                            crate::settings::set_wide(&this.store, false);
                                            this.editor.update(cx, |e, cx| e.relayout(cx));
                                            cx.notify();
                                        })))
                                        .child(seg("w-w", "宽版", wide).on_click(cx.listener(|this, _, _, cx| {
                                            crate::settings::set_wide(&this.store, true);
                                            this.editor.update(cx, |e, cx| e.relayout(cx));
                                            cx.notify();
                                        }))),
                                ),
                            )
                            .into_any_element(),
                    ))
                    .into_any_element()
            }
            SettingsTab::General => {
                let net = crate::settings::network();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(section(
                        "评论",
                        div()
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .child(
                                card_row("作者名", "显示在评论和文档标题下方", false).child(
                                    div()
                                        .w(px(196.))
                                        .px(px(10.))
                                        .py(px(5.))
                                        .rounded(px(7.))
                                        .bg(t.panel_bg)
                                        .border_1()
                                        .border_color(t.border_strong)
                                        .text_size(px(13.))
                                        .line_height(px(20.))
                                        .child(self.author_input.clone()),
                                ),
                            )
                            .into_any_element(),
                    ))
                    .child(section(
                        "隐私与网络",
                        div()
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .child(
                                card_row("允许联网", "用于网页卡片抓取标题和封面、检查应用更新；关闭后石渠完全离线", false).child(
                                    seg_group()
                                        .child(seg("net-on", "开启", net).on_click(cx.listener(|this, _, _, cx| {
                                            crate::settings::set_network(&this.store, true);
                                            cx.notify();
                                        })))
                                        .child(seg("net-off", "关闭", !net).on_click(cx.listener(|this, _, _, cx| {
                                            crate::settings::set_network(&this.store, false);
                                            cx.notify();
                                        }))),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.))
                                    .px(px(14.))
                                    .py(px(10.))
                                    .border_t_1()
                                    .border_color(t.border)
                                    .child(icon(if net { "globe" } else { "lock" }, 13.5, if net { t.accent } else { t.text_muted }))
                                    .child(div().text_size(px(12.)).text_color(t.text_secondary).child(crate::i18n::tr(if net {
                                        "当前已允许联网：文档内容仍只存在本地，不会上传到任何服务器"
                                    } else {
                                        "当前处于完全离线模式：不会发起任何网络请求"
                                    }))),
                            )
                            .into_any_element(),
                    ))
                    .into_any_element()
            }
            SettingsTab::Stats => {
                let (_, total, active, streak) = self.writing_summary(20);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(10.))
                            .child(kpi("近 20 周字数", total.to_string(), "累计输入字符"))
                            .child(kpi("活跃天数", format!("{active} 天"), "有过输入记录"))
                            .child(kpi("连续写作", format!("{streak} 天"), "当前连续天数")),
                    )
                    .child(section(
                        "写作热力图",
                        div().p(px(16.)).rounded(px(10.)).bg(t.sidebar_bg).border_1().border_color(t.border).child(self.render_heatmap(t)).into_any_element(),
                    ))
                    .into_any_element()
            }
            SettingsTab::Data => {
                let docs = self.docs.iter().filter(|d| !d.deleted).count();
                let objs = self.store.object_ids().len();
                let assets = dir_size(&crate::store::data_dir().join("assets"));
                let data_dir = crate::store::data_dir();
                let data_dir_reveal = data_dir.clone();
                let action_btn = |id: &'static str, ic: &'static str, label: &'static str| {
                    div()
                        .id(id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(12.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border_strong)
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(t.text)
                        .hover(|s| s.bg(t.hover))
                        .child(icon(ic, 13.5, t.text_secondary))
                        .child(crate::i18n::tr(label))
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(10.))
                            .child(kpi("文档数量", format!("{docs}"), "不含回收站"))
                            .child(kpi("嵌入对象", format!("{objs}"), "画板 / 表格 / 多维表"))
                            .child(kpi("本地资源", crate::convert::human_size(assets), "图片与附件体积")),
                    )
                    .child(section(
                        "数据目录",
                        div()
                            .p(px(14.))
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(div().text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr("文档、图片、嵌入对象全部保存在这台电脑上")))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .px(px(10.))
                                            .py(px(6.))
                                            .rounded(px(7.))
                                            .bg(t.panel_bg)
                                            .border_1()
                                            .border_color(t.border)
                                            .font_family("Menlo")
                                            .text_size(px(11.5))
                                            .text_color(t.text_secondary)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(data_dir.display().to_string()),
                                    )
                                    .child(action_btn("set-reveal", "external", "在访达中显示").on_click(cx.listener(move |_, _, _, cx| {
                                        cx.reveal_path(&data_dir_reveal.join(crate::store::DB_FILE));
                                    }))),
                            )
                            .into_any_element(),
                    ))
                    .child(section(
                        "备份与恢复",
                        div()
                            .rounded(px(10.))
                            .bg(t.sidebar_bg)
                            .border_1()
                            .border_color(t.border)
                            .child(
                                card_row("完整备份", "打包导出所有文档、嵌入对象与本地图片附件", false).child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(action_btn("set-backup", "download", "导出备份").on_click(cx.listener(|this, _, w, cx| {
                                            this.settings_open = false;
                                            w.dispatch_action(Box::new(ExportBackup), cx);
                                        })))
                                        .child(action_btn("set-restore", "restore", "从备份恢复").on_click(cx.listener(|this, _, w, cx| {
                                            this.settings_open = false;
                                            w.dispatch_action(Box::new(RestoreBackup), cx);
                                        }))),
                                ),
                            )
                            .into_any_element(),
                    ))
                    .into_any_element()
            }
            SettingsTab::Updates => self.render_update(t, cx).into_any_element(),
        };

        div()
            .absolute()
            .inset_0()
            .bg(t.overlay)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.settings_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("settings")
                    .w(px(720.))
                    .h(px(540.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(14.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_row()
                    // 左侧导航栏
                    .child(
                        div()
                            .w(px(184.))
                            .flex_none()
                            .h_full()
                            .bg(t.sidebar_bg)
                            .border_r_1()
                            .border_color(t.border)
                            .p(px(12.))
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .px(px(8.))
                                            .pt(px(4.))
                                            .pb(px(10.))
                                            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.text).child(crate::i18n::tr("设置")))
                                            .child(
                                                div()
                                                    .px(px(5.))
                                                    .py(px(1.))
                                                    .rounded(px(4.))
                                                    .bg(t.hover)
                                                    .text_size(px(10.5))
                                                    .text_color(t.text_muted)
                                                    .child("⌘,"),
                                            ),
                                    )
                                    .children(nav_items.into_iter().enumerate().map(|(i, (target, ic, label))| {
                                        let active = tab == target;
                                        let dot = target == SettingsTab::Updates && has_update;
                                        div()
                                            .id(("set-nav", i))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(9.))
                                            .px(px(10.))
                                            .py(px(7.))
                                            .rounded(px(8.))
                                            .border_1()
                                            .border_color(if active { t.border } else { gpui::transparent_black() })
                                            .when(active, |d| d.bg(t.panel_bg).shadow_sm().text_color(t.text).font_weight(FontWeight::MEDIUM))
                                            .when(!active, |d| d.text_color(t.text_secondary).hover(|s| s.bg(t.hover).text_color(t.text)))
                                            .cursor_pointer()
                                            .text_size(px(13.))
                                            .child(icon(ic, 14.5, if active { t.accent } else { t.text_muted }))
                                            .child(div().flex_1().child(crate::i18n::tr(label)))
                                            .when(dot, |d| d.child(div().size(px(7.)).rounded_full().bg(t.accent)))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.settings_tab = target;
                                                cx.notify();
                                            }))
                                    })),
                            )
                            // 左下角版本入口
                            .child(
                                div()
                                    .id("set-ver-badge")
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(7.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.hover))
                                    .text_size(px(11.5))
                                    .text_color(t.text_muted)
                                    .child(format!("{} v{cur_ver}", crate::store::APP_NAME))
                                    .when(has_update, |d| d.child(div().text_color(t.accent).child(crate::i18n::tr("有新版"))))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_tab = SettingsTab::Updates;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // 右侧主内容区（顶部固定标题栏 + 可滚正文）
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(px(56.))
                                    .flex_none()
                                    .px(px(24.))
                                    .border_b_1()
                                    .border_color(t.border)
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .text_size(px(15.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(t.text)
                                                    .child(crate::i18n::tr(tab_title)),
                                            )
                                            .child(div().text_size(px(11.5)).text_color(t.text_muted).child(crate::i18n::tr(tab_sub))),
                                    )
                                    .child(
                                        div()
                                            .id("set-close")
                                            .size(px(28.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(t.hover))
                                            .child(icon("x", 15., t.text_muted))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings_open = false;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(div().id("settings-body").flex_1().min_h(px(0.)).overflow_y_scroll().px(px(24.)).py(px(18.)).child(content)),
                    ),
            )
    }

    // ───────────── 命令面板 ─────────────

    pub fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.palette = None;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }
        let input = cx.new(|cx| LineInput::new("输入命令，例如「导出」「主题」「演示」", cx));
        let sub = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| {
            let q = input.read(cx).text();
            let n = cmd_matches(&q).len();
            match ev {
                InputEvent::Changed => {
                    if let Some(p) = &mut this.palette {
                        p.2 = 0;
                    }
                }
                InputEvent::Up => {
                    if let Some(p) = &mut this.palette {
                        p.2 = p.2.saturating_sub(1);
                    }
                }
                InputEvent::Down => {
                    if let Some(p) = &mut this.palette {
                        p.2 = (p.2 + 1).min(n.saturating_sub(1));
                    }
                }
                InputEvent::Submit { .. } => {
                    let sel = this.palette.as_ref().map(|p| p.2).unwrap_or(0);
                    if let Some(&i) = cmd_matches(&q).get(sel) {
                        this.run_command(i, window, cx);
                    }
                    return;
                }
                InputEvent::Cancel => {
                    this.palette = None;
                    window.focus(&this.editor.focus_handle(cx));
                }
            }
            cx.notify();
        });
        window.focus(&input.focus_handle(cx));
        self.palette = Some((input, sub, 0));
        self.more_menu = false;
        cx.notify();
    }

    fn run_command(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.editor.focus_handle(cx));
        (COMMANDS[i].3)(self, window, cx);
        cx.notify();
    }

    pub(super) fn render_palette(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (input, _, sel) = self.palette.as_ref()?;
        let q = input.read(cx).text();
        let items = cmd_matches(&q);
        let sel = (*sel).min(items.len().saturating_sub(1));
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .justify_center()
                .pt(px(110.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.palette = None;
                        window.focus(&this.editor.focus_handle(cx));
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(560.))
                        .h_auto()
                        .flex()
                        .flex_col()
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(12.))
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .px_4()
                                .h(px(50.))
                                .border_b_1()
                                .border_color(t.border)
                                .child(icon("command", 16., t.text_muted))
                                .child(div().flex_1().text_size(px(15.)).line_height(px(24.)).child(input.clone())),
                        )
                        .child(
                            div()
                                .id("palette-list")
                                .max_h(px(400.))
                                .overflow_y_scroll()
                                .p(px(6.))
                                .when(items.is_empty(), |d| {
                                    d.child(div().p_4().text_size(px(13.)).text_color(t.text_muted).child(crate::i18n::tr("没有匹配的命令")))
                                })
                                .children(items.iter().enumerate().map(|(k, &i)| {
                                    let (name, key, _, _) = COMMANDS[i];
                                    div()
                                        .id(("cmd", i))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .px_3()
                                        .py(px(8.))
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .text_size(px(14.))
                                        .when(k == sel, |d| d.bg(t.active))
                                        .hover(|s| s.bg(t.hover))
                                        .child(div().flex_1().text_color(t.text).child(crate::i18n::tr(name)))
                                        .child(div().text_size(px(12.)).text_color(t.text_muted).child(command_hint(name, key, cx)))
                                        .on_click(cx.listener(move |this, _, window, cx| this.run_command(i, window, cx)))
                                })),
                        ),
                ),
        )
    }

    // ───────────── PDF 导出 ─────────────

    pub fn export_pdf(&mut self, _: &ExportPdf, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let (title, html) = {
            let e = self.editor.read(cx);
            let title = e.content.title();
            (title.clone(), pdf_page(&title, &e.content.blocks))
        };
        let name = format!("{}.pdf", if title.trim().is_empty() { "未命名文档".to_string() } else { title.replace(['/', ':'], "-") });
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            this.update(cx, |this, cx| this.show_toast("正在生成 PDF…".into(), cx)).ok();
            let ok = crate::webview::html_to_pdf(cx, &html, &crate::store::data_dir(), &path).await;
            this.update(cx, |this, cx| {
                if ok {
                    this.show_toast(format!("已导出 PDF：{}", path.display()), cx);
                    cx.reveal_path(&path);
                } else {
                    this.show_toast("PDF 导出失败".into(), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// 测试脚本：直接导出 PDF 到指定路径。
    #[cfg(feature = "dev")]
    pub fn test_pdf(&mut self, path: String, cx: &mut Context<Self>) {
        let html = {
            let e = self.editor.read(cx);
            pdf_page(&e.content.title(), &e.content.blocks)
        };
        cx.spawn(async move |_: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            let ok = crate::webview::html_to_pdf(cx, &html, &crate::store::data_dir(), std::path::Path::new(&path)).await;
            eprintln!("pdf export: {ok}");
        })
        .detach();
    }
}

/// PDF 用的 HTML：与网页导出同样式，额外加页边距。
fn pdf_page(title: &str, blocks: &[Arc<Block>]) -> String {
    convert::full_html_page(title, &convert::blocks_to_html(blocks)).replace("</head>", "<style>body{margin:0 auto;padding:40px 48px}</style></head>")
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    _ => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}
