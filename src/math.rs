//! 公式渲染（P6-2）：KaTeX + 离屏 WebView，结果缓存为 2 倍分辨率 PNG。
//!
//! 渲染是异步的：界面调用 `request` 立即得到当前状态（就绪 / 渲染中 / 出错），
//! 渲染完成后刷新所有窗口。同一批次的公式放在一个页面里一次性渲染，逐个截图。

use crate::webview;
use gpui::{App, AsyncApp};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum MathState {
    Pending,
    Ready { path: PathBuf, width: f32, height: f32 },
    Error(String),
}

#[derive(Serialize, Deserialize)]
struct Meta {
    w: f32,
    h: f32,
    #[serde(default)]
    err: Option<String>,
}

#[derive(Default)]
struct Cache {
    map: crate::lru::Lru<String, MathState>,
    queue: Vec<Job>,
    running: bool,
}

#[derive(Clone)]
struct Job {
    key: String,
    latex: String,
    display: bool,
    dark: bool,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn key_for(latex: &str, display: bool, dark: bool) -> String {
    webview::hash_key(&[latex, if display { "d" } else { "i" }, if dark { "dark" } else { "light" }, "v2"])
}

/// 释放 KaTeX 资源到数据目录，返回目录路径。
pub fn katex_dir() -> PathBuf {
    let dir = crate::store::data_dir().join(format!("katex-{}", crate::katex_assets::VERSION));
    if !dir.join("katex.min.js").exists() {
        std::fs::create_dir_all(dir.join("fonts")).ok();
        for (name, bytes) in crate::katex_assets::FILES {
            std::fs::write(dir.join(name), bytes).ok();
        }
    }
    dir
}

/// 查询状态；需要渲染时只入队，由 `pump` 在下一帧启动渲染（用于没有 App 上下文的地方）。
pub fn peek(latex: &str, display: bool, dark: bool) -> MathState {
    let latex = latex.trim();
    if latex.is_empty() {
        return MathState::Error("空公式".into());
    }
    let key = key_for(latex, display, dark);
    if let Some(s) = CACHE.with(|c| c.borrow().map.get(&key).cloned()) {
        return s;
    }
    let dir = webview::cache_dir();
    if let Some(meta) = std::fs::read_to_string(dir.join(format!("math-{key}.json"))).ok().and_then(|s| serde_json::from_str::<Meta>(&s).ok()) {
        let state = match meta.err {
            Some(e) => MathState::Error(e),
            None => MathState::Ready { path: dir.join(format!("math-{key}.png")), width: meta.w, height: meta.h },
        };
        CACHE.with(|c| c.borrow_mut().map.insert(key, state.clone()));
        return state;
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.map.insert(key.clone(), MathState::Pending);
        c.queue.push(Job { key, latex: latex.to_string(), display, dark });
    });
    MathState::Pending
}

/// 有排队的公式且没有在渲染时，启动渲染。
pub fn pump(cx: &mut App) {
    let start = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        !c.queue.is_empty() && !std::mem::replace(&mut c.running, true)
    });
    if start {
        cx.spawn(async move |cx: &mut AsyncApp| run_queue(cx).await).detach();
    }
}

/// 查询公式渲染状态；未渲染过的公式会加入队列。
pub fn request(latex: &str, display: bool, dark: bool, cx: &mut App) -> MathState {
    let latex = latex.trim();
    if latex.is_empty() {
        return MathState::Error("空公式".into());
    }
    let key = key_for(latex, display, dark);
    let cached = CACHE.with(|c| c.borrow().map.get(&key).cloned());
    if let Some(s) = cached {
        return s;
    }
    // 磁盘缓存
    let dir = webview::cache_dir();
    let meta_path = dir.join(format!("math-{key}.json"));
    if let Some(meta) = std::fs::read_to_string(&meta_path).ok().and_then(|s| serde_json::from_str::<Meta>(&s).ok()) {
        let state = match meta.err {
            Some(e) => MathState::Error(e),
            None => MathState::Ready { path: dir.join(format!("math-{key}.png")), width: meta.w, height: meta.h },
        };
        CACHE.with(|c| c.borrow_mut().map.insert(key, state.clone()));
        return state;
    }
    let start = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.map.insert(key.clone(), MathState::Pending);
        c.queue.push(Job { key, latex: latex.to_string(), display, dark });
        !std::mem::replace(&mut c.running, true)
    });
    if start {
        cx.spawn(async move |cx: &mut AsyncApp| run_queue(cx).await).detach();
    }
    MathState::Pending
}

async fn run_queue(cx: &mut AsyncApp) {
    // 稍等片刻，让同一帧里请求的公式合并成一批
    cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
    loop {
        let batch: Vec<Job> = CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let n = c.queue.len().min(40);
            c.queue.drain(..n).collect()
        });
        if batch.is_empty() {
            CACHE.with(|c| c.borrow_mut().running = false);
            break;
        }
        render_batch(cx, batch).await;
        cx.update(|cx| cx.refresh_windows()).ok();
    }
}

async fn render_batch(cx: &AsyncApp, batch: Vec<Job>) {
    let dir = katex_dir();
    // 一个批次内统一明暗主题（同一时刻界面只有一种主题）
    let dark = batch[0].dark;
    let color = if dark { "#e3e4e6" } else { "#1f2329" };
    let items: Vec<serde_json::Value> = batch.iter().map(|j| serde_json::json!({ "src": j.latex, "display": j.display })).collect();
    let spans: String = (0..batch.len()).map(|i| format!("<div class=\"row\"><span class=\"m\" id=\"m{i}\"></span></div>")).collect();
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="katex.min.css"><script src="katex.min.js"></script>
<style>html,body{{margin:0;background:transparent;color:{color};font-size:15px}}body{{padding:6px}}.row{{margin:0 0 10px 0}}.m{{display:inline-block;padding:1px 2px}}.katex{{font-size:1.12em}}.katex-display{{margin:0}}.katex-display>.katex{{font-size:1.25em}}</style>
</head><body>{spans}<script>
var items = {items};
window.__errs = [];
items.forEach(function(it, i) {{
  var el = document.getElementById('m' + i);
  try {{ katex.render(it.src, el, {{ displayMode: it.display, throwOnError: true, output: 'html', strict: false }}); }}
  catch (e) {{ window.__errs[i] = String(e.message || e); el.textContent = ''; }}
}});
window.__rects = function() {{
  return items.map(function(_, i) {{
    if (window.__errs[i]) return null;
    var r = document.getElementById('m' + i).getBoundingClientRect();
    return [Math.floor(r.left), Math.floor(r.top + window.scrollY), Math.ceil(r.width), Math.ceil(r.height)];
  }});
}};
window.__extra = function() {{ return JSON.stringify(window.__errs); }};
document.fonts.ready.then(function() {{ window.__done = true; }});
</script></body></html>"#,
        items = serde_json::Value::Array(items)
    );
    let (snaps, extra) = webview::render_elements(cx, &html, &dir, batch.len(), 1400., Some("window.__extra()")).await;
    let errs: Vec<Option<String>> = extra.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let cache = webview::cache_dir();
    for (i, job) in batch.iter().enumerate() {
        let state = match (snaps.get(i).and_then(|s| s.as_ref()), errs.get(i).cloned().flatten()) {
            (_, Some(err)) => {
                let msg = err.replace("KaTeX parse error: ", "");
                write_meta(&cache, &job.key, Meta { w: 0., h: 0., err: Some(msg.clone()) });
                MathState::Error(msg)
            }
            (Some(s), None) => {
                let path = cache.join(format!("math-{}.png", job.key));
                std::fs::write(&path, &s.png).ok();
                write_meta(&cache, &job.key, Meta { w: s.width as f32, h: s.height as f32, err: None });
                MathState::Ready { path, width: s.width as f32, height: s.height as f32 }
            }
            (None, None) => MathState::Error("渲染失败".into()),
        };
        let _ = dark;
        CACHE.with(|c| c.borrow_mut().map.insert(job.key.clone(), state));
    }
}

fn write_meta(dir: &std::path::Path, key: &str, meta: Meta) {
    if let Ok(s) = serde_json::to_string(&meta) {
        std::fs::write(dir.join(format!("math-{key}.json")), s).ok();
    }
}

/// 已经渲染好的公式图片（浅色主题），导出 HTML 用。
pub fn cached_path(latex: &str, display: bool) -> Option<PathBuf> {
    let key = key_for(latex.trim(), display, false);
    let dir = webview::cache_dir();
    let png = dir.join(format!("math-{key}.png"));
    png.exists().then_some(png)
}
