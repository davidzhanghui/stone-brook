//! 离屏 WebView 服务（X2）。
//!
//! GPUI 没有浏览器内核，公式、HTML 块、网页卡片兜底截图、PDF 导出都借用系统的 WKWebView：
//! 在一个看不见的窗口里放一个 WKWebView，所有渲染请求排队复用它，截图结果按内容哈希缓存成 PNG。
//! 所有 WebKit 调用都在主线程进行（GPUI 的前台执行器就是主线程）。

use block2::RcBlock;
use gpui::{AsyncApp, BackgroundExecutor};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWindow, NSWindowStyleMask};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSObjectProtocol;
use objc2_foundation::{NSData, NSDictionary, NSError, NSNumber, NSObjectNSKeyValueCoding, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{WKNavigationDelegate, WKSnapshotConfiguration, WKWebView, WKWebViewConfiguration};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// 导航守卫（WKWebView 对 delegate 是弱引用，必须自己持有）。0 = 只允许本地，1 = 允许联网（抓网页封面用）
    static GUARDS: [Retained<NavGuard>; 2] = [NavGuard::new(false), NavGuard::new(true)];
}

struct Host {
    _window: Retained<NSWindow>,
    view: Retained<WKWebView>,
    js_view: Retained<WKWebView>,
    /// 抓取外部网页用的独立 WebView：允许顶层导航，与渲染公式 / HTML 块的视图不共享任何状态
    fetch_view: Retained<WKWebView>,
}

// ───────────── 导航守卫 ─────────────
//
// CSP 的 `default-src 'none'` 管不了顶层跳转：`<meta http-equiv="refresh">`（无脚本也生效）或 `location.href=`
// 都能把页面带到任意网址，绕过「联网」开关，也能把页面内容拼在 URL 里带出去。
// 规则：主框架只允许本地地址（file / about / data / blob / applewebdata）；用户点击的 http(s) 链接交给系统浏览器打开；
// 其余一律取消。子框架（iframe）不在这里管，由 CSP 与联网开关控制。

struct GuardIvars {
    allow_remote: bool,
}

objc2::define_class!(
    // SAFETY: NSObject 没有子类化要求；NavGuard 没有实现 Drop
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "StonebrookNavGuard"]
    #[ivars = GuardIvars]
    struct NavGuard;

    unsafe impl NSObjectProtocol for NavGuard {}

    unsafe impl WKNavigationDelegate for NavGuard {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(
            &self,
            _web_view: &WKWebView,
            action: &objc2_web_kit::WKNavigationAction,
            handler: &block2::DynBlock<dyn Fn(objc2_web_kit::WKNavigationActionPolicy)>,
        ) {
            use objc2_web_kit::{WKNavigationActionPolicy as P, WKNavigationType};
            let main = unsafe { action.targetFrame() }.is_none_or(|f| unsafe { f.isMainFrame() });
            let url = unsafe { action.request() }.URL();
            let scheme = url.as_ref().and_then(|u| u.scheme()).map(|s| s.to_string().to_lowercase()).unwrap_or_default();
            let decision = if !main || self.ivars().allow_remote || nav_allowed_locally(&scheme) {
                P::Allow
            } else {
                if unsafe { action.navigationType() } == WKNavigationType::LinkActivated && matches!(scheme.as_str(), "http" | "https" | "mailto") {
                    if let Some(u) = &url {
                        objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(u);
                    }
                }
                P::Cancel
            };
            handler.call((decision,));
        }
    }
);

impl NavGuard {
    fn new(allow_remote: bool) -> Retained<Self> {
        let this = Self::alloc(mtm()).set_ivars(GuardIvars { allow_remote });
        unsafe { objc2::msg_send![super(this), init] }
    }
}

/// 主框架可以导航到的本地地址。
fn nav_allowed_locally(scheme: &str) -> bool {
    matches!(scheme, "file" | "about" | "data" | "blob" | "applewebdata" | "")
}

/// 渲染选项。
#[derive(Clone)]
pub struct PageOptions {
    /// 视口宽度（CSS 像素）
    pub width: f64,
    /// 允许页面脚本
    pub scripts: bool,
    /// 等待页面设置 `window.__done = true`（最多等 timeout）
    pub wait_done_flag: bool,
    pub timeout: Duration,
    /// 截图最大高度
    pub max_height: f64,
}

impl Default for PageOptions {
    fn default() -> Self {
        PageOptions { width: 760., scripts: false, wait_done_flag: false, timeout: Duration::from_secs(6), max_height: 4000. }
    }
}

/// 一次截图的结果：PNG 字节 + 显示尺寸（CSS 像素）。
pub struct Snapshot {
    pub png: Vec<u8>,
    pub width: f64,
    pub height: f64,
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("WebKit 只能在主线程使用")
}

fn make_view(scripts: bool) -> Retained<WKWebView> {
    make_view_full(scripts, false, false)
}

/// autoplay：允许媒体无需用户点击直接播放（播放页里点一下即播放）
fn make_view_with(scripts: bool, autoplay: bool) -> Retained<WKWebView> {
    make_view_full(scripts, autoplay, false)
}

/// 所有 WebView 都用不落盘的数据仓库（Cookie / localStorage / 缓存不写进应用的默认存储，彼此也不共享），
/// 并挂上导航守卫。`allow_remote` 只给抓取外部网页的视图。
fn make_view_full(scripts: bool, autoplay: bool, allow_remote: bool) -> Retained<WKWebView> {
    let mtm = mtm();
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setWebsiteDataStore(&objc2_web_kit::WKWebsiteDataStore::nonPersistentDataStore(mtm));
        if autoplay {
            config.setMediaTypesRequiringUserActionForPlayback(objc2_web_kit::WKAudiovisualMediaTypes::None);
        }
        let prefs = config.preferences();
        // 隐藏的 WebView 默认会被系统降频甚至暂停，关闭这个策略才能稳定离屏渲染。
        // 该接口 macOS 14 起才有；应用声明支持 macOS 12+，旧系统上直接调用会因未知 selector 崩溃。
        if prefs.respondsToSelector(objc2::sel!(setInactiveSchedulingPolicy:)) {
            prefs.setInactiveSchedulingPolicy(objc2_web_kit::WKInactiveSchedulingPolicy::None);
        }
        let page_prefs = config.defaultWebpagePreferences();
        page_prefs.setAllowsContentJavaScript(scripts);
        let frame = CGRect::new(CGPoint::new(0., 0.), CGSize::new(800., 600.));
        let view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
        // 透明背景：公式等截图需要叠加在任意底色上
        view.setValue_forKey(Some(&NSNumber::new_bool(false)), &NSString::from_str("drawsBackground"));
        GUARDS.with(|g| view.setNavigationDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*g[allow_remote as usize]))));
        view
    }
}

fn with_host<R>(f: impl FnOnce(&Host) -> R) -> R {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        if h.is_none() {
            let mtm = mtm();
            let window = unsafe {
                let rect = CGRect::new(CGPoint::new(-30000., -30000.), CGSize::new(1600., 1200.));
                let w = NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                );
                w.setReleasedWhenClosed(false);
                w.setIgnoresMouseEvents(true);
                w.setAlphaValue(0.0);
                w
            };
            let view = make_view(false);
            let js_view = make_view(true);
            let fetch_view = make_view_full(true, false, true);
            if let Some(content) = window.contentView() {
                content.addSubview(&view);
                content.addSubview(&js_view);
                content.addSubview(&fetch_view);
            }
            window.orderFrontRegardless();
            *h = Some(Host { _window: window, view, js_view, fetch_view });
        }
        f(h.as_ref().unwrap())
    })
}

async fn sleep(ex: &BackgroundExecutor, ms: u64) {
    ex.timer(Duration::from_millis(ms)).await;
}

/// 串行化：同一时间只有一个任务使用 WebView。
async fn acquire(ex: &BackgroundExecutor) -> BusyGuard {
    while BUSY.with(|b| b.get()) {
        sleep(ex, 15).await;
    }
    BUSY.with(|b| b.set(true));
    BusyGuard
}

struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.with(|b| b.set(false));
    }
}

fn view_for(scripts: bool) -> Retained<WKWebView> {
    with_host(|h| if scripts { h.js_view.clone() } else { h.view.clone() })
}

/// 执行一段 JS，返回字符串结果（结果会被 String(...) 包装）。
pub async fn eval(view: &WKWebView, ex: &BackgroundExecutor, js: &str) -> Option<String> {
    let out: Rc<RefCell<Option<Option<String>>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let block = RcBlock::new(move |res: *mut AnyObject, _err: *mut NSError| {
        let s = unsafe { res.as_ref() }.and_then(|o| o.downcast_ref::<NSString>()).map(|s| s.to_string());
        *out2.borrow_mut() = Some(s);
    });
    let code = NSString::from_str(&format!("(function(){{ try {{ return String({js}); }} catch(e) {{ return 'ERR:' + e; }} }})()"));
    unsafe { view.evaluateJavaScript_completionHandler(&code, Some(&block)) };
    let start = Instant::now();
    loop {
        if let Some(v) = out.borrow_mut().take() {
            return v;
        }
        if start.elapsed() > Duration::from_secs(5) {
            return None;
        }
        sleep(ex, 10).await;
    }
}

fn set_frame(view: &WKWebView, w: f64, h: f64) {
    view.setFrame(CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, h)));
}

async fn wait_loaded(view: &WKWebView, ex: &BackgroundExecutor, opts: &PageOptions) {
    let start = Instant::now();
    sleep(ex, 30).await;
    while unsafe { view.isLoading() } && start.elapsed() < opts.timeout {
        sleep(ex, 20).await;
    }
    if opts.wait_done_flag {
        while start.elapsed() < opts.timeout {
            if eval(view, ex, "window.__done === true").await.as_deref() == Some("true") {
                break;
            }
            sleep(ex, 25).await;
        }
    }
}

/// 截取 rect（CSS 像素）区域，输出 2 倍分辨率 PNG。
async fn snapshot(view: &WKWebView, ex: &BackgroundExecutor, x: f64, y: f64, w: f64, h: f64) -> Option<Snapshot> {
    let out: Rc<RefCell<Option<Option<Vec<u8>>>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let block = RcBlock::new(move |img: *mut NSImage, _err: *mut NSError| {
        let png = unsafe { img.as_ref() }.and_then(|img| unsafe {
            let cg = img.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None)?;
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
            let data: Retained<NSData> = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
            Some(data.to_vec())
        });
        *out2.borrow_mut() = Some(png);
    });
    unsafe {
        let mtm = mtm();
        let config = WKSnapshotConfiguration::new(mtm);
        config.setRect(CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)));
        config.setSnapshotWidth(Some(&NSNumber::new_f64(w * 2.)));
        config.setAfterScreenUpdates(true);
        view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
    }
    let start = Instant::now();
    loop {
        if let Some(v) = out.borrow_mut().take() {
            return v.map(|png| Snapshot { png, width: w, height: h });
        }
        if start.elapsed() > Duration::from_secs(5) {
            return None;
        }
        sleep(ex, 10).await;
    }
}

/// 把一段 HTML 渲染成整页截图（高度自适应内容）。
pub async fn render_html(cx: &AsyncApp, html: &str, base_dir: Option<&Path>, opts: PageOptions) -> Option<Snapshot> {
    let ex = cx.background_executor().clone();
    let _guard = acquire(&ex).await;
    let view = view_for(opts.scripts);
    // 视口先压到很小：scrollHeight 不会小于视口高度
    set_frame(&view, opts.width, 16.);
    load_html(&view, html, base_dir);
    wait_loaded(&view, &ex, &opts).await;
    let h: f64 = eval(&view, &ex, "Math.ceil(Math.max(document.body.scrollHeight, document.body.getBoundingClientRect().bottom))")
        .await
        .and_then(|s| s.parse().ok())
        .unwrap_or(200.);
    let h = h.clamp(20., opts.max_height);
    set_frame(&view, opts.width, h);
    sleep(&ex, 60).await;
    snapshot(&view, &ex, 0., 0., opts.width, h).await
}

fn load_html(view: &WKWebView, html: &str, base_dir: Option<&Path>) {
    unsafe {
        match base_dir {
            Some(dir) => {
                // 需要读取本地资源（KaTeX 字体等）时：写成文件再以文件 URL 加载，并授予目录读权限
                let file = dir.join(format!(".render-{}.html", std::process::id()));
                std::fs::write(&file, html).ok();
                let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
                let dir_url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
                view.loadFileURL_allowingReadAccessToURL(&url, &dir_url);
            }
            None => {
                view.loadHTMLString_baseURL(&NSString::from_str(html), None);
            }
        }
    }
}

/// 加载 HTML 后，按若干元素的位置逐个截图（批量渲染公式用）。
pub async fn render_elements(
    cx: &AsyncApp,
    html: &str,
    base_dir: &Path,
    count: usize,
    width: f64,
    extra_js: Option<&str>,
) -> (Vec<Option<Snapshot>>, Option<String>) {
    let ex = cx.background_executor().clone();
    let _guard = acquire(&ex).await;
    let view = view_for(true);
    set_frame(&view, width, 600.);
    load_html(&view, html, Some(base_dir));
    let opts = PageOptions { scripts: true, wait_done_flag: true, ..Default::default() };
    wait_loaded(&view, &ex, &opts).await;
    let h: f64 = eval(&view, &ex, "Math.ceil(document.documentElement.scrollHeight)").await.and_then(|s| s.parse().ok()).unwrap_or(600.);
    set_frame(&view, width, h.max(100.));
    sleep(&ex, 40).await;
    let rects = eval(&view, &ex, "JSON.stringify(window.__rects())").await.unwrap_or_default();
    let rects: Vec<Option<[f64; 4]>> = serde_json::from_str(&rects).unwrap_or_default();
    let extra = match extra_js {
        Some(js) => eval(&view, &ex, js).await,
        None => None,
    };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        match rects.get(i).copied().flatten() {
            Some([x, y, w, h]) if w > 0. && h > 0. => out.push(snapshot(&view, &ex, x, y, w, h).await),
            _ => out.push(None),
        }
    }
    (out, extra)
}

/// 加载一个网页（允许脚本与网络），截取首屏作为封面。用于网页卡片在没有 og:image 时兜底。
pub async fn snapshot_url(cx: &AsyncApp, url: &str) -> Option<Snapshot> {
    let ex = cx.background_executor().clone();
    let _guard = acquire(&ex).await;
    let view = with_host(|h| h.fetch_view.clone());
    set_frame(&view, 1200., 630.);
    unsafe {
        let u = NSURL::URLWithString(&NSString::from_str(url))?;
        view.loadRequest(&NSURLRequest::requestWithURL(&u));
    }
    let opts = PageOptions { timeout: Duration::from_secs(10), ..Default::default() };
    wait_loaded(&view, &ex, &opts).await;
    sleep(&ex, 1200).await;
    let snap = snapshot(&view, &ex, 0., 0., 1200., 630.).await;
    unsafe {
        view.loadHTMLString_baseURL(&NSString::from_str("<html></html>"), None);
    }
    snap
}

/// 把 HTML 文件导出为 PDF（单页长图式 PDF，保留全部样式）。
pub async fn html_to_pdf(cx: &AsyncApp, html: &str, base_dir: &Path, out: &Path) -> bool {
    let ex = cx.background_executor().clone();
    let _guard = acquire(&ex).await;
    let view = view_for(false);
    set_frame(&view, 820., 1000.);
    load_html(&view, html, Some(base_dir));
    wait_loaded(&view, &ex, &PageOptions::default()).await;
    let h: f64 = eval(&view, &ex, "Math.ceil(document.documentElement.scrollHeight)").await.and_then(|s| s.parse().ok()).unwrap_or(1000.);
    set_frame(&view, 820., h);
    sleep(&ex, 80).await;
    let result: Rc<RefCell<Option<Option<Vec<u8>>>>> = Rc::new(RefCell::new(None));
    let r2 = result.clone();
    let block = RcBlock::new(move |data: *mut NSData, _err: *mut NSError| {
        *r2.borrow_mut() = Some(unsafe { data.as_ref() }.map(|d| d.to_vec()));
    });
    unsafe { view.createPDFWithConfiguration_completionHandler(None, &block) };
    let start = Instant::now();
    loop {
        if let Some(v) = result.borrow_mut().take() {
            return v.is_some_and(|bytes| std::fs::write(out, bytes).is_ok());
        }
        if start.elapsed() > Duration::from_secs(15) {
            return false;
        }
        sleep(&ex, 20).await;
    }
}

/// 渲染结果的磁盘缓存目录。
pub fn cache_dir() -> PathBuf {
    let dir = crate::store::data_dir().join("cache");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn hash_key(parts: &[&str]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

// ───────────────────────────── 实时 WebView（HTML 块交互模式）─────────────────────────────

thread_local! {
    static LIVE: RefCell<Option<(u64, Retained<WKWebView>, Retained<objc2_app_kit::NSView>)>> = const { RefCell::new(None) };
}

/// 实时 WebView 的内容来源。
#[derive(Clone)]
pub enum LiveSrc {
    /// 手写源码（套沙箱外壳）
    Source { page: String, scripts: bool },
    /// 上传的整页 HTML 文件；network=false 时注入 CSP 禁止外部请求
    File { path: PathBuf, network: bool },
    /// 本地生成的播放页（webm 等）：以文件 URL 加载，允许读取 root 目录（页面与媒体文件的共同上级）
    Media { page: PathBuf, root: PathBuf },
    /// 系统 Quick Look 预览（Office / iWork 等）：原生 QLPreviewView，可滚动翻页
    QuickLook { path: PathBuf },
}

impl LiveSrc {
    fn scripts(&self) -> bool {
        match self {
            LiveSrc::Source { scripts, .. } => *scripts,
            _ => true,
        }
    }
}

/// 在 GPUI 窗口上叠加一个真实的 WKWebView。bounds 为 GPUI 窗口坐标（左上原点）。
pub fn show_live(window: &gpui::Window, block_id: u64, src: &LiveSrc, bounds: gpui::Bounds<gpui::Pixels>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    if block_id & crate::media::EXTERNAL != 0 {
        EXT_SEEN.with(|s| s.set(true));
    }
    let ns_view: &objc2_app_kit::NSView = unsafe { &*(h.ns_view.as_ptr() as *const objc2_app_kit::NSView) };
    let view_h = ns_view.bounds().size.height;
    let frame = CGRect::new(
        CGPoint::new(f64::from(bounds.origin.x), view_h - f64::from(bounds.origin.y) - f64::from(bounds.size.height)),
        CGSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
    );
    if let LiveSrc::QuickLook { path } = src {
        hide_web();
        show_ql(ns_view, block_id, path, frame);
        return;
    }
    hide_ql();
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let reuse = l.as_ref().is_some_and(|(id, _, _)| *id == block_id);
        if !reuse {
            if let Some((_, old, _)) = l.take() {
                old.removeFromSuperview();
            }
            let v = make_view_with(src.scripts(), matches!(src, LiveSrc::Media { .. }));
            // 文件预览底下垫着首屏快照：透明的 WebView 滚动后快照会透出来与内容叠在一起，必须不透明
            if !matches!(src, LiveSrc::Source { .. }) {
                unsafe { v.setValue_forKey(Some(&NSNumber::new_bool(true)), &NSString::from_str("drawsBackground")) };
            }
            match src {
                LiveSrc::Source { page, .. } => unsafe {
                    v.loadHTMLString_baseURL(&NSString::from_str(page), None);
                },
                LiveSrc::File { path, network } => load_file(&v, path, *network),
                LiveSrc::Media { page, root } => unsafe {
                    let url = NSURL::fileURLWithPath(&NSString::from_str(&page.to_string_lossy()));
                    let root = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&root.to_string_lossy()), true);
                    v.loadFileURL_allowingReadAccessToURL(&url, &root);
                },
                LiveSrc::QuickLook { .. } => {}
            }
            ns_view.addSubview(&v);
            *l = Some((block_id, v, unsafe { Retained::retain(ns_view as *const _ as *mut objc2_app_kit::NSView) }.expect("ns_view")));
        }
        if let Some((_, v, _)) = l.as_ref() {
            v.setFrame(frame);
        }
    });
}

/// 加载上传的 HTML 文件。联网：以文件 URL 加载（相对路径资源可读）；不联网：读入内容并注入 CSP。
fn load_file(view: &WKWebView, path: &Path, network: bool) {
    let dir = path.parent().unwrap_or(Path::new("/"));
    unsafe {
        if network {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let dir_url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
            view.loadFileURL_allowingReadAccessToURL(&url, &dir_url);
        } else {
            let html = String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).to_string();
            let csp = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: file:; style-src 'unsafe-inline'; font-src data:; script-src 'unsafe-inline';\">";
            let html = match html.to_lowercase().find("<head").and_then(|i| html[i..].find('>').map(|j| i + j + 1)) {
                Some(at) => format!("{}{csp}{}", &html[..at], &html[at..]),
                None => format!("{csp}{html}"),
            };
            let base = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&dir.to_string_lossy()), true);
            view.loadHTMLString_baseURL(&NSString::from_str(&html), Some(&base));
        }
    }
}

pub fn live_block() -> Option<u64> {
    LIVE.with(|l| l.borrow().as_ref().map(|(id, _, _)| *id)).or_else(|| QL.with(|q| q.borrow().as_ref().map(|(id, ..)| *id)))
}

pub fn hide_live() {
    hide_web();
    hide_ql();
}

thread_local! {
    /// 文档以外（id 带 media::EXTERNAL）的界面上次 show_live 以来是否还在显示
    static EXT_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 文档每帧收起自己不用的原生视图；电子表格附件预览等外部界面持有的，只在它不再显示时才收起。
pub fn hide_live_unless_external() {
    if live_block().is_some_and(|id| id & crate::media::EXTERNAL != 0) && EXT_SEEN.with(|s| s.replace(false)) {
        return;
    }
    hide_live();
}

// ───────────────────────────── Quick Look 实时预览 ─────────────────────────────

#[link(name = "Quartz", kind = "framework")]
extern "C" {}

thread_local! {
    /// (块 id, 文件, QLPreviewView, 父视图)
    static QL: RefCell<Option<(u64, PathBuf, Retained<objc2_app_kit::NSView>, Retained<objc2_app_kit::NSView>)>> = const { RefCell::new(None) };
}

/// 在 GPUI 窗口上叠加系统的 QLPreviewView（和访达里按空格看到的一样，Office / iWork 可滚动翻页）。
fn show_ql(parent: &objc2_app_kit::NSView, block_id: u64, path: &Path, frame: CGRect) {
    QL.with(|q| {
        let mut q = q.borrow_mut();
        let reuse = q.as_ref().is_some_and(|(id, p, ..)| *id == block_id && p == path);
        if !reuse {
            if let Some((_, _, old, _)) = q.take() {
                close_ql(&old);
            }
            let Some(cls) = objc2::runtime::AnyClass::get(c"QLPreviewView") else { return };
            let v: Retained<objc2_app_kit::NSView> = unsafe {
                let obj: objc2::rc::Allocated<objc2_app_kit::NSView> = objc2::msg_send![cls, alloc];
                // QLPreviewViewStyleNormal = 0
                objc2::msg_send![obj, initWithFrame: frame, style: 0usize]
            };
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            unsafe {
                let _: () = objc2::msg_send![&*v, setPreviewItem: &*url];
            }
            parent.addSubview(&v);
            *q = Some((block_id, path.to_path_buf(), v, unsafe { Retained::retain(parent as *const _ as *mut objc2_app_kit::NSView) }.expect("ns_view")));
        }
        if let Some((_, _, v, _)) = q.as_ref() {
            v.setFrame(frame);
        }
    });
}

fn close_ql(v: &objc2_app_kit::NSView) {
    unsafe {
        let _: () = objc2::msg_send![v, close];
    }
    v.removeFromSuperview();
}

fn hide_ql() {
    QL.with(|q| {
        if let Some((_, _, v, parent)) = q.borrow_mut().take() {
            close_ql(&v);
            if let Some(win) = parent.window() {
                let _ = win.makeFirstResponder(Some(&parent));
            }
        }
    });
}

fn hide_web() {
    LIVE.with(|l| {
        if let Some((_, v, parent)) = l.borrow_mut().take() {
            v.removeFromSuperview();
            // WebView 持有过键盘焦点（输入法也跟着它）：移除后把焦点交还给 GPUI 的内容视图
            if let Some(win) = parent.window() {
                let _ = win.makeFirstResponder(Some(&parent));
            }
        }
    });
}

/// 给页面加上内容安全策略：禁止一切外部请求（图片仅允许 data: 与本地文件）。
pub fn sandbox_html(body: &str, scripts: bool, dark: bool) -> String {
    let script_src = if scripts { "'unsafe-inline'" } else { "'none'" };
    let (fg, bg) = if dark { ("#e3e4e6", "transparent") } else { ("#1f2329", "transparent") };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: file:; style-src 'unsafe-inline'; font-src data:; script-src {script_src};\">\
<style>html,body{{margin:0;padding:0;background:{bg};color:{fg};font:15px/1.6 -apple-system,'PingFang SC',sans-serif;}}body{{padding:2px}}</style>\
</head><body>{body}</body></html>"
    )
}

// AnyThread 仅用于 NSBitmapImageRep::alloc
#[allow(unused_imports)]
use AnyThread as _;
#[allow(unused_imports)]
use MainThreadOnly as _;

// ───────────────────────────── HTML 快照缓存 ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum SnapState {
    Pending,
    Ready { path: PathBuf, width: f32, height: f32 },
    Failed,
}

thread_local! {
    static SNAPS: RefCell<crate::lru::Lru<String, SnapState>> = RefCell::new(crate::lru::Lru::new(512));
}

/// HTML 块快照：已缓存直接返回，否则排队渲染（完成后刷新窗口）。
pub fn request_html(source: &str, scripts: bool, dark: bool, width: f32, cx: &mut gpui::App) -> SnapState {
    let key = hash_key(&[source, if scripts { "js" } else { "nojs" }, if dark { "dark" } else { "light" }, &format!("{width:.0}")]);
    if let Some(s) = SNAPS.with(|m| m.borrow().get(&key).cloned()) {
        return s;
    }
    let dir = cache_dir();
    let png = dir.join(format!("html-{key}.png"));
    let meta = dir.join(format!("html-{key}.json"));
    if let Some((w, h)) = std::fs::read_to_string(&meta).ok().and_then(|s| serde_json::from_str::<(f32, f32)>(&s).ok()) {
        let st = SnapState::Ready { path: png, width: w, height: h };
        SNAPS.with(|m| m.borrow_mut().insert(key, st.clone()));
        return st;
    }
    SNAPS.with(|m| m.borrow_mut().insert(key.clone(), SnapState::Pending));
    let html = sandbox_html(source, scripts, dark);
    cx.spawn(async move |cx: &mut AsyncApp| {
        let opts = PageOptions { width: width as f64, scripts, ..Default::default() };
        let st = match render_html(cx, &html, None, opts).await {
            Some(snap) if std::fs::write(&png, &snap.png).is_ok() => {
                std::fs::write(&meta, serde_json::to_string(&(snap.width as f32, snap.height as f32)).unwrap_or_default()).ok();
                SnapState::Ready { path: png, width: snap.width as f32, height: snap.height as f32 }
            }
            _ => SnapState::Failed,
        };
        SNAPS.with(|m| m.borrow_mut().insert(key, st));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    SnapState::Pending
}

/// 上传 HTML 文件的快照（整页首屏，最高 max_h）：已缓存直接返回，否则排队渲染。
pub fn request_file(path: &Path, network: bool, width: f32, max_h: f32, cx: &mut gpui::App) -> SnapState {
    let stamp = std::fs::metadata(path).ok().map(|m| format!("{}-{:?}", m.len(), m.modified().ok())).unwrap_or_default();
    let key = hash_key(&[&path.to_string_lossy(), &stamp, if network { "net" } else { "nonet" }, &format!("{width:.0}x{max_h:.0}")]);
    if let Some(s) = SNAPS.with(|m| m.borrow().get(&key).cloned()) {
        return s;
    }
    let dir = cache_dir();
    let png = dir.join(format!("htmlfile-{key}.png"));
    let meta = dir.join(format!("htmlfile-{key}.json"));
    if let Some((w, h)) = std::fs::read_to_string(&meta).ok().and_then(|s| serde_json::from_str::<(f32, f32)>(&s).ok()) {
        if png.exists() {
            let st = SnapState::Ready { path: png, width: w, height: h };
            SNAPS.with(|m| m.borrow_mut().insert(key, st.clone()));
            return st;
        }
    }
    SNAPS.with(|m| m.borrow_mut().insert(key.clone(), SnapState::Pending));
    let path = path.to_path_buf();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let st = match render_file(cx, &path, network, width as f64, max_h as f64).await {
            Some(snap) if std::fs::write(&png, &snap.png).is_ok() => {
                std::fs::write(&meta, serde_json::to_string(&(snap.width as f32, snap.height as f32)).unwrap_or_default()).ok();
                SnapState::Ready { path: png, width: snap.width as f32, height: snap.height as f32 }
            }
            _ => SnapState::Failed,
        };
        SNAPS.with(|m| m.borrow_mut().insert(key, st));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    SnapState::Pending
}

async fn render_file(cx: &AsyncApp, path: &Path, network: bool, width: f64, max_h: f64) -> Option<Snapshot> {
    let ex = cx.background_executor().clone();
    let _guard = acquire(&ex).await;
    let view = view_for(true);
    set_frame(&view, width, max_h);
    load_file(&view, path, network);
    let opts = PageOptions { width, scripts: true, timeout: Duration::from_secs(10), ..Default::default() };
    wait_loaded(&view, &ex, &opts).await;
    // 等字体 / 图片就绪（最多 4 秒），再留一小段时间给入场动画
    let ready = "document.readyState==='complete' && (!document.fonts || document.fonts.status==='loaded') && Array.from(document.images).every(i=>i.complete)";
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(if network { 4 } else { 1 }) {
        if eval(&view, &ex, ready).await.as_deref() == Some("true") {
            break;
        }
        sleep(&ex, 60).await;
    }
    sleep(&ex, 150).await;
    // 先把视口压矮再量：scrollHeight 至少是视口高度，按 max_h 的视口量会把短文档也截成 max_h 高
    set_frame(&view, width, 60.);
    sleep(&ex, 60).await;
    let full: f64 = eval(&view, &ex, "Math.ceil(document.documentElement.scrollHeight)").await.and_then(|s| s.parse().ok()).unwrap_or(max_h);
    // 至少保留一屏（按视口高度排版的页面压矮后会量得很小）
    let h = full.max(max_h.min(460.)).clamp(60., max_h);
    set_frame(&view, width, h);
    sleep(&ex, 80).await;
    let snap = snapshot(&view, &ex, 0., 0., width, h).await;
    unsafe {
        view.loadHTMLString_baseURL(&NSString::from_str("<html></html>"), None);
    }
    snap
}
