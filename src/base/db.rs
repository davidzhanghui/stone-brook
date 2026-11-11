//! 多维表格的 SQLite 存储：字段 / 记录 / 视图逐行存，改一个格子只写一行。
//!
//! objects 表里仍有一条 kind='base' 的对象，内容只是 `BaseMeta`（名称、数据表顺序），
//! 文档里的 `Embed { kind: "base", id }` 引用方式与画板、电子表格一致。

use super::model::*;
use crate::store::Store;
use rusqlite::params;

/// 修改历史保留天数
const HISTORY_DAYS: i64 = 180;

fn store() -> Option<&'static Store> {
    crate::store::global()
}

/// 序列化一行；失败时整个事务放弃（不能把空串写进数据库覆盖原记录）。
fn to_json<T: serde::Serialize>(v: &T) -> rusqlite::Result<String> {
    crate::assets::encode_string(v).map_err(|e| crate::store::encode_error("多维表格", e))
}

/// 把一批修改（取前 / 后状态）放进写队列，在一个事务里写入（表结构见 `store::MIGRATIONS`）。
/// 失败原因由 `Store::take_error` 统一上报。
pub fn write(base: &str, changes: &[Change], after: bool) {
    let Some(store) = store() else { return };
    let (base, changes) = (base.to_string(), changes.to_vec());
    store.write(None, move |c| {
        let base = base.as_str();
        let tx = c.transaction()?;
        for ch in &changes {
            match ch {
                Change::Meta(..) => {} // 由 objects 表保存
                Change::Table(tid, a, b) => match if after { b } else { a } {
                    Some(m) => tx.execute("INSERT OR REPLACE INTO base_tables VALUES (?1, ?2, ?3)", params![base, tid, to_json(m)?]).map(|_| ())?,
                    None => tx.execute("DELETE FROM base_tables WHERE base_id = ?1 AND table_id = ?2", params![base, tid]).map(|_| ())?,
                },
                Change::Field(tid, fid, a, b) => match if after { b } else { a } {
                    Some(f) => tx.execute("INSERT OR REPLACE INTO base_fields VALUES (?1, ?2, ?3, ?4)", params![base, tid, fid, to_json(f)?]).map(|_| ())?,
                    None => tx.execute("DELETE FROM base_fields WHERE base_id = ?1 AND field_id = ?2", params![base, fid]).map(|_| ())?,
                },
                Change::Record(tid, rid, a, b) => {
                    let (old, new) = if after { (a, b) } else { (b, a) };
                    log_history(&tx, base, rid, old.as_ref(), new.as_ref());
                    match new {
                        Some(r) => tx
                            .prepare_cached("INSERT OR REPLACE INTO base_records VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?
                            .execute(params![base, tid, rid, to_json(r)?, r.created, r.updated])
                            .map(|_| ())?,
                        None => tx.prepare_cached("DELETE FROM base_records WHERE base_id = ?1 AND record_id = ?2")?.execute(params![base, rid]).map(|_| ())?,
                    }
                }
                Change::View(tid, vid, a, b) => match if after { b } else { a } {
                    Some(v) => tx.execute("INSERT OR REPLACE INTO base_views VALUES (?1, ?2, ?3, ?4)", params![base, tid, vid, to_json(v)?]).map(|_| ())?,
                    None => tx.execute("DELETE FROM base_views WHERE base_id = ?1 AND view_id = ?2", params![base, vid]).map(|_| ())?,
                },
            }
        }
        tx.commit()
    });
}

/// 记录修改历史：逐字段写前 / 后值（新建、删除记为字段 ""）。
fn log_history(tx: &rusqlite::Transaction, base: &str, rid: &str, old: Option<&Record>, new: Option<&Record>) {
    let at = now();
    let put = |fid: &str, a: Option<&serde_json::Value>, b: Option<&serde_json::Value>| {
        if let Ok(mut st) = tx.prepare_cached("INSERT INTO base_history VALUES (?1, ?2, ?3, ?4, ?5, ?6)") {
            st.execute(params![base, rid, fid, a.map(|v| v.to_string()), b.map(|v| v.to_string()), at]).ok();
        }
    };
    match (old, new) {
        (None, Some(_)) => put("", None, Some(&serde_json::Value::from("created"))),
        (Some(_), None) => put("", Some(&serde_json::Value::from("deleted")), None),
        (Some(a), Some(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.cells.keys().chain(b.cells.keys()).collect();
            for k in keys {
                if a.cells.get(k) != b.cells.get(k) {
                    put(k, a.cells.get(k), b.cells.get(k));
                }
            }
        }
        _ => {}
    }
}

/// 清理过期的修改历史（启动时在后台排队执行一次，不阻塞打开历史面板）。
pub fn prune_history(store: &Store) {
    let cutoff = now() - HISTORY_DAYS * 86_400_000;
    store.write(None, move |c| c.execute("DELETE FROM base_history WHERE at < ?1", [cutoff]).map(|_| ()));
}

/// 一条记录的修改历史（新的在前）：(字段 id, 前值, 后值, 时间)。
pub fn history(base: &str, rid: &str) -> Vec<(String, Option<serde_json::Value>, Option<serde_json::Value>, i64)> {
    let Some(store) = store() else { return vec![] };
    store.flush();
    let cutoff = now() - HISTORY_DAYS * 86_400_000;
    store.with_conn(|c| {
        let Ok(mut st) = c.prepare(
            "SELECT field_id, before, after, at FROM base_history WHERE base_id = ?1 AND record_id = ?2 AND at >= ?3 ORDER BY at DESC, rowid DESC LIMIT 200",
        ) else {
            return vec![];
        };
        st.query_map(params![base, rid, cutoff], |r| {
            let p = |s: Option<String>| s.and_then(|s| serde_json::from_str(&s).ok());
            Ok((r.get::<_, String>(0)?, p(r.get(1)?), p(r.get(2)?), r.get::<_, i64>(3)?))
        })
        .map(|it| it.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
    })
}

/// 清空一个多维表格的修改历史。
pub fn clear_history(base: &str) {
    if let Some(store) = store() {
        let base = base.to_string();
        store.write(None, move |c| c.execute("DELETE FROM base_history WHERE base_id = ?1", [base]).map(|_| ()));
    }
}

/// 读一类行。解析不了的行跳过（其余照常显示），数量累加到 `bad`，由调用方上报。
fn rows<T: serde::de::DeserializeOwned>(c: &rusqlite::Connection, sql: &str, base: &str, tid: &str, bad: &mut usize) -> Vec<T> {
    let mut st = match c.prepare(sql) {
        Ok(st) => st,
        Err(_) => {
            *bad += 1;
            return vec![];
        }
    };
    let Ok(it) = st.query_map(params![base, tid], |r| r.get::<_, String>(0)) else {
        *bad += 1;
        return vec![];
    };
    let mut out = vec![];
    for s in it {
        match s.ok().and_then(|s| crate::assets::decode(s.as_bytes()).ok()) {
            Some(v) => out.push(v),
            None => *bad += 1,
        }
    }
    out
}

/// 从数据库装载整个多维表格。
pub fn load(id: &str, meta: BaseMeta) -> Base {
    let t0 = std::time::Instant::now();
    let b = load_inner(id, meta);
    super::stress::record(&super::stress::LOAD_US, t0);
    b
}

fn load_inner(id: &str, meta: BaseMeta) -> Base {
    let Some(store) = store() else { return Base::from_parts(id, meta, vec![]) };
    store.flush();
    let mut bad = 0usize;
    let mut unreadable = vec![];
    let tables = store.with_conn(|c| {
        let mut out = vec![];
        for tid in &meta.tables {
            let tm: Option<TableMeta> = c
                .query_row("SELECT data FROM base_tables WHERE base_id = ?1 AND table_id = ?2", params![id, tid], |r| r.get::<_, String>(0))
                .ok()
                .and_then(|s| crate::assets::decode(s.as_bytes()).ok());
            let Some(tm) = tm else {
                unreadable.push(tid.clone());
                continue;
            };
            let fields: Vec<Field> = rows(c, "SELECT data FROM base_fields WHERE base_id = ?1 AND table_id = ?2", id, tid, &mut bad);
            let records: Vec<Record> = rows(c, "SELECT data FROM base_records WHERE base_id = ?1 AND table_id = ?2", id, tid, &mut bad);
            let views: Vec<View> = rows(c, "SELECT data FROM base_views WHERE base_id = ?1 AND table_id = ?2", id, tid, &mut bad);
            out.push(Table::assemble(tm, fields, records, views));
        }
        out
    });
    if bad > 0 || !unreadable.is_empty() {
        let mut msg = format!("多维表格「{}」部分数据无法读取：", meta.name);
        if !unreadable.is_empty() {
            msg.push_str(&format!("{} 张数据表暂不显示", unreadable.len()));
        }
        if bad > 0 {
            msg.push_str(&format!("{}{bad} 行字段 / 记录 / 视图被跳过", if unreadable.is_empty() { "" } else { "，" }));
        }
        msg.push_str("。原始数据仍保留在数据库中，未做修改。");
        store.report_error(msg);
    }
    // 显示用的元数据去掉读不出来的表，但它们记在 `unreadable` 里，保存元数据时补回去
    let mut shown = meta;
    shown.tables.retain(|t| !unreadable.contains(t));
    let mut b = Base::from_parts(id, shown, tables);
    b.unreadable = unreadable;
    b
}

/// 删除一个多维表格的全部行（资源回收用）。
pub fn delete(store: &Store, id: &str) {
    let id = id.to_string();
    store.write(None, move |c| {
        let tx = c.transaction()?;
        for t in ["base_tables", "base_fields", "base_records", "base_views", "base_history"] {
            tx.execute(&format!("DELETE FROM {t} WHERE base_id = ?1"), [&id])?;
        }
        tx.commit()
    });
}
