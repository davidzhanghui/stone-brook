//! 多维表格全屏编辑器：顶栏、数据表侧栏、视图标签、工具栏，以及各视图共用的状态与操作。
//!
//! 具体视图在 grid / kanban / gallery / calendar / gantt / form 里实现（都是 `impl BaseEditor`），
//! 弹出面板在 panels.rs，单元格显示与编辑在 cells.rs，记录详情在 detail.rs。

use super::model::*;
use super::widgets::*;
use super::BaseRef;
use crate::line_input::{InputEvent, LineInput};
use crate::theme::{theme, Theme};
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::Value;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;

actions!(
    base,
    [
        BUp, BDown, BLeft, BRight, BExtUp, BExtDown, BExtLeft, BExtRight, BTab, BShiftTab, BEnter, BEscape, BDelete, BUndo, BRedo, BCopy, BPaste, BSpace,
        BFind, BSelectAll, BNewRecord, BFirst, BLast, BExpand
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Base");
    cx.bind_keys([
        KeyBinding::new("up", BUp, c),
        KeyBinding::new("down", BDown, c),
        KeyBinding::new("left", BLeft, c),
        KeyBinding::new("right", BRight, c),
        KeyBinding::new("shift-up", BExtUp, c),
        KeyBinding::new("shift-down", BExtDown, c),
        KeyBinding::new("shift-left", BExtLeft, c),
        KeyBinding::new("shift-right", BExtRight, c),
        KeyBinding::new("tab", BTab, c),
        KeyBinding::new("shift-tab", BShiftTab, c),
        KeyBinding::new("enter", BEnter, c),
        KeyBinding::new("escape", BEscape, c),
        KeyBinding::new("backspace", BDelete, c),
        KeyBinding::new("delete", BDelete, c),
        KeyBinding::new("cmd-z", BUndo, c),
        KeyBinding::new("cmd-shift-z", BRedo, c),
        KeyBinding::new("cmd-y", BRedo, c),
        KeyBinding::new("cmd-c", BCopy, c),
        KeyBinding::new("cmd-v", BPaste, c),
        KeyBinding::new("space", BSpace, c),
        KeyBinding::new("cmd-f", BFind, c),
        KeyBinding::new("cmd-a", BSelectAll, c),
        KeyBinding::new("shift-enter", BNewRecord, c),
        KeyBinding::new("cmd-up", BFirst, c),
        KeyBinding::new("cmd-down", BLast, c),
        KeyBinding::new("cmd-enter", BExpand, c),
    ]);
}

pub enum BaseEvent {
    Close,
    /// 文档内嵌时点「全屏」
    Fullscreen,
}

impl EventEmitter<BaseEvent> for BaseEditor {}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Popup {
    Fields,
    Filter,
    Sort,
    Group,
    RowHeight,
    KanbanSet,
    DateSet,
    More,
    NewView,
    ViewMenu(String),
    TableMenu(String),
    ColMenu(String),
    RowMenu(String),
    Stat(String),
    FieldEdit,
    Options {
        rid: String,
        fid: String,
    },
    Date {
        rid: String,
        fid: String,
        ym: (i32, u32),
    },
    Links {
        rid: String,
        fid: String,
    },
    People {
        rid: String,
        fid: String,
    },
    Files {
        rid: String,
        fid: String,
    },
    KanbanCol(String),
    /// 填色规则（R2）
    Colors,
    /// 勾选行后的批量修改（R11）
    BatchEdit,
    /// 删除字段前列出依赖（R6）
    DelField(String),
    /// 字段编组（R6）
    FieldGroups,
    /// 自动化（R15）
    Automations,
    /// 表单设置（R8）
    FormSet,
    /// 导入：新建数据表 / 追加到当前表（R13）
    Import(String),
    /// 仪表盘组件配置（R14）
    Widget(String),
    /// 导出菜单
    Export,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PickTarget {
    Conj,
    SortField(usize),
    GroupField(usize),
    FieldType,
    LinkTable,
    NumFmt,
    DateFmt,
    Currency,
    RatingSym,
    RatingMax,
    KanbanField,
    Cover,
    DateField,
    EndField,
    /// 由 `pick_cb` 处理选择结果
    Cb,
}

#[derive(Clone)]
pub(super) struct PickItem {
    pub id: String,
    pub label: String,
    pub icon: Option<&'static str>,
    pub color: Option<u8>,
    pub group: Option<&'static str>,
}

impl PickItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        PickItem { id: id.into(), label: label.into(), icon: None, color: None, group: None }
    }
    pub fn icon(mut self, i: &'static str) -> Self {
        self.icon = Some(i);
        self
    }
}

pub(super) struct Pick {
    pub target: PickTarget,
    pub items: Vec<PickItem>,
    pub selected: Vec<String>,
    pub multi: bool,
    pub at: Point<Pixels>,
}

/// 新建 / 编辑字段的草稿（点「确定」才写入）。
pub(super) struct Draft {
    pub fid: Option<String>,
    pub kind: FieldType,
    pub prop: Prop,
    /// 插入到第几个字段之后（None 为末尾）
    pub insert_at: Option<usize>,
    pub err: Option<String>,
}

/// 表格视图选区：光标 + 锚点。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Sel {
    pub rid: String,
    pub fid: String,
    pub arid: String,
    pub afid: String,
}

pub(super) enum Drag {
    Select,
    ColResize {
        fid: String,
        start_x: Pixels,
        orig: f32,
    },
    Col {
        fid: String,
        start: Point<Pixels>,
        moved: bool,
    },
    Row {
        rid: String,
        start: Point<Pixels>,
        moved: bool,
    },
    Card {
        rid: String,
        from: String,
        start: Point<Pixels>,
        grab: Point<Pixels>,
        moved: bool,
        w: f32,
    },
    Bar {
        rid: String,
        mode: u8,
        start_x: Pixels,
        orig: (i64, i64),
        moved: bool,
    },
    CalItem {
        rid: String,
        start: Point<Pixels>,
        moved: bool,
    },
    /// 仪表盘组件拖动 / 缩放：(x, y, w, h)
    Widget {
        id: String,
        resize: bool,
        start: Point<Pixels>,
        orig: (u8, u8, u8, u8),
    },
}

pub struct BaseEditor {
    pub obj_id: String,
    pub(super) base: BaseRef,
    pub(super) focus: FocusHandle,
    pub(super) tid: String,
    pub(super) vid: String,
    pub(super) search: String,
    pub(super) search_open: bool,
    pub(super) sel: Option<Sel>,
    pub(super) editing: Option<(String, String)>,
    pub(super) popup: Option<(Popup, Point<Pixels>)>,
    pub(super) pick: Option<Pick>,
    pub(super) draft: Option<Draft>,
    pub(super) inputs: HashMap<String, (Entity<LineInput>, Subscription)>,
    pub(super) detail: Option<String>,
    pub(super) drag: Option<Drag>,
    pub(super) mouse: Point<Pixels>,
    pub(super) scroll: (f32, f32),
    pub(super) grid_bounds: Bounds<Pixels>,
    pub(super) frame: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    pub(super) toast: Option<(String, usize)>,
    toast_seq: usize,
    pub(super) sidebar: bool,
    pub(super) rows_cache: Option<((u64, String, String, String), Rc<Vec<String>>)>,
    pub(super) kanban_scroll: ScrollHandle,
    pub(super) gallery_scroll: UniformListScrollHandle,
    pub(super) detail_scroll: ScrollHandle,
    pub(super) cal_month: (i32, u32),
    pub(super) gantt_scroll: f32,
    pub(super) form_vals: HashMap<String, Value>,
    pub(super) form_done: bool,
    pub(super) renaming: Option<String>,
    /// 上一帧表格视图的排版（键盘移动、弹层定位、拖拽落点都用它）
    pub(super) gl: super::grid::GridLayout,
    /// 详情面板里输入框对应的记录（换记录时重建输入框）
    pub(super) detail_loaded: Option<String>,
    /// 二级下拉的回调（PickTarget::Cb）
    pub(super) pick_cb: Option<Rc<dyn Fn(&mut BaseEditor, String, &mut Context<BaseEditor>)>>,
    /// 勾选的行（批量操作，R11）
    pub(super) checked: std::collections::HashSet<String>,
    /// 填色结果缓存：(数据版本, 视图)
    pub(super) colors_cache: Option<((u64, String, String), Rc<super::calc::Colors>)>,
    /// 树化后的行缓存
    pub(super) tree_cache: Option<((u64, String, String, String, Vec<String>), Rc<Vec<(String, usize, bool)>>)>,
    /// 树缓存对应的结果列表（持有它，避免地址被复用导致误命中）
    pub(super) tree_src: Option<Rc<Vec<String>>>,
    /// 表格排版缓存的键（数据 / 视图配置没变时不重排 10 万行）
    pub(super) gl_key: String,
    /// 正在看的仪表盘（R14）
    pub(super) dash: Option<String>,
    /// 视图配置的临时修改（关闭「立即保存」时，R5）
    pub(super) view_draft: Option<View>,
    /// 详情页标签：0 字段 1 关联 2 历史 3 评论
    pub(super) detail_tab: u8,
    /// 查找定位：当前第几处
    pub(super) find_pos: usize,
    pub(super) replace_open: bool,
    /// 文档内嵌模式（R12）：没有顶栏和侧栏，高度固定
    pub embedded: bool,
    pub embed_h: f32,
    /// 日历周 / 日视图的锚点日期
    pub(super) cal_day: i64,
    /// 自动化面板里正在编辑的自动化
    pub(super) auto_sel: Option<String>,
    /// 每条记录的评论数
    pub(super) comment_counts: HashMap<String, usize>,
    /// 「cb:」开头的输入框：内容变化时调用的回调
    pub(super) input_cbs: HashMap<String, Rc<dyn Fn(&mut BaseEditor, String, &mut Context<BaseEditor>)>>,
    /// 面板里的临时状态（批量修改的字段等）
    pub(super) scratch: HashMap<String, String>,
    /// 附件全屏预览：(记录, 字段, 第几个附件)，同电子表格的附件预览
    pub(super) file_view: Option<(String, String, usize)>,
    /// 全屏预览正在用的播放器 / 原生预览视图 id：关闭预览时释放
    fv_media: Option<u64>,
    /// 全屏预览里图片查看器的状态（缩放 / 旋转 / 平移），每次打开预览清空
    img_views: HashMap<u64, crate::editor::ImgView>,
    /// 图片原始尺寸缓存
    image_sizes: HashMap<String, Option<(u32, u32)>>,
}

impl Focusable for BaseEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BaseEditor {
    pub fn new(obj_id: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let base = crate::objects::base(&obj_id).unwrap_or_else(|| Rc::new(RefCell::new(Base::new_default(&obj_id, ViewKind::Grid))));
        let (tid, vid) = {
            let b = base.borrow();
            let tid = if b.table(&b.meta.active).is_some() { b.meta.active.clone() } else { b.meta.tables.first().cloned().unwrap_or_default() };
            let vid = b
                .table(&tid)
                .map(|t| if t.views.contains_key(&t.meta.active_view) { t.meta.active_view.clone() } else { t.meta.views.first().cloned().unwrap_or_default() })
                .unwrap_or_default();
            (tid, vid)
        };
        use chrono::Datelike;
        let now = chrono::Local::now();
        BaseEditor {
            obj_id,
            base,
            focus: cx.focus_handle(),
            tid,
            vid,
            search: String::new(),
            search_open: false,
            sel: None,
            editing: None,
            popup: None,
            pick: None,
            draft: None,
            inputs: HashMap::new(),
            detail: None,
            drag: None,
            mouse: Point::default(),
            scroll: (0., 0.),
            grid_bounds: Bounds::default(),
            frame: Rc::new(RefCell::new(HashMap::new())),
            toast: None,
            toast_seq: 0,
            sidebar: true,
            rows_cache: None,
            kanban_scroll: ScrollHandle::new(),
            gallery_scroll: UniformListScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            cal_month: (now.year(), now.month()),
            gantt_scroll: 0.,
            form_vals: HashMap::new(),
            form_done: false,
            renaming: None,
            gl: Default::default(),
            detail_loaded: None,
            pick_cb: None,
            checked: Default::default(),
            colors_cache: None,
            tree_cache: None,
            dash: None,
            tree_src: None,
            gl_key: String::new(),
            view_draft: None,
            detail_tab: 0,
            find_pos: 0,
            replace_open: false,
            embedded: false,
            embed_h: 420.,
            cal_day: super::formula::today(),
            auto_sel: None,
            comment_counts: HashMap::new(),
            input_cbs: HashMap::new(),
            scratch: HashMap::new(),
            file_view: None,
            fv_media: None,
            img_views: HashMap::new(),
            image_sizes: HashMap::new(),
        }
        .start_timers(cx)
    }

    /// 文档内嵌的多维表格（R12）：可直接编辑，「全屏」进入完整编辑页。
    pub fn new_embedded(obj_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut e = Self::new(obj_id, window, cx);
        e.embedded = true;
        e.sidebar = false;
        e
    }

    /// 定时自动化：每 20 秒看一次是否到点。
    fn start_timers(mut self, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this: WeakEntity<BaseEditor>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(20)).await;
            let alive = this
                .update(cx, |this, cx| {
                    if this.embedded {
                        return;
                    }
                    let ran = this.bm().run_schedule(crate::store::now_ms());
                    if ran {
                        crate::objects::touch(&this.obj_id);
                        this.after_commit(cx);
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
        self.refresh_comment_counts();
        self
    }

    pub(super) fn refresh_comment_counts(&mut self) {
        self.comment_counts.clear();
        if let Some(st) = crate::objects::store() {
            for c in st.comments(&format!("base:{}", self.obj_id)) {
                if !c.resolved {
                    *self.comment_counts.entry(c.anchor.clone()).or_default() += 1;
                }
            }
        }
    }

    /// 提交之后：自动化产生的通知弹出来（写库失败由工作区统一提示）。
    pub(super) fn after_commit(&mut self, cx: &mut Context<Self>) {
        let notices = std::mem::take(&mut self.bm().notices);
        for (title, body) in notices {
            self.show_toast(&format!("{title}：{body}"), cx);
            notify_system(&title, &body);
        }
    }

    // ───────────── 数据访问 ─────────────

    pub(super) fn b(&self) -> Ref<'_, Base> {
        self.base.borrow()
    }
    pub(super) fn bm(&self) -> RefMut<'_, Base> {
        self.base.borrow_mut()
    }

    pub(super) fn view(&self) -> View {
        if let Some(d) = self.view_draft.as_ref().filter(|d| d.id == self.vid) {
            return d.clone();
        }
        self.b().view(&self.tid, &self.vid).cloned().unwrap_or_else(|| View::new("表格视图", ViewKind::Grid))
    }

    pub(super) fn field(&self, fid: &str) -> Option<Field> {
        self.b().field(&self.tid, fid).cloned()
    }

    pub(super) fn fields(&self) -> Vec<Field> {
        self.b().table(&self.tid).map(|t| t.field_list().into_iter().cloned().collect()).unwrap_or_default()
    }

    /// 当前视图可见的字段（按视图顺序）。
    pub(super) fn visible(&self) -> Vec<Field> {
        let b = self.b();
        let Some(t) = b.table(&self.tid) else { return vec![] };
        let Some(v) = t.views.get(&self.vid) else { return vec![] };
        t.visible_fields(v).iter().filter_map(|f| t.fields.get(f).cloned()).collect()
    }

    /// 树化后的当前视图记录：(记录, 层级深度, 是否命中筛选)；没开层级时深度都是 0。
    pub(super) fn tree_rows(&mut self) -> Rc<Vec<(String, usize, bool)>> {
        let rows = self.rows();
        let v = self.view();
        let hier = self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some());
        // 没开层级时树就是原列表：按结果列表的地址缓存，改格子不用重建
        let key = (
            if hier { self.b().ver } else { Rc::as_ptr(&rows) as usize as u64 },
            self.tid.clone(),
            self.vid.clone(),
            self.search.clone(),
            if hier { v.collapsed.clone() } else { vec![] },
        );
        if let Some((k, t)) = &self.tree_cache {
            if *k == key && (hier || self.tree_src.as_ref().is_some_and(|s| Rc::ptr_eq(s, &rows))) {
                return t.clone();
            }
        }
        let t = Rc::new(self.b().tree(&self.tid, &rows, &v.collapsed));
        self.tree_cache = Some((key, t.clone()));
        self.tree_src = Some(rows);
        t
    }

    /// 当前视图的填色结果（按数据版本缓存，R2）。
    pub(super) fn colors(&mut self) -> Rc<super::calc::Colors> {
        let v = self.view();
        let key = (self.b().ver, self.vid.clone(), serde_json::to_string(&v.colors).unwrap_or_default());
        if let Some((k, c)) = &self.colors_cache {
            if *k == key {
                return c.clone();
            }
        }
        let rows = self.rows();
        let c = Rc::new(if v.colors.is_empty() { Default::default() } else { self.b().colorize(&self.tid, &v, &rows) });
        self.colors_cache = Some((key, c.clone()));
        c
    }

    /// 当前视图的记录（筛选 + 搜索 + 排序）。按版本缓存；版本变了先尝试增量更新（只挪动改过的记录）。
    pub(super) fn rows(&mut self) -> Rc<Vec<String>> {
        let v = self.view();
        let ver = self.b().ver;
        let key = (
            self.tid.clone(),
            self.vid.clone(),
            format!("{}\u{1}{}\u{1}{}", self.search, serde_json::to_string(&v.filter).unwrap_or_default(), serde_json::to_string(&v.sorts).unwrap_or_default()),
        );
        if let Some(((k_ver, k_tid, k_vid, k_q), rows)) = &mut self.rows_cache {
            if (k_tid.as_str(), k_vid.as_str(), k_q.as_str()) == (key.0.as_str(), key.1.as_str(), key.2.as_str()) {
                if *k_ver == ver {
                    return rows.clone();
                }
                let mut list = (**rows).clone();
                let ok = self.base.borrow().refresh_rows(&key.0, &v, &self.search, &mut list, *k_ver);
                if ok {
                    let rc = Rc::new(list);
                    self.rows_cache = Some(((ver, key.0, key.1, key.2), rc.clone()));
                    return rc;
                }
            }
        }
        let rows = Rc::new(self.b().view_records(&self.tid, &v, &self.search));
        self.rows_cache = Some(((ver, key.0, key.1, key.2), rows.clone()));
        rows
    }

    // ───────────── 修改 ─────────────

    pub(super) fn commit(&mut self, cx: &mut Context<Self>) {
        self.commit_merge(None, cx);
    }

    pub(super) fn commit_merge(&mut self, merge: Option<&str>, cx: &mut Context<Self>) {
        self.bm().commit_merge(merge);
        crate::objects::touch(&self.obj_id);
        self.after_commit(cx);
        cx.notify();
    }

    /// 视图配置改动是否立即保存（关掉后约定：先临时生效，点「保存」才写入）。
    pub(super) fn autosave_views() -> bool {
        crate::objects::store().and_then(|s| s.get_kv("base_view_autosave")).as_deref() != Some("0")
    }

    pub(super) fn update_view(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut View)) {
        let mut v = self.view();
        if v.locked {
            self.show_toast("视图已锁定，解锁后才能修改配置", cx);
            return;
        }
        f(&mut v);
        if !Self::autosave_views() {
            self.view_draft = Some(v);
            cx.notify();
            return;
        }
        let tid = self.tid.clone();
        self.bm().put_view(&tid, v);
        self.commit(cx);
    }

    pub(super) fn set_cell(&mut self, rid: &str, fid: &str, v: Value, merge: bool, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        self.bm().set_cell(&tid, rid, fid, v);
        let key = format!("cell:{rid}:{fid}");
        self.commit_merge(merge.then_some(key.as_str()), cx);
    }

    /// 新增记录：`preset` 为预填字段（分组 / 看板列），`after` 为插在哪条记录之后。
    pub(super) fn add_record(&mut self, preset: Vec<(String, Value)>, after: Option<&str>, cx: &mut Context<Self>) -> Option<String> {
        let tid = self.tid.clone();
        let rank = after.and_then(|a| {
            let b = self.b();
            let t = b.table(&tid)?;
            let i = t.order.iter().position(|r| r == a)?;
            let next = t.order.get(i + 1).and_then(|r| t.records.get(r)).map(|r| r.rank);
            Some(rank_between(t.records.get(a).map(|r| r.rank), next))
        });
        let r = self.bm().new_record(&tid, rank)?;
        for (f, v) in preset {
            self.bm().set_cell(&tid, &r.id, &f, v);
        }
        // 带筛选时，新记录满足「等于」条件的字段自动填上（新记录不会立刻被筛掉）
        let v = self.view();
        for c in &v.filter.conds {
            if c.op == Op::Is && !c.value.is_null() {
                let kind = self.field(&c.field).map(|f| f.kind);
                let val = match kind {
                    Some(FieldType::Select) | Some(FieldType::Stage) => str_list(Some(&c.value)).first().map(|s| Value::from(s.clone())),
                    Some(FieldType::Checkbox) => Some(c.value.clone()),
                    Some(FieldType::Text) => c.value.as_str().map(Value::from),
                    _ => None,
                };
                if let Some(val) = val {
                    self.bm().set_cell(&tid, &r.id, &c.field, val);
                }
            }
        }
        self.commit(cx);
        Some(r.id)
    }

    pub(super) fn delete_records(&mut self, rids: &[String], cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        for r in rids {
            self.bm().delete_record(&tid, r);
        }
        self.commit(cx);
        if self.detail.as_ref().is_some_and(|d| rids.contains(d)) {
            self.detail = None;
        }
        self.sel = None;
        self.show_toast(&format!("已删除 {} 条记录 · ⌘Z 撤销", rids.len()), cx);
    }

    pub(super) fn duplicate_record(&mut self, rid: &str, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let nid = self.bm().duplicate_record(&tid, rid);
        self.commit(cx);
        if nid.is_some() {
            self.show_toast("已创建记录副本", cx);
        }
    }

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let ok = self.bm().undo();
        self.inputs.retain(|k, _| !k.starts_with("d:"));
        if ok {
            crate::objects::touch(&self.obj_id);
            self.fix_current();
        }
        cx.notify();
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let ok = self.bm().redo();
        self.inputs.retain(|k, _| !k.starts_with("d:"));
        if ok {
            crate::objects::touch(&self.obj_id);
            self.fix_current();
        }
        cx.notify();
    }

    /// 撤销后当前数据表 / 视图可能已不存在。
    fn fix_current(&mut self) {
        let (tid, vid) = {
            let b = self.b();
            let tid = if b.table(&self.tid).is_some() { self.tid.clone() } else { b.meta.tables.first().cloned().unwrap_or_default() };
            let vid = b
                .table(&tid)
                .map(|t| if t.views.contains_key(&self.vid) { self.vid.clone() } else { t.meta.views.first().cloned().unwrap_or_default() })
                .unwrap_or_default();
            (tid, vid)
        };
        self.tid = tid;
        self.vid = vid;
    }

    /// 点击按钮字段（R16）。
    pub(super) fn press_button(&mut self, rid: &str, fid: &str, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let url = self.bm().press_button(&tid, rid, fid);
        self.commit(cx);
        if let Some(u) = url {
            cx.open_url(&u);
        }
    }

    /// 开启 / 关闭记录层级（关闭时保留「父记录」字段，只是不再按树显示）。
    pub(super) fn toggle_hierarchy(&mut self, cx: &mut Context<Self>) {
        self.close_popups();
        let tid = self.tid.clone();
        let on = self.b().table(&tid).is_some_and(|t| t.meta.parent_field.is_some());
        if on {
            let Some(mut m) = self.b().table(&tid).map(|t| t.meta.clone()) else { return };
            m.parent_field = None;
            self.bm().put_table(m);
            self.commit(cx);
            self.show_toast("已关闭记录层级，「父记录」字段保留", cx);
        } else {
            let existing = self.fields().into_iter().find(|f| f.kind == FieldType::Link && f.prop.single && f.prop.table == tid).map(|f| f.id);
            match existing {
                Some(fid) => {
                    let Some(mut m) = self.b().table(&tid).map(|t| t.meta.clone()) else { return };
                    m.parent_field = Some(fid);
                    self.bm().put_table(m);
                }
                None => {
                    self.bm().enable_hierarchy(&tid);
                }
            }
            self.commit(cx);
            self.show_toast("已开启记录层级：右键记录「添加子记录」，或把记录拖到另一条上", cx);
        }
    }

    /// 保存 / 丢弃临时的视图配置。
    pub(super) fn save_view_draft(&mut self, save: bool, cx: &mut Context<Self>) {
        if let Some(v) = self.view_draft.take() {
            if save {
                let tid = self.tid.clone();
                self.bm().put_view(&tid, v);
                self.commit(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn switch_view(&mut self, vid: &str, cx: &mut Context<Self>) {
        self.close_popups();
        self.view_draft = None;
        self.dash = None;
        self.checked.clear();
        self.vid = vid.to_string();
        self.sel = None;
        self.scroll = (0., 0.);
        self.form_done = false;
        let tid = self.tid.clone();
        let meta = self.b().table(&tid).map(|t| t.meta.clone());
        if let Some(mut m) = meta {
            if m.active_view != vid {
                m.active_view = vid.to_string();
                self.bm().put_table(m);
                // 切换视图不进撤销栈
                self.bm().commit_silent();
                crate::objects::touch(&self.obj_id);
            }
        }
        cx.notify();
    }

    pub(super) fn switch_table(&mut self, tid: &str, cx: &mut Context<Self>) {
        self.close_popups();
        self.view_draft = None;
        self.dash = None;
        self.checked.clear();
        self.tid = tid.to_string();
        let vid = self
            .b()
            .table(tid)
            .map(|t| if t.views.contains_key(&t.meta.active_view) { t.meta.active_view.clone() } else { t.meta.views.first().cloned().unwrap_or_default() })
            .unwrap_or_default();
        self.vid = vid;
        self.sel = None;
        self.scroll = (0., 0.);
        let mut m = self.b().meta.clone();
        if m.active != tid {
            m.active = tid.to_string();
            self.bm().set_meta(m);
            self.bm().commit_silent();
            crate::objects::touch(&self.obj_id);
        }
        cx.notify();
    }

    pub(super) fn add_view(&mut self, kind: ViewKind, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let n = self.b().table(&tid).map(|t| t.views.values().filter(|v| v.kind == kind).count()).unwrap_or(0);
        let name = if n == 0 { kind.label().to_string() } else { format!("{} {}", kind.label(), n + 1) };
        let mut v = View::new(&name, kind);
        let fields = self.fields();
        match kind {
            ViewKind::Kanban => {
                // 没有可分组字段时新建一个「状态」单选
                v.group_field = fields
                    .iter()
                    .find(|f| f.kind == FieldType::Select || f.kind == FieldType::Stage)
                    .or_else(|| fields.iter().find(|f| f.kind.kanban_groupable()))
                    .map(|f| f.id.clone());
                if v.group_field.is_none() {
                    let mut f = Field::new("状态", FieldType::Select);
                    for n in ["待处理", "进行中", "已完成"] {
                        f.ensure_opt(n);
                    }
                    v.group_field = Some(f.id.clone());
                    self.insert_field(f, None);
                }
                v.cover = fields.iter().find(|f| f.kind == FieldType::Attachment).map(|f| f.id.clone());
            }
            ViewKind::Gallery => v.cover = fields.iter().find(|f| f.kind == FieldType::Attachment).map(|f| f.id.clone()),
            ViewKind::Calendar | ViewKind::Gantt => {
                let dates: Vec<&Field> = fields.iter().filter(|f| f.kind == FieldType::Date).collect();
                if dates.is_empty() {
                    let s = Field::new(if kind == ViewKind::Gantt { "开始日期" } else { "日期" }, FieldType::Date);
                    v.date_field = Some(s.id.clone());
                    self.insert_field(s, None);
                } else {
                    v.date_field = Some(dates[0].id.clone());
                }
                if kind == ViewKind::Gantt {
                    if dates.len() >= 2 {
                        v.end_field = Some(dates[1].id.clone());
                    } else {
                        let e = Field::new("截止日期", FieldType::Date);
                        v.end_field = Some(e.id.clone());
                        self.insert_field(e, None);
                    }
                    v.scale = 0;
                }
            }
            _ => {}
        }
        let vid = v.id.clone();
        let mut m = self.b().table(&tid).map(|t| t.meta.clone()).unwrap_or_default();
        m.views.push(vid.clone());
        m.active_view = vid.clone();
        self.bm().put_view(&tid, v);
        self.bm().put_table(m);
        self.commit(cx);
        self.switch_view(&vid, cx);
    }

    /// 把字段加进当前数据表（不提交）。
    pub(super) fn insert_field(&mut self, f: Field, at: Option<usize>) {
        let tid = self.tid.clone();
        let Some(mut m) = self.b().table(&tid).map(|t| t.meta.clone()) else { return };
        match at {
            Some(i) if i < m.fields.len() => m.fields.insert(i + 1, f.id.clone()),
            _ => m.fields.push(f.id.clone()),
        }
        self.bm().put_field(&tid, f);
        self.bm().put_table(m);
    }

    pub(super) fn add_table(&mut self, cx: &mut Context<Self>) {
        let n = self.b().meta.tables.len() + 1;
        let tid = self.bm().add_table(&format!("数据表 {n}"), ViewKind::Grid, 5);
        self.commit(cx);
        self.switch_table(&tid, cx);
    }

    pub(super) fn show_toast(&mut self, s: &str, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((s.to_string(), seq));
        cx.notify();
        cx.spawn(async move |this: WeakEntity<BaseEditor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(2600)).await;
            this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|t| t.1 == seq) {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn view_file(&self, rid: &str, fid: &str, idx: usize) -> Option<Attachment> {
        attachments(self.b().record(&self.tid, rid).and_then(|r| r.cells.get(fid))).into_iter().nth(idx)
    }

    /// 附件全屏预览（应用内预览，同电子表格；不再直接用系统默认应用打开）。
    pub(super) fn open_file_view(&mut self, rid: &str, fid: &str, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.view_file(rid, fid, idx) else { return };
        self.close_popups();
        crate::editor::file_view_opening(self, &a.path, &a.name);
        self.file_view = Some((rid.to_string(), fid.to_string(), idx));
        window.focus(&self.focus);
        cx.notify();
    }

    fn render_file_view(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let a = self.file_view.clone().and_then(|(rid, fid, idx)| self.view_file(&rid, &fid, idx));
        let Some(a) = a else {
            // 关闭了（或附件已被删掉）：停掉播放器、收起原生预览视图
            self.file_view = None;
            crate::editor::release_file_view(self);
            return None;
        };
        let (path, name) = (a.path.clone(), a.name.clone());
        Some(crate::editor::file_view_el(
            self,
            &a.name,
            &a.path,
            a.size,
            t,
            window,
            cx,
            |this, w, cx| {
                this.file_view = None;
                w.focus(&this.focus);
                cx.notify();
            },
            move |_, cx| {
                crate::editor::save_file_as(&path, &name, cx, |this, dst, cx| {
                    let msg = dst.map(|d| format!("已下载到 {}", d.display())).unwrap_or_else(|| "下载失败".into());
                    this.show_toast(&msg, cx);
                })
            },
        ))
    }

    pub(super) fn close_popups(&mut self) {
        self.popup = None;
        self.pick = None;
        self.pick_cb = None;
        self.draft = None;
        self.inputs.retain(|k, _| k == "search" || k.starts_with("d:") || k.starts_with("form:") || k.starts_with("cb:keep"));
        self.input_cbs.retain(|k, _| self.inputs.contains_key(k));
    }

    pub(super) fn open_popup(&mut self, p: Popup, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let same = self.popup.as_ref().is_some_and(|x| x.0 == p);
        self.close_popups();
        if !same {
            self.popup = Some((p, at));
        }
        cx.notify();
    }

    // ───────────── 输入框 ─────────────

    /// 取（必要时创建）一个以 key 标识的输入框；事件统一交给 `on_input`。
    pub(super) fn input(&mut self, key: &str, placeholder: &str, init: &str, focus: bool, window: &mut Window, cx: &mut Context<Self>) -> Entity<LineInput> {
        if let Some((i, _)) = self.inputs.get(key) {
            return i.clone();
        }
        let input = cx.new(|cx| LineInput::new(placeholder, cx));
        input.update(cx, |i, cx| {
            i.set_text(init, cx);
            if !focus {
                i.collapse_to_end(cx);
            }
        });
        let k = key.to_string();
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| {
            let text = input.read(cx).text();
            this.on_input(&k, ev, text, window, cx);
        });
        if focus {
            window.focus(&input.focus_handle(cx));
        }
        self.inputs.insert(key.to_string(), (input.clone(), sub));
        input
    }

    pub(super) fn input_text(&self, key: &str, cx: &App) -> String {
        self.inputs.get(key).map(|(i, _)| i.read(cx).text()).unwrap_or_default()
    }

    fn on_input(&mut self, key: &str, ev: &InputEvent, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let submit = matches!(ev, InputEvent::Submit { .. });
        let cancel = matches!(ev, InputEvent::Cancel);
        let changed = matches!(ev, InputEvent::Changed);
        let (head, rest) = key.split_once(':').unwrap_or((key, ""));
        match head {
            "search" => {
                if changed {
                    self.search = text;
                    self.scroll.1 = 0.;
                    cx.notify();
                } else if cancel {
                    self.search.clear();
                    self.search_open = false;
                    self.inputs.remove("search");
                    window.focus(&self.focus);
                    cx.notify();
                } else if submit {
                    let shift = matches!(ev, InputEvent::Submit { shift: true });
                    self.find_step(if shift { -1 } else { 1 }, cx);
                }
            }
            "cell" => {
                if submit {
                    self.commit_cell_edit(cx);
                    let shift = matches!(ev, InputEvent::Submit { shift: true });
                    self.move_sel(if shift { -1 } else { 1 }, 0, false, cx);
                    window.focus(&self.focus);
                } else if cancel {
                    self.editing = None;
                    self.inputs.remove("cell");
                    window.focus(&self.focus);
                    cx.notify();
                }
            }
            "fd" => {
                if submit && rest == "name" {
                    self.save_draft(window, cx);
                } else if cancel {
                    self.close_popups();
                    window.focus(&self.focus);
                    cx.notify();
                } else if changed && rest == "expr" {
                    if let Some(d) = self.draft.as_mut() {
                        d.prop.expr = text;
                        d.err = None;
                    }
                    cx.notify();
                } else if changed && rest == "prefix" {
                    if let Some(d) = self.draft.as_mut() {
                        d.prop.prefix = text;
                    }
                    cx.notify();
                } else if changed {
                    if let Some(oid) = rest.strip_prefix("opt:") {
                        if let Some(o) = self.draft.as_mut().and_then(|d| d.prop.options.iter_mut().find(|o| o.id == oid)) {
                            o.name = text;
                        }
                        cx.notify();
                    }
                }
            }
            "flt" => {
                if changed {
                    let i: usize = rest.parse().unwrap_or(0);
                    let mut v = self.view();
                    if let Some(c) = v.filter.conds.get_mut(i) {
                        c.value = Value::from(text);
                    }
                    let tid = self.tid.clone();
                    self.bm().put_view(&tid, v);
                    self.commit_merge(Some("filter"), cx);
                }
            }
            "q" => cx.notify(),
            "cb" => {
                if changed {
                    if let Some(cb) = self.input_cbs.get(key).cloned() {
                        cb(self, text, cx);
                    }
                } else if submit || cancel {
                    window.focus(&self.focus);
                    cx.notify();
                }
            }
            "d" => {
                // 记录详情里的文本类字段：边输入边保存（连续输入合并为一步撤销）
                if changed {
                    if let (Some(rid), Some(f)) = (self.detail.clone(), self.field(rest)) {
                        let v = super::io::text_to_value(&mut f.clone(), &text).unwrap_or(Value::Null);
                        let v = if matches!(f.kind, FieldType::Text | FieldType::Url | FieldType::Phone | FieldType::Email) { Value::from(text.clone()) } else { v };
                        self.set_cell(&rid, rest, v, true, cx);
                    }
                } else if submit || cancel {
                    window.focus(&self.focus);
                }
            }
            "form" => {
                if changed {
                    self.form_vals.insert(rest.to_string(), Value::from(text));
                }
            }
            "rename" => {
                if submit || cancel {
                    if submit || !text.trim().is_empty() {
                        self.apply_rename(rest, text.trim(), cx);
                    }
                    self.renaming = None;
                    self.inputs.remove(key);
                    window.focus(&self.focus);
                    cx.notify();
                }
            }
            "opt"
                // 看板列（选项）重命名
                if (submit || cancel) => {
                    if let Some(fid) = self.view().group_field {
                        if let Some(mut f) = self.field(&fid) {
                            if let Some(o) = f.prop.options.iter_mut().find(|o| o.id == rest) {
                                if !text.trim().is_empty() {
                                    o.name = text.trim().to_string();
                                }
                            }
                            let tid = self.tid.clone();
                            self.bm().put_field(&tid, f);
                            self.commit(cx);
                        }
                    }
                    self.renaming = None;
                    self.inputs.remove(key);
                    self.close_popups();
                    window.focus(&self.focus);
                    cx.notify();
                }
            _ => {}
        }
    }

    /// rename:base / rename:table:ID / rename:view:ID
    fn apply_rename(&mut self, what: &str, name: &str, cx: &mut Context<Self>) {
        if name.is_empty() {
            return;
        }
        let (kind, id) = what.split_once(':').unwrap_or((what, ""));
        match kind {
            "base" => {
                let mut m = self.b().meta.clone();
                m.name = name.into();
                self.bm().set_meta(m);
            }
            "table" => {
                let Some(mut m) = self.b().table(id).map(|t| t.meta.clone()) else { return };
                m.name = name.into();
                self.bm().put_table(m);
            }
            "view" => {
                let tid = self.tid.clone();
                let Some(mut v) = self.b().view(&tid, id).cloned() else { return };
                v.name = name.into();
                self.bm().put_view(&tid, v);
            }
            _ => return,
        }
        self.commit(cx);
    }

    pub(super) fn start_rename(&mut self, what: &str, current: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popups();
        let key = format!("rename:{what}");
        let i = self.input(&key, "名称", current, true, window, cx);
        i.update(cx, |i, cx| i.set_text(current, cx));
        self.renaming = Some(what.to_string());
        cx.notify();
    }

    pub(super) fn rename_input(&self, what: &str) -> Option<Entity<LineInput>> {
        (self.renaming.as_deref() == Some(what)).then(|| self.inputs.get(&format!("rename:{what}")).map(|x| x.0.clone())).flatten()
    }

    // ───────────── 文件 ─────────────

    /// 选择文件并追加到附件字段（图片顺带生成缩略图）。
    pub(super) fn pick_files(&mut self, rid: String, fid: String, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions {
                files: true, directories: false, multiple: true, prompt: Some("添加附件".into()), ..Default::default()
            });
        cx.spawn(async move |this: WeakEntity<BaseEditor>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            // 存储句柄是线程局部的：资源目录须在主线程取好再交给后台复制，否则后台拿不到、附件全部丢失
            let Some(assets) = crate::objects::store().map(|s| s.assets_dir()) else { return };
            let files: Vec<Attachment> = cx.background_executor().spawn(async move { paths.iter().filter_map(|p| import_file(&assets, p)).collect() }).await;
            this.update(cx, |this, cx| {
                let tid = this.tid.clone();
                let mut list = attachments(this.b().record(&tid, &rid).and_then(|r| r.cells.get(&fid)));
                list.extend(files);
                this.set_cell(&rid, &fid, serde_json::to_value(list).unwrap_or(Value::Null), false, cx);
            })
            .ok();
        })
        .detach();
    }

    // ───────────── 渲染：顶栏 / 侧栏 / 视图标签 / 工具栏 ─────────────

    /// 查找命中的单元格（只在可见的文本类字段里找），按显示顺序。
    pub(super) fn find_hits(&mut self) -> Vec<(String, String)> {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let rows = self.rows();
        let fields = self.visible();
        let b = self.b();
        let mut out = vec![];
        for r in rows.iter() {
            for f in &fields {
                if b.text(&self.tid, r, &f.id).to_lowercase().contains(&q) {
                    out.push((r.clone(), f.id.clone()));
                }
            }
        }
        out
    }

    /// 上一处 / 下一处：选中并滚动到命中的单元格。
    pub(super) fn find_step(&mut self, d: i32, cx: &mut Context<Self>) {
        let hits = self.find_hits();
        if hits.is_empty() {
            return;
        }
        let n = hits.len() as i32;
        let cur = self.sel.as_ref().and_then(|s| hits.iter().position(|h| h.0 == s.rid && h.1 == s.fid));
        let next = match cur {
            Some(i) => (i as i32 + d).rem_euclid(n),
            None => {
                if d > 0 {
                    0
                } else {
                    n - 1
                }
            }
        } as usize;
        self.find_pos = next;
        let (rid, fid) = hits[next].clone();
        self.sel = Some(Sel { rid: rid.clone(), fid: fid.clone(), arid: rid, afid: fid });
        self.ensure_visible();
        cx.notify();
    }

    /// 把查找文本替换掉（只改文本类字段）。
    pub(super) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let q = self.search.trim().to_string();
        if q.is_empty() {
            return;
        }
        let with = self.input_text("cb:keep:replace", cx);
        let tid = self.tid.clone();
        let mut n = 0;
        for (rid, fid) in self.find_hits() {
            let Some(f) = self.field(&fid) else { continue };
            if !matches!(f.kind, FieldType::Text | FieldType::Url | FieldType::Phone | FieldType::Email) {
                continue;
            }
            let cur = self.b().record(&tid, &rid).and_then(|r| r.cells.get(&fid)).and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            let lower = cur.to_lowercase();
            let ql = q.to_lowercase();
            // 不区分大小写替换
            let mut out = String::new();
            let mut i = 0;
            while let Some(p) = lower[i..].find(&ql) {
                out.push_str(&cur[i..i + p]);
                out.push_str(&with);
                i += p + ql.len();
            }
            out.push_str(&cur[i..]);
            if out != cur {
                self.bm().set_cell(&tid, &rid, &fid, Value::from(out));
                n += 1;
            }
        }
        self.commit(cx);
        self.show_toast(&format!("已替换 {n} 个单元格（只替换文本类字段）"), cx);
    }

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        let s = self.search.clone();
        let i = self.input("search", "搜索记录", &s, true, window, cx);
        window.focus(&i.focus_handle(cx));
        cx.notify();
    }

    /// 勾选行后出现的批量操作条（R11）。
    fn render_batch_bar(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let n = self.checked.len();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .h(px(40.))
            .flex_none()
            .bg(t.accent_soft)
            .border_b_1()
            .border_color(t.border)
            .child(div().text_size(px(13.)).text_color(t.accent).mr(px(8.)).child(format!("已选 {n} 条记录")))
            .child(
                tool_btn("bb-edit", "pencil", "批量修改", false, t)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::BatchEdit, ev.position, cx))),
            )
            .child(tool_btn("bb-copy", "copy", "创建副本", false, t).on_click(cx.listener(|this, _, _, cx| {
                let tid = this.tid.clone();
                let rows = this.rows();
                let ids: Vec<String> = rows.iter().filter(|r| this.checked.contains(*r)).cloned().collect();
                for r in &ids {
                    this.bm().duplicate_record(&tid, r);
                }
                this.commit(cx);
                this.show_toast(&format!("已复制 {} 条记录", ids.len()), cx);
            })))
            .child(tool_btn("bb-csv", "download", "导出所选", false, t).on_click(cx.listener(|this, _, _, cx| this.export_checked(cx))))
            .child(tool_btn("bb-del", "trash", "删除", false, t).on_click(cx.listener(|this, _, _, cx| {
                let ids: Vec<String> = this.checked.drain().collect();
                this.delete_records(&ids, cx);
            })))
            .child(div().flex_1())
            .child(tool_btn("bb-all", "square-check", "全选", false, t).on_click(cx.listener(|this, _, _, cx| {
                let rows = this.rows();
                this.checked = rows.iter().cloned().collect();
                cx.notify();
            })))
            .child(tool_btn("bb-clear", "x", "取消选择", false, t).on_click(cx.listener(|this, _, _, cx| {
                this.checked.clear();
                cx.notify();
            })))
            .into_any_element()
    }

    fn export_checked(&mut self, cx: &mut Context<Self>) {
        let mut v = self.view();
        let ids = self.checked.clone();
        let tid = self.tid.clone();
        let csv = {
            let b = self.b();
            let t = b.table(&tid);
            let fields = t.map(|t| t.visible_fields(&v)).unwrap_or_default();
            let mut out = String::from("\u{feff}");
            out.push_str(&fields.iter().filter_map(|f| b.field(&tid, f)).map(|f| super::io::csv_escape(&f.name)).collect::<Vec<_>>().join(","));
            out.push('\n');
            v.filter.conds.clear();
            for r in b.view_records(&tid, &v, "").into_iter().filter(|r| ids.contains(r)) {
                out.push_str(&fields.iter().map(|f| super::io::csv_escape(&b.text(&tid, &r, f))).collect::<Vec<_>>().join(","));
                out.push('\n');
            }
            out
        };
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some("所选记录.csv"));
        cx.spawn(async move |_: WeakEntity<BaseEditor>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            if std::fs::write(&path, csv).is_ok() {
                cx.update(|cx| cx.reveal_path(&path)).ok();
            }
        })
        .detach();
    }

    fn render_toast(&self, t: &Theme) -> Option<AnyElement> {
        let (msg, _) = self.toast.as_ref()?;
        Some(
            div()
                .absolute()
                .bottom(px(48.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(16.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(if t.dark { rgb(0xe3e4e6) } else { rgb(0x1f2329) })
                        .text_color(if t.dark { rgb(0x1f2329) } else { rgb(0xffffff) })
                        .text_size(px(13.))
                        .shadow_lg()
                        .child(msg.clone()),
                )
                .into_any_element(),
        )
    }

    // ───────────── 导入导出 ─────────────

    pub(super) fn import_file_dialog(&mut self, cx: &mut Context<Self>) {
        self.close_popups();
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入 Excel / CSV".into()),
            ..Default::default()
        });
        cx.spawn(async move |this: WeakEntity<BaseEditor>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(p) = paths.first().cloned() else { return };
            this.update(cx, |this, cx| this.open_popup(Popup::Import(p.to_string_lossy().to_string()), point(px(420.), px(160.)), cx)).ok();
        })
        .detach();
    }

    /// 导入：`append` 为追加到当前数据表，否则新建数据表（xlsx 的每个工作表各一张）。
    pub fn import_path(&mut self, p: &std::path::Path, append: bool, cx: &mut Context<Self>) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("导入的数据".into());
        let sheets: Vec<(String, Vec<Vec<String>>)> = if ext == "xlsx" {
            let s = super::io::read_xlsx_sheets(&p.to_string_lossy());
            if s.len() == 1 {
                vec![(name.clone(), s.into_iter().next().map(|x| x.1).unwrap_or_default())]
            } else {
                s
            }
        } else {
            std::fs::read(p)
                .ok()
                .map(|bytes| {
                    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| encoding_rs::GBK.decode(&bytes).0.into_owned());
                    vec![(name.clone(), super::io::parse_csv(&text))]
                })
                .unwrap_or_default()
        };
        let sheets: Vec<(String, Vec<Vec<String>>)> = sheets.into_iter().filter(|s| !s.1.is_empty()).collect();
        if sheets.is_empty() {
            self.show_toast("文件里没有可导入的数据", cx);
            return;
        }
        if append {
            let tid = self.tid.clone();
            let mut n = 0;
            for (_, rows) in sheets {
                n += super::io::append_rows(&mut self.bm(), &tid, rows);
            }
            self.commit(cx);
            self.show_toast(&format!("已追加 {n} 条记录，列名对不上的已新建字段"), cx);
            return;
        }
        let mut last = None;
        let mut total = 0;
        let count = sheets.len();
        for (sname, rows) in sheets {
            total += rows.len().saturating_sub(1);
            last = super::io::import_rows(&mut self.bm(), &sname, rows).or(last);
        }
        self.commit(cx);
        if let Some(tid) = last {
            self.switch_table(&tid, cx);
            self.show_toast(&format!("已导入 {count} 张数据表、{total} 条记录，字段类型已自动识别"), cx);
        }
    }

    pub(super) fn export_csv(&mut self, cx: &mut Context<Self>) {
        self.close_popups();
        let v = self.view();
        let csv = super::io::view_csv(&self.b(), &self.tid, &v);
        let tname = self.b().table(&self.tid).map(|t| t.meta.name.clone()).unwrap_or_default();
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{tname}-{}.csv", v.name)));
        cx.spawn(async move |_: WeakEntity<BaseEditor>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            if std::fs::write(&path, csv).is_ok() {
                cx.update(|cx| cx.reveal_path(&path)).ok();
            }
        })
        .detach();
    }

    // ───────────── 拖拽 / 键盘分发 ─────────────

    fn root_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.mouse = ev.position;
        if self.drag.is_none() {
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.end_drag(ev.position, window, cx);
            return;
        }
        match &self.drag {
            Some(Drag::Card { .. }) => self.kanban_drag_move(ev.position, cx),
            Some(Drag::Bar { .. }) => self.gantt_drag_move(ev.position, cx),
            Some(Drag::Widget { .. }) => cx.notify(),
            Some(Drag::CalItem { start, .. }) => {
                let start = *start;
                if let Some(Drag::CalItem { moved, .. }) = &mut self.drag {
                    *moved = *moved || (ev.position - start).magnitude() > 4.;
                }
                cx.notify();
            }
            _ => self.grid_drag_move(ev.position, cx),
        }
    }

    fn end_drag(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.drag.take() else { return };
        match d {
            Drag::Card { .. } => self.kanban_drop(d, pos, cx),
            Drag::Bar { .. } => self.gantt_drop(d, cx),
            Drag::CalItem { .. } => self.calendar_drop(d, pos, cx),
            Drag::Widget { .. } => self.dashboard_drop(d, pos, cx),
            _ => self.grid_drop(d, pos, window, cx),
        }
        cx.notify();
    }

    fn on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_view.is_some() {
            self.file_view = None;
        } else if self.pick.is_some() {
            self.pick = None;
        } else if self.popup.is_some() {
            self.close_popups();
        } else if self.detail.is_some() {
            self.detail = None;
            self.inputs.retain(|k, _| !k.starts_with("d:"));
        } else if self.editing.is_some() {
            self.editing = None;
        } else if self.search_open {
            self.search_open = false;
            self.search.clear();
            self.inputs.remove("search");
        } else if self.sel.is_some() {
            self.sel = None;
        } else {
            cx.emit(BaseEvent::Close);
        }
        window.focus(&self.focus);
        cx.notify();
    }
}

/// 发 macOS 系统通知（自动化的「发送通知」动作）。
pub fn notify_system(title: &str, body: &str) {
    crate::sysui::notify(title, body);
}

/// 把外部文件拷进资源目录 `assets`（图片额外生成 480px 缩略图；可在后台线程调用）。
pub fn import_file(assets: &std::path::Path, src: &std::path::Path) -> Option<Attachment> {
    let name = src.file_name()?.to_string_lossy().to_string();
    let dir = assets.join(&crate::store::new_uuid()[..8]);
    std::fs::create_dir_all(&dir).ok()?;
    let dst = dir.join(&name);
    crate::assets::import_copy(src, &dst).ok()?;
    let size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
    let mut a = Attachment { path: dst.to_string_lossy().to_string(), name, size, thumb: String::new() };
    if a.is_image() {
        if let Ok(img) = image::open(&dst) {
            if img.width() > 600 || img.height() > 600 {
                let th = img.thumbnail(600, 600);
                let tp = dir.join(format!("thumb-{}.png", &crate::store::new_uuid()[..6]));
                if th.save(&tp).is_ok() {
                    a.thumb = tp.to_string_lossy().to_string();
                }
            }
        }
    }
    Some(a)
}

impl Render for BaseEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        // 视图可能被撤销掉
        if self.b().view(&self.tid, &self.vid).is_none() {
            self.fix_current();
        }
        let kind = self.view().kind;
        let content: AnyElement = if self.dash.is_some() {
            self.render_dashboard(&t, window, cx)
        } else {
            match kind {
                ViewKind::Grid => self.render_grid(&t, window, cx).into_any_element(),
                ViewKind::Kanban => self.render_kanban(&t, window, cx).into_any_element(),
                ViewKind::Gallery => self.render_gallery(&t, window, cx).into_any_element(),
                ViewKind::Calendar => self.render_calendar(&t, window, cx).into_any_element(),
                ViewKind::Gantt => self.render_gantt(&t, window, cx).into_any_element(),
                ViewKind::Form => self.render_form(&t, window, cx).into_any_element(),
            }
        };
        let batch = (!self.checked.is_empty() && self.dash.is_none()).then(|| self.render_batch_bar(&t, cx));
        let popup = self.render_popup(&t, window, cx);
        let pick = self.render_pick(&t, cx);
        let detail = self.render_detail(&t, window, cx);
        let ghost = self.render_drag_ghost(&t, cx);
        let toast = self.render_toast(&t);
        let file_view = self.render_file_view(&t, window, cx);
        let perf = (super::stress::SHOW.load(std::sync::atomic::Ordering::Relaxed) && !self.embedded).then(|| {
            use super::stress::*;
            let (h, m) = {
                let b = self.b();
                let c = b.cache.borrow();
                (c.hits, c.misses)
            };
            let rate = if h + m == 0 { 0. } else { h as f64 * 100. / (h + m) as f64 };
            let n = self.b().table(&self.tid).map(|t| t.records.len()).unwrap_or(0);
            div()
                .absolute()
                .bottom(px(44.))
                .right(px(16.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .shadow_lg()
                .font_family("Menlo")
                .text_size(px(11.))
                .text_color(t.text_secondary)
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(format!("● 视图管线 {:.2}ms · 提交 {:.2}ms · 载入 {:.0}ms", ms(&PIPE_US), ms(&COMMIT_US), ms(&LOAD_US)))
                .child(format!("{n} 条记录 · 计算缓存命中率 {rate:.1}%（{h} / {}）", h + m))
        });
        let topbar = (!self.embedded).then(|| self.render_topbar(&t, window, cx));
        let sidebar = (self.sidebar && !self.embedded).then(|| self.render_sidebar(&t, cx));
        let in_dash = self.dash.is_some();
        let tabs = (!in_dash).then(|| self.render_view_tabs(&t, cx));
        let toolbar = (!in_dash).then(|| self.render_toolbar(&t, window, cx));
        div()
            .key_context("Base")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .font_family("PingFang SC")
            .on_action(cx.listener(|this, _: &BUp, _, cx| this.move_sel(-1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &BDown, _, cx| this.move_sel(1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &BLeft, _, cx| this.move_sel(0, -1, false, cx)))
            .on_action(cx.listener(|this, _: &BRight, _, cx| this.move_sel(0, 1, false, cx)))
            .on_action(cx.listener(|this, _: &BExtUp, _, cx| this.move_sel(-1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &BExtDown, _, cx| this.move_sel(1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &BExtLeft, _, cx| this.move_sel(0, -1, true, cx)))
            .on_action(cx.listener(|this, _: &BExtRight, _, cx| this.move_sel(0, 1, true, cx)))
            .on_action(cx.listener(|this, _: &BTab, _, cx| this.move_sel(0, 1, false, cx)))
            .on_action(cx.listener(|this, _: &BShiftTab, _, cx| this.move_sel(0, -1, false, cx)))
            .on_action(cx.listener(|this, _: &BEnter, w, cx| this.start_cell_edit(None, w, cx)))
            .on_action(cx.listener(|this, _: &BSpace, _, cx| {
                if let Some(s) = &this.sel {
                    this.detail = Some(s.rid.clone());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &BEscape, w, cx| this.on_escape(w, cx)))
            .on_action(cx.listener(|this, _: &BDelete, _, cx| this.clear_selection(cx)))
            .on_action(cx.listener(|this, _: &BUndo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &BRedo, _, cx| this.redo(cx)))
            .on_action(cx.listener(|this, _: &BCopy, _, _| this.copy_selection()))
            .on_action(cx.listener(|this, _: &BPaste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &BFind, w, cx| this.open_search(w, cx)))
            .on_action(cx.listener(|this, _: &BSelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &BFirst, _, cx| this.move_sel(-1_000_000, 0, false, cx)))
            .on_action(cx.listener(|this, _: &BLast, _, cx| this.move_sel(1_000_000, 0, false, cx)))
            .on_action(cx.listener(|this, _: &BExpand, _, cx| {
                if let Some(s) = &this.sel {
                    this.detail = Some(s.rid.clone());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &BNewRecord, _, cx| {
                let after = this.sel.as_ref().map(|s| s.rid.clone());
                if let Some(rid) = this.add_record(vec![], after.as_deref(), cx) {
                    let fid = this.visible().first().map(|f| f.id.clone()).unwrap_or_default();
                    this.sel = Some(Sel { rid: rid.clone(), fid: fid.clone(), arid: rid, afid: fid });
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.type_to_edit(ev, window, cx)))
            .on_mouse_move(cx.listener(Self::root_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, w, cx| this.end_drag(ev.position, w, cx)))
            .children(topbar)
            .when(self.embedded, |d| d.rounded(px(8.)).border_1().border_color(t.border).overflow_hidden())
            .child(
                div().flex_1().min_h(px(0.)).flex().flex_row().children(sidebar).child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .children(tabs)
                        .children(toolbar)
                        .children(batch)
                        .child(div().flex_1().min_h(px(0.)).relative().child(content)),
                ),
            )
            .children(detail)
            .children(popup)
            .children(pick)
            .children(ghost)
            .children(toast)
            .children(file_view)
            .children(perf)
    }
}

impl crate::editor::ImgHost for BaseEditor {
    fn img_views(&mut self) -> &mut HashMap<u64, crate::editor::ImgView> {
        &mut self.img_views
    }
}

impl crate::editor::FileViewHost for BaseEditor {
    fn fv_media(&mut self) -> &mut Option<u64> {
        &mut self.fv_media
    }
    fn image_sizes(&mut self) -> &mut HashMap<String, Option<(u32, u32)>> {
        &mut self.image_sizes
    }
}
