//! 本地存储：SQLite（~/Stonebrook/stonebrook.db）。
//! 文档内容以 JSON 存一列，另存一份纯文本用于全文搜索。
//!
//! 写入模型：所有写操作都进同一个后台写线程的有序队列（单写者，顺序与提交顺序一致）。
//! 带 key 的任务（文档 / 嵌入对象保存）在队列里按 key 合并：撤掉排队中的旧快照，新快照追加到队尾，
//! 因此合并后的任务总是在它之前提交的所有任务之后执行。
//! 界面线程用另一条连接读数据（`PRAGMA query_only`，WAL 下读写互不阻塞），写操作在这条连接上会直接报错。
//! 需要「写完立刻读到」的地方用 `write_sync`，退出前用 `flush` 等队列写完。
//!
//! 故障隔离：写任务 panic 会被捕获并记为写入错误，写线程继续工作；`flush` 有超时，不会让界面线程无限等待。

use crate::model::DocContent;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Params, Row};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// `flush` 最长等待时间：超过就放弃等待并报错，避免写线程卡住时界面线程跟着卡死。
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// 用户可见的品牌名。
pub const APP_NAME: &str = "石渠";
pub const APP_NAME_EN: &str = "Stonebrook";
/// 数据目录名与数据库文件名。
pub const DATA_DIR_NAME: &str = "Stonebrook";
pub const DB_FILE: &str = "stonebrook.db";
pub const DATA_DIR_ENV: &str = "STONEBROOK_DATA_DIR";

/// 测试里改 `STONEBROOK_DATA_DIR`（进程级环境变量）前先拿这把锁，避免并行的测试互相覆盖。
#[cfg(test)]
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Debug)]
pub struct DocMeta {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
    pub updated: i64,
    pub deleted: bool,
}

/// 一条评论：挂在某篇文档的某个块上。
#[derive(Clone, Debug)]
pub struct Comment {
    pub id: String,
    /// 块在文档中的序号（保存时按 `block_id` 回写，块被插入 / 删除 / 移动后依然跟着原块）
    pub block: usize,
    /// 运行期的块 id（`Block::id` 不落盘，加载时按序号映射，编辑过程中靠它跟踪块）
    pub block_id: u64,
    /// 选中文字的引用快照，便于定位
    pub anchor: String,
    pub body: String,
    pub author: String,
    pub created: i64,
    pub resolved: bool,
}

/// 网页卡片缓存的一行。
#[derive(Clone, Debug, Default)]
pub struct LinkCardRow {
    pub title: String,
    pub descr: String,
    pub image: String,
    pub favicon: String,
    pub site: String,
    pub ok: bool,
}

/// 文档 JSON 解析失败：原始数据已备份到 `backup`。
#[derive(Clone, Debug)]
pub struct Corrupt {
    pub backup: PathBuf,
}

type Job = Box<dyn FnOnce(&mut Connection) -> rusqlite::Result<()> + Send>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<(Option<String>, Job)>,
    busy: bool,
}

/// 后台写线程。
struct Writer {
    q: Mutex<Queue>,
    cv: Condvar,
    /// 写线程已退出（正常情况下不会发生；置位后 `write` 直接报错、`flush` 不再等待）
    dead: AtomicBool,
}

impl Writer {
    fn new() -> Self {
        Writer { q: Mutex::new(Queue::default()), cv: Condvar::new(), dead: AtomicBool::new(false) }
    }
}

/// 写线程退出时（无论什么原因）置 `dead` 并唤醒所有等待者。
struct DeadOnDrop<'a>(&'a Writer);

impl Drop for DeadOnDrop<'_> {
    fn drop(&mut self) {
        self.0.dead.store(true, Ordering::SeqCst);
        if let Ok(mut q) = self.0.q.lock() {
            q.busy = false;
        }
        self.0.cv.notify_all();
    }
}

/// 把编码失败转成写任务错误（不写入任何数据）。
pub fn encode_error(what: &str, e: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(format!("{what}序列化失败：{e}").into())
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "未知错误".to_string()
    }
}

#[derive(Clone)]
pub struct Store {
    /// 界面线程的只读连接
    conn: Arc<Mutex<Connection>>,
    writer: Arc<Writer>,
    /// 最近一次写库失败的原因（界面层取走后提示一次）
    error: Arc<Mutex<Option<String>>>,
    pub data_dir: PathBuf,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 数据目录：默认 ~/Stonebrook，可用 STONEBROOK_DATA_DIR 覆盖。
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
        return PathBuf::from(dir);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(DATA_DIR_NAME)
}

static GLOBAL: OnceLock<Store> = OnceLock::new();

/// 进程级的存储实例（启动时 `install` 一次；单元测试里可能没有）。
pub fn global() -> Option<&'static Store> {
    GLOBAL.get()
}

/// 安装进程级存储；已安装过则忽略。
pub fn install(store: Store) {
    let _ = GLOBAL.set(store);
}

/// 有序的建表 / 升级脚本：第 i 条把库从版本 i 升到 i+1（版本号存在 `PRAGMA user_version`）。
/// 只能在末尾追加，不能修改已发布的条目。
const MIGRATIONS: &[&str] = &[
    // 1：基线（老库里这些表可能已存在，所以全部 IF NOT EXISTS）
    "CREATE TABLE IF NOT EXISTS docs (
        id TEXT PRIMARY KEY,
        parent TEXT,
        title TEXT NOT NULL DEFAULT '',
        sort REAL NOT NULL DEFAULT 0,
        created INTEGER NOT NULL,
        updated INTEGER NOT NULL,
        deleted INTEGER NOT NULL DEFAULT 0,
        content TEXT NOT NULL,
        plain TEXT NOT NULL DEFAULT ''
     );
     CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
     CREATE TABLE IF NOT EXISTS objects (id TEXT PRIMARY KEY, kind TEXT NOT NULL, data BLOB NOT NULL, updated INTEGER NOT NULL);
     CREATE TABLE IF NOT EXISTS sync_blocks (id TEXT PRIMARY KEY, blocks TEXT NOT NULL, source TEXT, updated INTEGER NOT NULL);
     CREATE TABLE IF NOT EXISTS link_cards (url TEXT PRIMARY KEY, title TEXT, descr TEXT, image TEXT, favicon TEXT, site TEXT, fetched_at INTEGER NOT NULL, ok INTEGER NOT NULL);
     CREATE TABLE IF NOT EXISTS templates (id TEXT PRIMARY KEY, name TEXT NOT NULL, content TEXT NOT NULL, created INTEGER NOT NULL);
     CREATE TABLE IF NOT EXISTS comments (
        id TEXT PRIMARY KEY,
        doc TEXT NOT NULL,
        block INTEGER NOT NULL,
        anchor TEXT NOT NULL DEFAULT '',
        body TEXT NOT NULL,
        author TEXT NOT NULL DEFAULT '',
        created INTEGER NOT NULL,
        resolved INTEGER NOT NULL DEFAULT 0
     );
     CREATE INDEX IF NOT EXISTS comments_doc ON comments(doc, resolved);
     CREATE TABLE IF NOT EXISTS base_tables  (base_id TEXT NOT NULL, table_id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(base_id, table_id));
     CREATE TABLE IF NOT EXISTS base_fields  (base_id TEXT NOT NULL, table_id TEXT NOT NULL, field_id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(base_id, field_id));
     CREATE TABLE IF NOT EXISTS base_records (base_id TEXT NOT NULL, table_id TEXT NOT NULL, record_id TEXT NOT NULL, data TEXT NOT NULL,
                                              created INTEGER NOT NULL DEFAULT 0, updated INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(base_id, record_id));
     CREATE TABLE IF NOT EXISTS base_views   (base_id TEXT NOT NULL, table_id TEXT NOT NULL, view_id TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(base_id, view_id));
     CREATE INDEX IF NOT EXISTS base_records_table ON base_records(base_id, table_id);
     CREATE TABLE IF NOT EXISTS base_history (base_id TEXT NOT NULL, record_id TEXT NOT NULL, field_id TEXT NOT NULL,
                                              before TEXT, after TEXT, at INTEGER NOT NULL);
     CREATE INDEX IF NOT EXISTS base_history_rec ON base_history(base_id, record_id, at);",
    // 2：全文索引（trigram 分词，中英文都能做子串匹配；少于 3 个字的查询回退到扫描）
    "CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(id UNINDEXED, title, plain, tokenize = 'trigram');
     INSERT INTO docs_fts (id, title, plain) SELECT id, title, plain FROM docs;",
    // 3：全文索引改为按整数 rowid 关联（`docs.fts` ↔ `docs_fts.rowid`）。
    //    原来按 UNINDEXED 的 id 列删除，FTS5 只能逐行扫描整个索引；按 rowid 删改是 O(log n)。
    //    不直接用 docs 的隐含 rowid：没有 INTEGER PRIMARY KEY 的表在 VACUUM 时 rowid 可能被重排。
    "ALTER TABLE docs ADD COLUMN fts INTEGER;
     UPDATE docs SET fts = rowid;
     CREATE UNIQUE INDEX IF NOT EXISTS docs_fts_rowid ON docs(fts);
     DROP TABLE IF EXISTS docs_fts;
     CREATE VIRTUAL TABLE docs_fts USING fts5(title, plain, tokenize = 'trigram');
     INSERT INTO docs_fts (rowid, title, plain) SELECT fts, title, plain FROM docs;",
];

/// 当前代码支持的库 schema 版本（备份恢复时用来拒绝来自更新版本的备份）。
pub fn schema_version() -> usize {
    MIGRATIONS.len()
}

/// 升级库结构。库版本比代码新时拒绝打开（旧代码读写新库可能损坏数据）；
/// 对已有数据执行迁移前，先把整个库复制到 `recovery/pre-migrate-v<N>-<时间>.db`，复制失败就不迁移。
fn migrate(conn: &mut Connection, data_dir: Option<&Path>) -> Result<()> {
    let ver: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    if ver > MIGRATIONS.len() {
        anyhow::bail!("数据库来自更新版本的石渠（库版本 {ver}，当前应用支持到 {}），请先升级应用", MIGRATIONS.len());
    }
    if ver == MIGRATIONS.len() {
        return Ok(());
    }
    let has_data: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'docs')", [], |r| r.get(0))?;
    if let (true, Some(dir)) = (has_data, data_dir) {
        let rec = dir.join("recovery");
        std::fs::create_dir_all(&rec)?;
        let dst = rec.join(format!("pre-migrate-v{ver}-{}.db", now_ms()));
        conn.execute("VACUUM INTO ?1", [dst.to_string_lossy().as_ref()]).map_err(|e| anyhow::anyhow!("迁移前备份数据库失败（{e}），已取消升级"))?;
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(ver) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn open_conn(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

/// 界面线程用的读连接：`query_only` 让任何误写当场报错，「单写者」由机制而非约定保证。
fn open_read_conn(path: &Path) -> Result<Connection> {
    let conn = open_conn(path)?;
    conn.execute_batch("PRAGMA query_only=ON;")?;
    Ok(conn)
}

impl Writer {
    fn run(&self, mut conn: Connection, error: Arc<Mutex<Option<String>>>) {
        let _guard = DeadOnDrop(self);
        loop {
            let job = {
                let mut q = self.q.lock().unwrap_or_else(|e| e.into_inner());
                while q.jobs.is_empty() {
                    q = self.cv.wait(q).unwrap_or_else(|e| e.into_inner());
                }
                q.busy = true;
                q.jobs.pop_front().map(|(_, j)| j)
            };
            if let Some(job) = job {
                // 单个任务 panic（序列化闭包里的 bug 等）不能带走写线程，否则之后的保存全部静默失效
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut conn)));
                let msg = match res {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("保存失败：{e}")),
                    Err(p) => Some(format!("保存失败（内部错误）：{}", panic_message(&*p))),
                };
                if let Some(msg) = msg {
                    eprintln!("{msg}");
                    *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
                }
            }
            self.q.lock().unwrap_or_else(|e| e.into_inner()).busy = false;
            self.cv.notify_all();
        }
    }
}

impl Store {
    pub fn open() -> Result<Self> {
        let data_dir = data_dir();
        std::fs::create_dir_all(data_dir.join("assets"))?;
        let path = data_dir.join(DB_FILE);
        let mut wconn = open_conn(&path)?;
        migrate(&mut wconn, Some(&data_dir))?;
        let conn = open_read_conn(&path)?;
        Self::start(wconn, conn, data_dir)
    }

    /// 在指定目录打开（测试用，不碰进程级环境变量）。
    #[cfg(test)]
    pub fn open_in(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir.join("assets"))?;
        let mut w = open_conn(&dir.join(DB_FILE))?;
        migrate(&mut w, Some(dir))?;
        let conn = open_read_conn(&dir.join(DB_FILE))?;
        Self::start(w, conn, dir.to_path_buf())
    }

    fn start(wconn: Connection, conn: Connection, data_dir: PathBuf) -> Result<Self> {
        let writer = Arc::new(Writer::new());
        let error = Arc::new(Mutex::new(None));
        {
            let (w, e) = (writer.clone(), error.clone());
            std::thread::Builder::new().name("stonebrook-writer".into()).spawn(move || w.run(wconn, e))?;
        }
        Ok(Store { conn: Arc::new(Mutex::new(conn)), writer, error, data_dir })
    }

    /// 生成整库一致快照（备份导出用）。用独立连接执行，不占用界面线程的读连接。
    pub fn snapshot_to(&self, dst: &Path) -> Result<()> {
        let c = open_conn(&self.data_dir.join(DB_FILE))?;
        c.execute("VACUUM INTO ?1", [dst.to_string_lossy().as_ref()])?;
        Ok(())
    }

    /// 打开一条独立的只读连接（后台线程做大查询用，不和界面线程抢同一把锁）。
    pub fn open_reader(&self) -> Result<Connection> {
        open_read_conn(&self.data_dir.join(DB_FILE))
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.data_dir.join("assets")
    }

    // ───────────── 基础设施 ─────────────

    /// 只读连接（界面线程查询用）。写操作请走 `write` / `write_sync`。
    pub fn with_conn<R>(&self, f: impl FnOnce(&mut Connection) -> R) -> R {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut conn)
    }

    /// 提交一个写任务（后台按顺序执行）。
    /// 带 `key` 的任务若队列里已有同 key 的未执行任务，撤掉旧的、新的追加到队尾（只写最新的，且不越过之前提交的任务）。
    pub fn write(&self, key: Option<String>, job: impl FnOnce(&mut Connection) -> rusqlite::Result<()> + Send + 'static) {
        if self.writer.dead.load(Ordering::SeqCst) {
            self.report_error("保存失败：写入线程已停止，请重启应用（未保存的修改仍在内存中）");
            return;
        }
        let mut q = self.writer.q.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = &key {
            if let Some(i) = q.jobs.iter().position(|(x, _)| x.as_ref() == Some(k)) {
                q.jobs.remove(i);
            }
        }
        q.jobs.push_back((key, Box::new(job)));
        drop(q);
        self.writer.cv.notify_all();
    }

    /// 提交写任务并等它（以及之前排队的任务）写完。
    pub fn write_sync(&self, job: impl FnOnce(&mut Connection) -> rusqlite::Result<()> + Send + 'static) {
        self.write(None, job);
        self.flush();
    }

    /// 等待写队列清空（退出前、需要读到刚写入的数据前调用）。返回是否在 `FLUSH_TIMEOUT` 内写完。
    pub fn flush(&self) -> bool {
        self.flush_timeout(FLUSH_TIMEOUT)
    }

    pub fn flush_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut q = self.writer.q.lock().unwrap_or_else(|e| e.into_inner());
        while !q.jobs.is_empty() || q.busy {
            if self.writer.dead.load(Ordering::SeqCst) {
                drop(q);
                self.report_error("保存失败：写入线程已停止，请重启应用");
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                drop(q);
                self.report_error("保存仍在进行中，等待超时（数据可能尚未写入磁盘）");
                return false;
            }
            q = self.writer.cv.wait_timeout(q, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }

    /// 取走最近一次存储错误（写入失败、读取失败、对象损坏等，界面层轮询后提示一次）。
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// 记录一个需要让用户看到的存储错误。
    pub fn report_error(&self, msg: impl Into<String>) {
        let msg = msg.into();
        eprintln!("{msg}");
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
    }

    /// 查询多行。出错时返回空并进错误通道（否则「查询失败」和「没有数据」无法区分，用户会以为数据没了）。
    fn rows<T>(&self, sql: &str, p: impl Params, f: impl FnMut(&Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
        let res = self.with_conn(|c| {
            c.prepare_cached(sql).and_then(|mut st| {
                st.query_map(p, f).map(|it| {
                    // 单行读不出来时跳过这一行（其余照常显示），但要让用户知道
                    let (mut ok, mut bad) = (vec![], None);
                    for r in it {
                        match r {
                            Ok(v) => ok.push(v),
                            Err(e) => bad = Some(e),
                        }
                    }
                    (ok, bad)
                })
            })
        });
        match res {
            Ok((v, None)) => v,
            Ok((v, Some(e))) => {
                self.report_error(format!("部分数据读取失败：{e}"));
                v
            }
            Err(e) => {
                self.report_error(format!("读取数据失败：{e}"));
                vec![]
            }
        }
    }

    /// 查询一行（出错时返回 None 并进错误通道）。
    fn row<T>(&self, sql: &str, p: impl Params, f: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>) -> Option<T> {
        self.try_row(sql, p, f).unwrap_or_else(|e| {
            self.report_error(format!("读取数据失败：{e}"));
            None
        })
    }

    /// 查询一行，区分「没有这行」(`Ok(None)`) 与「读取失败」(`Err`)。
    fn try_row<T>(&self, sql: &str, p: impl Params, f: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>) -> rusqlite::Result<Option<T>> {
        self.with_conn(|c| c.query_row(sql, p, f).optional())
    }

    // ───────────── 文档 ─────────────

    /// 正文里链接到 `id` 的文档（不含回收站和它自己）：(ID, 标题)，最近修改的在前。
    /// 文档链接存为 `stonebrook://doc/<id>`，直接在正文 JSON 里查。
    pub fn backlinks(&self, id: &str) -> Vec<(String, String)> {
        if id.is_empty() || id.contains(['%', '_']) {
            return vec![];
        }
        let pat = format!("%{}{id}%", crate::editor::DOC_LINK_PREFIX);
        self.rows("SELECT id, title FROM docs WHERE deleted = 0 AND id != ?1 AND content LIKE ?2 ORDER BY updated DESC", params![id, pat], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
    }

    /// 未链接的提及：正文里出现了 `title`、却没有链接到 `id` 的文档：(ID, 标题)。标题少于 2 个字不查（噪声太多）。
    pub fn unlinked_mentions(&self, id: &str, title: &str) -> Vec<(String, String)> {
        let title = title.trim();
        if title.chars().count() < 2 || id.is_empty() || id.contains(['%', '_']) {
            return vec![];
        }
        let link = format!("{}{id}", crate::editor::DOC_LINK_PREFIX);
        self.rows(
            "SELECT id, title FROM docs WHERE deleted = 0 AND id != ?1 AND instr(plain, ?2) > 0 AND instr(content, ?3) = 0 ORDER BY updated DESC LIMIT 30",
            params![id, title, link],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// 把文档 `doc` 里第一处未加链接的 `title` 改成指向 `target` 的文档链接。返回是否改动。
    pub fn link_mention(&self, doc: &str, target: &str, title: &str) -> bool {
        self.flush();
        let Some((mut content, None)) = self.load(doc) else { return false };
        let url = format!("{}{target}", crate::editor::DOC_LINK_PREFIX);
        for b in content.blocks.iter_mut().skip(1) {
            let n = b.slot_count();
            for c in 0..n {
                let Some(t) = b.slot(c) else { continue };
                if b.kind.is_source() || matches!(b.kind, crate::model::BlockKind::Code { .. }) {
                    break;
                }
                let hit =
                    t.text.match_indices(title).map(|(i, _)| i).find(|&i| !t.runs().any(|(r, m)| m.link.is_some() && r.start < i + title.len() && i < r.end));
                if let Some(i) = hit {
                    let blk = std::sync::Arc::make_mut(b);
                    if let Some(t) = blk.slot_mut(c) {
                        t.update_marks(i..i + title.len(), |m| m.link = Some(url.clone()));
                    }
                    self.save(doc, &content);
                    return true;
                }
            }
        }
        false
    }

    /// 文档之间的链接（图谱用）：(源, 目标)，不含回收站里的文档。
    pub fn link_edges(&self) -> Vec<(String, String)> {
        self.flush();
        let re = regex::Regex::new(&format!("{}([0-9A-Za-z-]+)", regex::escape(crate::editor::DOC_LINK_PREFIX))).expect("regex");
        let docs: Vec<(String, String)> = self.rows("SELECT id, content FROM docs WHERE deleted = 0", [], |r| Ok((r.get(0)?, r.get(1)?)));
        let mut out = vec![];
        for (id, content) in docs {
            let mut seen = std::collections::HashSet::new();
            for cap in re.captures_iter(&content) {
                let to = cap[1].to_string();
                if to != id && seen.insert(to.clone()) {
                    out.push((id.clone(), to));
                }
            }
        }
        out
    }

    pub fn list(&self) -> Vec<DocMeta> {
        self.rows("SELECT id, parent, title, updated, deleted FROM docs ORDER BY sort, created", [], |r| {
            Ok(DocMeta { id: r.get(0)?, parent: r.get(1)?, title: r.get(2)?, updated: r.get(3)?, deleted: r.get::<_, i64>(4)? != 0 })
        })
    }

    /// 读取文档。JSON 损坏时把原文备份到 `recovery/` 目录，返回一篇说明情况的文档和 `Corrupt`。
    pub fn load(&self, id: &str) -> Option<(DocContent, Option<Corrupt>)> {
        let raw: String = self.row("SELECT content FROM docs WHERE id = ?1", [id], |r| r.get(0))?;
        match DocContent::try_from_json(&raw) {
            Ok(c) => Some((c, None)),
            Err(e) => {
                let dir = self.data_dir.join("recovery");
                let backup = dir.join(format!("{id}-{}.json", now_ms()));
                let saved = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&backup, &raw));
                if let Err(io) = saved {
                    eprintln!("备份损坏文档失败：{io}");
                    return None; // 备份不了就不打开，避免后续保存覆盖原数据
                }
                eprintln!("文档 {id} 解析失败（{e}），原文已备份到 {}", backup.display());
                Some((DocContent::corrupt_notice(&backup.to_string_lossy()), Some(Corrupt { backup })))
            }
        }
    }

    pub fn create(&self, parent: Option<&str>, content: &DocContent) -> String {
        let id = new_uuid();
        let now = now_ms();
        let (pid, title, plain) = (parent.map(str::to_string), content.title(), content.plain_text());
        let json = content.try_to_json();
        let doc = id.clone();
        self.write_sync(move |c| {
            let json = json.map_err(|e| encode_error("文档", e))?;
            let tx = c.transaction()?;
            let (sort, fts): (f64, i64) =
                tx.query_row("SELECT COALESCE(MAX(sort), 0) + 1, COALESCE(MAX(fts), 0) + 1 FROM docs", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            tx.execute(
                "INSERT INTO docs (id, parent, title, sort, created, updated, content, plain, fts) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
                params![doc, pid, title, sort, now, json, plain, fts],
            )?;
            tx.execute("INSERT INTO docs_fts (rowid, title, plain) VALUES (?1, ?2, ?3)", params![fts, title, plain])?;
            tx.commit()
        });
        id
    }

    /// 保存文档：放进写队列，同一文档未写的旧快照会被替换。序列化在写线程里做；序列化失败时不写入（不会用空内容覆盖）。
    pub fn save(&self, id: &str, content: &DocContent) {
        let (doc, content) = (id.to_string(), content.clone());
        self.write(Some(format!("doc:{id}")), move |c| {
            let json = content.try_to_json().map_err(|e| encode_error("文档", e))?;
            let (title, plain) = (content.title(), content.plain_text());
            let tx = c.transaction()?;
            let n =
                tx.execute("UPDATE docs SET title = ?2, content = ?3, plain = ?4, updated = ?5 WHERE id = ?1", params![doc, title, json, plain, now_ms()])?;
            if n == 0 {
                return tx.commit(); // 文档已被永久删除
            }
            let fts: Option<i64> = tx.query_row("SELECT fts FROM docs WHERE id = ?1", [&doc], |r| r.get(0)).optional()?.flatten();
            let fts = match fts {
                Some(n) => {
                    tx.execute("DELETE FROM docs_fts WHERE rowid = ?1", [n])?;
                    n
                }
                None => {
                    let n: i64 = tx.query_row("SELECT COALESCE(MAX(fts), 0) + 1 FROM docs", [], |r| r.get(0))?;
                    tx.execute("UPDATE docs SET fts = ?2 WHERE id = ?1", params![doc, n])?;
                    n
                }
            };
            tx.execute("INSERT INTO docs_fts (rowid, title, plain) VALUES (?1, ?2, ?3)", params![fts, title, plain])?;
            tx.commit()
        });
    }

    pub fn set_deleted(&self, id: &str, deleted: bool) {
        let id = id.to_string();
        // 连同子文档一起移入 / 移出回收站
        self.write_sync(move |c| {
            c.execute(
                "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT d.id FROM docs d JOIN sub ON d.parent = sub.id)
                 UPDATE docs SET deleted = ?2 WHERE id IN sub",
                params![id, deleted as i64],
            )
            .map(|_| ())
        });
    }

    /// 永久删除文档（连同子文档与它们的评论）。
    pub fn purge(&self, id: &str) {
        let id = id.to_string();
        self.write_sync(move |c| {
            const SUB: &str = "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT d.id FROM docs d JOIN sub ON d.parent = sub.id)";
            let tx = c.transaction()?;
            // 先删评论和索引（依赖 docs 里的父子关系），最后删文档本身
            tx.execute(&format!("{SUB} DELETE FROM comments WHERE doc IN sub"), [&id])?;
            tx.execute(&format!("{SUB} DELETE FROM docs_fts WHERE rowid IN (SELECT fts FROM docs WHERE id IN sub AND fts IS NOT NULL)"), [&id])?;
            tx.execute(&format!("{SUB} DELETE FROM docs WHERE id IN sub"), [&id])?;
            tx.commit()
        });
    }

    /// 全文搜索：标题 / 正文子串匹配，返回 (id, title, 片段)。
    /// 3 个字及以上走 FTS5 trigram 索引；更短的查询（如两个汉字）回退到逐篇扫描。
    pub fn search(&self, q: &str, limit: usize) -> Vec<(String, String, String)> {
        let q = q.trim();
        if q.is_empty() {
            return vec![];
        }
        let map = |r: &Row<'_>| {
            let plain: String = r.get(2)?;
            Ok((r.get(0)?, r.get(1)?, snippet(&plain, q)))
        };
        if q.chars().count() >= 3 {
            // 整体作为一个短语，转义双引号
            let phrase = format!("\"{}\"", q.replace('"', "\"\""));
            return self.rows(
                "SELECT d.id, d.title, d.plain FROM docs_fts f JOIN docs d ON d.fts = f.rowid
                 WHERE docs_fts MATCH ?1 AND d.deleted = 0
                 ORDER BY (instr(lower(d.title), lower(?2)) > 0) DESC, d.updated DESC LIMIT ?3",
                params![phrase, q, limit as i64],
                map,
            );
        }
        self.rows(
            "SELECT id, title, plain, instr(lower(title), lower(?1)) AS t, instr(lower(plain), lower(?1)) AS p
             FROM docs WHERE deleted = 0 AND (t > 0 OR p > 0)
             ORDER BY (t > 0) DESC, updated DESC LIMIT ?2",
            params![q, limit as i64],
            map,
        )
    }

    /// 可能引用嵌入对象的全部 JSON：文档（含回收站）、用户模板、同步块（资源回收时扫描引用用）。
    /// 用独立连接读（后台回收时不占界面线程的读连接）；任何一行读失败都返回错误——
    /// 引用集合不完整时回收会误删仍在使用的对象，宁可这次不回收。
    pub fn reference_texts(&self) -> Result<Vec<String>> {
        self.flush();
        let c = self.open_reader()?;
        let mut st = c.prepare("SELECT content FROM docs UNION ALL SELECT content FROM templates UNION ALL SELECT blocks FROM sync_blocks")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    /// 资源回收用：数据库里所有可能引用 `assets/` 文件的文本（含二进制对象里的 JSON 片段）。
    /// 任何一处读失败都返回错误（引用不完整时回收会误删）。
    pub fn asset_reference_texts(&self) -> Result<Vec<String>> {
        self.flush();
        let c = self.open_reader()?;
        let mut out = vec![];
        for sql in [
            "SELECT content FROM docs",
            "SELECT content FROM templates",
            "SELECT blocks FROM sync_blocks",
            "SELECT data FROM objects",
            "SELECT data FROM base_tables",
            "SELECT data FROM base_fields",
            "SELECT data FROM base_records",
            "SELECT data FROM base_views",
            "SELECT COALESCE(image, '') || ' ' || COALESCE(favicon, '') FROM link_cards",
            "SELECT v FROM kv",
        ] {
            let mut st = c.prepare(sql)?;
            // 电子表格对象是二进制（扩展 JSON 在末尾），按字节读再有损转文本
            let rows = st.query_map([], |r| {
                Ok(match r.get_ref(0)? {
                    rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => String::from_utf8_lossy(b).into_owned(),
                    _ => String::new(),
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    // ───────────── 嵌入对象 ─────────────

    /// 保存对象：同一对象未写的旧版本会被替换。`encode` 在写线程里执行；编码失败时不写入（不会用空内容覆盖原数据）。
    pub fn save_object_with(&self, id: &str, kind: &str, encode: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static) {
        let (oid, kind) = (id.to_string(), kind.to_string());
        self.write(Some(format!("obj:{id}")), move |c| {
            let data = encode().map_err(|e| encode_error("嵌入对象", e))?;
            if data.is_empty() {
                return Err(encode_error("嵌入对象", "编码结果为空"));
            }
            c.execute("INSERT OR REPLACE INTO objects (id, kind, data, updated) VALUES (?1, ?2, ?3, ?4)", params![oid, kind, data, now_ms()]).map(|_| ())
        });
    }

    pub fn save_object(&self, id: &str, kind: &str, data: Result<Vec<u8>, String>) {
        self.save_object_with(id, kind, move || data);
    }

    /// 读对象原始字节：`Ok(None)` 表示不存在，`Err` 表示读取失败（两者必须区分，否则读失败会被当成「新对象」覆盖）。
    pub fn load_object(&self, id: &str) -> rusqlite::Result<Option<(String, Vec<u8>)>> {
        self.flush();
        self.try_row("SELECT kind, data FROM objects WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
    }

    /// 对象的 (id, kind, 更新时间)。
    pub fn object_ids(&self) -> Vec<(String, String, i64)> {
        self.rows("SELECT id, kind, updated FROM objects", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    }

    /// 对象的原始字节（不经过缓存，元数据浏览用）。
    pub fn objects_of_kind(&self, kind: &str) -> Vec<(String, Vec<u8>)> {
        self.flush();
        self.rows("SELECT id, data FROM objects WHERE kind = ?1 ORDER BY updated DESC", [kind], |r| Ok((r.get(0)?, r.get(1)?)))
    }

    pub fn delete_object(&self, id: &str) {
        let (oid, key) = (id.to_string(), format!("obj:{id}"));
        // 先撤掉还没写的保存，否则删完又被写回来
        self.writer.q.lock().unwrap_or_else(|e| e.into_inner()).jobs.retain(|(k, _)| k.as_ref() != Some(&key));
        self.write(None, move |c| c.execute("DELETE FROM objects WHERE id = ?1", [oid]).map(|_| ()));
    }

    // ───────────── 同步块 ─────────────

    pub fn save_sync(&self, id: &str, blocks_json: &str, source: &str) {
        if blocks_json.is_empty() {
            self.report_error("保存同步块失败：内容序列化失败，已跳过本次写入");
            return;
        }
        let (sid, json, src) = (id.to_string(), blocks_json.to_string(), source.to_string());
        self.write(Some(format!("sync:{id}")), move |c| {
            c.execute("INSERT OR REPLACE INTO sync_blocks (id, blocks, source, updated) VALUES (?1, ?2, ?3, ?4)", params![sid, json, src, now_ms()]).map(|_| ())
        });
    }

    pub fn load_sync(&self, id: &str) -> Option<(String, String)> {
        self.flush();
        self.row("SELECT blocks, source FROM sync_blocks WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))
    }

    /// 同步块列表：(id, 来源文档 id, 预览文字, 更新时间)
    pub fn list_sync(&self) -> Vec<(String, String, String, i64)> {
        self.flush();
        self.rows("SELECT id, source, blocks, updated FROM sync_blocks ORDER BY updated DESC", [], |r| {
            let json: String = r.get(2)?;
            let preview = crate::model::blocks_from_json(&json).map(|b| b.iter().map(|x| x.plain_text()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), preview.chars().take(60).collect(), r.get(3)?))
        })
    }

    // ───────────── 网页卡片缓存 ─────────────

    pub fn save_link_card(&self, url: &str, card: &LinkCardRow) {
        let (url, c) = (url.to_string(), card.clone());
        self.write(Some(format!("card:{url}")), move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO link_cards (url, title, descr, image, favicon, site, fetched_at, ok) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![url, c.title, c.descr, crate::assets::to_ref(&c.image), crate::assets::to_ref(&c.favicon), c.site, now_ms(), c.ok as i64],
            )
            .map(|_| ())
        });
    }

    pub fn load_link_card(&self, url: &str) -> Option<LinkCardRow> {
        self.row("SELECT title, descr, image, favicon, site, ok FROM link_cards WHERE url = ?1", [url], |r| {
            let s = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
            Ok(LinkCardRow {
                title: s(0)?,
                descr: s(1)?,
                image: crate::assets::resolve(&s(2)?),
                favicon: crate::assets::resolve(&s(3)?),
                site: s(4)?,
                ok: r.get::<_, i64>(5)? != 0,
            })
        })
    }

    // ───────────── 模板 ─────────────

    pub fn save_template(&self, name: &str, content_json: &str) -> String {
        let id = new_uuid();
        if content_json.is_empty() {
            self.report_error("保存模板失败：内容序列化失败");
            return id;
        }
        let (tid, name, json) = (id.clone(), name.to_string(), content_json.to_string());
        self.write_sync(move |c| {
            c.execute("INSERT INTO templates (id, name, content, created) VALUES (?1, ?2, ?3, ?4)", params![tid, name, json, now_ms()]).map(|_| ())
        });
        id
    }

    pub fn list_templates(&self) -> Vec<(String, String, String)> {
        self.rows("SELECT id, name, content FROM templates ORDER BY created DESC", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    }

    pub fn delete_template(&self, id: &str) {
        let id = id.to_string();
        self.write_sync(move |c| c.execute("DELETE FROM templates WHERE id = ?1", [id]).map(|_| ()));
    }

    // ───────────── 评论 ─────────────

    /// 某文档的评论（未解决的在前，按创建时间）。
    pub fn comments(&self, doc: &str) -> Vec<Comment> {
        self.flush();
        self.rows("SELECT id, block, anchor, body, author, created, resolved FROM comments WHERE doc = ?1 ORDER BY resolved, created", [doc], |r| {
            Ok(Comment {
                id: r.get(0)?,
                block: r.get::<_, i64>(1)?.max(0) as usize,
                block_id: 0,
                anchor: r.get(2)?,
                body: r.get(3)?,
                author: r.get(4)?,
                created: r.get(5)?,
                resolved: r.get::<_, i64>(6)? != 0,
            })
        })
    }

    pub fn add_comment(&self, doc: &str, block: usize, anchor: &str, body: &str, author: &str) -> Comment {
        let c = Comment {
            id: new_uuid(),
            block,
            block_id: 0,
            anchor: anchor.to_string(),
            body: body.to_string(),
            author: author.to_string(),
            created: now_ms(),
            resolved: false,
        };
        let (x, doc) = (c.clone(), doc.to_string());
        self.write(None, move |conn| {
            conn.execute(
                "INSERT INTO comments (id, doc, block, anchor, body, author, created, resolved) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                params![x.id, doc, block as i64, x.anchor, x.body, x.author, x.created],
            )
            .map(|_| ())
        });
        c
    }

    pub fn set_comment_resolved(&self, id: &str, resolved: bool) {
        let id = id.to_string();
        self.write(None, move |c| c.execute("UPDATE comments SET resolved = ?2 WHERE id = ?1", params![id, resolved as i64]).map(|_| ()));
    }

    /// 评论所在块的序号变了（块被移动 / 前面插入删除了块）。
    pub fn set_comment_block(&self, id: &str, block: usize) {
        let id = id.to_string();
        self.write(None, move |c| c.execute("UPDATE comments SET block = ?2 WHERE id = ?1", params![id, block as i64]).map(|_| ()));
    }

    pub fn delete_comment(&self, id: &str) {
        let id = id.to_string();
        self.write(None, move |c| c.execute("DELETE FROM comments WHERE id = ?1", [id]).map(|_| ()));
    }

    // ───────────── 键值 ─────────────

    pub fn get_kv(&self, k: &str) -> Option<String> {
        self.row("SELECT v FROM kv WHERE k = ?1", [k], |r| r.get(0))
    }

    pub fn set_kv(&self, k: &str, v: &str) {
        let (k, v) = (k.to_string(), v.to_string());
        self.write(Some(format!("kv:{k}")), move |c| c.execute("INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)", params![k, v]).map(|_| ()));
    }

    /// 在写线程里原子地给整数值加上 `delta`（读改写不会因为排队而丢计数）。
    pub fn add_kv_int(&self, k: &str, delta: i64) {
        let k = k.to_string();
        self.write(None, move |c| {
            c.execute("INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = CAST(CAST(v AS INTEGER) + ?2 AS TEXT)", params![k, delta])
                .map(|_| ())
        });
    }
}

/// 从一堆文本里收集所有形如 32 位十六进制的 id（对象 id 都是无连字符的 UUID）。
pub fn collect_ids(texts: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for t in texts {
        let b = t.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_hexdigit() {
                let s = i;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                if i - s == 32 {
                    out.insert(t[s..i].to_string());
                }
            } else {
                i += 1;
            }
        }
    }
    out
}

/// 取匹配位置附近的一段文字作为搜索结果摘要（按字符逐个比较，避免大小写转换改变字节长度后错位）。
fn snippet(plain: &str, q: &str) -> String {
    let hay: Vec<char> = plain.chars().collect();
    let needle: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
    let lower: Vec<char> = hay.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let pos = (!needle.is_empty() && needle.len() <= lower.len()).then(|| lower.windows(needle.len()).position(|w| w == needle.as_slice())).flatten();
    let start = pos.map_or(0, |p| p.saturating_sub(20));
    let take = if pos.is_some() { 70 } else { 60 };
    hay[start..].iter().take(take).collect::<String>().replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_ids_finds_only_32_hex_tokens() {
        let a = "a".repeat(32);
        let ids = collect_ids(&[format!(r#"{{"kind":"board","id":"{a}"}} 123 {}"#, "b".repeat(33))]);
        assert!(ids.contains(&a));
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn snippet_centers_on_match() {
        let s = format!("{}关键字{}", "前".repeat(40), "后".repeat(40));
        let sn = snippet(&s, "关键字");
        assert!(sn.starts_with(&"前".repeat(20)));
        assert!(sn.contains("关键字"));
    }

    #[test]
    fn backlinks_find_documents_that_link_here() {
        use crate::model::{Block, BlockKind, RichText};
        let _env = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("stonebrook-backlinks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var(DATA_DIR_ENV, &dir);
        let s = Store::open().unwrap();
        let target = s.create(None, &DocContent::new_empty("被引用的文档"));
        let link = |title: &str| {
            let r = RichText::styled_link("被引用的文档", &format!("{}{target}", crate::editor::DOC_LINK_PREFIX));
            DocContent::from_blocks(vec![Block::new(BlockKind::Title, RichText::plain(title)), Block::new(BlockKind::Text, r)])
        };
        let a = s.create(None, &link("引用者甲"));
        let b = s.create(None, &link("引用者乙（在回收站）"));
        let _c = s.create(None, &DocContent::new_empty("无关文档"));
        s.set_deleted(&b, true);
        s.flush();
        assert_eq!(s.backlinks(&target), vec![(a.clone(), "引用者甲".to_string())]);
        assert!(s.backlinks(&a).is_empty());
        assert!(s.backlinks("%").is_empty(), "LIKE 通配符不能当成 ID");
        std::env::remove_var(DATA_DIR_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlinked_mentions_and_link_them() {
        use crate::model::{Block, BlockKind, RichText};
        let _env = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("stonebrook-mentions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var(DATA_DIR_ENV, &dir);
        let s = Store::open().unwrap();
        let target = s.create(None, &DocContent::new_empty("石渠计划"));
        let doc = |title: &str, body: &str| {
            DocContent::from_blocks(vec![Block::new(BlockKind::Title, RichText::plain(title)), Block::new(BlockKind::Text, RichText::plain(body))])
        };
        let a = s.create(None, &doc("周报", "本周推进石渠计划，下周继续石渠计划"));
        let _b = s.create(None, &doc("无关", "别的内容"));
        s.flush();
        assert_eq!(s.unlinked_mentions(&target, "石渠计划"), vec![(a.clone(), "周报".to_string())]);
        assert!(s.unlinked_mentions(&target, "石").is_empty(), "单字标题不查");
        assert!(s.link_mention(&a, &target, "石渠计划"));
        s.flush();
        assert!(s.unlinked_mentions(&target, "石渠计划").is_empty(), "已链接就不再算未链接提及");
        assert_eq!(s.backlinks(&target), vec![(a.clone(), "周报".to_string())]);
        let (c, _) = s.load(&a).unwrap();
        let runs: Vec<_> = c.blocks[1].text.runs().map(|(r, m)| (c.blocks[1].text.text[r].to_string(), m.link.is_some())).collect();
        assert_eq!(runs[1], ("石渠计划".to_string(), true), "只链接第一处");
        assert!(!runs.last().unwrap().1);
        assert_eq!(s.link_edges(), vec![(a.clone(), target.clone())]);
        std::env::remove_var(DATA_DIR_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 在独立目录里起一个 Store（不碰进程级环境变量）。
    fn temp_store(tag: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("stonebrook-{tag}-{}", new_uuid()));
        (Store::open_in(&dir).unwrap(), dir)
    }

    #[test]
    fn writer_survives_panicking_job() {
        let (s, dir) = temp_store("panic");
        s.write(None, |_| panic!("故意的 panic"));
        assert!(s.flush_timeout(Duration::from_secs(5)), "panic 后 flush 不能卡死");
        let err = s.take_error().expect("panic 要进错误通道");
        assert!(err.contains("故意的 panic"), "{err}");
        // 写线程仍然在工作
        s.set_kv("after", "1");
        assert!(s.flush());
        assert_eq!(s.get_kv("after").as_deref(), Some("1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encode_failure_does_not_overwrite_object() {
        let (s, dir) = temp_store("encode");
        s.save_object("a".repeat(32).as_str(), "board", Ok(b"{\"v\":1}".to_vec()));
        s.flush(); // 否则两次保存会按 key 合并，只执行后一次
        s.save_object("a".repeat(32).as_str(), "board", Err("坏数据".into()));
        s.flush();
        assert!(s.take_error().is_some());
        let (_, bytes) = s.load_object(&"a".repeat(32)).unwrap().unwrap();
        assert_eq!(bytes, b"{\"v\":1}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn coalesced_job_runs_after_earlier_jobs() {
        let (s, dir) = temp_store("order");
        // 堵住写线程，保证后面三个任务同时在队列里
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        s.write(None, move |_| {
            let mut open = g.0.lock().unwrap();
            while !*open {
                open = g.1.wait(open).unwrap();
            }
            Ok(())
        });
        s.set_kv("k", "A1");
        s.write(None, |c| c.execute("UPDATE kv SET v = v || '+B' WHERE k = 'k'", []).map(|_| ()));
        s.set_kv("k", "A2");
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        s.flush();
        // A1 被撤掉，执行顺序为 B、A2：B 时还没有这行，最终是 A2
        assert_eq!(s.get_kv("k").as_deref(), Some("A2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_connection_rejects_writes() {
        let (s, dir) = temp_store("ro");
        let r = s.with_conn(|c| c.execute("INSERT INTO kv (k, v) VALUES ('x', 'y')", []));
        assert!(r.is_err(), "只读连接不能写");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_newer_schema_and_backs_up_before_migrating() {
        let dir = std::env::temp_dir().join(format!("stonebrook-mig-{}", new_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(DB_FILE);
        // 造一个 v2 的库（迁移 1、2），放一篇文档
        {
            let c = open_conn(&path).unwrap();
            for (i, sql) in MIGRATIONS.iter().take(2).enumerate() {
                c.execute_batch(sql).unwrap();
                c.pragma_update(None, "user_version", (i + 1) as i64).unwrap();
            }
            c.execute("INSERT INTO docs (id, title, created, updated, content, plain) VALUES ('d1', '迁移测试文档', 0, 0, '{}', '正文内容在这里')", [])
                .unwrap();
            c.execute("INSERT INTO docs_fts (id, title, plain) VALUES ('d1', '迁移测试文档', '正文内容在这里')", []).unwrap();
        }
        let mut c = open_conn(&path).unwrap();
        migrate(&mut c, Some(&dir)).unwrap();
        let backups: Vec<_> = std::fs::read_dir(dir.join("recovery")).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].file_name().to_string_lossy().starts_with("pre-migrate-v2-"));
        let ver: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver as usize, MIGRATIONS.len());
        let hit: String =
            c.query_row("SELECT d.id FROM docs_fts f JOIN docs d ON d.fts = f.rowid WHERE docs_fts MATCH '\"内容在\"'", [], |r| r.get(0)).unwrap();
        assert_eq!(hit, "d1", "存量文档迁到按 rowid 关联的索引");
        // 库版本比代码新：拒绝打开
        c.pragma_update(None, "user_version", (MIGRATIONS.len() + 1) as i64).unwrap();
        assert!(migrate(&mut c, Some(&dir)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writer_coalesces_and_flushes() {
        let (s, dir) = temp_store("store-test");
        for i in 0..50 {
            s.set_kv("k", &i.to_string());
        }
        s.flush();
        assert_eq!(s.get_kv("k").as_deref(), Some("49"));
        let id = s.create(None, &DocContent::new_empty("测试文档标题"));
        assert_eq!(s.search("文档标", 10).len(), 1, "FTS 命中");
        assert_eq!(s.search("文档", 10).len(), 1, "短查询回退扫描");
        s.save(&id, &DocContent::new_empty("改过的标题内容"));
        s.flush();
        assert!(s.search("文档标", 10).is_empty(), "保存后旧索引被替换");
        assert_eq!(s.search("过的标", 10).len(), 1);
        s.purge(&id);
        assert!(s.search("过的标", 10).is_empty());
        assert!(s.search("文档标", 10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
