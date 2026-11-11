//! 石渠 Stonebrook —— 纯本地的块级文档编辑器（Rust + GPUI）。

mod assets;
#[cfg(feature = "dev")]
mod autotest;
mod backup;
mod base;
mod board;
mod canvas;
mod clipboard;
mod convert;
mod diagram;
mod docxml;
mod editor;
mod embed;
mod highlight;
mod i18n;
mod katex_assets;
mod line_input;
mod linkcard;
mod lru;
mod math;
mod media;
mod model;
#[cfg(test)]
mod model_fixture_tests;
mod objects;
mod seed;
mod settings;
mod sheet;
mod shortcuts;
mod sketch;
mod store;
mod sysui;
mod templates;
mod text_area;
mod theme;
mod theme_css;
mod ui;
mod updater;
mod webview;
mod words;
mod workspace;

use editor::*;
use gpui::*;
use workspace::*;

fn bind_keys(cx: &mut App) {
    let e = Some("Editor");
    cx.bind_keys([
        // 光标
        KeyBinding::new("left", MoveLeft, e),
        KeyBinding::new("right", MoveRight, e),
        KeyBinding::new("up", MoveUp, e),
        KeyBinding::new("down", MoveDown, e),
        KeyBinding::new("ctrl-b", MoveLeft, e),
        KeyBinding::new("ctrl-f", MoveRight, e),
        KeyBinding::new("ctrl-p", MoveUp, e),
        KeyBinding::new("ctrl-n", MoveDown, e),
        KeyBinding::new("shift-left", SelectLeft, e),
        KeyBinding::new("shift-right", SelectRight, e),
        KeyBinding::new("shift-up", SelectUp, e),
        KeyBinding::new("shift-down", SelectDown, e),
        KeyBinding::new("alt-left", WordLeft, e),
        KeyBinding::new("alt-right", WordRight, e),
        KeyBinding::new("alt-shift-left", SelectWordLeft, e),
        KeyBinding::new("alt-shift-right", SelectWordRight, e),
        KeyBinding::new("cmd-left", LineStart, e),
        KeyBinding::new("cmd-right", LineEnd, e),
        KeyBinding::new("ctrl-a", LineStart, e),
        KeyBinding::new("ctrl-e", LineEnd, e),
        KeyBinding::new("home", LineStart, e),
        KeyBinding::new("end", LineEnd, e),
        KeyBinding::new("cmd-shift-left", SelectLineStart, e),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, e),
        KeyBinding::new("shift-home", SelectLineStart, e),
        KeyBinding::new("shift-end", SelectLineEnd, e),
        KeyBinding::new("cmd-up", DocStart, e),
        KeyBinding::new("cmd-down", DocEnd, e),
        KeyBinding::new("cmd-shift-up", SelectDocStart, e),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, e),
        // 编辑
        KeyBinding::new("backspace", Backspace, e),
        KeyBinding::new("shift-backspace", Backspace, e),
        KeyBinding::new("ctrl-h", Backspace, e),
        KeyBinding::new("delete", Delete, e),
        KeyBinding::new("ctrl-d", Delete, e),
        KeyBinding::new("alt-backspace", DeleteWordBack, e),
        KeyBinding::new("cmd-backspace", DeleteLineBack, e),
        KeyBinding::new("enter", Enter, e),
        KeyBinding::new("shift-enter", SoftEnter, e),
        KeyBinding::new("tab", Tab, e),
        KeyBinding::new("shift-tab", ShiftTab, e),
        KeyBinding::new("cmd-a", SelectAll, e),
        KeyBinding::new("cmd-c", Copy, e),
        KeyBinding::new("cmd-x", Cut, e),
        KeyBinding::new("cmd-v", Paste, e),
        KeyBinding::new("cmd-shift-v", PastePlain, e),
        KeyBinding::new("cmd-z", Undo, e),
        KeyBinding::new("cmd-shift-z", Redo, e),
        KeyBinding::new("cmd-y", Redo, e),
        KeyBinding::new("escape", Escape, e),
        // 格式
        KeyBinding::new("cmd-b", Bold, e),
        KeyBinding::new("cmd-i", Italic, e),
        KeyBinding::new("cmd-u", Underline, e),
        KeyBinding::new("cmd-shift-x", Strike, e),
        KeyBinding::new("cmd-e", InlineCode, e),
        KeyBinding::new("cmd-k", InsertLink, e),
        KeyBinding::new("cmd-alt-0", SetText, e),
        KeyBinding::new("cmd-alt-1", SetH1, e),
        KeyBinding::new("cmd-alt-2", SetH2, e),
        KeyBinding::new("cmd-alt-3", SetH3, e),
        KeyBinding::new("cmd-alt-4", SetH4, e),
        KeyBinding::new("cmd-shift-7", SetOrdered, e),
        KeyBinding::new("cmd-shift-8", SetBullet, e),
        KeyBinding::new("cmd-shift-9", SetTodo, e),
        KeyBinding::new("cmd-shift-.", SetQuote, e),
        KeyBinding::new("cmd-alt-c", SetCode, e),
        KeyBinding::new("cmd-enter", ToggleTodo, e),
        KeyBinding::new("cmd-d", DuplicateBlock, e),
        KeyBinding::new("cmd-ctrl-up", MoveBlockUp, e),
        KeyBinding::new("cmd-ctrl-down", MoveBlockDown, e),
        KeyBinding::new("alt-shift-up", MoveBlockUp, e),
        KeyBinding::new("alt-shift-down", MoveBlockDown, e),
        KeyBinding::new("cmd-f", Find, e),
        KeyBinding::new("cmd-g", FindNext, e),
        KeyBinding::new("cmd-shift-g", FindPrev, e),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, e),
        KeyBinding::new("cmd-alt-t", InsertTimestamp, e),
        KeyBinding::new("cmd-.", ToggleFold, e),
        KeyBinding::new("cmd-alt-.", FocusBlock, e),
        // 全局
        KeyBinding::new("cmd-n", NewDoc, None),
        KeyBinding::new("cmd-shift-n", NewSubDoc, None),
        KeyBinding::new("cmd-p", QuickOpen, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("f5", Present, None),
        KeyBinding::new("cmd-\\", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-o", ToggleOutline, None),
        KeyBinding::new("cmd-shift-f", FocusMode, None),
        KeyBinding::new("cmd-shift-d", CycleTheme, None),
        KeyBinding::new("cmd-alt-p", TogglePerf, None),
        KeyBinding::new("cmd-alt-r", ToggleReadMode, None),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("cmd-shift-e", ExportMarkdown, None),
        KeyBinding::new("cmd-o", ImportMarkdown, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    line_input::bind_keys(cx);
    text_area::bind_keys(cx);
    board::view::bind_keys(cx);
    sketch::view::bind_keys(cx);
    sheet::bind_keys(cx);
    base::editor::bind_keys(cx);
}

/// 改了快捷键后重设菜单：菜单项上显示的按键在设置菜单时从按键映射里取。
pub fn refresh_menus(cx: &mut App) {
    cx.set_menus(menus());
}

fn menus() -> Vec<Menu> {
    #[allow(unused_mut)]
    let mut menus = vec![
        Menu {
            name: crate::store::APP_NAME.into(),
            items: vec![
                MenuItem::action(format!("关于{}", crate::store::APP_NAME), About),
                MenuItem::action(crate::i18n::tr("检查更新…"), CheckUpdate),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("设置…"), OpenSettings),
                MenuItem::action(crate::i18n::tr("命令面板…"), CommandPalette),
                MenuItem::action(crate::i18n::tr("快捷键…"), ShowShortcuts),
                MenuItem::separator(),
                MenuItem::os_submenu("服务", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(format!("退出{}", crate::store::APP_NAME), Quit),
            ],
        },
        Menu {
            name: crate::i18n::tr("文件").into(),
            items: vec![
                MenuItem::action(crate::i18n::tr("新建文档"), NewDoc),
                MenuItem::action(crate::i18n::tr("新建子文档"), NewSubDoc),
                MenuItem::action(crate::i18n::tr("快速打开…"), QuickOpen),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("导入 Markdown…"), ImportMarkdown),
                MenuItem::action(crate::i18n::tr("导出为 Markdown…"), ExportMarkdown),
                MenuItem::action(crate::i18n::tr("导出为 HTML…"), ExportHtml),
                MenuItem::action(crate::i18n::tr("导出为飞书 DocxXML…"), ExportDocxXml),
                MenuItem::action(crate::i18n::tr("导出为 PDF…"), ExportPdf),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("导出完整备份…"), ExportBackup),
                MenuItem::action(crate::i18n::tr("从备份恢复…"), RestoreBackup),
                MenuItem::action(crate::i18n::tr("演示模式"), Present),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("移到回收站"), TrashDoc),
            ],
        },
        Menu {
            name: crate::i18n::tr("编辑").into(),
            items: vec![
                MenuItem::action(crate::i18n::tr("撤销"), Undo),
                MenuItem::action(crate::i18n::tr("重做"), Redo),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("剪切"), Cut),
                MenuItem::action(crate::i18n::tr("复制"), Copy),
                MenuItem::action(crate::i18n::tr("粘贴"), Paste),
                MenuItem::action(crate::i18n::tr("粘贴为纯文本"), PastePlain),
                MenuItem::action(crate::i18n::tr("全选"), SelectAll),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("查找…"), Find),
                MenuItem::action(crate::i18n::tr("查找下一个"), FindNext),
                MenuItem::action(crate::i18n::tr("查找上一个"), FindPrev),
            ],
        },
        Menu {
            name: crate::i18n::tr("格式").into(),
            items: vec![
                MenuItem::action(crate::i18n::tr("粗体"), Bold),
                MenuItem::action(crate::i18n::tr("斜体"), Italic),
                MenuItem::action(crate::i18n::tr("下划线"), Underline),
                MenuItem::action(crate::i18n::tr("删除线"), Strike),
                MenuItem::action(crate::i18n::tr("行内代码"), InlineCode),
                MenuItem::action(crate::i18n::tr("链接…"), InsertLink),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("正文"), SetText),
                MenuItem::action(crate::i18n::tr("一级标题"), SetH1),
                MenuItem::action(crate::i18n::tr("二级标题"), SetH2),
                MenuItem::action(crate::i18n::tr("三级标题"), SetH3),
                MenuItem::action(crate::i18n::tr("四级标题"), SetH4),
                MenuItem::action(crate::i18n::tr("有序列表"), SetOrdered),
                MenuItem::action(crate::i18n::tr("无序列表"), SetBullet),
                MenuItem::action(crate::i18n::tr("待办事项"), SetTodo),
                MenuItem::action(crate::i18n::tr("引用"), SetQuote),
                MenuItem::action(crate::i18n::tr("代码块"), SetCode),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("插入图片…"), InsertImage),
                MenuItem::action(crate::i18n::tr("创建块副本"), DuplicateBlock),
                MenuItem::action(crate::i18n::tr("上移块"), MoveBlockUp),
                MenuItem::action(crate::i18n::tr("下移块"), MoveBlockDown),
            ],
        },
        Menu {
            name: crate::i18n::tr("视图").into(),
            items: vec![
                MenuItem::action(crate::i18n::tr("侧边栏"), ToggleSidebar),
                MenuItem::action(crate::i18n::tr("大纲"), ToggleOutline),
                MenuItem::action(crate::i18n::tr("专注模式"), FocusMode),
                MenuItem::action(crate::i18n::tr("切换深浅色"), CycleTheme),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::tr("性能面板"), TogglePerf),
                MenuItem::action(crate::i18n::tr("打开欢迎文档"), OpenWelcome),
            ],
        },
    ];
    #[cfg(feature = "dev")]
    if let Some(view) = menus.last_mut() {
        view.items.push(MenuItem::separator());
        view.items.push(MenuItem::action(crate::i18n::tr("生成百万字压测文档"), StressTest));
        view.items.push(MenuItem::action(crate::i18n::tr("生成 10 万行多维表格压测"), BaseStressTest));
    }
    menus
}

/// 把 panic 信息落到数据目录的 panic.log。
///
/// 双击打开的 .app 没有终端接住 stderr，崩溃后只能看到系统「意外退出」对话框，
/// 具体是哪一行、什么原因全丢了。这里额外写一份文件，并保留默认钩子（终端里仍能看到）。
fn install_panic_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = store::data_dir().join("panic.log");
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(非字符串 panic)".to_string()
        };
        let bt = std::backtrace::Backtrace::force_capture();
        let body = format!("[{at}] thread={name}\n位置: {loc}\n消息: {msg}\n\n{bt}\n{}\n", "-".repeat(70));
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(body.as_bytes());
        }
        default_hook(info);
    }));
}

/// 退出前：把还没编码的电子表格入队，并等写队列全部写完。
pub fn flush_all() {
    objects::flush_dirty(true);
    if let Some(s) = store::global() {
        s.flush();
    }
}

/// 正常退出：落盘，有下载好的新版本就在退出后装上。
fn quit_app(cx: &mut App) {
    flush_all();
    updater::install_on_quit();
    cx.quit();
}

fn main() {
    install_panic_log();
    // 上次选择了「从备份恢复」：在打开数据库之前换入
    let restore = backup::apply_pending();
    let store = match store::Store::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法打开数据库: {e:#}");
            // 双击启动的 .app 没有终端，必须给用户一个看得见的说明
            let mut msg = format!("{e:#}\n\n数据目录：{}", store::data_dir().display());
            if let Some(Err(r)) = &restore {
                msg.push_str(&format!("\n\n{r}"));
            }
            sysui::fatal_dialog("石渠无法打开数据库", &msg, &store::data_dir());
            std::process::exit(1);
        }
    };
    // 恢复结果放进存储的提示通道，窗口打开后由工作区以 toast 显示
    if let Some(r) = restore {
        store.report_error(r.unwrap_or_else(|e| e));
    }
    settings::load(&store);
    i18n::set_english(store.get_kv("lang").as_deref() == Some("en"));
    board::iconlib::warm();
    store::install(store.clone());
    updater::auto_check(&store);
    // 后台回收：7 天前起不再被任何文档引用的嵌入对象
    {
        let s = store.clone();
        std::thread::spawn(move || {
            base::db::prune_history(&s);
            let n = objects::gc(&s);
            if n > 0 {
                eprintln!("回收了 {n} 个未引用对象");
            }
            // 资源文件：对象回收之后再扫引用；引用读不全就跳过，宁可不回收
            match s.asset_reference_texts() {
                Ok(texts) => {
                    let (n, bytes) = assets::gc_assets(&s.assets_dir(), &texts);
                    if n > 0 {
                        eprintln!("回收了 {n} 个未引用资源（{} MB）", bytes / 1024 / 1024);
                    }
                }
                Err(e) => eprintln!("资源回收跳过：{e}"),
            }
            assets::trim_cache(&s.data_dir.join("cache"), assets::CACHE_LIMIT);
        });
    }
    Application::new().with_assets(ui::Assets).run(move |cx: &mut App| {
        cx.set_global(theme::Theme::light());
        sketch::fonts::register(cx);
        bind_keys(cx);
        // 记下内置按键作为基准，再叠加用户在快捷键面板里改过的按键
        shortcuts::init(cx);
        cx.set_menus(menus());
        cx.on_action(|_: &Quit, cx: &mut App| quit_app(cx));
        // 电子表格停手后落盘（只在有脏表时做事）
        cx.spawn(async move |cx: &mut AsyncApp| loop {
            cx.background_executor().timer(objects::SHEET_DEBOUNCE / 2).await;
            if cx.update(|_| objects::flush_dirty(false)).is_err() {
                break;
            }
        })
        .detach();
        cx.on_action(|_: &About, _: &mut App| {});

        let bounds = Bounds::centered(None, size(px(1320.), px(860.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(crate::store::APP_NAME.into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(18.))),
                    }),
                    window_min_size: Some(size(px(720.), px(480.))),
                    app_id: Some("app.stonebrook".into()),
                    // 脚本测试模式下用浮动窗口，避免被其他窗口遮挡后 GPUI 暂停绘制
                    kind: if std::env::var_os("STONEBROOK_SCRIPT").is_some() { WindowKind::Floating } else { WindowKind::Normal },
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Workspace::new(store, window, cx)),
            )
            .expect("open window");

        window
            .update(cx, |ws, window, cx| {
                let editor = ws.editor().clone();
                window.focus(&editor.focus_handle(cx));
                // 关窗口前同步保存
                window.on_window_should_close(cx, move |_, cx| {
                    editor.update(cx, |e, _| e.save_now_queued());
                    flush_all();
                    true
                });
            })
            .ok();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                quit_app(cx);
            }
        })
        .detach();
        cx.activate(true);
        #[cfg(feature = "dev")]
        if let Some(script) = std::env::var("STONEBROOK_SCRIPT").ok().and_then(|p| std::fs::read_to_string(p).ok()) {
            autotest::run(script, window, cx);
        }
    });
}
