//! 资源文件（`assets/`）的引用格式与回收。
//!
//! **引用格式**：内存里资源一律是绝对路径（渲染、导出、系统接口直接用）；落盘时在序列化边界把
//! 「以 assets 目录开头的字符串」换成相对引用 `stonebrook://asset/<相对路径>`，读入时再解析回绝对路径。
//! 转换按 JSON 值逐个字符串进行（不是对整段文本做替换），不受 JSON 转义影响；
//! 只有整个字符串以前缀开头才转换，正文里提到的路径不受影响。
//! 这样数据目录可以整体移动 / 换用户名 / 换机器，备份恢复也不再需要改写路径。
//!
//! **回收**：`gc` 扫描数据库里全部文本，收集被引用的资源；`assets/` 下顶层条目（文件或目录）
//! 未被引用且 7 天以上没有修改的移除。`cache/` 按总大小做 LRU（按修改时间淘汰最旧的）。

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 落盘时的相对引用前缀。
pub const REF_PREFIX: &str = "stonebrook://asset/";

/// `cache/` 目录的容量上限。
pub const CACHE_LIMIT: u64 = 512 * 1024 * 1024;

/// 未被引用的资源保留多久（撤销、跨会话粘贴还可能用到）。
const GC_GRACE_MS: u128 = 7 * 24 * 3600 * 1000;

pub fn dir() -> PathBuf {
    crate::store::data_dir().join("assets")
}

fn dir_prefix() -> String {
    let mut p = dir().to_string_lossy().to_string();
    if !p.ends_with('/') {
        p.push('/');
    }
    p
}

/// 绝对路径 → 相对引用（不在 assets 目录下的原样返回）。
pub fn to_ref(s: &str) -> String {
    to_ref_with(s, &dir_prefix()).unwrap_or_else(|| s.to_string())
}

fn to_ref_with(s: &str, prefix: &str) -> Option<String> {
    s.strip_prefix(prefix).filter(|rel| !rel.is_empty() && !rel.contains("..")).map(|rel| format!("{REF_PREFIX}{rel}"))
}

/// 相对引用 → 绝对路径（其他字符串原样返回，旧数据里的绝对路径也照常可用）。
pub fn resolve(s: &str) -> String {
    resolve_with(s, &dir_prefix()).unwrap_or_else(|| s.to_string())
}

fn resolve_with(s: &str, prefix: &str) -> Option<String> {
    s.strip_prefix(REF_PREFIX).filter(|rel| !rel.contains("..")).map(|rel| format!("{prefix}{rel}"))
}

fn walk(v: &mut Value, f: &dyn Fn(&str) -> Option<String>) {
    match v {
        Value::String(s) => {
            if let Some(n) = f(s) {
                *s = n;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| walk(x, f)),
        Value::Object(o) => o.values_mut().for_each(|x| walk(x, f)),
        _ => {}
    }
}

/// 把 JSON 值里的资源绝对路径换成相对引用（落盘前）。
pub fn refs_out(v: &mut Value) {
    let p = dir_prefix();
    walk(v, &|s| to_ref_with(s, &p));
}

/// 把 JSON 值里的相对引用解析成绝对路径（读入后）。
pub fn refs_in(v: &mut Value) {
    let p = dir_prefix();
    walk(v, &|s| resolve_with(s, &p));
}

/// 序列化为落盘用的 JSON 字节（资源路径转成相对引用）。
pub fn encode<T: Serialize + ?Sized>(x: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut v = serde_json::to_value(x)?;
    refs_out(&mut v);
    serde_json::to_vec(&v)
}

pub fn encode_string<T: Serialize + ?Sized>(x: &T) -> Result<String, serde_json::Error> {
    let mut v = serde_json::to_value(x)?;
    refs_out(&mut v);
    serde_json::to_string(&v)
}

/// 从落盘的 JSON 解码（相对引用解析成绝对路径）。
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let mut v: Value = serde_json::from_slice(bytes)?;
    refs_in(&mut v);
    serde_json::from_value(v)
}

/// 一段 JSON 文本整体转换（电子表格扩展数据等已是 JSON 字符串的场合）。非 JSON 原样返回。
pub fn json_text_out(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(mut v) => {
            refs_out(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| s.to_string())
        }
        Err(_) => s.to_string(),
    }
}

pub fn json_text_in(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(mut v) => {
            refs_in(&mut v);
            serde_json::to_string(&v).unwrap_or_else(|_| s.to_string())
        }
        Err(_) => s.to_string(),
    }
}

/// 把外部文件拷进 assets：拷完把修改时间设为现在。
/// APFS 上 `fs::copy` 走 clonefile 会保留源文件的修改时间，不改的话刚导入的老照片会被回收当成「7 天前的未引用文件」。
pub fn import_copy(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let n = std::fs::copy(src, dst)?;
    if let Ok(f) = std::fs::File::options().write(true).open(dst) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Ok(n)
}

// ───────────── 回收 ─────────────

/// 从文本里收集被引用的资源顶层名（`assets/` 下的第一级文件名或目录名）。
/// 同时认相对引用与旧数据里的绝对路径。
pub fn collect_refs(texts: &[String], abs_prefix: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for t in texts {
        for pat in [REF_PREFIX, abs_prefix] {
            let mut rest = t.as_str();
            while let Some(i) = rest.find(pat) {
                rest = &rest[i + pat.len()..];
                // 顶层名到 `/` 或 JSON 字符串结束（引号、反斜杠转义）为止
                let end = rest.find(|c: char| matches!(c, '/' | '"' | '\\' | '\'' | ')' | '<' | '>') || c.is_whitespace()).unwrap_or(rest.len());
                if end > 0 {
                    out.insert(rest[..end].to_string());
                }
            }
        }
    }
    out
}

fn age_ms(meta: &std::fs::Metadata) -> u128 {
    meta.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_millis()).unwrap_or(0)
}

/// 回收未被引用的资源：返回 (删除条目数, 释放字节数)。`texts` 必须是完整的引用集合，读失败时不要调用。
pub fn gc_assets(assets: &Path, texts: &[String]) -> (usize, u64) {
    let mut prefix = assets.to_string_lossy().to_string();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    let refs = collect_refs(texts, &prefix);
    let mut n = 0;
    let mut bytes = 0;
    let Ok(rd) = std::fs::read_dir(assets) else { return (0, 0) };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || refs.contains(&name) {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if age_ms(&meta) < GC_GRACE_MS {
            continue;
        }
        let size = if meta.is_dir() { dir_size(&e.path()) } else { meta.len() };
        let ok = if meta.is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
        if ok.is_ok() {
            n += 1;
            bytes += size;
        }
    }
    (n, bytes)
}

pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// `cache/` 超过 `limit` 时按修改时间从旧到新删除文件，直到降到上限的 80%。返回删除的文件数。
pub fn trim_cache(cache: &Path, limit: u64) -> usize {
    let mut files = vec![];
    collect_files(cache, &mut files);
    let total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return 0;
    }
    files.sort_by_key(|f| f.2);
    let target = limit / 5 * 4;
    let (mut cur, mut n) = (total, 0);
    for (p, size, _) in files {
        if cur <= target {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            cur -= size;
            n += 1;
        }
    }
    n
}

fn collect_files(p: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            collect_files(&e.path(), out);
        } else {
            out.push((e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_round_trip_only_whole_strings_under_assets() {
        let p = "/data/assets/";
        assert_eq!(to_ref_with("/data/assets/ab/x.png", p).as_deref(), Some("stonebrook://asset/ab/x.png"));
        assert_eq!(to_ref_with("/other/x.png", p), None);
        assert_eq!(to_ref_with("见 /data/assets/x.png", p), None, "正文里提到的路径不转换");
        assert_eq!(to_ref_with("/data/assets/../stonebrook.db", p), None);
        assert_eq!(resolve_with("stonebrook://asset/ab/x.png", p).as_deref(), Some("/data/assets/ab/x.png"));
        assert_eq!(resolve_with("stonebrook://asset/../x", p), None);
        let mut v = serde_json::json!({"a": ["/data/assets/1.png", {"b": "/data/assets/d/2.png"}], "c": 3});
        walk(&mut v, &|s| to_ref_with(s, p));
        assert_eq!(v["a"][1]["b"], "stonebrook://asset/d/2.png");
        walk(&mut v, &|s| resolve_with(s, p));
        assert_eq!(v["a"][0], "/data/assets/1.png");
    }

    #[test]
    fn collect_refs_finds_top_level_names() {
        let texts = vec![
            r#"{"path":"stonebrook://asset/ab12cd34/照片.png","x":"stonebrook://asset/u.png"}"#.to_string(),
            r#"{"old":"/data/assets/legacy.jpg"}"#.to_string(),
        ];
        let r = collect_refs(&texts, "/data/assets/");
        assert!(r.contains("ab12cd34") && r.contains("u.png") && r.contains("legacy.jpg"));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn gc_removes_only_old_unreferenced_entries() {
        let dir = std::env::temp_dir().join(format!("stonebrook-assets-gc-{}", crate::store::new_uuid()));
        std::fs::create_dir_all(dir.join("keepdir")).unwrap();
        for f in ["keep.png", "drop.png", "fresh.png", "keepdir/a.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86400);
        for f in ["keep.png", "drop.png", "keepdir"] {
            std::fs::File::options().write(true).open(dir.join(f)).or_else(|_| std::fs::File::open(dir.join(f))).unwrap().set_modified(old).unwrap();
        }
        let texts = vec![format!("{REF_PREFIX}keep.png {REF_PREFIX}keepdir/a.png")];
        let (n, _) = gc_assets(&dir, &texts);
        assert_eq!(n, 1);
        assert!(!dir.join("drop.png").exists());
        assert!(dir.join("keep.png").exists() && dir.join("fresh.png").exists() && dir.join("keepdir/a.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trim_cache_drops_oldest_first() {
        let dir = std::env::temp_dir().join(format!("stonebrook-cache-lru-{}", crate::store::new_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now();
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            std::fs::write(dir.join(name), vec![0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(dir.join(name)).unwrap();
            f.set_modified(now - std::time::Duration::from_secs((3 - i as u64) * 1000)).unwrap();
        }
        assert_eq!(trim_cache(&dir, 250), 1);
        assert!(!dir.join("old").exists() && dir.join("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
