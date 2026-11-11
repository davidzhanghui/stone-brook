//! 脚本驱动的端到端测试（仅在设置 STONEBROOK_SCRIPT 时启用）。
//!
//! 通过 GPUI 自身的事件管线注入按键 / 鼠标 / 输入法事件，走的是与真实用户完全相同的代码路径：
//! 快捷键匹配 → Action → 编辑器；字符 → InputHandler；鼠标 → 命中测试。
//!
//! 脚本每行一条命令：
//!   key cmd-b            按键（GPUI keystroke 语法）
//!   type 你好 world       逐字输入
//!   ime nihao            输入法组字（marked text）
//!   commit 你好           输入法上屏
//!   click X Y [N] [shift] 窗口坐标点击（N 为连击次数）
//!   rclick X Y [N] / rmove X Y  投递真实 NSEvent 鼠标点击 / 移动
//!   drag X1 Y1 X2 Y2     拖拽
//!   scroll DY            滚动
//!   html 文件路径          把 HTML 文件放进剪贴板（模拟从网页复制）
//!   htmlfile 文件路径      把 HTML 文件设为光标所在 HTML 块的内容（模拟上传）
//!   attach 路径… / fileview 块 视图 / filefull 块 / drawio 路径
//!   wait MS / shot NAME / dump / quit
//!
//! `shot`：设了 STONEBROOK_SHOT_DIR 时在进程内截取本窗口存为 <目录>/NAME.png（不需要屏幕录制权限）；
//! 否则调用 STONEBROOK_SHOT_CMD 指定的外部命令。

use crate::workspace::Workspace;
use gpui::*;
use std::time::Duration;

pub fn run(script: String, handle: WindowHandle<Workspace>, cx: &mut App) {
    let Ok(ws) = handle.entity(cx) else { return };
    let window: AnyWindowHandle = handle.into();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(Duration::from_millis(800)).await;
        for raw in script.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
            match cmd {
                "wait" => {
                    let ms = arg.parse().unwrap_or(300);
                    cx.background_executor().timer(Duration::from_millis(ms)).await;
                    continue;
                }
                "shot" => {
                    cx.background_executor().timer(Duration::from_millis(350)).await;
                    // 优先在进程内截自己的窗口：不需要「屏幕录制」权限
                    if let Ok(dir) = std::env::var("STONEBROOK_SHOT_DIR") {
                        let path = std::path::Path::new(&dir).join(format!("{arg}.png"));
                        cx.update_window(window, |_, window, _| self_shot(window, &path)).ok();
                        continue;
                    }
                    if let Ok(shot) = std::env::var("STONEBROOK_SHOT_CMD") {
                        let name = arg.to_string();
                        cx.background_executor().spawn(async move { std::process::Command::new(shot).arg(name).status().ok() }).await;
                    }
                    continue;
                }
                "rclick" => {
                    // rclick X Y [N] [right]：投递真实 NSEvent 鼠标事件（命中测试、track_focus 自动聚焦都会走到）
                    let nums: Vec<f32> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    let right = arg.contains("right");
                    let n = nums.get(2).copied().unwrap_or(1.) as isize;
                    for i in 1..=n {
                        cx.update_window(window, |_, window, _| post_ns_mouse(window, nums[0], nums[1], i, right)).ok();
                        cx.background_executor().timer(Duration::from_millis(60)).await;
                    }
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    continue;
                }
                "rdrag" => {
                    // rdrag X1 Y1 X2 Y2：投递真实 NSEvent 左键拖拽（按下 → 分段拖动 → 抬起）
                    let n: Vec<f32> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    cx.update_window(window, |_, window, _| post_ns_mouse_events(window, n[0], n[1], 1, &[MouseKind::LeftDown])).ok();
                    cx.background_executor().timer(Duration::from_millis(60)).await;
                    for i in 1..=8 {
                        let t = i as f32 / 8.;
                        let (x, y) = (n[0] + (n[2] - n[0]) * t, n[1] + (n[3] - n[1]) * t);
                        cx.update_window(window, |_, window, _| post_ns_mouse_events(window, x, y, 1, &[MouseKind::LeftDragged])).ok();
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                    }
                    cx.update_window(window, |_, window, _| post_ns_mouse_events(window, n[2], n[3], 1, &[MouseKind::LeftUp])).ok();
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    continue;
                }
                "rmove" => {
                    // rmove X Y：投递真实 NSEvent 鼠标移动（悬停效果会走到）
                    let nums: Vec<f32> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    cx.update_window(window, |_, window, _| post_ns_mouse_events(window, nums[0], nums[1], 0, &[MouseKind::Moved])).ok();
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    continue;
                }
                "nskey" => {
                    // 逐键投递真实 NSEvent 到本进程事件队列：会经过 NSTextInputContext，也就是真实输入法
                    // 序列里 '_' 为空格、'#' 为回车
                    for ch in arg.chars() {
                        cx.update_window(window, |_, window, _| post_ns_key(window, ch)).ok();
                        cx.background_executor().timer(Duration::from_millis(140)).await;
                    }
                    cx.background_executor().timer(Duration::from_millis(200)).await;
                    continue;
                }
                "type" => {
                    for ch in arg.chars() {
                        let arg = ch.to_string();
                        cx.update_window(window, |_, window, cx| {
                            let ks = if ch == ' ' {
                                Keystroke { key: "space".into(), key_char: Some(" ".into()), ..Default::default() }
                            } else {
                                Keystroke { key: arg.clone(), key_char: Some(arg.clone()), ..Default::default() }
                            };
                            window.dispatch_keystroke(ks, cx);
                        })
                        .ok();
                        cx.background_executor().timer(Duration::from_millis(12)).await;
                    }
                    cx.background_executor().timer(Duration::from_millis(120)).await;
                    continue;
                }
                _ => {}
            }
            let arg = arg.to_string();
            let cmd = cmd.to_string();
            let ws = ws.clone();
            cx.update_window(window, |_, window, cx| exec(&cmd, &arg, &ws, window, cx)).ok();
            cx.background_executor().timer(Duration::from_millis(140)).await;
        }
    })
    .detach();
}

/// 在窗口坐标 (x, y) 投递一次鼠标按下 + 抬起（click_count = n）。
fn post_ns_mouse(window: &Window, x: f32, y: f32, n: isize, right: bool) {
    post_ns_mouse_events(window, x, y, n, if right { &[MouseKind::RightDown, MouseKind::RightUp] } else { &[MouseKind::LeftDown, MouseKind::LeftUp] });
}

#[derive(Clone, Copy)]
enum MouseKind {
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    Moved,
    LeftDragged,
}

/// 在窗口坐标 (x, y) 投递一组真实 NSEvent 鼠标事件。
fn post_ns_mouse_events(window: &Window, x: f32, y: f32, n: isize, kinds: &[MouseKind]) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view: &NSView = unsafe { &*(h.ns_view.as_ptr() as *const NSView) };
    let Some(win) = view.window() else { return };
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    // AppKit 窗口坐标原点在左下
    let loc = NSPoint::new(x as f64, view.bounds().size.height - y as f64);
    let now = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    for kind in kinds {
        let ty = match kind {
            MouseKind::LeftDown => NSEventType::LeftMouseDown,
            MouseKind::LeftUp => NSEventType::LeftMouseUp,
            MouseKind::RightDown => NSEventType::RightMouseDown,
            MouseKind::RightUp => NSEventType::RightMouseUp,
            MouseKind::Moved => NSEventType::MouseMoved,
            MouseKind::LeftDragged => NSEventType::LeftMouseDragged,
        };
        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty,
            loc,
            NSEventModifierFlags::empty(),
            now,
            win.windowNumber(),
            None,
            0,
            n,
            1.0,
        );
        if let Some(ev) = ev {
            app.postEvent_atStart(&ev, false);
        }
    }
}

/// 构造一对 keyDown / keyUp 的 NSEvent 投递到本进程（异步入队，不会重入 GPUI）。
fn post_ns_key(window: &Window, ch: char) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::{NSPoint, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    const CODES: &[(char, u16)] = &[
        ('a', 0),
        ('s', 1),
        ('d', 2),
        ('f', 3),
        ('h', 4),
        ('g', 5),
        ('z', 6),
        ('x', 7),
        ('c', 8),
        ('v', 9),
        ('b', 11),
        ('q', 12),
        ('w', 13),
        ('e', 14),
        ('r', 15),
        ('y', 16),
        ('t', 17),
        ('1', 18),
        ('2', 19),
        ('3', 20),
        ('4', 21),
        ('6', 22),
        ('5', 23),
        ('9', 25),
        ('7', 26),
        ('8', 28),
        ('0', 29),
        ('o', 31),
        ('u', 32),
        ('i', 34),
        ('p', 35),
        ('l', 37),
        ('j', 38),
        ('k', 40),
        ('n', 45),
        ('m', 46),
        ('_', 49),
        ('#', 36),
    ];
    let Some(&(_, code)) = CODES.iter().find(|(c, _)| *c == ch) else { return };
    let chars = match ch {
        '_' => " ".to_string(),
        '#' => "\r".to_string(),
        c => c.to_string(),
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view: &NSView = unsafe { &*(h.ns_view.as_ptr() as *const NSView) };
    let Some(win) = view.window() else { return };
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let s = NSString::from_str(&chars);
    let now = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    for ty in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty,
            NSPoint::new(0., 0.),
            NSEventModifierFlags::empty(),
            now,
            win.windowNumber(),
            None,
            &s,
            &s,
            false,
            code,
        );
        if let Some(ev) = ev {
            app.postEvent_atStart(&ev, false);
        }
    }
}

/// 截取本进程的 GPUI 窗口写成 PNG（应用截自己的窗口不受屏幕录制权限限制）。
#[allow(deprecated)]
fn self_shot(window: &Window, path: &std::path::Path) {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSView};
    use objc2_core_graphics::{CGRectNull, CGWindowImageOption, CGWindowListCreateImage, CGWindowListOption};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view: &NSView = unsafe { &*(h.ns_view.as_ptr() as *const NSView) };
    let Some(win) = view.window() else { return };
    let num = win.windowNumber() as u32;
    let img = CGWindowListCreateImage(unsafe { CGRectNull }, CGWindowListOption::OptionIncludingWindow, num, CGWindowImageOption::BoundsIgnoreFraming);
    let Some(img) = img else {
        eprintln!("SHOT 失败：无法截取窗口 {num}");
        return;
    };
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &img);
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) };
    if let Some(d) = data {
        std::fs::write(path, d.to_vec()).ok();
        println!("SHOT {}", path.display());
    }
}

fn click(ws: &Entity<Workspace>, window: &mut Window, cx: &mut App, x: f32, y: f32, clicks: usize, shift: bool) {
    let editor = ws.read(cx).editor().clone();
    window.focus(&editor.focus_handle(cx));
    editor.update(cx, |e, cx| {
        if let Some(p) = e.hit_test(point(px(x), px(y))) {
            e.mouse_select(p, clicks, shift, cx);
        }
        e.root_mouse_up(
            &MouseUpEvent { button: MouseButton::Left, position: point(px(x), px(y)), modifiers: Modifiers::default(), click_count: clicks },
            window,
            cx,
        );
    });
}

fn exec(cmd: &str, arg: &str, ws: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    let nums: Vec<f32> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    match COMMANDS.iter().find(|(names, _)| names.contains(&cmd)) {
        Some((_, f)) => f(cmd, arg, &nums, ws, window, cx),
        None => eprintln!("未知命令: {cmd}"),
    }
}

type Cmd = fn(&str, &str, &[f32], &Entity<Workspace>, &mut Window, &mut App);

/// 命令名 → 处理函数（多个别名共用一项）。
#[allow(unused_variables)]
static COMMANDS: &[(&[&str], Cmd)] = &[
    (&["key"], |cmd, arg, nums, ws, window, cx| {
        if let Ok(ks) = Keystroke::parse(arg) {
            window.dispatch_keystroke(ks, cx);
        }
    }),
    (&["ime", "commit"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            if cmd == "ime" {
                e.replace_and_mark_text_in_range(None, arg, None, window, cx);
            } else {
                e.replace_text_in_range(None, arg, window, cx);
            }
        });
    }),
    (&["click"], |cmd, arg, nums, ws, window, cx| {
        let shift = arg.contains("shift");
        let clicks = nums.get(2).copied().unwrap_or(1.) as usize;
        click(ws, window, cx, nums[0], nums[1], clicks, shift);
    }),
    (&["move"], |cmd, arg, nums, ws, window, cx| {
        // 只移动指针（不按键），走与真实鼠标移动相同的编辑区处理
        let position = point(px(nums[0]), px(nums[1]));
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            e.root_mouse_move(&MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }, window, cx);
        });
    }),
    (&["win"], |cmd, arg, nums, ws, window, cx| {
        // 打印窗口在屏幕上的位置，供外部真实鼠标注入换算坐标
        let b = window.bounds();
        println!("WIN x={} y={} w={} h={}", f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
    }),
    (&["drag"], |cmd, arg, nums, ws, window, cx| {
        let (a, b) = (point(px(nums[0]), px(nums[1])), point(px(nums[2]), px(nums[3])));
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            if let Some(p) = e.hit_test(a) {
                e.mouse_select(p, 1, false, cx);
            }
            for i in 1..=8 {
                let t = i as f32 / 8.;
                let position = point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                e.root_mouse_move(&MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }, window, cx);
            }
            e.root_mouse_up(&MouseUpEvent { button: MouseButton::Left, position: b, modifiers: Modifiers::default(), click_count: 1 }, window, cx);
        });
    }),
    (&["scroll"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            e.list.scroll_by(px(nums[0]));
            cx.notify();
        });
    }),
    (&["action"], |cmd, arg, nums, ws, window, cx| {
        if let Ok(a) = cx.build_action(arg, None) {
            window.dispatch_action(a, cx);
        } else {
            eprintln!("未知 action: {arg}");
        }
    }),
    (&["perf"], |cmd, arg, nums, ws, window, cx| {
        let e = ws.read(cx).editor().read(cx);
        let (med, p95, max) = e.perf_stats();
        println!("PERF blocks={} median={med:.2}ms p95={p95:.2}ms max={max:.2}ms samples={}", e.content.blocks.len(), e.caches.frame_times.len());
    }),
    (&["blockmenu"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            e.pop.block_menu = Some(nums[0] as usize);
            cx.notify();
        });
    }),
    (&["files"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        let paths = arg.split_whitespace().map(std::path::PathBuf::from).collect();
        editor.update(cx, |e, cx| e.insert_files(paths, cx));
    }),
    // images 路径…：同斜杠菜单「图片」
    (&["images"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        let paths = arg.split_whitespace().map(std::path::PathBuf::from).collect();
        editor.update(cx, |e, cx| e.insert_images(paths, cx));
    }),
    // attach 路径…：同斜杠菜单「视频或文件」；fileview 块序号 视图(0 预览 / 1 卡片 / 2 文本)
    // drawio 路径：导入 draw.io 文件为画板
    (&["drawio"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.import_drawio(vec![std::path::PathBuf::from(arg.trim())], cx));
    }),
    (&["attach"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        let paths = arg.split_whitespace().map(std::path::PathBuf::from).collect();
        editor.update(cx, |e, cx| e.insert_attachments(paths, cx));
    }),
    (&["fileview"], |cmd, arg, nums, ws, window, cx| {
        let n: Vec<usize> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.test_file_view(n.first().copied().unwrap_or(1), n.get(1).copied().unwrap_or(0) as u8, cx));
    }),
    // videofull 块序号：放大预览（视频进入全屏浮层）
    (&["videofull"], |cmd, arg, nums, ws, window, cx| {
        let n: usize = arg.trim().parse().unwrap_or(1);
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.test_open_attachment(n, cx));
    }),
    // filefull 块序号：放大预览（全屏预览浮层）
    (&["filefull"], |cmd, arg, nums, ws, window, cx| {
        let n: usize = arg.trim().parse().unwrap_or(1);
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.test_file_full(n, cx));
    }),
    (&["html"], |cmd, arg, nums, ws, window, cx| {
        if let Ok(h) = std::fs::read_to_string(arg) {
            crate::clipboard::write("(html)", Some(&h), None);
        }
    }),
    (&["htmlfile"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| {
            let ix = e.core.head.block;
            e.set_html_file(ix, std::path::PathBuf::from(arg.trim()), cx);
        });
    }),
    (&["textfile"], |cmd, arg, nums, ws, window, cx| {
        if let Ok(t) = std::fs::read_to_string(arg) {
            crate::clipboard::write(&t, None, None);
        }
    }),
    (&["diag2board"], |cmd, arg, nums, ws, window, cx| {
        let ix = arg.trim().parse::<usize>().unwrap_or(1);
        let editor = ws.read(cx).editor().clone();
        editor.update(cx, |e, cx| e.test_diagram_to_board(ix, cx));
    }),
    (&["board"], |cmd, arg, nums, ws, window, cx| {
        // board tool NAME | board pop NAME | board menu | board picker | board drag X1 Y1 X2 Y2（世界坐标）| board quick SIDE | board esc
        if let Some(b) = ws.read(cx).board_view() {
            let parts: Vec<&str> = arg.split_whitespace().collect();
            let n: Vec<f32> = parts.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
            b.update(cx, |b, cx| match parts.first().copied().unwrap_or("") {
                "tool" => b.test_set_tool(parts.get(1).copied().unwrap_or(""), cx),
                "pop" => b.test_popover(parts.get(1).copied().unwrap_or(""), cx),
                "menu" => b.test_menu(cx),
                "picker" => b.test_picker(cx),
                "drag" if n.len() >= 4 => b.test_world_drag(crate::canvas::pt(n[0], n[1]), crate::canvas::pt(n[2], n[3]), window, cx),
                "quick" => b.test_quick(n.first().copied().unwrap_or(1.) as usize, window, cx),
                "geo" => b.test_geo(),
                "stress" => b.test_stress(n.first().copied().unwrap_or(500.) as usize, cx),
                "pick" => b.test_pick(n.first().copied().unwrap_or(0.) as usize, window, cx),
                _ => b.test_cmd(&parts, window, cx),
            });
        }
    }),
    // docsheet：在文档光标处插入一个电子表格（文档内直接编辑）
    (&["docsheet"], |cmd, arg, nums, ws, window, cx| {
        let id = crate::objects::create(crate::objects::ObjKind::Sheet, None);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.insert_embed("sheet", &id, cx));
    }),
    // livesheet 子命令：对文档里第一个内嵌表格执行 sheet 测试命令
    (&["livesheet"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor.clone();
        let v = editor.read(cx).live_sheets.values().next().map(|v| v.0.clone());
        if let Some(v) = v {
            let (sub, rest) = arg.split_once(' ').unwrap_or((arg, ""));
            if sub == "focus" {
                let fh = v.read(cx).focus.clone();
                window.focus(&fh);
            } else {
                v.update(cx, |s, cx| s.test(sub, rest, window, cx));
            }
        }
    }),
    (&["opensheet"], |cmd, arg, nums, ws, window, cx| {
        ws.update(cx, |w, cx| w.open_embed("sheet", if arg.is_empty() { "autotest-sheet" } else { arg }, window, cx))
    }),
    (&["opensketch"], |cmd, arg, nums, ws, window, cx| {
        let id = if arg.is_empty() { crate::objects::create(crate::objects::ObjKind::Sketch, None) } else { arg.to_string() };
        ws.update(cx, |w, cx| w.open_embed("sketch", &id, window, cx));
    }),
    (&["sketch"], |cmd, arg, nums, ws, window, cx| {
        // sketch tool NAME | sketch drag X1 Y1 X2 Y2 [shift] | sketch click X Y [N] | sketch text X Y 文字 | sketch key NAME | sketch dump | sketch open PATH
        if let Some(s) = ws.read(cx).sketch_view() {
            s.update(cx, |s, cx| s.test_cmd(arg, window, cx));
        }
    }),
    (&["openboard"], |cmd, arg, nums, ws, window, cx| {
        ws.update(cx, |w, cx| w.open_embed("board", if arg.is_empty() { "autotest-board" } else { arg }, window, cx))
    }),
    (&["boardedit"], |cmd, arg, nums, ws, window, cx| {
        if let Some(b) = ws.read(cx).board_view() {
            b.update(cx, |b, cx| b.test_edit(arg.parse().unwrap_or(0), window, cx));
        }
    }),
    (&["boarddump"], |cmd, arg, nums, ws, window, cx| {
        if let Some(b) = ws.read(cx).board_view() {
            b.read(cx).test_dump(window, cx);
        }
    }),
    (&["boardlib", "boardlayout", "boardsel", "boardrough", "boardpen"], |cmd, arg, nums, ws, window, cx| {
        if let Some(b) = ws.read(cx).board_view() {
            b.update(cx, |b, cx| match cmd {
                "boardlib" => {
                    let (n, text) = arg.split_once(' ').unwrap_or((arg, ""));
                    b.test_lib(n.parse().unwrap_or(0), text, cx);
                }
                "boardsel" => b.test_select(arg.parse().unwrap_or(0), cx),
                "boardrough" => b.test_rough(cx),
                "boardpen" => b.test_pen(cx),
                _ => b.test_layout(cx),
            });
        }
    }),
    (&["sheet"], |cmd, arg, nums, ws, window, cx| {
        if let Some(v) = ws.read(cx).embed_view() {
            let (sub, rest) = arg.split_once(' ').unwrap_or((arg, ""));
            if let Ok(s) = v.downcast::<crate::sheet::SheetEditor>() {
                s.update(cx, |s, cx| s.test(sub, rest, window, cx));
            }
        }
    }),
    (&["openbase"], |cmd, arg, nums, ws, window, cx| {
        // openbase [grid|kanban|gallery|gantt]：新建一个多维表格并全屏打开
        let vk = match arg {
            "kanban" => crate::base::ViewKind::Kanban,
            "gallery" => crate::base::ViewKind::Gallery,
            "gantt" => crate::base::ViewKind::Gantt,
            _ => crate::base::ViewKind::Grid,
        };
        // openbase last：打开最近的一个已有多维表格（验证重启后从数据库读回）
        if arg == "last" {
            if let Some((id, ..)) = crate::objects::list_bases().into_iter().next() {
                ws.update(cx, |w, cx| w.open_embed("base", &id, window, cx));
            }
            return;
        }
        let id = crate::objects::create_base(vk);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.insert_embed("base", &id, cx));
        ws.update(cx, |w, cx| w.open_embed("base", &id, window, cx));
    }),
    (&["base"], |cmd, arg, nums, ws, window, cx| {
        if let Some(v) = ws.read(cx).embed_view() {
            let (sub, rest) = arg.split_once(' ').unwrap_or((arg, ""));
            if let Ok(b) = v.downcast::<crate::base::editor::BaseEditor>() {
                b.update(cx, |b, cx| b.test(sub, rest, window, cx));
            }
        }
    }),
    (&["closeembed"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.close_embed(window, cx))),
    (&["tplopen"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.open_templates(arg == "new", cx))),
    (&["tplpick"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.apply_template(arg.parse().ok(), None, window, cx))),
    (&["tplsave"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.save_as_template(cx))),
    (&["syncmake"], |cmd, arg, nums, ws, window, cx| ws.read(cx).editor.clone().update(cx, |e, cx| e.make_sync(cx))),
    (&["syncins"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor.clone();
        let id = ws.read(cx).store.list_sync().get(arg.parse::<usize>().unwrap_or(0)).map(|x| x.0.clone());
        if let Some(id) = id {
            editor.update(cx, |e, cx| e.insert_sync(&id, cx));
        }
    }),
    (&["opendoc"], |cmd, arg, nums, ws, window, cx| {
        let id = ws.read(cx).store.list().into_iter().find(|d| d.title == arg).map(|d| d.id);
        if let Some(id) = id {
            ws.update(cx, |w, cx| w.open_doc(&id, window, cx));
        }
    }),
    (&["pdf"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.test_pdf(arg.to_string(), cx))),
    (&["urlpaste"], |cmd, arg, nums, ws, window, cx| {
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| if arg == "card" { e.url_paste_card(cx) } else { e.url_paste_title(cx) });
    }),
    (&["docmind"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.open_doc_mindmap(window, cx))),
    // 图片：imgwidth N W | imgcrop N X Y W H | imgcropreset N | align N 0|1|2 | imgalt N 文字 | imgdesc
    (&["imgwidth"], |cmd, arg, nums, ws, window, cx| {
        let parts: Vec<&str> = arg.split_whitespace().collect();
        let editor = ws.read(cx).editor.clone();
        if parts.len() == 2 {
            let (ix, w) = (parts[0].parse::<usize>().unwrap_or(1), parts[1].parse::<f32>().unwrap_or(400.));
            editor.update(cx, |e, cx| e.set_image_width(ix, Some(w), cx));
        }
    }),
    (&["imgfit"], |cmd, arg, nums, ws, window, cx| {
        let ix = arg.parse::<usize>().unwrap_or(1);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.set_image_width(ix, None, cx));
    }),
    (&["imgcrop"], |cmd, arg, nums, ws, window, cx| {
        let p: Vec<f32> = arg.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let ix = arg.split_whitespace().next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        if p.len() == 4 {
            let editor = ws.read(cx).editor.clone();
            editor.update(cx, |e, cx| e.set_image_crop(ix, Some((p[0], p[1], p[2], p[3])), cx));
        }
    }),
    (&["crop"], |cmd, arg, nums, ws, window, cx| {
        // crop start B | crop drag <nw|n|ne|e|se|s|sw|w|move> DX DY（像素）| crop commit | crop cancel
        let parts: Vec<&str> = arg.split_whitespace().collect();
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| match parts.first().copied().unwrap_or("") {
            "start" => e.start_crop(parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1), cx),
            "drag" => {
                use crate::editor::CropHandle as H;
                let h = match parts.get(1).copied().unwrap_or("") {
                    "nw" => H::NW,
                    "n" => H::N,
                    "ne" => H::NE,
                    "e" => H::E,
                    "se" => H::SE,
                    "s" => H::S,
                    "sw" => H::SW,
                    "w" => H::W,
                    _ => H::Move,
                };
                let dx = parts.get(2).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let dy = parts.get(3).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let o = point(px(500.), px(400.));
                e.crop_begin_drag(h, o, cx);
                e.crop_drag_move(point(o.x + px(dx), o.y + px(dy)), cx);
                if let Some(c) = e.sess.image_crop.as_mut() {
                    c.drag = None;
                    println!("CROP rect={:?}", c.rect);
                }
            }
            "commit" => e.crop_commit(cx),
            "cancel" => e.crop_cancel(cx),
            _ => {}
        });
    }),
    (&["preview"], |cmd, arg, nums, ws, window, cx| {
        let ix = arg.trim().parse::<usize>().unwrap_or(1);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.open_image_preview(ix, cx));
    }),
    (&["imgcropreset"], |cmd, arg, nums, ws, window, cx| {
        let ix = arg.parse::<usize>().unwrap_or(1);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.reset_image_crop(ix, cx));
    }),
    (&["imgalt"], |cmd, arg, nums, ws, window, cx| {
        let mut it = arg.splitn(2, ' ');
        let ix = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        let text = it.next().unwrap_or("").to_string();
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| {
            e.checkpoint(crate::editor::EditKind::Other);
            if let crate::model::BlockKind::Image { alt, .. } = &mut e.block_mut(ix).kind {
                *alt = text;
            }
            e.changed(cx);
        });
    }),
    (&["align"], |cmd, arg, nums, ws, window, cx| {
        let mut it = arg.split_whitespace();
        let ix = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        let a = it.next().and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.set_align(ix, a, cx));
    }),
    (&["imgdesc"], |cmd, arg, nums, ws, window, cx| {
        let e = ws.read(cx).editor().read(cx);
        for (i, b) in e.content.blocks.iter().enumerate() {
            if let crate::model::BlockKind::Image { path, alt, width, crop } = &b.kind {
                let nat = e.image_natural(i);
                println!("IMG {i} natural={:?} alt={:?} width={:?} crop={:?} box={:?}", nat, alt, width, crop, e.image_display_box(i));
                let _ = path;
            }
        }
        println!("COMMENTS {:?}", e.comments.iter().map(|c| (c.block, e.comment_block_ix(c), c.body.clone(), c.resolved)).collect::<Vec<_>>());
    }),
    (&["comment"], |cmd, arg, nums, ws, window, cx| {
        // comment 块号 内容
        let mut it = arg.splitn(2, ' ');
        let ix = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        let body = it.next().unwrap_or("测试评论").to_string();
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.test_add_comment(ix, &body, cx));
    }),
    (&["resolve"], |cmd, arg, nums, ws, window, cx| {
        let id = arg.to_string();
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| {
            if id == "first" {
                if let Some(c) = e.comments.first().cloned() {
                    e.resolve_comment(&c.id, true, cx);
                }
            }
        });
    }),
    (&["cmtside"], |cmd, arg, nums, ws, window, cx| ws.update(cx, |w, cx| w.toggle_comments_sidebar(cx))),
    // 模拟真实输入法往「评论输入框」里送 marked text / 上屏。
    // 崩溃点是 IME 回调（replace_and_mark_text_in_range），脚本能走同一条路径。
    (&["cinput"], |cmd, arg, nums, ws, window, cx| {
        // cinput open|close|ime <文字>|commit <文字>|raw <start> <end> <文字>
        let (sub, rest) = arg.split_once(' ').unwrap_or((arg, ""));
        let editor = ws.read(cx).editor().clone();
        match sub {
            "open" => {
                let ix = rest.trim().parse::<usize>().unwrap_or(1);
                editor.update(cx, |e, cx| e.test_open_comment_input(ix, window, cx));
            }
            "close" => editor.update(cx, |e, cx| {
                e.sess.comment_input = None;
                e.sess.comment_open = None;
                cx.notify();
            }),
            "ime" | "commit" | "raw" => {
                editor.update(cx, |e, cx| e.test_comment_ime(sub, rest, window, cx));
            }
            "state" => {
                let e = ws.read(cx).editor().read(cx);
                let (open, text) = match e.sess.comment_input.as_ref() {
                    Some(ci) => (true, ci.input.read(cx).text().to_string()),
                    None => (false, String::new()),
                };
                println!("CINPUT open={open} text={text:?} panel={:?}", e.sess.comment_open);
            }
            "dismiss" => {
                // 等价于「点了浮层外面」
                editor.update(cx, |e, cx| e.dismiss_comments(window, cx));
            }
            "outside" => {
                // outside X Y：模拟在窗口坐标 (X,Y) 按下鼠标，走浮层外关闭判断
                let mut it = rest.split_whitespace();
                let x = it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let y = it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                editor.update(cx, |e, cx| e.comment_outside(point(px(x), px(y)), window, cx));
            }
            "bounds" => {
                let e = ws.read(cx).editor().read(cx);
                let f = e.frame.borrow();
                println!("CBOUNDS input={:?} panel={:?}", f.comment_input_bounds, f.comment_panel_bounds);
            }
            _ => eprintln!("未知 cinput 子命令: {sub}"),
        }
    }),
    // 表格：tbl list | tbl ins R C | tbl op B OP
    (&["tbl"], |cmd, arg, nums, ws, window, cx| {
        let parts: Vec<&str> = arg.split_whitespace().collect();
        let sub = parts.first().copied().unwrap_or("list").to_string();
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| {
            let show = |e: &crate::editor::Editor| {
                println!("PICKER {:?}", e.pop.table_picker.as_ref().map(|t| (t.rows, t.cols, t.block)));
                for (i, b) in e.content.blocks.iter().enumerate() {
                    if let crate::model::BlockKind::Table { rows, cols, cells, header_row, header_col, widths, merges } = &b.kind {
                        println!(
                            "TABLE ix={i} rows={rows} cols={cols} cells={} header_row={header_row} header_col={header_col} widths={widths:?} merges={merges:?}",
                            cells.len()
                        );
                    }
                }
            };
            match sub.as_str() {
                "ins" => {
                    let r = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
                    let c = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
                    let b = e.core.head.block;
                    e.pop.slash = Some(crate::editor::SlashMenu { block: b, start: e.core.head.offset, selected: 0, scrolled: Default::default() });
                    e.insert_table_from_slash(r, c, cx);
                    println!("TABLE inserted {r}x{c} at {b}");
                }
                "picker" => {
                    let r = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
                    let c = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
                    let b = e.core.head.block;
                    e.pop.table_picker = Some(crate::editor::TablePicker { block: b, rows: r, cols: c, y: px(240.) });
                    cx.notify();
                    println!("TABLE picker open {r}x{c}");
                }
                "sel" => {
                    // tbl sel B R1 C1 R2 C2：模拟框选多个单元格
                    let nums: Vec<usize> = parts.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
                    if nums.len() >= 5 {
                        let (b, r1, c1, r2, c2) = (nums[0], nums[1], nums[2], nums[3], nums[4]);
                        if let crate::model::BlockKind::Table { cols, .. } = &e.content.blocks[b].kind {
                            let cols = *cols;
                            e.core.anchor = crate::editor::Pos::new(b, r1 * cols + c1, 0);
                            e.core.head = crate::editor::Pos::new(b, r2 * cols + c2, 0);
                            println!("SELECT from ({r1},{c1}) to ({r2},{c2})");
                        }
                    }
                }
                "frag" => {
                    // tbl frag B R1 C1 R2 C2：打印表格选区复制出的片段
                    let nums: Vec<usize> = parts.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
                    if nums.len() >= 5 {
                        let (b, r1, c1, r2, c2) = (nums[0], nums[1], nums[2], nums[3], nums[4]);
                        if let crate::model::BlockKind::Table { cols, .. } = &e.content.blocks[b].kind {
                            let cols = *cols;
                            e.core.anchor = crate::editor::Pos::new(b, r1 * cols + c1, 0);
                            e.core.head = crate::editor::Pos::new(b, r2 * cols + c2, 0);
                            let frag = e.selection_fragment();
                            let refs: Vec<&crate::model::Block> = frag.iter().collect();
                            println!("FRAG {} blocks\n{}", frag.len(), crate::convert::blocks_to_md(&refs));
                        }
                    }
                }
                "layout" => {
                    // tbl layout B：打印表格前几格的实际布局区域
                    let b = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(2);
                    let f = e.frame.borrow();
                    for c in 0..10 {
                        if let Some(l) = f.layouts.get(&(b, c)) {
                            println!("CELL b={b} c={c} {:?}", l.bounds());
                        }
                    }
                    if let Some(g) = e.content.blocks.get(b).and_then(|blk| e.caches.table_geo.borrow().get(&blk.id).cloned()) {
                        println!(
                            "GEO grid={:?} rows={} cols={:?}",
                            g.grid,
                            g.rows.len(),
                            g.cols.iter().map(|c| (f32::from(c.left()), f32::from(c.size.width))).collect::<Vec<_>>()
                        );
                    }
                }
                "bounds" => {
                    let f = e.frame.borrow();
                    println!("SLASH_BOUNDS {:?} CURSOR {:?} ANCHOR {:?}", f.slash_bounds, f.cursor, e.slash_at);
                    if e.pop.table_picker.is_some() {
                        println!("PICKER_POS {:?}", e.table_picker_pos(window));
                    }
                }
                "hover" => match parts.get(1).copied().unwrap_or("") {
                    "open" => {
                        e.show_slash_table_picker(point(px(400.), px(300.)), cx);
                        println!("HOVER open");
                    }
                    "close" => {
                        e.schedule_close_table_picker(cx);
                        println!("HOVER close");
                    }
                    "keep" => {
                        e.keep_table_picker();
                        println!("HOVER keep");
                    }
                    _ => {}
                },
                "coldrag" => {
                    // tbl coldrag B C DX：拖动第 C 列右侧边线 DX 像素
                    let nums: Vec<f32> = parts.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
                    if nums.len() >= 3 {
                        let (b, c, dx) = (nums[0] as usize, nums[1] as usize, nums[2]);
                        let id = e.content.blocks[b].id;
                        let x = e.caches.table_geo.borrow().get(&id).and_then(|g| g.cols.get(c).map(|r| r.right()));
                        if let Some(x) = x {
                            e.start_table_col_drag(b, c, point(x, px(0.)), cx);
                            e.drag_table_col(x + px(dx), cx);
                            e.end_table_col_drag(cx);
                        } else {
                            println!("TABLE coldrag: 没有几何信息");
                        }
                    }
                }
                "select" => {
                    // tbl select B R1 C1 R2 C2：模拟点击行 / 列手柄
                    let n: Vec<usize> = parts.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
                    if n.len() >= 5 {
                        e.table_select(n[0], (n[1], n[2], n[3], n[4]), cx);
                        println!("TABLE rect {:?}", e.table_sel_rect(n[0]));
                    }
                }
                "op" => {
                    let b = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let op = parts.get(2).copied().unwrap_or("");
                    e.table_op(b, op, cx);
                    println!("TABLE op {op} on {b}");
                }
                _ => {}
            }
            show(e);
        });
    }),
    (&["exporttest"], |cmd, arg, nums, ws, window, cx| {
        // exporttest <md|html> <路径>：不弹保存框，直接写盘验证导出内容
        let mut it = arg.splitn(2, ' ');
        let kind = it.next().unwrap_or("md").to_string();
        let out = it.next().unwrap_or("/tmp/stonebrook-export").trim().to_string();
        let e = ws.read(cx).editor().read(cx);
        let blocks = &e.content.blocks;
        let body = if kind == "html" {
            crate::convert::full_html_page(&e.content.title(), &crate::convert::blocks_to_html(blocks))
        } else {
            crate::convert::blocks_to_md(blocks)
        };
        match std::fs::write(&out, body) {
            Ok(_) => println!("EXPORT ok {out}"),
            Err(err) => println!("EXPORT fail {err}"),
        }
    }),
    (&["cmtopen"], |cmd, arg, nums, ws, window, cx| {
        let mut it = arg.splitn(2, ' ');
        let ix = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        let editor = ws.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.toggle_comment_open(ix, cx));
    }),
    (&["text"], |cmd, arg, nums, ws, window, cx| crate::clipboard::write(&arg.replace("\\n", "\n"), None, None)),
    (&["dump"], |cmd, arg, nums, ws, window, cx| {
        let e = ws.read(cx).editor().read(cx);
        println!("===DUMP {} blocks / head {:?} anchor {:?}===", e.content.blocks.len(), e.core.head, e.core.anchor);
        println!("{}", crate::convert::blocks_to_md(&e.content.blocks));
        println!("===END===");
    }),
    (&["quit"], |cmd, arg, nums, ws, window, cx| {
        ws.read(cx).editor().clone().update(cx, |e, _| e.save_now_queued());
        crate::flush_all();
        cx.quit();
    }),
];
