//! 网页卡片（P7-1）：抓取一次，永久离线缓存。

use gpui::{App, AsyncApp};
use std::cell::RefCell;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub desc: String,
    pub image: String,
    pub favicon: String,
    pub site: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CardState {
    Loading,
    Ready(Card),
    /// 抓取失败或未联网：显示为书签卡片
    Offline,
}

thread_local! {
    static CARDS: RefCell<crate::lru::Lru<String, CardState>> = RefCell::new(crate::lru::Lru::new(512));
}

fn store() -> Option<&'static crate::store::Store> {
    crate::store::global()
}

pub fn domain(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => u.host_str().unwrap_or("").trim_start_matches("www.").to_string(),
        Err(_) => {
            let s = url.split("://").nth(1).unwrap_or(url);
            s.split('/').next().unwrap_or(s).trim_start_matches("www.").to_string()
        }
    }
}

fn from_db(url: &str) -> Option<CardState> {
    let r = store()?.load_link_card(url)?;
    Some(if r.ok { CardState::Ready(Card { title: r.title, desc: r.descr, image: r.image, favicon: r.favicon, site: r.site }) } else { CardState::Offline })
}

/// 查询卡片状态；未缓存且允许联网时开始抓取。
pub fn request(url: &str, cx: &mut App) -> CardState {
    if let Some(s) = CARDS.with(|c| c.borrow().get(url).cloned()) {
        return s;
    }
    if let Some(s) = from_db(url) {
        CARDS.with(|c| c.borrow_mut().insert(url.to_string(), s.clone()));
        return s;
    }
    if !crate::settings::network() {
        return CardState::Offline;
    }
    start_fetch(url, cx);
    CardState::Loading
}

pub fn refresh(url: &str, cx: &mut App) {
    if crate::settings::network() {
        start_fetch(url, cx);
    }
}

pub fn cached_text(url: &str) -> (String, String) {
    match CARDS.with(|c| c.borrow().get(url).cloned()).or_else(|| from_db(url)) {
        Some(CardState::Ready(c)) => (c.title, c.desc),
        _ => (domain(url), String::new()),
    }
}

fn start_fetch(url: &str, cx: &mut App) {
    CARDS.with(|c| c.borrow_mut().insert(url.to_string(), CardState::Loading));
    let url = url.to_string();
    let assets = crate::store::data_dir().join("assets");
    let task = cx.background_executor().spawn({
        let url = url.clone();
        async move { fetch(&url, &assets) }
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut card = task.await;
        // 没有封面图：用离屏 WebView 打开网页截取首屏
        if let Some(c) = card.as_mut() {
            if c.image.is_empty() {
                if let Some(snap) = crate::webview::snapshot_url(cx, &url).await {
                    let path = crate::store::data_dir().join("assets").join(format!("card-{}.png", crate::webview::hash_key(&[&url])));
                    if std::fs::write(&path, &snap.png).is_ok() {
                        c.image = path.to_string_lossy().to_string();
                    }
                }
            }
        }
        let state = match &card {
            Some(c) => CardState::Ready(c.clone()),
            None => CardState::Offline,
        };
        if let Some(s) = store() {
            let row = match &card {
                Some(c) => crate::store::LinkCardRow {
                    title: c.title.clone(),
                    descr: c.desc.clone(),
                    image: c.image.clone(),
                    favicon: c.favicon.clone(),
                    site: c.site.clone(),
                    ok: true,
                },
                None => Default::default(),
            };
            s.save_link_card(&url, &row);
        }
        CARDS.with(|c| c.borrow_mut().insert(url.clone(), state));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
}

/// 代理：优先环境变量（终端启动），否则读 macOS 系统代理（从访达启动时没有环境变量）。
fn proxy() -> Option<ureq::Proxy> {
    static P: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let url = P.get_or_init(|| {
        for k in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"] {
            if let Ok(v) = std::env::var(k) {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        let out = std::process::Command::new("/usr/sbin/scutil").arg("--proxy").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let get = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).and_then(|r| r.trim().strip_prefix(':')).map(|v| v.trim().to_string()));
        if get("HTTPSEnable").as_deref() == Some("1") {
            return Some(format!("http://{}:{}", get("HTTPSProxy")?, get("HTTPSPort")?));
        }
        if get("SOCKSEnable").as_deref() == Some("1") {
            return Some(format!("socks5://{}:{}", get("SOCKSProxy")?, get("SOCKSPort")?));
        }
        None
    });
    url.as_deref().and_then(|u| ureq::Proxy::new(u).ok())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(proxy())
        .timeout_global(Some(Duration::from_secs(8)))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .build()
        .new_agent()
}

/// 大文件下载用的连接（应用更新）：只限制连接与首包超时，不限制总时长；走与网页卡片相同的代理设置。
pub(crate) fn agent_with_timeout(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(proxy())
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(secs)))
        .user_agent(concat!("Stonebrook/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn get_bytes(url: &str, limit: u64) -> Option<(Vec<u8>, String)> {
    let mut resp = agent().get(url).call().ok()?;
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let bytes = resp.body_mut().with_config().limit(limit).read_to_vec().ok()?;
    Some((bytes, ctype))
}

fn decode_html(bytes: &[u8], ctype: &str) -> String {
    // 先看 HTTP 头，再看 <meta charset>，中文网站常见 GBK
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase();
    let label = ctype
        .split("charset=")
        .nth(1)
        .map(|s| s.trim().to_string())
        .or_else(|| head.split("charset=").nth(1).map(|s| s.trim_start_matches(['"', '\'']).chars().take_while(|c| c.is_alphanumeric() || *c == '-').collect()))
        .unwrap_or_else(|| "utf-8".into());
    let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    enc.decode(bytes).0.into_owned()
}

fn absolutize(base: &str, href: &str) -> String {
    url::Url::parse(base).and_then(|b| b.join(href)).map(|u| u.to_string()).unwrap_or_else(|_| href.to_string())
}

/// 是否是可以访问的公网 http(s) 地址。用于页面里「间接」引用的资源（粘贴内容中的远程图片、网页的封面与图标）：
/// 这些地址不是用户自己输入的，不能让一段粘贴内容驱使应用去访问回环 / 内网 / 链路本地地址。
/// 域名会先解析，任何一个解析结果落在非公网段都拒绝。
pub fn is_public_http(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else { return false };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let port = url.port_or_known_default().unwrap_or(80);
    match url.host() {
        Some(url::Host::Ipv4(ip)) => public_ip(std::net::IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => public_ip(std::net::IpAddr::V6(ip)),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local") || d.ends_with(".internal") {
                return false;
            }
            use std::net::ToSocketAddrs;
            match (d.as_str(), port).to_socket_addrs() {
                Ok(addrs) => {
                    let addrs: Vec<_> = addrs.collect();
                    !addrs.is_empty() && addrs.iter().all(|a| public_ip(a.ip()))
                }
                Err(_) => false,
            }
        }
        None => false,
    }
}

fn public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64/10 运营商 NAT
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18/15 基准测试
                || o[0] >= 224)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = v.segments();
            !(v.is_loopback() || v.is_unspecified() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || (s[0] & 0xff00) == 0xff00)
        }
    }
}

/// 只访问公网地址的下载：重定向逐跳检查（不交给 HTTP 库自动跟随，否则会被 302 带到内网）。
fn get_public_bytes(url: &str, limit: u64) -> Option<(Vec<u8>, String)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(proxy())
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .build()
        .new_agent();
    let mut cur = url.to_string();
    for _ in 0..5 {
        if !is_public_http(&cur) {
            return None;
        }
        let mut resp = agent.get(&cur).call().ok()?;
        if resp.status().is_redirection() {
            let loc = resp.headers().get("location").and_then(|v| v.to_str().ok())?.to_string();
            cur = absolutize(&cur, &loc);
            continue;
        }
        let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
        let bytes = resp.body_mut().with_config().limit(limit).read_to_vec().ok()?;
        return Some((bytes, ctype));
    }
    None
}

fn download(url: &str, dir: &std::path::Path, prefix: &str) -> String {
    let Some((bytes, ctype)) = get_public_bytes(url, 5 * 1024 * 1024) else { return String::new() };
    if bytes.len() < 64 {
        return String::new();
    }
    let ext = if ctype.contains("png") {
        "png"
    } else if ctype.contains("svg") {
        "svg"
    } else if ctype.contains("icon") {
        "ico"
    } else if ctype.contains("gif") {
        "gif"
    } else if ctype.contains("webp") {
        "webp"
    } else {
        "jpg"
    };
    // GPUI 不认识 ico / svg / webp：统一转成 PNG
    let path = dir.join(format!("{prefix}-{}.png", crate::webview::hash_key(&[url])));
    let png = match ext {
        "png" | "jpg" | "gif" => image::load_from_memory(&bytes).ok(),
        _ => image::load_from_memory(&bytes).ok(),
    };
    match png {
        Some(img) => {
            if img.save_with_format(&path, image::ImageFormat::Png).is_ok() {
                path.to_string_lossy().to_string()
            } else {
                String::new()
            }
        }
        None => String::new(),
    }
}

/// 在后台线程执行：抓取网页并解析元数据。
pub fn fetch(url: &str, assets: &std::path::Path) -> Option<Card> {
    let (bytes, ctype) = get_bytes(url, 2 * 1024 * 1024)?;
    if !ctype.is_empty() && !ctype.contains("html") {
        // 直接是图片 / PDF 等资源
        return Some(Card {
            title: url.rsplit('/').next().unwrap_or(url).to_string(),
            desc: ctype,
            image: String::new(),
            favicon: String::new(),
            site: domain(url),
        });
    }
    let html = decode_html(&bytes, &ctype);
    let doc = scraper::Html::parse_document(&html);
    let meta = |names: &[&str]| -> String {
        for n in names {
            for attr in ["property", "name", "itemprop"] {
                let Ok(sel) = scraper::Selector::parse(&format!("meta[{attr}=\"{n}\"]")) else { continue };
                if let Some(c) = doc.select(&sel).next().and_then(|e| e.value().attr("content")) {
                    let c = c.trim();
                    if !c.is_empty() {
                        return c.to_string();
                    }
                }
            }
        }
        String::new()
    };
    let mut title = meta(&["og:title", "twitter:title"]);
    if title.is_empty() {
        let sel = scraper::Selector::parse("title").unwrap();
        title = doc.select(&sel).next().map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
    }
    let desc = meta(&["og:description", "twitter:description", "description"]);
    let site = {
        let s = meta(&["og:site_name", "application-name"]);
        if s.is_empty() {
            domain(url)
        } else {
            s
        }
    };
    let image_url = meta(&["og:image", "og:image:url", "twitter:image", "twitter:image:src", "image"]);
    let icon_sel = scraper::Selector::parse("link[rel~=\"icon\"], link[rel=\"apple-touch-icon\"], link[rel=\"shortcut icon\"]").unwrap();
    let icon_href =
        doc.select(&icon_sel).filter_map(|e| e.value().attr("href")).next_back().map(|h| absolutize(url, h)).unwrap_or_else(|| absolutize(url, "/favicon.ico"));
    let image = if image_url.is_empty() { String::new() } else { download(&absolutize(url, &image_url), assets, "cover") };
    let favicon = download(&icon_href, assets, "icon");
    Some(Card { title: if title.is_empty() { domain(url) } else { title }, desc: desc.chars().take(240).collect(), image, favicon, site })
}

/// 下载网络图片到本地（粘贴 HTML 中的远程图片）。
pub fn download_image(url: &str, assets: &std::path::Path) -> Option<String> {
    let p = download(url, assets, "img");
    (!p.is_empty()).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::{absolutize, is_public_http};

    #[test]
    fn private_addresses_are_rejected() {
        for u in [
            "http://127.0.0.1/a.png",
            "http://localhost:8080/",
            "http://10.0.0.5/x",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "file:///etc/passwd",
            "ftp://example.com/",
        ] {
            assert!(!is_public_http(u), "{u}");
        }
        assert!(is_public_http("http://93.184.216.34/x.png"));
    }

    #[test]
    fn absolutize_handles_relative_forms() {
        assert_eq!(absolutize("https://a.com/x/y.html", "/i.png"), "https://a.com/i.png");
        assert_eq!(absolutize("https://a.com/x/y.html", "i.png"), "https://a.com/x/i.png");
        assert_eq!(absolutize("https://a.com/x/", "//cdn.b.com/i.png"), "https://cdn.b.com/i.png");
    }
}
