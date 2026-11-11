//! 块编辑器。
//!
//! 结构：
//! - `mod.rs`   状态、选区、撤销、保存
//! - `ops.rs`   编辑操作（输入、删除、回车、格式、块转换……）
//! - `nav.rs`   光标移动与命中测试
//! - `clip.rs`  复制 / 粘贴 / 拖放
//! - `input.rs` IME（输入法）接入
//! - `render.rs` 渲染（虚拟列表 + 每个文本槽一个 StyledText）

mod archive;
mod attach;
mod audio;
mod blocks;
mod button;
mod clip;
mod countdown;
mod fileview;
pub(crate) mod fold;
mod imgview;
mod input;
mod links;
pub use links::DOC_LINK_PREFIX;
#[cfg(test)]
mod fuzz_tests;
mod media;
mod menus_render;
mod nav;
mod numbering;
mod ops;
mod reading;
mod render;
mod sync;
mod table_render;
mod timeline;
mod toolbar_render;
mod urlpaste;

pub use attach::is_image_name;
pub(crate) use attach::{
    file_icon, file_live_src, file_style, preview_kind, static_preview, video_player, Glyph, PlayerOpts, Preview, StaticPreview, STATIC_MAX_H,
};
pub(crate) use audio::audio_player;
pub(crate) use blocks::video_meta;
pub use blocks::{col_name, GpuiMeasure};
pub(crate) use fileview::{file_view_el, file_view_opening, release_file_view, save_file_as, FileViewHost};
pub(crate) use imgview::{img_viewer, viewable as img_viewable, ImgHost, ImgView, FULL_KEY as IMG_FULL_KEY};
pub use media::fmt_comment_time;
pub use render::fmt_num;

use crate::line_input::{InputEvent, LineInput};
use crate::model::{Block, DocContent, Marks, RichText};
use crate::store::Store;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        Backspace,
        Delete,
        DeleteWordBack,
        DeleteLineBack,
        Enter,
        SoftEnter,
        Tab,
        ShiftTab,
        SelectAll,
        Copy,
        Cut,
        Paste,
        PastePlain,
        Undo,
        Redo,
        Bold,
        Italic,
        Underline,
        Strike,
        InlineCode,
        InsertLink,
        SetText,
        SetH1,
        SetH2,
        SetH3,
        SetH4,
        SetBullet,
        SetOrdered,
        SetTodo,
        SetQuote,
        SetCode,
        ToggleTodo,
        DuplicateBlock,
        MoveBlockUp,
        MoveBlockDown,
        Escape,
        Find,
        FindNext,
        FindPrev,
        ShowCharacterPalette,
        InsertImage,
        InsertTimestamp,
        ToggleFold,
        FocusBlock
    ]
);

/// 光标位置：第几个块、块内第几个文本槽（表格单元格 / 分栏）、槽内 UTF-8 偏移。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub block: usize,
    pub cell: usize,
    pub offset: usize,
}

impl Pos {
    pub fn new(block: usize, cell: usize, offset: usize) -> Self {
        Pos { block, cell, offset }
    }
    pub fn slot(&self) -> (usize, usize) {
        (self.block, self.cell)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Typing,
    Delete,
    Other,
}

#[derive(Clone)]
struct Snapshot {
    blocks: Vec<Arc<Block>>,
    anchor: Pos,
    head: Pos,
}

/// 每帧渲染时收集的几何信息（文本布局、光标位置……），供命中测试 / 上下移动 / IME 使用。
#[derive(Default)]
pub struct FrameInfo {
    pub layouts: HashMap<(usize, usize), TextLayout>,
    pub voids: HashMap<usize, Bounds<Pixels>>,
    pub cursor: Option<Bounds<Pixels>>,
    pub sel_start: Option<Point<Pixels>>,
    /// 文本绘图预览：块序号 → (视图变换, 场景)，用于拖动节点
    pub diagrams: HashMap<usize, (crate::canvas::paint::View, Arc<crate::canvas::Scene>)>,
    /// HTML 块预览区域（交互模式叠加原生 WebView 用）
    pub html_bounds: HashMap<u64, Bounds<Pixels>>,
    /// 本帧渲染过的视频块 id 与播放器区域
    pub videos: HashMap<u64, Bounds<Pixels>>,
    /// 本帧图片块的显示区域（裁剪 / 缩放手柄换算用）
    pub images: HashMap<usize, Bounds<Pixels>>,
    /// 本帧每个块的整块区域（评论浮层定位用）
    pub blocks: HashMap<usize, Bounds<Pixels>>,
    /// 本帧评论输入浮层区域（判断点击是否落在浮层内）
    pub comment_input_bounds: Option<Bounds<Pixels>>,
    /// 本帧评论列表浮层区域
    pub comment_panel_bounds: Option<Bounds<Pixels>>,
    /// 本帧斜杠菜单区域（行列选择器定位用）
    pub slash_bounds: Option<Bounds<Pixels>>,
    /// 含行内公式的文本槽的偏移映射
    pub maps: HashMap<(usize, usize), Rc<render::DisplayMap>>,
}

/// 块属性编辑浮层（倒计时目标时间、按钮链接、网页卡片地址……）
pub struct PropEdit {
    pub input: Entity<LineInput>,
    pub at: Point<Pixels>,
    _sub: Subscription,
}

pub struct DiagramDrag {
    pub block: usize,
    pub node: String,
    pub start: crate::canvas::Pt,
    pub base: (f32, f32),
    /// 是否真的拖动过（只按下抬起不算）
    pub moved: bool,
}

/// 拖动图片角上的手柄等比缩放。
pub struct ImageDrag {
    pub block: usize,
    /// 拖动开始时的显示宽度
    pub start_w: f32,
    /// 拖动开始时的指针位置
    pub start_px: Point<Pixels>,
    /// 当前显示宽度预览
    pub width: f32,
    /// 指针横移 1px 对应的宽度变化：右侧手柄 +1、左侧 -1，居中图片两侧同时变化再 ×2
    pub dir: f32,
}

/// 图片裁剪模式：显示完整原图，拖动裁剪框的边角或整体移动，↵ 完成、Esc 取消。
pub struct CropEdit {
    pub block: usize,
    /// 裁剪框，按原图比例 (x, y, w, h)
    pub rect: (f32, f32, f32, f32),
    pub drag: Option<CropDrag>,
}

pub struct CropDrag {
    pub handle: CropHandle,
    pub start: Point<Pixels>,
    pub rect0: (f32, f32, f32, f32),
}

/// 裁剪框上被拖动的部位。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CropHandle {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

/// 浮动工具栏的下拉菜单。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TbMenu {
    Kind,
    Align,
    Color,
}

pub struct SlashMenu {
    pub block: usize,
    /// '/' 所在偏移
    pub start: usize,
    pub selected: usize,
    /// 上次滚动到的 (选中项, 关键词)：只在它们变化时滚到选中项，拖滚动条 / 滚轮时不被拉回去
    pub scrolled: std::cell::RefCell<Option<(usize, String)>>,
}

pub struct FindState {
    pub input: Entity<LineInput>,
    pub matches: Vec<(Pos, usize)>,
    pub current: usize,
    _sub: Subscription,
}

pub struct LinkEdit {
    pub input: Entity<LineInput>,
    _sub: Subscription,
}

/// 评论输入浮层。
pub struct CommentInput {
    pub block: usize,
    pub anchor: String,
    pub input: Entity<LineInput>,
    _sub: Subscription,
}

/// 表格几何（跨帧保留，按块 id 索引）：渲染时读上一帧的结果来摆放插入线与列宽拖柄。
#[derive(Default, Clone)]
pub struct TableGeo {
    /// 整张表（含手柄）在窗口中的区域
    pub grid: Bounds<Pixels>,
    /// 各列手柄条区域（给出每列的横向位置与宽度）
    pub cols: Vec<Bounds<Pixels>>,
    /// 各行手柄条区域（给出每行的纵向位置与高度）
    pub rows: Vec<Bounds<Pixels>>,
    /// 跨行合并格（按左上角单元格下标）需要的总高度
    pub need: HashMap<usize, f32>,
}

/// 拖动表格列边线调整列宽。
pub struct TableColDrag {
    pub block: usize,
    /// 被拖动的是第 `col` 列右侧的边线
    pub col: usize,
    pub start_x: Pixels,
    /// 拖动开始时各列像素宽度
    pub start: Vec<f32>,
    /// 当前预览宽度
    pub cur: Vec<f32>,
}

/// 鼠标停在表格行列边界上时显示的「插入」提示：(块, 是否为列, 插入位置)。
#[derive(Clone, Copy, PartialEq)]
pub struct TableInsertHover {
    pub block: usize,
    pub col: bool,
    pub at: usize,
}

/// 插入表格前的行列选择器。
pub struct TablePicker {
    /// 斜杠菜单所在的块（插入位置由它决定）
    pub block: usize,
    /// 当前预览的行列数
    pub rows: usize,
    pub cols: usize,
    /// 浮层顶边（跟随触发它的菜单项，避免鼠标横移时落到面板外）
    pub y: Pixels,
}

pub enum EditorEvent {
    Changed,
    Toast(String),
    /// ⌘ + 点击了指向某篇文档的链接
    OpenDoc(String),
    /// 打开嵌入对象的全屏编辑器
    OpenEmbed {
        kind: String,
        id: String,
    },
    /// 按钮块：创建当前文档副本
    DuplicateDoc,
    /// 打开模板选择面板
    PickTemplate,
    /// 「关联已有多维表格」
    PickBase,
}

impl EventEmitter<EditorEvent> for Editor {}

/// 编辑器里的弹出菜单 / 浮层状态。
#[derive(Default)]
pub struct Popups {
    pub slash: Option<SlashMenu>,
    pub block_menu: Option<usize>,
    pub lang_menu: Option<usize>,
    pub link_edit: Option<LinkEdit>,
    /// 浮动工具栏当前展开的下拉（文本类型 / 对齐 / 颜色）
    pub tb_menu: Option<TbMenu>,
    pub prop_edit: Option<PropEdit>,
    /// 按钮配置浮层
    pub button_edit: Option<button::ButtonEdit>,
    /// 倒计时设置弹窗
    pub countdown_edit: Option<countdown::CountdownEdit>,
    /// 正在显示颜色浮层的倒计时块
    pub countdown_color: Option<usize>,
    /// 时间轴右上角菜单
    pub tl_menu: Option<timeline::TlMenu>,
    /// 斜杠菜单里 hover「表格」时右侧弹出的行列选择器
    pub table_picker: Option<TablePicker>,
    /// 打开了「配色」下拉菜单的绘图块
    pub diagram_theme_menu: Option<usize>,
    pub sync_menu: bool,
    /// 有序列表编号菜单
    pub ordered_menu: Option<numbering::OrderedMenu>,
    /// `@` 文档选择菜单
    pub mention: Option<links::MentionMenu>,
}

/// 编辑核心：选区、输入法、撤销 / 重做。切换文档时整体重置。
#[derive(Default)]
pub struct EditCore {
    pub anchor: Pos,
    pub head: Pos,
    pub marked: Option<Range<usize>>,
    pub goal_x: Option<Pixels>,
    pub pending_marks: Option<Marks>,
    undo_stack: std::collections::VecDeque<Snapshot>,
    redo_stack: Vec<Snapshot>,
    pub last_edit: Option<(EditKind, Instant)>,
    pub selecting: bool,
}

/// 渲染期缓存：只影响性能，丢掉会在下一帧重建。
#[derive(Default)]
pub struct RenderCaches {
    pub numbers: (u64, Vec<usize>),
    pub stats: (u64, usize),
    pub frame_times: VecDeque<f32>,
    pub render_start: Option<Instant>,
    pub image_sizes: RefCell<HashMap<String, Option<(u32, u32)>>>,
    /// 表格几何缓存（按块 id）
    pub table_geo: Rc<RefCell<HashMap<u64, TableGeo>>>,
    pub diagram_cache: RefCell<HashMap<u64, Result<Arc<crate::canvas::Scene>, crate::diagram::DiagramError>>>,
    pub diagram_good: RefCell<HashMap<u64, Arc<crate::canvas::Scene>>>,
    /// 文本绘图源码区的滚动状态（按块 id）
    pub diagram_scroll: RefCell<HashMap<u64, (ScrollHandle, crate::ui::ScrollDrag)>>,
    /// 不换行空格宽度 / 字号（每帧测量）
    pub nbsp_w: std::cell::Cell<f32>,
    pub nbsp_measured: bool,
    pub reveal_pending: std::cell::Cell<u8>,
    /// 隐藏块掩码缓存：((版本, 聚焦根), 掩码)
    pub fold_cache: RefCell<Option<((u64, Option<u64>), Rc<Vec<bool>>)>>,
}

/// 当前文档的界面会话状态（浮层、预览、拖动、阅读模式…），指向块 id / 下标，切换文档时整体重置。
#[derive(Default)]
pub struct DocSession {
    pub html_live: Option<u64>,
    pub video_full: Option<u64>,
    /// 「视频或文件」块的全屏预览（顶栏 退出 / 文件名 / 下载 + 下方内容）
    pub file_full: Option<u64>,
    /// 文件块里图片的查看状态（缩放 / 旋转 / 平移），不写进文档
    pub img_views: HashMap<u64, imgview::ImgView>,
    /// 鼠标悬停的文件块（显示视图切换工具栏）
    pub file_hover: Option<u64>,
    pub diagram_drag: Option<DiagramDrag>,
    pub image_drag: Option<ImageDrag>,
    /// 图片裁剪模式
    pub image_crop: Option<CropEdit>,
    /// 全屏查看大图
    pub image_preview: Option<String>,
    /// 评论输入浮层
    pub comment_input: Option<CommentInput>,
    /// 正在回复的块
    pub comment_open: Option<usize>,
    /// 悬停工具栏当前显示的按钮块 id + 延迟隐藏序号（鼠标从按钮移向工具栏途中不消失）
    pub button_bar: Option<u64>,
    /// 正在拖动的表格列边线
    pub table_col_drag: Option<TableColDrag>,
    /// 行列边界上的「插入」提示
    pub table_insert_hover: Option<TableInsertHover>,
    /// 弹框查看大图的绘图块 id
    pub diagram_preview: Option<u64>,
    /// 大图弹框的缩放倍数（相对「适应窗口」）
    pub diagram_preview_zoom: f32,
    /// 在预览图上按下鼠标的 (块, 位置)：抬起时没移动就算单击（弹框预览）
    pub diagram_click: Option<(usize, Point<Pixels>)>,
    pub url_paste: Option<urlpaste::UrlPaste>,
    /// 阅读模式：不接受修改（见 reading.rs）
    pub read_only: bool,
    /// 进入阅读模式时的文档快照，漏进来的修改回滚到它
    pub ro_doc: Option<Vec<Arc<Block>>>,
    /// 正在编辑的行内公式 (块, 槽, 偏移)
    pub inline_math_at: Option<(usize, usize, usize)>,
    /// 聚焦模式的根块 id（见 fold.rs）
    pub focus_root: Option<u64>,
}

/// 格式工具栏的悬停 / 下拉状态，跨文档保留。
#[derive(Default)]
pub struct ToolbarState {
    /// 「其他标题」子项是否展开
    pub tb_more_heading: bool,
    /// 悬停展开的工具栏下拉：鼠标所在的悬停区（触发按钮 / 面板）个数，移出后延时收起，
    /// 到点时仍为 0 才收起（同一次移动里「进面板」「出按钮」的先后不固定，不能只看最后一个事件）
    pub tb_hover_count: usize,
    pub tb_hover_epoch: usize,
    /// 上次使用的 (文字颜色, 背景色)，点工具栏「A」直接套用
    pub last_color: (Option<u8>, Option<u8>),
}

pub struct Editor {
    pub doc_id: String,
    /// `@` 菜单可选的文档（工作区设置）
    doc_list: links::DocList,
    pub content: DocContent,
    pub version: u64,
    /// 文档内的各种弹出菜单 / 浮层（可以同时存在，Esc 按优先级逐个收起）
    pub pop: Popups,
    pub store: Store,
    pub focus: FocusHandle,
    pub list: ListState,
    pub frame: Rc<RefCell<FrameInfo>>,
    pub cursor_visible: bool,
    last_activity: Instant,
    pub find: Option<FindState>,
    pub show_perf: bool,
    dirty: bool,
    save_task: Option<Task<()>>,
    _blink: Task<()>,
    pub typed_today: usize,
    /// 已弹过结束提醒的倒计时（块 id）
    countdown_fired: std::collections::HashSet<u64>,
    /// 文档里直接可编辑的电子表格（对象 id → 内嵌表格视图）
    pub live_sheets: std::collections::HashMap<String, (Entity<crate::sheet::SheetEditor>, Subscription)>,
    /// 文档里内嵌的可交互多维表格（R12）
    pub live_bases: std::collections::HashMap<String, (Entity<crate::base::editor::BaseEditor>, Subscription)>,
    /// 「视频或文件」后台拷贝中的文件：目标路径 → 进度（千分比）
    pub uploads: HashMap<String, Arc<std::sync::atomic::AtomicU32>>,
    pub file_hover_epoch: u64,
    /// 正文实际可用宽度（窗口窄于 content_w 时会更小），跨帧保留
    pub avail_w: Rc<std::cell::Cell<f32>>,
    /// 当前文档的评论（缓存）
    pub comments: Vec<crate::store::Comment>,
    /// 行列选择器的延迟关闭序号（鼠标在菜单项与浮层之间移动时避免误关）
    pub table_hover_epoch: u64,
    pub button_bar_epoch: u64,
    /// 斜杠菜单本帧的锚点（未 snap）；菜单 bounds 尚未测到时用它兜底
    pub slash_at: Point<Pixels>,
    /// 单击弹框的延迟序号：双击会让它作废
    pub diagram_click_epoch: u64,
    /// 各同步组上次写入同步表的内容
    sync_saved: std::collections::HashMap<String, String>,
    pub slash_scroll: ScrollHandle,
    /// 斜杠菜单滚动条的拖动状态
    pub slash_bar: crate::ui::ScrollDrag,
    /// 是否需要闪烁光标（渲染时按焦点 / 窗口激活状态更新）
    blink_on: bool,
    pub core: EditCore,
    pub caches: RenderCaches,
    pub sess: DocSession,
    pub tb: ToolbarState,
}

pub const MAX_INDENT: u8 = 8;

impl Editor {
    pub fn new(store: Store, doc_id: String, content: DocContent, cx: &mut Context<Self>) -> Self {
        let n = content.blocks.len();
        let blink = cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(Duration::from_millis(530)).await;
            let alive = this.update(cx, |this, cx| {
                if !this.blink_on {
                    this.cursor_visible = true;
                } else if this.last_activity.elapsed() > Duration::from_millis(500) {
                    this.cursor_visible = !this.cursor_visible;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        });
        let start = Pos::new(if n > 1 { 1 } else { 0 }, 0, 0);
        // 启动时就把已有评论读进缓存，否则重启后图片工具栏 / 顶部角标都显示 0
        let mut comments = store.comments(&doc_id);
        for c in &mut comments {
            c.block_id = content.blocks.get(c.block).map_or(0, |b| b.id);
        }
        Editor {
            doc_id,
            doc_list: Default::default(),
            content,
            version: 1,
            pop: Popups::default(),
            store,
            focus: cx.focus_handle(),
            list: ListState::new(n, ListAlignment::Top, px(600.)),
            frame: Rc::new(RefCell::new(FrameInfo::default())),
            cursor_visible: true,
            last_activity: Instant::now(),
            find: None,
            show_perf: false,
            dirty: false,
            save_task: None,
            _blink: blink,
            typed_today: 0,
            countdown_fired: Default::default(),
            live_sheets: Default::default(),
            live_bases: Default::default(),
            uploads: HashMap::new(),
            file_hover_epoch: 0,
            avail_w: Rc::new(std::cell::Cell::new(render::content_w())),
            comments,
            table_hover_epoch: 0,
            button_bar_epoch: 0,
            slash_at: point(px(0.), px(0.)),
            diagram_click_epoch: 0,
            sync_saved: Default::default(),
            slash_scroll: ScrollHandle::new(),
            slash_bar: Default::default(),
            blink_on: false,
            core: EditCore { anchor: start, head: start, ..Default::default() },
            caches: RenderCaches { nbsp_w: std::cell::Cell::new(0.3), frame_times: VecDeque::with_capacity(240), ..Default::default() },
            sess: DocSession::default(),
            tb: ToolbarState { last_color: (None, Some(3)), ..Default::default() },
        }
    }

    /// 切换到另一篇文档（复用同一个编辑器实例）。
    pub fn load(&mut self, doc_id: String, content: DocContent, cx: &mut Context<Self>) {
        self.save_now(cx);
        self.doc_id = doc_id;
        self.content = content;
        self.pull_sync();
        let n = self.content.blocks.len();
        self.version += 1;
        self.list.reset(n);
        let empty_title = self.content.blocks[0].text.is_empty();
        let start = Pos::new(if empty_title || n < 2 { 0 } else { 1 }, 0, 0);
        // 换文档：编辑核心与会话状态整体重置（浮层、预览、阅读模式、聚焦根都指向旧文档的块）
        self.core = EditCore { anchor: start, head: start, ..Default::default() };
        self.sess = DocSession::default();
        self.pop.slash = None;
        self.pop.block_menu = None;
        self.pop.lang_menu = None;
        self.pop.ordered_menu = None;
        self.pop.mention = None;
        self.pop.link_edit = None;
        self.pop.diagram_theme_menu = None;
        self.pop.table_picker = None;
        self.refresh_comments();
        if self.find.is_some() {
            self.refresh_find(cx);
        }
        // 上次没下载完的网络图片
        self.fetch_remote_images(cx);
        cx.notify();
    }

    // ───────────── 基础查询 ─────────────

    pub fn slot_text(&self, slot: (usize, usize)) -> Option<&RichText> {
        self.content.blocks.get(slot.0).and_then(|b| b.slot(slot.1))
    }

    pub fn slot_len(&self, slot: (usize, usize)) -> usize {
        self.slot_text(slot).map(|t| t.len()).unwrap_or(0)
    }

    /// 每个块至少有一个「光标停靠位」（空块如分割线 / 图片也能被选中）。
    pub fn stops(&self, block: usize) -> usize {
        self.content.blocks.get(block).map(|b| b.slot_count().max(1)).unwrap_or(1)
    }

    pub fn clamp(&self, p: Pos) -> Pos {
        let block = p.block.min(self.content.blocks.len().saturating_sub(1));
        let cell = p.cell.min(self.stops(block) - 1);
        let mut offset = p.offset;
        if let Some(t) = self.slot_text((block, cell)) {
            offset = offset.min(t.len());
            while !t.text.is_char_boundary(offset) {
                offset -= 1;
            }
        } else {
            offset = 0;
        }
        Pos { block, cell, offset }
    }

    pub fn has_selection(&self) -> bool {
        self.core.anchor != self.core.head
    }

    pub fn sel(&self) -> (Pos, Pos) {
        if self.core.anchor <= self.core.head {
            (self.core.anchor, self.core.head)
        } else {
            (self.core.head, self.core.anchor)
        }
    }

    /// 选区在某个文本槽内覆盖的字节区间。
    pub fn sel_in_slot(&self, slot: (usize, usize)) -> Option<Range<usize>> {
        if !self.has_selection() {
            return None;
        }
        let (s, e) = self.sel();
        // 表格内跨格选择按矩形整格高亮（见 table_sel_rect），格内文字不再画选区
        if s.block == e.block && s.cell != e.cell && slot.0 == s.block && self.table_sel_rect(s.block).is_some() {
            return None;
        }
        if slot < s.slot() || slot > e.slot() {
            return None;
        }
        let from = if slot == s.slot() { s.offset } else { 0 };
        let to = if slot == e.slot() { e.offset } else { self.slot_len(slot) };
        (from < to || (from == to && slot != s.slot() && slot != e.slot())).then_some(from..to)
    }

    /// 选区落在同一张表格的多个单元格上时，返回选中的矩形 (r1, c1, r2, c2)。
    pub fn table_sel_rect(&self, b: usize) -> Option<(usize, usize, usize, usize)> {
        let (s, e) = self.sel();
        if s.block != b || e.block != b || s.cell == e.cell {
            return None;
        }
        match &self.content.blocks.get(b)?.kind {
            crate::model::BlockKind::Table { cols, merges, .. } => Some(crate::model::table_rect(*cols, merges, s.cell, e.cell)),
            _ => None,
        }
    }

    pub fn block_selected(&self, block: usize) -> bool {
        let (s, e) = self.sel();
        (self.has_selection() && s.block <= block && block <= e.block) || (self.core.head.block == block && self.content.blocks[block].kind.is_void())
    }

    /// 文本槽内模型偏移的屏幕坐标（考虑行内公式映射）。
    pub fn pos_for(&self, slot: (usize, usize), offset: usize) -> Option<Point<Pixels>> {
        let f = self.frame.borrow();
        let layout = f.layouts.get(&slot)?;
        let d = f.maps.get(&slot).map_or(offset, |m| m.m2d(offset));
        layout.position_for_index(d)
    }

    pub fn ordered_numbers(&mut self) -> &[usize] {
        if self.caches.numbers.0 != self.version {
            self.caches.numbers = (self.version, crate::convert::ordered_numbers(&self.content.blocks));
        }
        &self.caches.numbers.1
    }

    pub fn char_count(&mut self) -> usize {
        if self.caches.stats.0 != self.version {
            self.caches.stats = (self.version, self.content.char_count());
        }
        self.caches.stats.1
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        *self.caches.image_sizes.borrow_mut().entry(path.to_string()).or_insert_with(|| image::image_dimensions(path).ok())
    }

    // ───────────── 修改原语 ─────────────

    pub fn block_mut(&mut self, i: usize) -> &mut Block {
        self.caches.fold_cache.get_mut().take();
        self.list.splice(i..i + 1, 1);
        Arc::make_mut(&mut self.content.blocks[i])
    }

    pub fn slot_mut(&mut self, slot: (usize, usize)) -> &mut RichText {
        self.block_mut(slot.0).slot_mut(slot.1).expect("slot exists")
    }

    pub fn insert_blocks(&mut self, at: usize, blocks: Vec<Block>) {
        let n = blocks.len();
        self.caches.fold_cache.get_mut().take();
        self.content.blocks.splice(at..at, blocks.into_iter().map(Arc::new));
        self.list.splice(at..at, n);
    }

    pub fn remove_blocks(&mut self, range: Range<usize>) {
        let range = range.start.max(1)..range.end.min(self.content.blocks.len());
        if range.is_empty() {
            return;
        }
        self.caches.fold_cache.get_mut().take();
        self.content.blocks.drain(range.clone());
        self.list.splice(range, 0);
        if self.content.blocks.len() == 1 {
            self.insert_blocks(1, vec![Block::text("")]);
        }
    }

    /// 用新的块序列整体替换（撤销 / 重做），只 splice 真正变化的区间以保留滚动位置。
    fn replace_all(&mut self, new: Vec<Arc<Block>>) {
        let old = &self.content.blocks;
        let mut prefix = 0;
        while prefix < old.len() && prefix < new.len() && Arc::ptr_eq(&old[prefix], &new[prefix]) {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < old.len() - prefix && suffix < new.len() - prefix && Arc::ptr_eq(&old[old.len() - 1 - suffix], &new[new.len() - 1 - suffix]) {
            suffix += 1;
        }
        self.list.splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
        self.content.blocks = new;
        self.caches.fold_cache.get_mut().take();
    }

    /// 记录撤销点。连续输入 / 连续删除合并为一步。
    pub fn checkpoint(&mut self, kind: EditKind) {
        // 阅读模式：修改会在 changed() 里回滚，不记撤销点
        if self.sess.read_only {
            return;
        }
        let now = Instant::now();
        let coalesce = kind != EditKind::Other && self.core.last_edit.is_some_and(|(k, t)| k == kind && now.duration_since(t) < Duration::from_millis(1200));
        if !coalesce {
            self.core.undo_stack.push_back(Snapshot { blocks: self.content.blocks.clone(), anchor: self.core.anchor, head: self.core.head });
            if self.core.undo_stack.len() > 500 {
                self.core.undo_stack.pop_front();
            }
        }
        self.core.redo_stack.clear();
        self.core.last_edit = Some((kind, now));
    }

    pub fn break_coalescing(&mut self) {
        self.core.last_edit = None;
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snap) = self.core.undo_stack.pop_back() {
            self.core.redo_stack.push(Snapshot { blocks: self.content.blocks.clone(), anchor: self.core.anchor, head: self.core.head });
            self.restore(snap, cx);
        }
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snap) = self.core.redo_stack.pop() {
            self.core.undo_stack.push_back(Snapshot { blocks: self.content.blocks.clone(), anchor: self.core.anchor, head: self.core.head });
            self.restore(snap, cx);
        }
    }

    fn restore(&mut self, snap: Snapshot, cx: &mut Context<Self>) {
        self.replace_all(snap.blocks);
        self.core.anchor = self.clamp(snap.anchor);
        self.core.head = self.clamp(snap.head);
        self.core.marked = None;
        self.core.last_edit = None;
        self.pop.slash = None;
        self.changed(cx);
        self.reveal_head();
    }

    /// 内容发生变化：版本号 +1、安排自动保存、通知界面。
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        if self.reject_read_only_edit(cx) {
            return;
        }
        // 块被移动 / 删除后 `@` 菜单记的位置可能失效
        self.update_mention();
        // 编辑把光标带进了折叠区域（如合并到被收起的上一块）：展开。
        // 有些操作直接改块数组（移动块等），先丢掉掩码缓存
        self.caches.fold_cache.get_mut().take();
        if self.reveal_block(self.core.head.block) {
            self.list.reset(self.content.blocks.len());
        }
        // 回到正文编辑：收起评论卡片（正在写评论时除外）
        if self.sess.comment_input.is_none() {
            self.sess.comment_open = None;
        }
        self.version += 1;
        self.dirty = true;
        self.touch_activity();
        if self.find.is_some() {
            self.refresh_find(cx);
        }
        self.schedule_save(cx);
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    pub fn touch_activity(&mut self) {
        self.cursor_visible = true;
        self.last_activity = Instant::now();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            this.update(cx, |this, cx| this.save_now(cx)).ok();
        }));
    }

    /// 立即保存：放进写队列（序列化与写库都在写线程）。退出时由 `Store::flush` 保证写完。
    pub fn save_now(&mut self, _cx: &mut Context<Self>) {
        self.save_now_queued();
    }

    /// 同 `save_now`，不需要 `Context`（关窗口回调里用）。
    pub fn save_now_queued(&mut self) {
        self.sync_comment_blocks();
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.save_task = None;
        self.push_sync();
        self.flush_stats();
        self.store.save(&self.doc_id, &self.content);
    }

    pub fn is_saved(&self) -> bool {
        !self.dirty
    }

    // ───────────── 光标设置 ─────────────

    pub fn set_cursor(&mut self, p: Pos, extend: bool, cx: &mut Context<Self>) {
        // 裁剪中点到别处：同样直接应用当前裁剪框
        if self.sess.image_crop.as_ref().is_some_and(|c| c.block != p.block) {
            self.crop_commit(cx);
        }
        let p = self.snap_merge(self.clamp(p));
        // 光标进入折叠 / 聚焦之外的区域：先展开
        self.caches.fold_cache.get_mut().take();
        if self.reveal_block(p.block) {
            self.list.reset(self.content.blocks.len());
            self.schedule_save(cx);
        }
        self.core.head = p;
        if !extend {
            self.core.anchor = p;
        }
        self.core.marked = None;
        self.core.pending_marks = None;
        self.break_coalescing();
        self.touch_activity();
        if let Some(s) = &self.pop.slash {
            if p.block != s.block || p.offset <= s.start {
                self.pop.slash = None;
            }
        }
        self.pop.block_menu = None;
        self.pop.lang_menu = None;
        self.pop.ordered_menu = None;
        self.pop.mention = None;
        self.reveal_head();
        cx.notify();
    }

    /// 光标落进被合并覆盖的单元格时，归位到合并区左上角。
    pub fn snap_merge(&self, p: Pos) -> Pos {
        if let Some(blk) = self.content.blocks.get(p.block) {
            if let crate::model::BlockKind::Table { cols, merges, .. } = &blk.kind {
                let cols = (*cols).max(1);
                let (r, c) = (p.cell / cols, p.cell % cols);
                if let Some(m) = crate::model::merge_at(merges, r, c) {
                    if (m.row, m.col) != (r, c) {
                        return Pos::new(p.block, m.row * cols + m.col, 0);
                    }
                }
            }
        }
        p
    }

    /// 请求「让光标可见」。真正的滚动在本帧绘制完成后进行（那时才知道光标的精确坐标），
    /// 见 render.rs 中的 `finish_reveal`。
    pub fn reveal_head(&self) {
        self.caches.reveal_pending.set(3);
    }

    /// 绘制后调用：根据光标的真实位置精确滚动。返回是否需要再绘制一帧。
    pub(crate) fn finish_reveal(&self) -> bool {
        let left = self.caches.reveal_pending.get();
        if left == 0 {
            return false;
        }
        self.caches.reveal_pending.set(left - 1);
        if self.reveal_in_diagram_source() {
            return true;
        }
        let vp = self.list.viewport_bounds();
        if vp.size.height <= px(0.) {
            return false;
        }
        let margin = px(48.);
        let frame = self.frame.borrow();
        let target = frame.cursor.or_else(|| frame.voids.get(&self.core.head.block).copied());
        match target {
            Some(c) if c.top() >= vp.top() && c.bottom() <= vp.bottom() => {
                self.caches.reveal_pending.set(0);
                false
            }
            Some(c) if c.bottom() > vp.bottom() => {
                self.list.scroll_by(c.bottom() - vp.bottom() + margin);
                true
            }
            Some(c) => {
                self.list.scroll_by(c.top() - vp.top() - margin);
                true
            }
            None => {
                // 光标所在块不在渲染范围内：先把它滚到视口上方附近，下一帧再精调
                self.list.scroll_to(ListOffset { item_ix: self.core.head.block.saturating_sub(1), offset_in_item: px(0.) });
                true
            }
        }
    }

    /// 光标在文本绘图源码区（限高、可滚动）里时，先滚动源码区让光标可见。返回是否滚动了。
    fn reveal_in_diagram_source(&self) -> bool {
        let Some(blk) = self.content.blocks.get(self.core.head.block) else { return false };
        if !matches!(blk.kind, crate::model::BlockKind::Diagram { .. }) {
            return false;
        }
        let Some((h, _)) = self.caches.diagram_scroll.borrow().get(&blk.id).cloned() else { return false };
        let Some(c) = self.frame.borrow().cursor else { return false };
        let (view, max) = (h.bounds(), h.max_offset().height);
        if max <= px(0.) || view.size.height <= px(0.) {
            return false;
        }
        let margin = px(8.);
        let y = h.offset().y;
        let ny = if c.bottom() > view.bottom() - margin {
            y - (c.bottom() - view.bottom() + margin)
        } else if c.top() < view.top() + margin {
            y + (view.top() + margin - c.top())
        } else {
            return false;
        };
        let ny = ny.clamp(-max, px(0.));
        if ny == y {
            return false;
        }
        h.set_offset(point(h.offset().x, ny));
        true
    }

    pub fn scroll_to_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        self.set_cursor(Pos::new(ix, 0, 0), false, cx);
    }

    /// 在光标所在块之后插入若干块（导图大纲、对象嵌入等）。
    pub fn insert_blocks_after_cursor(&mut self, blocks: Vec<Block>, cx: &mut Context<Self>) {
        if blocks.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let at = (self.core.head.block + 1).min(self.content.blocks.len());
        let n = blocks.len();
        self.insert_blocks(at, blocks);
        self.set_cursor(Pos::new(at + n - 1, 0, 0), false, cx);
        self.changed(cx);
    }

    pub fn insert_embed(&mut self, kind: &str, id: &str, cx: &mut Context<Self>) {
        let b = Block::new(crate::model::BlockKind::Embed { kind: kind.into(), id: id.into() }, crate::model::RichText::default());
        self.insert_blocks_after_cursor(vec![b], cx);
    }

    /// 字号 / 页宽变化后重新排版（保留滚动位置）。
    pub fn relayout(&mut self, cx: &mut Context<Self>) {
        let top = self.list.logical_scroll_top();
        self.list.reset(self.content.blocks.len());
        self.list.scroll_to(top);
        self.version += 1;
        cx.notify();
    }

    /// 把今天的输入字数累加到写作统计。
    fn flush_stats(&mut self) {
        if self.typed_today == 0 {
            return;
        }
        let key = format!("stats:{}", chrono::Local::now().format("%Y-%m-%d"));
        self.store.add_kv_int(&key, self.typed_today as i64);
        self.typed_today = 0;
    }

    pub fn current_section(&self) -> usize {
        self.list.logical_scroll_top().item_ix
    }

    // ───────────── 查找 ─────────────

    pub fn open_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = &self.find {
            let input = f.input.clone();
            window.focus(&input.focus_handle(cx));
            return;
        }
        let input = cx.new(|cx| LineInput::new("在文档中查找", cx));
        // 有选区时用选中文本作为初始查询
        if let Some(r) = self.sel_in_slot(self.core.head.slot()).filter(|_| self.core.anchor.slot() == self.core.head.slot()) {
            if let Some(t) = self.slot_text(self.core.head.slot()) {
                let q = t.text[r].to_string();
                input.update(cx, |i, cx| i.set_text(&q, cx));
            }
        }
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => this.refresh_find(cx),
            InputEvent::Submit { shift } => this.find_step(if *shift { -1 } else { 1 }, cx),
            InputEvent::Cancel => this.close_find(window, cx),
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.find = Some(FindState { input, matches: vec![], current: 0, _sub: sub });
        self.refresh_find(cx);
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let Some(f) = &self.find else { return };
        let q = f.input.read(cx).text().to_lowercase();
        let mut matches = Vec::new();
        if !q.is_empty() {
            for (bi, b) in self.content.blocks.iter().enumerate() {
                for c in 0..b.slot_count() {
                    let Some(t) = b.slot(c) else { continue };
                    let lower = t.text.to_lowercase();
                    if lower.len() != t.text.len() {
                        // 大小写转换改变了字节长度（少见），退化为精确匹配
                        for (ix, _) in t.text.match_indices(q.as_str()) {
                            matches.push((Pos::new(bi, c, ix), q.len()));
                        }
                        continue;
                    }
                    for (ix, _) in lower.match_indices(q.as_str()) {
                        matches.push((Pos::new(bi, c, ix), q.len()));
                    }
                }
            }
        }
        let head = self.core.head;
        let f = self.find.as_mut().unwrap();
        f.current = matches.iter().position(|(p, _)| *p >= head).unwrap_or(0);
        f.matches = matches;
        cx.notify();
    }

    pub fn find_step(&mut self, dir: i32, cx: &mut Context<Self>) {
        let Some(f) = &mut self.find else { return };
        if f.matches.is_empty() {
            return;
        }
        let n = f.matches.len() as i32;
        f.current = ((f.current as i32 + dir).rem_euclid(n)) as usize;
        let (p, len) = f.matches[f.current];
        if self.reveal_block(p.block) {
            self.list.reset(self.content.blocks.len());
        }
        self.core.anchor = p;
        self.core.head = Pos { offset: p.offset + len, ..p };
        self.reveal_head();
        cx.notify();
    }

    pub fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(1, cx);
    }

    pub fn find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(-1, cx);
    }

    pub fn perf_stats(&self) -> (f32, f32, f32) {
        if self.caches.frame_times.is_empty() {
            return (0., 0., 0.);
        }
        let mut v: Vec<f32> = self.caches.frame_times.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = v[v.len() / 2];
        let p95 = v[((v.len() as f32 * 0.95) as usize).min(v.len() - 1)];
        let max = *v.last().unwrap();
        (med, p95, max)
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
