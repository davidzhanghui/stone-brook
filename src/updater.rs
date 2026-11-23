//! 应用内更新：查 GitHub Releases 的最新版本 → 后台下载本机架构的 `.app.zip` → 校验 SHA-256 → 解压待装；
//! 用户点「重启并安装」（或正常退出时）由一个脱离本进程的小脚本在本进程退出后替换 `.app` 并（可选）重新打开。
//!
//! - 所有网络访问受「允许联网」开关控制；自动检查每 6 小时最多一次（`kv: update_checked`），
//!   失败按 1/2/4/6 小时退避（`kv: update_failures` / `kv: update_retry_at`），运行期间定时重试；
//!   可在设置里关闭（`kv: auto_update`），也可跳过某个版本（`kv: skipped_version`）。
//! - 下载好的包记录在 `kv: pending_update`，重启后仍能接着安装；启动时清理不再需要的暂存版本。
//! - 发布产物由 `.github/workflows/release.yml` 生成：`Stonebrook-<版本>-<arm64|x86_64>.app.zip`、`SHA256SUMS.txt` 与 `SHA256SUMS.txt.sig`。
//!   客户端先用内置 Ed25519 公钥验签 `SHA256SUMS.txt.sig`，再校验安装包 SHA-256；任一环节缺失或不匹配即拒绝安装。
//! - 只有从 `.app` 包启动时才能自动安装（`cargo run` 开发模式下只能下载，然后在访达里手动替换）。
//! - 替换前旧版本移到 `<数据目录>/updates/previous.app`，新版本装不上时脚本会把旧版本放回去。

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

pub const REPO: &str = "davidzhanghui/stone-brook";
const CHECK_INTERVAL_MS: i64 = 6 * 3600 * 1000;
const RETRY_BASE_MS: i64 = 3600 * 1000;
const RETRY_MAX_MS: i64 = 6 * 3600 * 1000;
const PERIODIC_SECS: u64 = 15 * 60;
const PREVIOUS_KEEP_MS: i64 = 7 * 24 * 3600 * 1000;

const KV_PENDING: &str = "pending_update";
const KV_SKIPPED: &str = "skipped_version";
const KV_CHECKED: &str = "update_checked";
const KV_RETRY: &str = "update_retry_at";
const KV_FAILURES: &str = "update_failures";
const KV_AUTO_INSTALL: &str = "auto_install_update";

/// 发版签名公钥（Ed25519 原始 32 字节，对应 GitHub Actions Secret `RELEASE_SIGNING_KEY`）。
const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0x14, 0xdb, 0x68, 0x46, 0x8a, 0x75, 0x4a, 0xe3, 0xec, 0x4c, 0x58, 0x9a, 0xf0, 0x3a, 0x2e, 0x6c, 0x5d, 0x53, 0x31, 0xa9, 0x0e, 0x23, 0x12, 0xbb, 0xc6, 0xa7,
    0x15, 0x36, 0xb1, 0x23, 0x66, 0xcd,
];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub zip_name: String,
    pub zip_url: String,
    pub sums_url: Option<String>,
    #[serde(default)]
    pub sig_url: Option<String>,
    #[serde(default)]
    pub size: u64,
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
/// 用户本次会话是否显式选择了「退出应用时安装」
static INSTALL_ON_QUIT: AtomicBool = AtomicBool::new(false);
/// 自动检查 / 自动安装的内存缓存（0=未加载，1=关闭，2=开启），避免异步写库导致界面切换回弹
static AUTO_UPDATE_CACHE: AtomicU8 = AtomicU8::new(0);
static AUTO_INSTALL_CACHE: AtomicU8 = AtomicU8::new(0);
static GOT: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn state() -> State {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn arm_install_on_quit(on: bool) {
    INSTALL_ON_QUIT.store(on, Ordering::SeqCst);
    CHANGED.store(true, Ordering::SeqCst);
}

pub fn is_install_on_quit_armed() -> bool {
    INSTALL_ON_QUIT.load(Ordering::SeqCst)
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

/// 供设置面板展示的本机架构名。
pub fn arch_label() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "Apple Silicon"
    } else {
        "Intel"
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

/// 版本号会拼进暂存目录路径：只允许字母数字和 `.` `-` `_`，且不能以 `.` 开头。
fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 64 && !v.starts_with('.') && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// 从 GitHub API 的 release JSON 里取出本机要用的产物。
fn parse_release(v: &serde_json::Value) -> Result<Release> {
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("发布信息缺少版本号"))?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !valid_version(&version) {
        bail!("发布信息里的版本号不合法：{version}");
    }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find(|a| a["name"].as_str().is_some_and(pred)).map(|a| {
            (a["name"].as_str().unwrap_or("").to_string(), a["browser_download_url"].as_str().unwrap_or("").to_string(), a["size"].as_u64().unwrap_or(0))
        })
    };
    let suffix = format!("-{}.app.zip", arch());
    let (zip_name, zip_url, size) = find(&|n| n.ends_with(&suffix)).ok_or_else(|| anyhow!("新版本 {version} 没有本机（{}）的安装包", arch()))?;
    let sums_url = find(&|n| n == "SHA256SUMS.txt").map(|x| x.1);
    let sig_url = find(&|n| n == "SHA256SUMS.txt.sig").map(|x| x.1);
    Ok(Release {
        version,
        notes: v["body"].as_str().unwrap_or("").to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        zip_name,
        zip_url,
        sums_url,
        sig_url,
        size,
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

/// 某个目录所在卷的可用字节数。
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // statvfs 只读取文件系统统计，不动目录内容
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

/// 用内置 Ed25519 公钥验证 `SHA256SUMS.txt` 的离线数字签名。
fn verify_sums_signature(sums_bytes: &[u8], sig_bytes: &[u8]) -> Result<()> {
    if sig_bytes.len() != 64 {
        bail!("签名文件长度异常（{} 字节，应为 64 字节），已拒绝安装", sig_bytes.len());
    }
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &RELEASE_PUBLIC_KEY)
        .verify(sums_bytes, sig_bytes)
        .map_err(|_| anyhow!("发布签名校验失败（Ed25519 不匹配），安装包可能被篡改，已拒绝安装"))
}

/// 下载、验签、校验并解压（阻塞，后台线程调用）。返回解压出的 `.app` 路径。
pub fn download(rel: &Release) -> Result<PathBuf> {
    if !crate::settings::network() {
        bail!("已关闭联网，无法下载更新");
    }
    if !valid_version(&rel.version) {
        bail!("版本号不合法：{}", rel.version);
    }
    let sums_url = rel.sums_url.as_ref().ok_or_else(|| anyhow!("发布里没有校验文件 SHA256SUMS.txt，为安全起见不安装"))?;
    let sig_url = rel.sig_url.as_ref().ok_or_else(|| anyhow!("发布里没有签名文件 SHA256SUMS.txt.sig，为安全起见不安装"))?;
    let mut sums_resp = get(sums_url)?;
    let sums_bytes = sums_resp.body_mut().with_config().limit(1024 * 1024).read_to_vec()?;
    let mut sig_resp = get(sig_url)?;
    let sig_bytes = sig_resp.body_mut().with_config().limit(4096).read_to_vec()?;
    verify_sums_signature(&sums_bytes, &sig_bytes)?;
    let sums = String::from_utf8(sums_bytes).context("校验文件不是有效的 UTF-8 文本")?;
    let want = expected_sum(&sums, &rel.zip_name).ok_or_else(|| anyhow!("校验文件里没有 {}", rel.zip_name))?;

    let root = updates_dir();
    std::fs::create_dir_all(&root)?;
    // 压缩包 + 解压后的 .app 大约是压缩包的三倍
    if rel.size > 0 {
        if let Some(free) = free_space(&root) {
            let need = rel.size.saturating_mul(3);
            if free < need {
                bail!("磁盘空间不足：安装需要约 {}，当前可用 {}", crate::convert::human_size(need), crate::convert::human_size(free));
            }
        }
    }
    let dir = root.join(&rel.version);
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
pid="$1"; new="$2"; dst="$3"; bak="$4"; relaunch="$5"; stage="$6"; log="$7"; mark="$8"
fail() {
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >>"$log"
  printf '%s\n' "$*" >"$mark"
  exit 1
}
i=0
while kill -0 "$pid" 2>/dev/null; do
  sleep 0.2; i=$((i+1))
  [ "$i" -gt 300 ] && fail "等待旧版本退出超时（60 秒）"
done
rm -rf "$bak"
mv "$dst" "$bak" || fail "没有权限替换 $dst"
if mv "$new" "$dst"; then
  xattr -cr "$dst" 2>/dev/null
  rm -rf "$stage"
  rm -f "$mark"
  printf '%s 已更新到新版本\n' "$(date '+%Y-%m-%d %H:%M:%S')" >>"$log"
else
  mv "$bak" "$dst" || fail "新版本替换失败，且恢复旧版本失败，请从官网重新下载安装"
  fail "新版本替换失败，已恢复旧版本"
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
    let stage = app.parent().and_then(Path::parent).ok_or_else(|| anyhow!("更新包的暂存目录不对：{}", app.display()))?;
    let root = updates_dir();
    std::fs::create_dir_all(&root)?;
    let bak = root.join("previous.app");
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(INSTALL_SH)
        .arg("stonebrook-update")
        .arg(std::process::id().to_string())
        .arg(app)
        .arg(&dst)
        .arg(&bak)
        .arg(if relaunch { "1" } else { "0" })
        .arg(stage)
        .arg(root.join("install.log"))
        .arg(root.join("install-failed"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("启动安装程序失败")?;
    // 已经交给安装脚本：退出流程里不要再装一次
    INSTALL_ON_QUIT.store(false, Ordering::SeqCst);
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
    std::thread::spawn(move || match check() {
        Ok(None) => {
            record_ok();
            set(if manual { State::UpToDate } else { State::Idle });
        }
        Ok(Some(rel)) => {
            record_ok();
            let skipped = crate::store::global().and_then(|s| s.get_kv(KV_SKIPPED)).unwrap_or_default();
            if !manual && skipped == rel.version {
                set(State::Idle);
            } else if auto_download {
                fetch(rel);
            } else {
                set(State::Available(rel));
            }
        }
        Err(e) => {
            record_fail();
            set(if manual { State::Failed(e.to_string()) } else { State::Idle });
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
        Ok(app) => {
            if let Some(s) = crate::store::global() {
                s.set_kv(KV_SKIPPED, "");
                if let Ok(raw) = serde_json::to_string(&(rel.clone(), app.clone())) {
                    s.set_kv(KV_PENDING, &raw);
                }
            }
            set(State::Ready { release: rel, app });
        }
        Err(e) => {
            record_fail();
            set(State::Failed(format!("下载新版本失败：{e}")));
        }
    }
}

/// 记下不想装的版本；同时清理已下载的暂存包与退出安装标记，避免下次启动复活。
pub fn skip_version(store: &crate::store::Store, version: &str) {
    store.set_kv(KV_SKIPPED, version);
    store.set_kv(KV_PENDING, "");
    INSTALL_ON_QUIT.store(false, Ordering::SeqCst);
    if valid_version(version) {
        let _ = std::fs::remove_dir_all(updates_dir().join(version));
    }
    set(State::Idle);
}

/// 启动时调用：恢复上次下载好的更新、清理暂存目录、提示上次安装失败，并开始定时检查。
pub fn init(store: &crate::store::Store) {
    restore_pending();
    let keep = match state() {
        State::Ready { release, .. } => Some(release.version),
        _ => None,
    };
    cleanup(keep.as_deref());
    report_install_failure(store);
    auto_check(store);
    let store = store.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(PERIODIC_SECS));
        auto_check(&store);
    });
}

/// 启动时调用：开了自动更新、允许联网、距上次检查超过 6 小时且不在退避期，就后台检查并下载。
pub fn auto_check(store: &crate::store::Store) {
    if !auto_enabled(store) || !crate::settings::network() || std::env::var_os("STONEBROOK_SCRIPT").is_some() {
        return;
    }
    if matches!(state(), State::Ready { .. }) {
        return;
    }
    let now = crate::store::now_ms();
    let last: i64 = store.get_kv(KV_CHECKED).and_then(|s| s.parse().ok()).unwrap_or(0);
    let retry: i64 = store.get_kv(KV_RETRY).and_then(|s| s.parse().ok()).unwrap_or(0);
    if retry > 0 {
        // 失败的退避优先于常规间隔
        if now < retry {
            return;
        }
    } else if now - last < CHECK_INTERVAL_MS {
        return;
    }
    run(false, true);
}

fn record_ok() {
    if let Some(s) = crate::store::global() {
        s.set_kv(KV_CHECKED, &crate::store::now_ms().to_string());
        s.set_kv(KV_RETRY, "0");
        s.set_kv(KV_FAILURES, "0");
    }
}

/// 失败退避：1、2、4 小时，最多 6 小时。
fn retry_delay(failures: u32) -> i64 {
    (RETRY_BASE_MS << failures.saturating_sub(1).min(3)).min(RETRY_MAX_MS)
}

fn record_fail() {
    if let Some(s) = crate::store::global() {
        let n = s.get_kv(KV_FAILURES).and_then(|v| v.parse().ok()).unwrap_or(0u32).saturating_add(1);
        s.set_kv(KV_FAILURES, &n.to_string());
        s.set_kv(KV_RETRY, &(crate::store::now_ms() + retry_delay(n)).to_string());
    }
}

fn pending() -> Option<(Release, PathBuf)> {
    let raw = crate::store::global()?.get_kv(KV_PENDING)?;
    serde_json::from_str(&raw).ok()
}

fn restore_pending() {
    let Some((rel, app)) = pending() else { return };
    let skipped = crate::store::global().and_then(|s| s.get_kv(KV_SKIPPED)).unwrap_or_default();
    let valid = skipped != rel.version
        && newer(&rel.version, current_version())
        && app.starts_with(updates_dir())
        && app.extension().is_some_and(|e| e == "app")
        && bundle_version(&app).as_deref() == Some(rel.version.as_str());
    if valid {
        set(State::Ready { release: rel, app });
    } else if let Some(s) = crate::store::global() {
        s.set_kv(KV_PENDING, "");
    }
}

/// 读 `.app` 包里的版本号（不引 plist 依赖，按 XML 文本取）。
fn bundle_version(app: &Path) -> Option<String> {
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
    let (_, rest) = plist.split_once("CFBundleShortVersionString")?;
    let (_, body) = rest.split_once("<string>")?;
    let (v, _) = body.split_once("</string>")?;
    Some(v.trim().to_string())
}

/// 清掉不再需要的暂存版本；上一版回滚副本保留 7 天。
fn cleanup(keep: Option<&str>) {
    let Ok(rd) = std::fs::read_dir(updates_dir()) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name == "install.log" || name == "install-failed" {
            continue;
        }
        if name == "previous.app" {
            let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_millis() as i64 > PREVIOUS_KEEP_MS);
            if old {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        if Some(name.as_str()) != keep {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn report_install_failure(store: &crate::store::Store) {
    let mark = updates_dir().join("install-failed");
    let Ok(msg) = std::fs::read_to_string(&mark) else { return };
    let _ = std::fs::remove_file(&mark);
    let msg = msg.trim();
    store.report_error(if msg.is_empty() {
        "上次更新安装失败，已保留旧版本".to_string()
    } else {
        format!("上次更新安装失败，已保留旧版本：{msg}")
    });
}

pub fn auto_enabled(store: &crate::store::Store) -> bool {
    match AUTO_UPDATE_CACHE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let v = store.get_kv("auto_update").as_deref() != Some("0");
            AUTO_UPDATE_CACHE.store(if v { 2 } else { 1 }, Ordering::Relaxed);
            v
        }
    }
}

pub fn set_auto(store: &crate::store::Store, on: bool) {
    AUTO_UPDATE_CACHE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    store.set_kv("auto_update", if on { "1" } else { "0" });
    if !on {
        AUTO_INSTALL_CACHE.store(1, Ordering::Relaxed);
        store.set_kv(KV_AUTO_INSTALL, "0");
    }
}

/// 是否开启「以后自动下载并安装更新」：开启后后台下完不弹窗打扰，退出时静默安装。
pub fn auto_install_enabled(store: &crate::store::Store) -> bool {
    match AUTO_INSTALL_CACHE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let v = store.get_kv(KV_AUTO_INSTALL).as_deref() == Some("1");
            AUTO_INSTALL_CACHE.store(if v { 2 } else { 1 }, Ordering::Relaxed);
            v
        }
    }
}

pub fn set_auto_install(store: &crate::store::Store, on: bool) {
    AUTO_INSTALL_CACHE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    store.set_kv(KV_AUTO_INSTALL, if on { "1" } else { "0" });
    if on {
        AUTO_UPDATE_CACHE.store(2, Ordering::Relaxed);
        store.set_kv("auto_update", "1");
    }
}

/// 正常退出时：仅在用户选择了「退出应用时安装」或开启了「以后自动下载并安装更新」时，顺手装上（不重新打开）。
pub fn install_on_quit() {
    let auto = crate::store::global().is_some_and(auto_install_enabled);
    if !auto && !is_install_on_quit_armed() {
        return;
    }
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
    fn version_names_are_path_safe() {
        assert!(valid_version("2.5.0"));
        assert!(valid_version("2.6.0-beta_1"));
        assert!(!valid_version(""));
        assert!(!valid_version("../evil"));
        assert!(!valid_version("2.5.0/../x"));
        assert!(!valid_version(".hidden"));
    }

    #[test]
    fn rejects_bad_version_tags() {
        let v = serde_json::json!({
            "tag_name": "v../9.0.0", "body": "", "html_url": "https://x/r",
            "assets": [{"name": "Stonebrook-9.0.0-arm64.app.zip", "browser_download_url": "https://x/a.zip"}]
        });
        assert!(parse_release(&v).is_err());
    }

    #[test]
    fn reads_bundle_version_from_plist() {
        let dir = std::env::temp_dir().join(format!("stonebrook-bundle-{}", crate::store::new_uuid()));
        std::fs::create_dir_all(dir.join("Contents")).unwrap();
        std::fs::write(
            dir.join("Contents/Info.plist"),
            "<?xml version=\"1.0\"?><plist><dict><key>CFBundleShortVersionString</key><string>2.6.0</string></dict></plist>",
        )
        .unwrap();
        assert_eq!(bundle_version(&dir).as_deref(), Some("2.6.0"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_update_round_trips() {
        let rel = Release {
            version: "2.6.0".into(),
            notes: "说明".into(),
            page: "https://x/r".into(),
            zip_name: "Stonebrook-2.6.0-arm64.app.zip".into(),
            zip_url: "https://x/a.zip".into(),
            sums_url: Some("https://x/s.txt".into()),
            sig_url: Some("https://x/s.txt.sig".into()),
            size: 12345,
        };
        let app = PathBuf::from("/Users/me/Stonebrook/updates/2.6.0/app/石渠.app");
        let raw = serde_json::to_string(&(rel.clone(), app.clone())).unwrap();
        let back: (Release, PathBuf) = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, (rel, app));
    }

    #[test]
    fn cleanup_keeps_pending_version() {
        let _env = crate::store::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("stonebrook-updates-{}", crate::store::new_uuid()));
        std::env::set_var(crate::store::DATA_DIR_ENV, &root);
        let dir = root.join("updates");
        std::fs::create_dir_all(dir.join("2.4.0/app")).unwrap();
        std::fs::create_dir_all(dir.join("2.5.0/app")).unwrap();
        std::fs::create_dir_all(dir.join("previous.app")).unwrap();
        std::fs::write(dir.join("install.log"), "x").unwrap();
        cleanup(Some("2.5.0"));
        assert!(dir.join("2.5.0/app").exists());
        assert!(!dir.join("2.4.0").exists());
        assert!(dir.join("previous.app").exists());
        assert!(dir.join("install.log").exists());
        std::env::remove_var(crate::store::DATA_DIR_ENV);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn retry_delays_back_off() {
        assert_eq!(retry_delay(0), RETRY_BASE_MS);
        assert_eq!(retry_delay(1), RETRY_BASE_MS);
        assert_eq!(retry_delay(2), 2 * RETRY_BASE_MS);
        assert_eq!(retry_delay(3), 4 * RETRY_BASE_MS);
        assert_eq!(retry_delay(9), RETRY_MAX_MS);
    }

    #[test]
    fn picks_asset_for_this_arch_and_checksum() {
        let v = serde_json::json!({
            "tag_name": "v9.0.0", "body": "说明", "html_url": "https://x/r",
            "assets": [
                {"name": "Stonebrook-9.0.0-arm64.app.zip", "browser_download_url": "https://x/a.zip", "size": 12345},
                {"name": "Stonebrook-9.0.0-x86_64.app.zip", "browser_download_url": "https://x/i.zip"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/s.txt"},
                {"name": "SHA256SUMS.txt.sig", "browser_download_url": "https://x/s.txt.sig"}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "9.0.0");
        assert!(r.zip_name.ends_with(&format!("-{}.app.zip", arch())));
        assert_eq!(r.sums_url.as_deref(), Some("https://x/s.txt"));
        assert_eq!(r.sig_url.as_deref(), Some("https://x/s.txt.sig"));
        assert_eq!(r.size, if cfg!(target_arch = "aarch64") { 12345 } else { 0 });
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

    #[test]
    fn skip_version_clears_pending_stage_and_quit_flag() {
        let _env = crate::store::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("stonebrook-upd-skip-{}", crate::store::new_uuid()));
        std::env::set_var(crate::store::DATA_DIR_ENV, &root);
        let store = crate::store::Store::open().unwrap();
        let staged = updates_dir().join("9.9.9");
        let app = staged.join("app/石渠.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            "<?xml version=\"1.0\"?><plist><dict><key>CFBundleShortVersionString</key><string>9.9.9</string></dict></plist>",
        )
        .unwrap();
        let rel = Release {
            version: "9.9.9".into(),
            notes: "notes".into(),
            page: "https://example.com".into(),
            zip_name: "Stonebrook-9.9.9-arm64.app.zip".into(),
            zip_url: "https://example.com/a.zip".into(),
            sums_url: None,
            sig_url: None,
            size: 100,
        };
        store.set_kv(KV_PENDING, &serde_json::to_string(&(rel.clone(), app.clone())).unwrap());
        set(State::Ready { release: rel, app });
        arm_install_on_quit(true);
        assert!(is_install_on_quit_armed());

        skip_version(&store, "9.9.9");
        store.flush();
        assert_eq!(state(), State::Idle);
        assert!(!is_install_on_quit_armed());
        assert_eq!(store.get_kv(KV_SKIPPED).as_deref(), Some("9.9.9"));
        assert_eq!(store.get_kv(KV_PENDING).as_deref(), Some(""));
        assert!(!staged.exists(), "跳过版本后应清理暂存目录");

        // 自动安装偏好与自动更新开关联动（先重置缓存确保从空库加载）
        AUTO_UPDATE_CACHE.store(0, Ordering::Relaxed);
        AUTO_INSTALL_CACHE.store(0, Ordering::Relaxed);
        assert!(!auto_install_enabled(&store));
        set_auto_install(&store, true);
        assert!(auto_install_enabled(&store));
        assert!(auto_enabled(&store));
        store.flush();
        assert_eq!(store.get_kv(KV_AUTO_INSTALL).as_deref(), Some("1"));
        set_auto(&store, false);
        assert!(!auto_enabled(&store));
        assert!(!auto_install_enabled(&store));
        store.flush();
        assert_eq!(store.get_kv(KV_AUTO_INSTALL).as_deref(), Some("0"));

        std::env::remove_var(crate::store::DATA_DIR_ENV);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ed25519_verifies_openssl_signature_and_rejects_tampering() {
        // 使用 OpenSSL 3 `pkeyutl -sign -rawin` 对测试消息生成的真实 Ed25519 签名，验证与客户端 ring 完全互通
        let msg = b"stonebrook-ed25519-self-test\n";
        let sig: [u8; 64] = [
            0x2c, 0x0d, 0x5a, 0xa4, 0x55, 0x2a, 0x17, 0xf6, 0x9f, 0xcc, 0x58, 0x51, 0x53, 0xc8, 0xd7, 0xeb, 0x73, 0x24, 0x9c, 0xad, 0x6e, 0x8e, 0x3f, 0x9c,
            0x17, 0x18, 0xd3, 0x08, 0xb7, 0x8d, 0xf9, 0xbc, 0x23, 0x8c, 0x81, 0x5c, 0xeb, 0xcc, 0xba, 0x56, 0x8f, 0xef, 0x27, 0x79, 0x1c, 0xf3, 0xaa, 0x5f,
            0x24, 0x3d, 0xf1, 0xa3, 0x22, 0xd6, 0xb3, 0x22, 0xd2, 0x35, 0xcf, 0xec, 0xf4, 0x62, 0xac, 0x0a,
        ];
        assert!(verify_sums_signature(msg, &sig).is_ok());

        // 篡改消息任意 1 字节必须拒签
        let mut tampered_msg = msg.to_vec();
        tampered_msg[0] ^= 0x01;
        assert!(verify_sums_signature(&tampered_msg, &sig).is_err());

        // 篡改签名任意 1 字节必须拒签
        let mut tampered_sig = sig;
        tampered_sig[63] ^= 0x01;
        assert!(verify_sums_signature(msg, &tampered_sig).is_err());

        // 签名长度不对必须拒签
        assert!(verify_sums_signature(msg, &sig[..63]).is_err());
    }
}
