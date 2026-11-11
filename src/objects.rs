//! 嵌入对象框架（X1）。
//!
//! 画板（含思维导图）/ 流程图 / 电子表格 / 多维表格这类「大对象」不放进文档 JSON，而是单独存进 objects 表，
//! 文档里只有一个 `Embed { kind, id }` 引用块。内存中有一层缓存，修改时版本号递增，
//! 文档内的预览据此刷新。删除引用块不会立即删除对象（撤销还要用），由资源回收统一清理。
//!
//! 保存：画板是不可变快照（`Arc`），直接交给写线程序列化；电子表格是原地修改的 `Rc<RefCell<_>>`，
//! 只能在界面线程编码，所以先标记为脏、停手 `SHEET_DEBOUNCE` 后由 `flush_dirty` 统一编码入队。

use crate::board::model::BoardDoc;
use crate::store::Store;
use ironcalc::base::UserModel;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type Sheet = Rc<RefCell<UserModel<'static>>>;

/// 电子表格停止修改多久后落盘
pub const SHEET_DEBOUNCE: Duration = Duration::from_millis(400);

/// 对象种类。磁盘与文档 JSON 里仍是字符串（`as_str`），内存里用枚举避免到处比较字符串。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Board,
    Flow,
    Sheet,
    Base,
    Sketch,
}

impl ObjKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "board" => Self::Board,
            "flow" => Self::Flow,
            "sheet" => Self::Sheet,
            "base" => Self::Base,
            "sketch" => Self::Sketch,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Board => "board",
            Self::Flow => "flow",
            Self::Sheet => "sheet",
            Self::Base => "base",
            Self::Sketch => "sketch",
        }
    }
}

#[derive(Clone)]
pub enum ObjData {
    Board(Arc<BoardDoc>),
    Sheet(Sheet),
    /// 多维表格：对象行里只存元数据，字段 / 记录 / 视图在 base_* 表里逐行存
    Base(crate::base::BaseRef),
    /// 草图：官方 `.excalidraw` 结构（不可变快照，写线程序列化）
    Sketch(Arc<crate::sketch::SketchDoc>),
}

struct Entry {
    kind: ObjKind,
    data: ObjData,
    version: u64,
    /// 电子表格：最近一次未落盘修改的时间
    dirty_since: Option<Instant>,
}

/// 内存里最多缓存多少个对象（超出时淘汰最久未用、且没被编辑器持有、没有未落盘修改的）
const CACHE_CAP: usize = 32;

/// 不能淘汰的对象：有未落盘的修改，或数据还被别处（编辑器、内嵌视图）持有——
/// 淘汰后再 `get` 会从数据库读出另一份实例，编辑器手里那份的修改就和缓存脱节了。
fn pinned(_: &String, e: &Entry) -> bool {
    e.dirty_since.is_some()
        || match &e.data {
            ObjData::Board(a) => Arc::strong_count(a) > 1,
            ObjData::Sketch(a) => Arc::strong_count(a) > 1,
            ObjData::Sheet(r) => Rc::strong_count(r) > 1,
            ObjData::Base(r) => Rc::strong_count(r) > 1,
        }
}

thread_local! {
    static OBJS: RefCell<crate::lru::Lru<String, Entry>> = RefCell::new(crate::lru::Lru::with_pin(CACHE_CAP, pinned));
}

thread_local! {
    static VER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// 全局递增的版本号：对象被淘汰后重新放回缓存也不会和旧版本号撞车（预览缓存按版本号失效）。
fn next_ver() -> u64 {
    VER.with(|v| {
        v.set(v.get() + 1);
        v.get()
    })
}

/// 从数据推出对象种类（缓存被淘汰后 `update` 重新放回时用）。
fn kind_of(data: &ObjData) -> ObjKind {
    match data {
        ObjData::Board(b) if b.flow => ObjKind::Flow,
        ObjData::Board(_) => ObjKind::Board,
        ObjData::Sheet(_) => ObjKind::Sheet,
        ObjData::Base(_) => ObjKind::Base,
        ObjData::Sketch(_) => ObjKind::Sketch,
    }
}

pub fn store() -> Option<Store> {
    crate::store::global().cloned()
}

pub fn new_sheet() -> UserModel<'static> {
    let mut m = UserModel::new_empty("Sheet", "en", "UTC", "en").expect("ironcalc");
    m.rename_sheet(0, "Sheet1").ok();
    m
}

/// 电子表格的存储格式：IronCalc 工作簿二进制，后面可以跟一段扩展数据（下拉列表、备注、图片等单元格对象，JSON）。
/// 带扩展数据时以 `EXT_MAGIC` 开头：magic + 工作簿长度（u32 LE）+ 工作簿 + 扩展 JSON；旧数据没有 magic，原样解码。
const EXT_MAGIC: &[u8; 8] = b"\0INKSX1\0";

pub fn sheet_to_bytes(m: &UserModel) -> Vec<u8> {
    let wb = m.to_bytes();
    // 扩展数据里的单元格图片 / 附件路径落盘为相对引用
    let ext = m.get_ext();
    let ext = if ext.is_empty() { String::new() } else { crate::assets::json_text_out(ext) };
    if ext.is_empty() {
        return wb;
    }
    let mut out = Vec::with_capacity(wb.len() + ext.len() + 12);
    out.extend_from_slice(EXT_MAGIC);
    out.extend_from_slice(&(wb.len() as u32).to_le_bytes());
    out.extend_from_slice(&wb);
    out.extend_from_slice(ext.as_bytes());
    out
}

pub fn sheet_from_bytes(bytes: &[u8]) -> Option<UserModel<'static>> {
    if bytes.len() >= 12 && &bytes[..8] == EXT_MAGIC {
        let n = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let wb = bytes.get(12..12 + n)?;
        let mut m = UserModel::from_bytes(wb, "en").ok()?;
        m.load_ext(crate::assets::json_text_in(&String::from_utf8_lossy(&bytes[12 + n..])));
        return Some(m);
    }
    UserModel::from_bytes(bytes, "en").ok()
}

/// 把对象交给写线程保存。画板在写线程里序列化；表格 / 多维表格元数据在这里编码（必须在界面线程）。
/// 编码失败时写线程不写入并上报错误（不会用空内容覆盖原数据）。
fn persist(store: &Store, id: &str, kind: ObjKind, data: &ObjData) {
    let json_err = |e: serde_json::Error| e.to_string();
    match data {
        ObjData::Board(b) => {
            let b = b.clone();
            store.save_object_with(id, kind.as_str(), move || crate::assets::encode(&*b).map_err(json_err));
        }
        ObjData::Sheet(s) => store.save_object(id, kind.as_str(), Ok(sheet_to_bytes(&s.borrow()))),
        ObjData::Base(b) => store.save_object(id, kind.as_str(), b.borrow().meta_bytes().map_err(json_err)),
        ObjData::Sketch(d) => {
            let d = d.clone();
            store.save_object_with(id, kind.as_str(), move || crate::assets::encode(&*d).map_err(json_err));
        }
    }
}

fn decode(id: &str, kind: ObjKind, bytes: &[u8]) -> Result<ObjData, String> {
    let e = |e: serde_json::Error| e.to_string();
    Ok(match kind {
        ObjKind::Base => ObjData::Base(Rc::new(RefCell::new(crate::base::db::load(id, crate::assets::decode(bytes).map_err(e)?)))),
        ObjKind::Board | ObjKind::Flow => ObjData::Board(Arc::new(crate::assets::decode(bytes).map_err(e)?)),
        ObjKind::Sheet => ObjData::Sheet(Rc::new(RefCell::new(sheet_from_bytes(bytes).ok_or("电子表格数据无法解码")?))),
        ObjKind::Sketch => ObjData::Sketch(Arc::new(crate::assets::decode(bytes).map_err(e)?)),
    })
}

/// 新对象 `id` 的默认内容。
fn default_data(id: &str, kind: ObjKind) -> ObjData {
    match kind {
        ObjKind::Sheet => ObjData::Sheet(Rc::new(RefCell::new(new_sheet()))),
        ObjKind::Flow => ObjData::Board(Arc::new(BoardDoc::new(true))),
        ObjKind::Sketch => ObjData::Sketch(Arc::new(crate::sketch::SketchDoc::new())),
        ObjKind::Board => ObjData::Board(Arc::new(BoardDoc::new(false))),
        ObjKind::Base => ObjData::Base(Rc::new(RefCell::new(new_base(id, crate::base::Base::new_default(id, crate::base::ViewKind::Grid))))),
    }
}

/// 多维表格的字段 / 记录 / 视图逐行存在 base_* 表里，新建时先整体写入。
fn new_base(id: &str, mut b: crate::base::Base) -> crate::base::Base {
    debug_assert_eq!(b.id, id);
    b.persist = store().is_some();
    b.save_all();
    b
}

fn insert(id: &str, kind: ObjKind, data: ObjData) {
    if let Some(s) = store() {
        persist(&s, id, kind, &data);
    }
    OBJS.with(|o| o.borrow_mut().insert(id.to_string(), Entry { kind, data, version: next_ver(), dirty_since: None }));
}

/// 新建对象并立即排队落盘。
pub fn create(kind: ObjKind, data: Option<ObjData>) -> String {
    let id = crate::store::new_uuid();
    let data = data.unwrap_or_else(|| default_data(&id, kind));
    insert(&id, kind, data);
    id
}

/// 对象读不出来的原因。
#[derive(Clone, Debug)]
pub enum LoadError {
    /// 没有存储（单元测试）
    NoStore,
    /// 数据库里没有这个对象
    Missing,
    /// 读数据库失败（可能是暂时的；不能当成「不存在」，否则新建的空对象会覆盖原数据）
    Read(String),
    /// 数据无法解码（损坏 / 来自更新版本）。原始字节已备份到 `backup`（备份失败时为 None）
    Corrupt { reason: String, backup: Option<std::path::PathBuf> },
}

impl LoadError {
    /// 给用户看的说明。
    pub fn message(&self) -> String {
        match self {
            LoadError::NoStore => "存储未初始化".into(),
            LoadError::Missing => "对象数据丢失".into(),
            LoadError::Read(e) => format!("读取对象失败：{e}"),
            LoadError::Corrupt { backup: Some(p), .. } => format!("对象数据无法解析，原始数据已备份到 {}，为避免覆盖已禁止编辑", p.display()),
            LoadError::Corrupt { backup: None, reason } => format!("对象数据无法解析（{reason}），备份失败，为避免覆盖已禁止编辑"),
        }
    }
}

thread_local! {
    /// 本次运行中解码失败的对象：只备份一次，之后直接返回同一个错误
    static BROKEN: RefCell<HashMap<String, LoadError>> = RefCell::new(HashMap::new());
}

/// 把无法解码的对象原样备份到 `recovery/`。
fn backup_corrupt(store: &Store, id: &str, kind: &str, bytes: &[u8]) -> Option<std::path::PathBuf> {
    let dir = store.data_dir.join("recovery");
    let path = dir.join(format!("obj-{kind}-{id}-{}.bin", crate::store::now_ms()));
    std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, bytes)).ok().map(|_| path)
}

/// 取对象（必要时从数据库加载），区分「不存在」「读失败」「损坏」。
pub fn load(id: &str) -> Result<ObjData, LoadError> {
    if let Some(d) = OBJS.with(|o| o.borrow().get(id).map(|e| e.data.clone())) {
        return Ok(d);
    }
    if let Some(e) = BROKEN.with(|b| b.borrow().get(id).cloned()) {
        return Err(e);
    }
    let store = store().ok_or(LoadError::NoStore)?;
    let (kind, data) = load_from(&store, id)?;
    OBJS.with(|o| o.borrow_mut().insert(id.to_string(), Entry { kind, data: data.clone(), version: next_ver(), dirty_since: None }));
    Ok(data)
}

/// 从数据库读取并解码（不经过缓存）。解码失败时备份原始字节、上报，并记进 `BROKEN`。
fn load_from(store: &Store, id: &str) -> Result<(ObjKind, ObjData), LoadError> {
    let (kind_s, bytes) = match store.load_object(id) {
        Ok(Some(x)) => x,
        Ok(None) => return Err(LoadError::Missing),
        Err(e) => return Err(LoadError::Read(e.to_string())),
    };
    let res = match ObjKind::parse(&kind_s) {
        Some(kind) => decode(id, kind, &bytes).map(|d| (kind, d)),
        None => Err(format!("未知的对象类型「{kind_s}」")),
    };
    match res {
        Ok(x) => Ok(x),
        Err(reason) => {
            let backup = backup_corrupt(store, id, &kind_s, &bytes);
            let err = LoadError::Corrupt { reason: reason.clone(), backup };
            store.report_error(format!("{}（{reason}）", err.message()));
            BROKEN.with(|b| b.borrow_mut().insert(id.to_string(), err.clone()));
            Err(err)
        }
    }
}

/// 取对象（必要时从数据库加载）。读不出来时返回 None；需要知道原因用 `load`。
pub fn get(id: &str) -> Option<ObjData> {
    load(id).ok()
}

pub fn version(id: &str) -> u64 {
    OBJS.with(|o| o.borrow().get(id).map(|e| e.version).unwrap_or(0))
}

pub fn sketch(id: &str) -> Option<Arc<crate::sketch::SketchDoc>> {
    match get(id)? {
        ObjData::Sketch(d) => Some(d),
        _ => None,
    }
}

pub fn board(id: &str) -> Option<Arc<BoardDoc>> {
    match get(id)? {
        ObjData::Board(b) => Some(b),
        _ => None,
    }
}

/// 新建多维表格（斜杠菜单「表格 / 看板 / 画册」：只是默认视图不同）。
pub fn create_base(kind: crate::base::ViewKind) -> String {
    let id = crate::store::new_uuid();
    let b = new_base(&id, crate::base::Base::new_default(&id, kind));
    insert(&id, ObjKind::Base, ObjData::Base(Rc::new(RefCell::new(b))));
    id
}

/// 按预设新建多维表格（「任务看板」「日程」）。
pub fn create_base_preset(preset: &str) -> String {
    let id = crate::store::new_uuid();
    let b = new_base(&id, crate::base::Base::new_preset(&id, preset));
    insert(&id, ObjKind::Base, ObjData::Base(Rc::new(RefCell::new(b))));
    id
}

pub fn base(id: &str) -> Option<crate::base::BaseRef> {
    match get(id)? {
        ObjData::Base(b) => Some(b),
        _ => None,
    }
}

/// 所有多维表格（「关联已有多维表格」用）：(id, 名称, 数据表数)。
/// 只解析对象行里的元数据，不把记录加载进内存（已打开的用内存里的最新名称）。
pub fn list_bases() -> Vec<(String, String, usize)> {
    let Some(s) = store() else { return vec![] };
    s.objects_of_kind("base")
        .into_iter()
        .filter_map(|(id, bytes)| {
            if let Some(ObjData::Base(b)) = OBJS.with(|o| o.borrow().get(&id).map(|e| e.data.clone())) {
                let b = b.borrow();
                return Some((id, b.meta.name.clone(), b.meta.tables.len()));
            }
            let meta: crate::base::model::BaseMeta = serde_json::from_slice(&bytes).ok()?;
            Some((id, meta.name, meta.tables.len()))
        })
        .collect()
}

pub fn sheet(id: &str) -> Option<Sheet> {
    match get(id)? {
        ObjData::Sheet(s) => Some(s),
        _ => None,
    }
}

/// 更新对象并排队保存（电子表格只标脏，停手后再编码）。
pub fn update(id: &str, data: ObjData) {
    let mut cached = true;
    let kind = OBJS.with(|o| {
        let mut o = o.borrow_mut();
        let Some(e) = o.get_mut(id) else {
            cached = false;
            return None;
        };
        e.data = data.clone();
        e.version = next_ver();
        if e.kind == ObjKind::Sheet {
            e.dirty_since = Some(Instant::now());
            return None;
        }
        Some(e.kind)
    });
    if !cached {
        // 损坏的对象绝不能写（会覆盖备份前的原始数据）；编辑入口已拦截，这里兜底并让用户知道
        if BROKEN.with(|b| b.borrow().contains_key(id)) {
            if let Some(store) = store() {
                store.report_error(format!("保存失败：对象 {id} 数据已损坏，本次修改没有保存"));
            }
            return;
        }
        // 已被缓存淘汰的对象：数据就是调用方给的最新版本，放回缓存并照常保存
        let kind = kind_of(&data);
        OBJS.with(|o| o.borrow_mut().insert(id.to_string(), Entry { kind, data: data.clone(), version: next_ver(), dirty_since: None }));
        if kind == ObjKind::Sheet {
            OBJS.with(|o| o.borrow_mut().get_mut(id).map(|e| e.dirty_since = Some(Instant::now())));
        } else if let Some(store) = store() {
            persist(&store, id, kind, &data);
        }
        return;
    }
    if let (Some(kind), Some(store)) = (kind, store()) {
        persist(&store, id, kind, &data);
    }
}

/// 把停手超过 `SHEET_DEBOUNCE` 的电子表格编码入队；`force` 时不论时间全部入队（退出前用）。
pub fn flush_dirty(force: bool) {
    let Some(store) = store() else { return };
    let due: Vec<(String, ObjKind, ObjData)> = OBJS.with(|o| {
        let mut o = o.borrow_mut();
        o.iter_mut()
            .filter(|(_, e)| e.dirty_since.is_some_and(|t| force || t.elapsed() >= SHEET_DEBOUNCE))
            .map(|(id, e)| {
                e.dirty_since = None;
                (id.clone(), e.kind, e.data.clone())
            })
            .collect()
    });
    for (id, kind, data) in due {
        persist(&store, &id, kind, &data);
    }
}

/// 删除对象（用完即弃的临时对象，如「以思维导图查看」生成的画板）。
pub fn remove(id: &str) {
    OBJS.with(|o| o.borrow_mut().remove(id));
    BROKEN.with(|b| b.borrow_mut().remove(id));
    if let Some(s) = store() {
        s.delete_object(id);
    }
}

/// 电子表格 / 多维表格在原地修改（RefCell），调用此函数递增版本并保存。
pub fn touch(id: &str) {
    if let Some(d) = get(id) {
        update(id, d);
    }
}

/// 复制对象（粘贴嵌入块时使用），返回新 id。
pub fn duplicate(id: &str) -> Option<String> {
    let data = get(id)?;
    let kind = OBJS.with(|o| o.borrow().get(id).map(|e| e.kind))?;
    let copy = match &data {
        ObjData::Board(b) => ObjData::Board(Arc::new((**b).clone())),
        ObjData::Sketch(d) => ObjData::Sketch(Arc::new((**d).clone())),
        ObjData::Sheet(s) => ObjData::Sheet(Rc::new(RefCell::new(sheet_from_bytes(&sheet_to_bytes(&s.borrow()))?))),
        ObjData::Base(b) => {
            let new_id = crate::store::new_uuid();
            let copy = b.borrow().duplicate(&new_id);
            copy.save_all();
            insert(&new_id, ObjKind::Base, ObjData::Base(Rc::new(RefCell::new(copy))));
            return Some(new_id);
        }
    };
    Some(create(kind, Some(copy)))
}

// ───────────── 导出 ─────────────

/// 电子表格已使用区域的显示值（最多 rows × cols）。
pub fn sheet_values(s: &Sheet, max_rows: i32, max_cols: i32) -> Vec<Vec<String>> {
    let m = s.borrow();
    let mut out = vec![];
    let mut last_row = 0;
    let mut last_col = 0;
    for r in 1..=max_rows {
        let mut row = vec![];
        for c in 1..=max_cols {
            let v = m.get_formatted_cell_value(0, r, c).unwrap_or_default();
            if !v.is_empty() {
                last_row = r;
                last_col = last_col.max(c);
            }
            row.push(v);
        }
        out.push(row);
    }
    out.truncate(last_row as usize);
    for row in out.iter_mut() {
        row.truncate(last_col.max(1) as usize);
    }
    out
}

pub fn to_markdown(kind: &str, id: &str) -> String {
    match get(id) {
        Some(ObjData::Sheet(s)) => {
            let vals = sheet_values(&s, 200, 26);
            if vals.is_empty() {
                return "（空表格）\n".into();
            }
            let mut out = String::new();
            for (i, row) in vals.iter().enumerate() {
                out.push_str(&format!("| {} |\n", row.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | ")));
                if i == 0 {
                    out.push_str(&format!("|{}\n", " --- |".repeat(row.len())));
                }
            }
            out
        }
        Some(ObjData::Base(b)) => crate::base::io::to_markdown(&b.borrow()),
        Some(ObjData::Sketch(d)) => crate::sketch::to_markdown(&d),
        Some(ObjData::Board(b)) => {
            if let Some(md) = b.mind_outline_md() {
                return md;
            }
            let texts: Vec<String> = b.els.iter().filter(|e| !e.text.is_empty()).map(|e| e.text.replace('\n', " ")).collect();
            format!("[{}：{}]\n", if kind == "flow" { "流程图" } else { "画板" }, texts.join(" / "))
        }
        None => String::new(),
    }
}

pub fn to_html(_kind: &str, id: &str) -> String {
    match get(id) {
        Some(ObjData::Board(b)) => {
            let mut scene = b.to_scene(false);
            scene.fit(16.);
            scene.bg = Some(crate::canvas::Col::rgb(0xffffff));
            format!("<div class=\"board\">{}</div>", crate::canvas::svg::to_svg(&scene))
        }
        Some(ObjData::Sheet(s)) => {
            let vals = sheet_values(&s, 500, 26);
            let mut out = String::from("<table border=\"1\" style=\"border-collapse:collapse\">");
            for row in vals {
                out.push_str("<tr>");
                for c in row {
                    out.push_str(&format!("<td>{}</td>", c.replace('&', "&amp;").replace('<', "&lt;")));
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
            out
        }
        Some(ObjData::Base(b)) => crate::base::io::to_html(&b.borrow()),
        Some(ObjData::Sketch(d)) => crate::sketch::to_html(&d),
        None => String::new(),
    }
}

/// 资源回收：删除未被任何文档（含回收站）、用户模板、同步块引用，且超过 7 天未修改的对象。
pub fn gc(store: &Store) -> usize {
    let refs = match store.reference_texts() {
        Ok(t) => crate::store::collect_ids(&t),
        Err(e) => {
            eprintln!("资源回收跳过：读取引用失败（{e}）");
            return 0;
        }
    };
    let week = 7 * 24 * 3600 * 1000;
    let now = crate::store::now_ms();
    let mut n = 0;
    for (id, kind, updated) in store.object_ids() {
        if now - updated > week && !refs.contains(&id) {
            store.delete_object(&id);
            if kind == "base" {
                crate::base::db::delete(store, &id);
            }
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("stonebrook-obj-{tag}-{}", crate::store::new_uuid()));
        (Store::open_in(&dir).unwrap(), dir)
    }

    #[test]
    fn corrupt_object_is_backed_up_and_reported() {
        let (s, dir) = temp_store("corrupt");
        let id = crate::store::new_uuid();
        s.save_object(&id, "board", Ok(b"{not json".to_vec()));
        s.flush();
        let err = load_from(&s, &id).err().expect("损坏的对象必须报错，不能当成空白画板");
        let LoadError::Corrupt { backup: Some(p), .. } = &err else { panic!("{err:?}") };
        assert_eq!(std::fs::read(p).unwrap(), b"{not json", "原始字节原样备份");
        assert!(s.take_error().is_some(), "损坏要进错误通道");
        // 原始数据没有被改动
        assert_eq!(s.load_object(&id).unwrap().unwrap().1, b"{not json");
        assert!(matches!(load_from(&s, &crate::store::new_uuid()), Err(LoadError::Missing)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_evicts_idle_objects_but_keeps_ones_in_use() {
        // 没有全局存储：只测缓存本身
        let held = sheet(&create(ObjKind::Sheet, None)).unwrap(); // 模拟编辑器持有
        let held_id = OBJS.with(|o| o.borrow().iter_ids().into_iter().next().unwrap());
        let mut ids: Vec<String> = (0..CACHE_CAP * 2).map(|_| create(ObjKind::Board, None)).collect();
        // 写队列里待保存的任务也持有数据（正确地钉住）；写完后再插入一个触发淘汰
        if let Some(s) = store() {
            s.flush();
        }
        ids.push(create(ObjKind::Board, None));
        let n = OBJS.with(|o| o.borrow().iter_ids().len());
        assert!(n <= CACHE_CAP + 1, "缓存要有上限：{n}");
        assert!(get(&held_id).is_some(), "被持有的对象不能淘汰");
        assert!(get(ids.last().unwrap()).is_some(), "最近用的还在");
        drop(held);
    }

    #[test]
    fn gc_keeps_referenced_and_recent_objects() {
        let (s, dir) = temp_store("gc");
        let (old_ref, old_unref, fresh) = (crate::store::new_uuid(), crate::store::new_uuid(), crate::store::new_uuid());
        for id in [&old_ref, &old_unref, &fresh] {
            s.save_object(id, "board", Ok(b"{}".to_vec()));
        }
        s.flush();
        let long_ago = crate::store::now_ms() - 30 * 24 * 3600 * 1000;
        let ids = (old_ref.clone(), old_unref.clone());
        s.write_sync(move |c| c.execute("UPDATE objects SET updated = ?1 WHERE id IN (?2, ?3)", rusqlite::params![long_ago, ids.0, ids.1]).map(|_| ()));
        // 一篇在回收站里的文档引用了 old_ref
        let doc = s.create(None, &crate::model::DocContent::new_empty(&format!("引用 {old_ref}")));
        s.set_deleted(&doc, true);
        assert_eq!(gc(&s), 1);
        s.flush();
        assert!(s.load_object(&old_unref).unwrap().is_none(), "超过 7 天且无引用：回收");
        assert!(s.load_object(&old_ref).unwrap().is_some(), "回收站文档引用的不能回收");
        assert!(s.load_object(&fresh).unwrap().is_some(), "不足 7 天的不能回收");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
