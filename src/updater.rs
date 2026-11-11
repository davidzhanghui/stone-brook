//! 应用内更新：查 GitHub Releases 的最新版本 → 后台下载本机架构的 `.app.zip` → 校验 SHA-256 → 解压待装；
//! 用户点「重启并安装」（或正常退出时）由一个脱离本进程的小脚本在本进程退出后替换 `.app` 并（可选）重新打开。
//!
//! - 所有网络访问受「允许联网」开关控制；自动检查每天最多一次（`kv: update_checked`），可在设置里关闭（`kv: auto_update`）。
//! - 发布产物由 `.github/workflows/release.yml` 生成：`Stonebrook-<版本>-<arm64|x86_64>.app.zip` 与 `SHA256SUMS.txt`。
//!   找不到校验文件或校验不一致时拒绝安装。
//! - 只有从 `.app` 包启动时才能自动安装（`cargo run` 开发模式下只能下载，然后在访达里手动替换）。
//! - 替换前旧版本移到 `<数据目录>/updates/previous.app`，新版本装不上时脚本会把旧版本放回去。

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const REPO: &str = "davidzhanghui/stone-brook";
const CHECK_INTERVAL_MS: i64 = 24 * 3600 * 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub zip_name: String,
    pub zip_url: String,
    pub sums_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    /// 已下载并解压好，等待安装
    Ready {
        release: Release,
        app: PathBuf,
    },
    Failed(String),
}

static STATE: Mutex<State> = Mutex::new(State::Idle);
/// 状态变化后置位，界面层轮询到就重绘
static CHANGED: AtomicBool = AtomicBool::new(false);
static GOT: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn state() -> State {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(s: State) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = s;
    CHANGED.store(true, Ordering::SeqCst);
}

/// 取走「状态变了」标志。
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::SeqCst)
}

/// 下载进度 (已下载, 总大小)。
pub fn progress() -> (u64, u64) {
    (GOT.load(Ordering::Relaxed), TOTAL.load(Ordering::Relaxed))
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 发布产物里的架构名。
fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    }
}

/// 版本比较：按点分数字逐段比（`v` 前缀、预发布后缀忽略）。
pub fn newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parse(remote), parse(local));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// 从 GitHub API 的 release JSON 里取出本机要用的产物。
fn parse_release(v: &serde_json::Value) -> Result<Release> {
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("发布信息缺少版本号"))?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets
            .iter()
            .find(|a| a["name"].as_str().is_some_and(pred))
            .map(|a| (a["name"].as_str().unwrap_or("").to_string(), a["browser_download_url"].as_str().unwrap_or("").to_string()))
    };
    let suffix = format!("-{}.app.zip", arch());
    let (zip_name, zip_url) = find(&|n| n.ends_with(&suffix)).ok_or_else(|| anyhow!("新版本 {version} 没有本机（{}）的安装包", arch()))?;
    let sums_url = find(&|n| n == "SHA256SUMS.txt").map(|x| x.1);
    Ok(Release {
        version,
        notes: v["body"].as_str().unwrap_or("").to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        zip_name,
        zip_url,
        sums_url,
    })
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    crate::linkcard::agent_with_timeout(30).get(url).header("Accept", "application/vnd.github+json").call().map_err(|e| anyhow!("网络请求失败：{e}"))
}

/// 查询最新版本（阻塞，后台线程调用）。有更新返回 Some。
pub fn check() -> Result<Option<Release>> {
    if !crate::settings::network() {
        bail!("已关闭联网，无法检查更新");
    }
    let mut resp = get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let body = resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&body).context("发布信息格式不对")?;
    let rel = parse_release(&v)?;
    Ok(newer(&rel.version, current_version()).then_some(rel))
}

fn updates_dir() -> PathBuf {
    crate::store::data_dir().join("updates")
}

/// 在 `SHA256SUMS.txt` 里找某个文件的哈希。
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (h, n) = (it.next()?, it.next()?);
        (n.trim_start_matches('*') == name).then(|| h.to_lowercase())
    })
}

fn sha256_file(p: &Path) -> Result<String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(p)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// 下载、校验并解压（阻塞，后台线程调用）。返回解压出的 `.app` 路径。
pub fn download(rel: &Release) -> Result<PathBuf> {
    if !crate::settings::network() {
        bail!("已关闭联网，无法下载更新");
    }
    let sums_url = rel.sums_url.as_ref().ok_or_else(|| anyhow!("发布里没有校验文件 SHA256SUMS.txt，为安全起见不安装"))?;
    let mut sums = get(sums_url)?;
    let sums = sums.body_mut().with_config().limit(1024 * 1024).read_to_string()?;
    let want = expected_sum(&sums, &rel.zip_name).ok_or_else(|| anyhow!("校验文件里没有 {}", rel.zip_name))?;

    let dir = updates_dir().join(&rel.version);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join(&rel.zip_name);
    let mut resp = get(&rel.zip_url)?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok()).unwrap_or(0);
    GOT.store(0, Ordering::Relaxed);
    TOTAL.store(total, Ordering::Relaxed);
    {
        let mut reader = resp.body_mut().with_config().limit(1024 * 1024 * 1024).reader();
        let mut out = std::fs::File::create(&zip)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out, &buf[..n])?;
            GOT.fetch_add(n as u64, Ordering::Relaxed);
            CHANGED.store(true, Ordering::SeqCst);
        }
    }
    let got = sha256_file(&zip)?;
    if got != want {
        let _ = std::fs::remove_dir_all(&dir);
        bail!("安装包校验失败（SHA-256 不一致），已丢弃");
    }
    let unpacked = dir.join("app");
    let st = std::process::Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&unpacked).status()?;
    if !st.success() {
        bail!("解压安装包失败");
    }
    let app = std::fs::read_dir(&unpacked)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| anyhow!("安装包里没有 .app"))?;
    let _ = std::fs::remove_file(&zip);
    Ok(app)
}

/// 当前运行的 `.app` 包路径（开发模式下不是从包里启动，返回 None）。
pub fn current_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    // <名字>.app/Contents/MacOS/<可执行文件>
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension().is_some_and(|e| e == "app") && exe.parent()?.ends_with("Contents/MacOS")).then(|| bundle.to_path_buf())
}

const INSTALL_SH: &str = r#"
pid="$1"; new="$2"; dst="$3"; bak="$4"; relaunch="$5"
i=0
while kill -0 "$pid" 2>/dev/null; do
  sleep 0.2; i=$((i+1)); [ "$i" -gt 300 ] && exit 1
done
rm -rf "$bak"
mv "$dst" "$bak" || exit 1
if mv "$new" "$dst"; then
  xattr -cr "$dst" 2>/dev/null
else
  mv "$bak" "$dst"
fi
[ "$relaunch" = "1" ] && open "$dst"
exit 0
"#;

/// 安排安装：启动一个脱离本进程的脚本，等本进程退出后替换 `.app`。调用方随后应正常退出应用。
pub fn schedule_install(app: &Path, relaunch: bool) -> Result<()> {
    let dst = current_bundle().ok_or_else(|| anyhow!("当前不是从 .app 启动（开发模式），请在访达里手动替换：{}", app.display()))?;
    let parent = dst.parent().ok_or_else(|| anyhow!("找不到应用所在目录"))?;
    let probe = parent.join(format!(".stonebrook-update-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|_| anyhow!("没有写入 {} 的权限，请手动安装：{}", parent.display(), app.display()))?;
    let _ = std::fs::remove_file(&probe);
    let bak = updates_dir().join("previous.app");
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(INSTALL_SH)
        .arg("stonebrook-update")
        .arg(std::process::id().to_string())
        .arg(app)
        .arg(&dst)
        .arg(&bak)
        .arg(if relaunch { "1" } else { "0" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("启动安装程序失败")?;
    // 已经交给安装脚本：退出流程里不要再装一次
    set(State::Idle);
    Ok(())
}

// ───────────── 状态机（后台线程驱动） ─────────────

/// 检查（并在 `auto_download` 时下载）。`manual` 为用户手动触发：失败也提示；自动检查失败静默。
pub fn run(manual: bool, auto_download: bool) {
    if matches!(state(), State::Checking | State::Downloading(_)) {
        return;
    }
    set(State::Checking);
    std::thread::spawn(move || {
        if let Some(s) = crate::store::global() {
            s.set_kv("update_checked", &crate::store::now_ms().to_string());
        }
        match check() {
            Ok(None) => set(if manual { State::UpToDate } else { State::Idle }),
            Ok(Some(rel)) if auto_download => fetch(rel),
            Ok(Some(rel)) => set(State::Available(rel)),
            Err(e) => set(if manual { State::Failed(e.to_string()) } else { State::Idle }),
        }
    });
}

/// 下载某个版本（后台线程）。
pub fn start_download(rel: Release) {
    if matches!(state(), State::Downloading(_)) {
        return;
    }
    std::thread::spawn(move || fetch(rel));
}

fn fetch(rel: Release) {
    set(State::Downloading(rel.clone()));
    match download(&rel) {
        Ok(app) => set(State::Ready { release: rel, app }),
        Err(e) => set(State::Failed(format!("下载新版本失败：{e}"))),
    }
}

/// 启动时调用：开了自动更新、允许联网、距上次检查超过一天，就在后台检查并下载。
pub fn auto_check(store: &crate::store::Store) {
    if !auto_enabled(store) || !crate::settings::network() || std::env::var_os("STONEBROOK_SCRIPT").is_some() {
        return;
    }
    let last: i64 = store.get_kv("update_checked").and_then(|s| s.parse().ok()).unwrap_or(0);
    if crate::store::now_ms() - last < CHECK_INTERVAL_MS {
        return;
    }
    run(false, true);
}

pub fn auto_enabled(store: &crate::store::Store) -> bool {
    store.get_kv("auto_update").as_deref() != Some("0")
}

pub fn set_auto(store: &crate::store::Store, on: bool) {
    store.set_kv("auto_update", if on { "1" } else { "0" });
}

/// 正常退出时：有下载好的新版本就顺手装上（不重新打开）。
pub fn install_on_quit() {
    if let State::Ready { app, .. } = state() {
        if let Err(e) = schedule_install(&app, false) {
            eprintln!("退出时安装更新失败：{e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(newer("2.3.0", "2.2.0"));
        assert!(newer("v2.10.0", "2.9.9"));
        assert!(newer("2.2.1", "2.2"));
        assert!(!newer("2.2.0", "2.2.0"));
        assert!(!newer("2.1.9", "2.2.0"));
        assert!(!newer("2.2.0-beta", "2.2.0"));
    }

    #[test]
    fn picks_asset_for_this_arch_and_checksum() {
        let v = serde_json::json!({
            "tag_name": "v9.0.0", "body": "说明", "html_url": "https://x/r",
            "assets": [
                {"name": "Stonebrook-9.0.0-arm64.app.zip", "browser_download_url": "https://x/a.zip"},
                {"name": "Stonebrook-9.0.0-x86_64.app.zip", "browser_download_url": "https://x/i.zip"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/s.txt"}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "9.0.0");
        assert!(r.zip_name.ends_with(&format!("-{}.app.zip", arch())));
        assert_eq!(r.sums_url.as_deref(), Some("https://x/s.txt"));
        let sums = "abc123  Stonebrook-9.0.0-arm64.app.zip\nDEF456  Stonebrook-9.0.0-x86_64.app.zip\n";
        assert_eq!(expected_sum(sums, "Stonebrook-9.0.0-x86_64.app.zip").as_deref(), Some("def456"));
        assert_eq!(expected_sum(sums, "nope.zip"), None);
    }

    #[test]
    fn sha256_of_known_input() {
        let p = std::env::temp_dir().join(format!("stonebrook-sha-{}", crate::store::new_uuid()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(p);
    }
}
