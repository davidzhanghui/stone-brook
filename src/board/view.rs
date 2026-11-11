//! 画板 / 流程图全屏编辑器（交互与界面）。
//!
//! 结构：
//! - `view.rs`：状态、快捷键、命中测试、整体渲染
//! - `interact.rs`：鼠标交互（选择、拖动吸附、缩放手柄、快捷创建、连线吸附、分区、右键平移）
//! - `ops.rs`：命令（撤销、剪贴板、层级、对齐、样式修改、原位文字编辑）
//! - `ui.rs`：左侧工具栏与浮出面板、顶栏、画笔栏、缩放栏、小地图
//! - `selbar.rs`：选中元素上方的浮动属性栏、下拉面板、右键菜单、图形选择器

use super::model::*;
use crate::canvas::paint::{paint_scene, View};
use crate::canvas::{pt, Pt, Rect, ShapeKind};
use crate::text_area::TextArea;
use crate::theme::theme;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

actions!(
    board,
    [
        BDelete,
        BUndo,
        BRedo,
        BSelectAll,
        BDuplicate,
        BCopy,
        BPaste,
        BGroup,
        BUngroup,
        BEscape,
        BNudgeLeft,
        BNudgeRight,
        BNudgeUp,
        BNudgeDown,
        BNudgeLeftBig,
        BNudgeRightBig,
        BNudgeUpBig,
        BNudgeDownBig,
        BZoomIn,
        BZoomOut,
        BZoomFit,
        BZoomReset,
        BEdit,
        BToolSelect,
        BToolHand,
        BToolRect,
        BToolSquare,
        BToolEllipse,
        BToolDiamond,
        BToolArrow,
        BToolPen,
        BToolText,
        BToolSticky,
        BToolEraser,
        BToolSection,
        BToggleGrid,
        BToggleQuick,
        BFront,
        BBack,
        BLock,
        BTab,
        BShiftTab,
        BCollapse,
        BFind,
        BCopyStyle,
        BPasteStyle,
        BLibrary,
        BFlipH,
        BFlipV,
        BCopyImage,
        BBold,
        BItalic,
        BUnderline,
        BStrike
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Board");
    cx.bind_keys([
        KeyBinding::new("backspace", BDelete, c),
        KeyBinding::new("delete", BDelete, c),
        KeyBinding::new("cmd-z", BUndo, c),
        KeyBinding::new("cmd-shift-z", BRedo, c),
        KeyBinding::new("cmd-a", BSelectAll, c),
        KeyBinding::new("cmd-d", BDuplicate, c),
        KeyBinding::new("cmd-c", BCopy, c),
        KeyBinding::new("cmd-v", BPaste, c),
        KeyBinding::new("cmd-g", BGroup, c),
        KeyBinding::new("cmd-shift-g", BUngroup, c),
        KeyBinding::new("cmd-]", BFront, c),
        KeyBinding::new("cmd-[", BBack, c),
        KeyBinding::new("cmd-shift-l", BLock, c),
        KeyBinding::new("escape", BEscape, c),
        KeyBinding::new("left", BNudgeLeft, c),
        KeyBinding::new("right", BNudgeRight, c),
        KeyBinding::new("up", BNudgeUp, c),
        KeyBinding::new("down", BNudgeDown, c),
        KeyBinding::new("shift-left", BNudgeLeftBig, c),
        KeyBinding::new("shift-right", BNudgeRightBig, c),
        KeyBinding::new("shift-up", BNudgeUpBig, c),
        KeyBinding::new("shift-down", BNudgeDownBig, c),
        KeyBinding::new("cmd-=", BZoomIn, c),
        KeyBinding::new("cmd--", BZoomOut, c),
        KeyBinding::new("cmd-1", BZoomFit, c),
        KeyBinding::new("shift-1", BZoomFit, c),
        KeyBinding::new("cmd-0", BZoomReset, c),
        KeyBinding::new("enter", BEdit, c),
        // 工具快捷键与画板一致：R 圆角矩形、S 矩形、O 圆形、D 菱形、L 连线、T 文本、N 便签、P 画笔、E 橡皮擦
        KeyBinding::new("v", BToolSelect, c),
        KeyBinding::new("h", BToolHand, c),
        KeyBinding::new("r", BToolRect, c),
        KeyBinding::new("s", BToolSquare, c),
        KeyBinding::new("o", BToolEllipse, c),
        KeyBinding::new("d", BToolDiamond, c),
        KeyBinding::new("l", BToolArrow, c),
        KeyBinding::new("a", BToolArrow, c),
        KeyBinding::new("p", BToolPen, c),
        KeyBinding::new("t", BToolText, c),
        KeyBinding::new("n", BToolSticky, c),
        KeyBinding::new("e", BToolEraser, c),
        KeyBinding::new("shift-s", BToolSection, c),
        KeyBinding::new("shift-g", BToggleGrid, c),
        KeyBinding::new("shift-q", BToggleQuick, c),
        // 思维导图：Tab 子节点、⇧Tab 父节点、⌘. 收起 / 展开
        KeyBinding::new("tab", BTab, c),
        KeyBinding::new("shift-tab", BShiftTab, c),
        KeyBinding::new("cmd-.", BCollapse, c),
        KeyBinding::new("cmd-f", BFind, c),
        KeyBinding::new("cmd-f", BFind, Some("BoardText")),
        // 复制 / 粘贴样式（格式刷）
        KeyBinding::new("cmd-alt-c", BCopyStyle, c),
        KeyBinding::new("shift-h", BFlipH, c),
        KeyBinding::new("cmd-b", BBold, c),
        KeyBinding::new("cmd-i", BItalic, c),
        KeyBinding::new("cmd-u", BUnderline, c),
        KeyBinding::new("cmd-shift-x", BStrike, c),
        // 编辑文字时也能用 ⌘B / ⌘I / ⌘U / ⇧⌘X 改整段文字样式
        KeyBinding::new("cmd-b", BBold, Some("BoardText")),
        KeyBinding::new("cmd-i", BItalic, Some("BoardText")),
        KeyBinding::new("cmd-u", BUnderline, Some("BoardText")),
        KeyBinding::new("cmd-shift-x", BStrike, Some("BoardText")),
        KeyBinding::new("shift-v", BFlipV, c),
        KeyBinding::new("cmd-shift-c", BCopyImage, c),
        KeyBinding::new("cmd-alt-v", BPasteStyle, c),
        // 更多图形
        KeyBinding::new("m", BLibrary, c),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Select,
    Hand,
    Shape(ShapeKind),
    /// 连线：路线类型 + 终点是否带箭头
    Connector(Route, bool),
    Pen,
    Text,
    Sticky,
    Eraser,
    /// 分区：预设尺寸（宽, 高）；单击按预设放置，拖动自定义大小
    Section(f32, f32),
    /// 泳道：0 垂直、1 水平、2 矩阵；单击画布放置
    Lane(u8),
    /// 思维导图：布局（0 向右、1 左右分布、2 向下）；单击画布放置
    Mind(u8),
}

/// 画笔栏的当前设置。
#[derive(Clone, Copy, PartialEq)]
pub struct PenState {
    pub highlighter: bool,
    pub color: usize,
    pub width: usize,
}

/// 画板偏好（画板右上角的「基础配置」），本次运行期间对所有画板生效。
#[derive(Clone, Copy)]
pub struct BoardPrefs {
    pub grid: bool,
    pub quick_create: bool,
    pub show_size: bool,
    pub precise_select: bool,
    pub minimap: bool,
}

pub static PREFS: Mutex<BoardPrefs> = Mutex::new(BoardPrefs { grid: true, quick_create: true, show_size: false, precise_select: false, minimap: false });

pub fn prefs() -> BoardPrefs {
    *PREFS.lock().unwrap()
}

pub(super) enum Drag {
    Move {
        start: Pt,
        orig: Vec<(u64, f32, f32, Option<Vec<[f32; 3]>>, Option<(End, End)>)>,
        moved: bool,
    },
    /// grid：表格 / 泳道原来的列宽与行高；kids：泳道里元素原来的位置（拉伸泳道时等比移动）；
    /// snap：代码分区（PlantUML）里元素缩放前的完整副本（拉伸分区时整体等比缩放）
    Resize {
        id: u64,
        handle: usize,
        orig: Rect,
        font: f32,
        grid: Option<(Vec<f32>, Vec<f32>)>,
        kids: Vec<(u64, f32, f32)>,
        snap: Vec<El>,
    },
    /// 拖动表格 / 泳道的网格线
    GridLine {
        id: u64,
        vertical: bool,
        k: usize,
        orig: f32,
        start: Pt,
    },
    /// 拖动思维导图的非根节点：松开在另一个节点上即换父节点；没动则视为再次单击（进入编辑）
    MindMove {
        id: u64,
        start: Pt,
        orig: Vec<(u64, f32, f32)>,
        moved: bool,
        reclick: bool,
    },
    Create {
        start: Pt,
        id: u64,
    },
    Marquee {
        start: Pt,
        cur: Pt,
    },
    Pan {
        start: Point<Pixels>,
        orig: Pt,
    },
    Pen {
        id: u64,
        last: Pt,
    },
    /// 拖出一条新连线（工具栏连线 / 快捷创建圆点）
    Connect {
        id: u64,
    },
    EndPoint {
        id: u64,
        which: usize,
    },
    Erase,
    /// 按在快捷创建圆点上：松开 = 快捷创建图形，拖动 = 拉出连线
    QuickDot {
        from: u64,
        side: Side,
        start: Point<Pixels>,
    },
    /// 按在生命线虚线上：松开 = 落下激活条，拖动 = 移动生命线
    Dash {
        life: u64,
        top: f32,
        start: Point<Pixels>,
    },
    /// 按在激活条边缘锚点上：拖动 = 拉出消息线，松开 = 选中激活条
    Port {
        from: u64,
        ax: f32,
        ay: f32,
        start: Point<Pixels>,
    },
    /// 拖动选中连线上的调整手柄：orig 为按下时的走线骨架；seg 为真时平移折线的第 idx 段，否则移动第 idx 个拐点
    Bend {
        id: u64,
        orig: Vec<Pt>,
        idx: usize,
        seg: bool,
        start: Pt,
        moved: bool,
    },
}

/// 连线调整手柄的种类（选中连线后线段中间的小手柄）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum BendAt {
    /// 折线的第 k 段（是否竖段），拖动整段平移
    Seg(usize, bool),
    /// 直线 / 曲线第 k 段的中点，拖动即在此加一个拐点
    Mid(usize),
    /// 直线 / 曲线已有的第 i 个点（骨架下标），拖动移动它
    Point(usize),
}

/// 左侧工具栏展开的浮出面板。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flyout {
    Shapes,
    Connectors,
    Sticky,
    Section,
    Table,
    Mind,
    More,
}

/// 浮动属性栏里展开的下拉面板。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Popover {
    ShapeKind,
    Fill,
    Border,
    FontSize,
    TextColor,
    Align,
    #[cfg_attr(not(feature = "dev"), allow(dead_code))]
    Route,
    StartArrow,
    EndArrow,
    LineStyle,
    StickyColor,
    Arrange,
    More,
    Settings,
    Export,
    Zoom,
    /// 表格：插入 / 删除行列
    TableOps,
    /// 思维导图：布局与分支线
    MindLayout,
}

/// 原位文字编辑。
pub(super) struct Editing {
    pub id: u64,
    pub part: Part,
    pub area: Entity<TextArea>,
    pub orig_text: String,
    pub orig_spans: Vec<crate::canvas::Span>,
    pub orig_rect: Rect,
    /// 编辑前的高度（表格单元格为所在行高），文字撑高后删字最多缩回到这里
    pub orig_floor: f32,
    /// 编辑前的元素（撤销点：文字撑高了表格行时连行高一起恢复）
    pub orig_el: El,
    pub _sub: Subscription,
}

/// 图形选择器：双击空白处、或把快捷创建的连线拖到空白处后弹出。
#[derive(Clone, Copy)]
pub(super) struct Picker {
    pub at: Point<Pixels>,
    pub world: Pt,
    /// 从哪个图形拉出的连线（选择后自动连上）
    pub conn: Option<u64>,
}

pub enum BoardEvent {
    Close,
    /// 预览画板（以思维导图查看文档）：插入到文档光标处
    Insert,
}

impl EventEmitter<BoardEvent> for BoardEditor {}

pub struct BoardEditor {
    pub obj_id: String,
    pub(super) doc: BoardDoc,
    pub(super) undo: std::collections::VecDeque<BoardDoc>,
    pub(super) redo: Vec<BoardDoc>,
    pub(super) offset: Pt,
    pub(super) zoom: f32,
    pub(super) tool: Tool,
    pub(super) tool_lock: bool,
    /// 工具栏「图形」按钮记住上一次用的图形
    pub(super) last_shape: ShapeKind,
    /// 工具栏「连线」按钮记住上一次用的连线类型
    pub(super) last_conn: (Route, bool),
    pub(super) sticky_color: usize,
    pub(super) pen: PenState,
    pub(super) sel: HashSet<u64>,
    pub(super) drag: Option<Drag>,
    pub(super) editing: Option<Editing>,
    pub(super) focus: FocusHandle,
    pub(super) canvas_bounds: Bounds<Pixels>,
    /// 对齐参考线：线段两端（世界坐标）
    pub(super) guides: Vec<(Pt, Pt)>,
    pub(super) hover: Option<u64>,
    /// 连线拖动中吸附到的锚点
    pub(super) snap_anchor: Option<(u64, Option<Side>)>,
    /// 鼠标悬停的快捷创建圆点
    pub(super) hover_dot: Option<Side>,
    pub(super) mouse_world: Pt,
    pub(super) clip: Vec<El>,
    pub(super) space_down: bool,
    pub(super) fitted: bool,
    pub(super) flyout: Option<Flyout>,
    pub(super) popover: Option<Popover>,
    /// 下拉面板的弹出位置（点击按钮时的鼠标位置）
    pub(super) pop_at: Point<Pixels>,
    /// 下拉是悬停属性栏按钮展开的（移开后自动收起；点击按钮后固定）
    pub(super) pop_by_hover: bool,
    /// 鼠标在属性栏下拉按钮 / 下拉面板上
    pub(super) pop_btn_hover: bool,
    pub(super) pop_panel_hover: bool,
    /// 延迟收起的代次：期间重新悬停则作废
    pub(super) pop_close_gen: u64,
    /// 右键菜单（窗口坐标, 是否作用于选中元素）
    pub(super) menu: Option<(Point<Pixels>, bool)>,
    pub(super) picker: Option<Picker>,
    /// 右键按下位置：移动超过阈值就当作平移，原地松开才弹菜单
    pub(super) right_down: Option<(Point<Pixels>, Pt, bool)>,
    /// 表格中选中的单元格（表格 id, 单元格序号）
    pub(super) cell_sel: Option<(u64, usize)>,
    /// 「更多图形」里选的线条样式：(起点箭头, 终点箭头, 虚线)；None 为工具栏默认
    pub(super) conn_preset: Option<(crate::canvas::Arrow, crate::canvas::Arrow, bool)>,
    /// 左侧「更多图形」面板是否打开、收起的分类
    pub(super) library: bool,
    /// 「图标」面板（搜索框与订阅）
    pub(super) icons: Option<(Entity<crate::line_input::LineInput>, Subscription)>,
    /// 「素材」面板里打开的素材集（None 为素材首页）与列表滚动
    pub(super) icon_pack: Option<&'static str>,
    pub(super) icon_scroll: UniformListScrollHandle,
    pub(super) lib_folded: HashSet<&'static str>,
    /// 格式刷：复制的样式；painting 为 Some 时处于刷样式状态（true 表示连续刷）
    pub(super) style_clip: Option<El>,
    /// 不透明度滑杆的位置与是否在拖动
    pub(super) slider_bounds: Bounds<Pixels>,
    pub(super) slider_drag: bool,
    /// 「更多」菜单里「层级」二级菜单展开中
    pub(super) layer_sub: bool,
    /// 悬停中的表格边界「+」：(是否插入行, 边界序号)
    pub(super) table_hot: Option<(bool, usize)>,
    pub(super) painting: Option<bool>,
    /// 搜索替换
    pub(super) find: Option<super::find::Find>,
    /// 模板库弹窗：当前分类
    pub(super) templates: Option<usize>,
    /// 空画板右侧的「从模板创建」面板（关掉或画板有内容后不再出现）与 ↑↓ 预览中的模板
    pub(super) quick_tpl: bool,
    pub(super) quick_pick: Option<usize>,
    /// PlantUML 代码绘图弹窗
    pub(super) code_dlg: Option<super::code::CodeDlg>,
    /// 拖动思维导图节点时的落点（新的父节点）
    pub(super) mind_target: Option<u64>,
    /// 鼠标悬停在可拖动的网格线上（是否竖线）
    pub(super) hover_line: Option<bool>,
    /// 悬停在选中图形的缩放手柄 / 边缘上（手柄序号，同 `Drag::Resize`）
    pub(super) hover_handle: Option<usize>,
    /// 悬停在选中连线的调整手柄上
    pub(super) hover_bend: Option<BendAt>,
    /// 单击已选中的图形：松开时若没拖动就进入文字编辑
    pub(super) click_edit: Option<u64>,
    /// 插入表格面板里悬停的行列数
    pub(super) table_pick: (usize, usize),
    /// 悬停生命线虚线时的激活条预览：(生命线, 顶部 y)
    pub(super) bar_ghost: Option<(u64, f32)>,
    /// 悬停激活条边缘时的消息线锚点：(激活条, ax, ay)
    pub(super) bar_port: Option<(u64, f32, f32)>,
    /// 拖动消息线时的落点
    pub(super) msg_snap: Option<super::seq::MsgSnap>,
    /// 端点样式下拉展开了「更多」
    pub(super) arrow_more: bool,
    /// 临时预览（以思维导图查看文档）：不在文档里，关掉即丢弃，可「插入到文档」
    pub preview: bool,
}

impl Focusable for BoardEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BoardEditor {
    pub fn new(obj_id: String, cx: &mut Context<Self>) -> Self {
        let doc = crate::objects::board(&obj_id).map(|b| (*b).clone()).unwrap_or_else(|| BoardDoc::new(false));
        let flow = doc.flow;
        let blank = doc.els.is_empty();
        BoardEditor {
            obj_id,
            doc,
            undo: Default::default(),
            redo: vec![],
            offset: pt(0., 0.),
            zoom: 1.,
            tool: Tool::Select,
            tool_lock: false,
            last_shape: ShapeKind::RoundRect,
            last_conn: if flow { (Route::Elbow, true) } else { (Route::Straight, true) },
            sticky_color: 0,
            pen: PenState { highlighter: false, color: 0, width: 1 },
            sel: HashSet::new(),
            drag: None,
            editing: None,
            focus: cx.focus_handle(),
            canvas_bounds: Bounds::default(),
            guides: vec![],
            hover: None,
            snap_anchor: None,
            hover_dot: None,
            mouse_world: pt(0., 0.),
            clip: vec![],
            space_down: false,
            fitted: false,
            flyout: None,
            popover: None,
            pop_at: point(px(0.), px(0.)),
            pop_by_hover: false,
            pop_btn_hover: false,
            pop_panel_hover: false,
            pop_close_gen: 0,
            menu: None,
            picker: None,
            right_down: None,
            cell_sel: None,
            conn_preset: None,
            library: false,
            icons: None,
            icon_pack: None,
            icon_scroll: UniformListScrollHandle::new(),
            lib_folded: HashSet::new(),
            style_clip: None,
            slider_bounds: Bounds::default(),
            slider_drag: false,
            layer_sub: false,
            table_hot: None,
            painting: None,
            find: None,
            templates: None,
            quick_tpl: blank,
            quick_pick: None,
            code_dlg: None,
            mind_target: None,
            hover_line: None,
            hover_handle: None,
            hover_bend: None,
            click_edit: None,
            table_pick: (0, 0),
            bar_ghost: None,
            bar_port: None,
            msg_snap: None,
            arrow_more: false,
            preview: false,
        }
    }

    pub(super) fn dark(cx: &App) -> bool {
        theme(cx).dark
    }

    pub(super) fn view(&self) -> View {
        View { origin: point(self.canvas_bounds.origin.x + px(self.offset.x), self.canvas_bounds.origin.y + px(self.offset.y)), scale: self.zoom }
    }

    pub(super) fn to_world(&self, p: Point<Pixels>) -> Pt {
        self.view().inv(p)
    }

    pub(super) fn snapshot(&mut self) {
        self.undo.push_back(self.doc.clone());
        if self.undo.len() > 200 {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// 写回对象存储（后台保存）。
    pub(super) fn commit(&mut self, cx: &mut Context<Self>) {
        self.doc.layout_minds();
        crate::objects::update(&self.obj_id, crate::objects::ObjData::Board(Arc::new(self.doc.clone())));
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        self.commit(cx);
        cx.emit(BoardEvent::Close);
    }

    /// 收起所有浮层（工具栏面板、下拉、右键菜单、图形选择器）。
    pub(super) fn close_overlays(&mut self) -> bool {
        let any = self.flyout.is_some() || self.popover.is_some() || self.menu.is_some() || self.picker.is_some();
        if any {
            self.flyout = None;
        }
        self.flyout = None;
        self.popover = None;
        self.menu = None;
        self.picker = None;
        any
    }

    // ───────────── 命中测试 ─────────────

    /// 最上层的元素。分区只在点到标题栏或分区内空白处时才命中。
    pub(super) fn hit(&self, p: Pt) -> Option<u64> {
        let tol = 6. / self.zoom;
        let lk = self.doc.lookup();
        for el in self.doc.els.iter().rev().filter(|e| !e.is_container() && lk.visible(e.id)) {
            match &el.kind {
                ElKind::Connector { .. } => {
                    // 先用包围盒粗筛，再逐段精算
                    let pts = BoardDoc::connector_points_in(&lk, el);
                    if !crate::canvas::geom::points_bounds(&pts).inflate(tol + el.stroke_w + 14.).contains(p) {
                        continue;
                    }
                    if pts.windows(2).any(|w| dist_seg(p, w[0], w[1]) < tol + el.stroke_w) {
                        return Some(el.id);
                    }
                    // 连线说明的文字也算
                    if !el.text.is_empty() && crate::canvas::geom::polyline_mid(&pts).dist(p) < 14. {
                        return Some(el.id);
                    }
                }
                ElKind::Pen { pts } => {
                    let w = if el.highlighter { el.stroke_w * 3. } else { el.stroke_w };
                    if pts.windows(2).any(|q| dist_seg(p, pt(q[0][0], q[0][1]), pt(q[1][0], q[1][1])) < tol + w * 2.) {
                        return Some(el.id);
                    }
                }
                _ => {
                    // 生命线只有头部和虚线能点中
                    if super::seq::life_contains(el, p, tol / 2.).unwrap_or_else(|| el.rect().inflate(tol / 2.).contains(p)) {
                        return Some(el.id);
                    }
                }
            }
        }
        self.doc
            .els
            .iter()
            .rev()
            .filter(|e| e.is_container())
            .find(|e| (e.is_section() && self.section_title_rect(e).contains(p)) || e.rect().contains(p))
            .map(|e| e.id)
    }

    /// 分区标题栏（分区上方的一条）。
    pub(super) fn section_title_rect(&self, e: &El) -> Rect {
        if e.code.is_some() {
            // 代码绘图：标题是左上角外侧的橙色标签，没有标题时不占位置
            if e.text.is_empty() {
                return Rect::default();
            }
            return code_title_rect(e, crate::canvas::geom::estimate_width(&e.text, 13., true) + 16.);
        }
        crate::canvas::rect(e.x, e.y - 24., e.w.min(crate::canvas::geom::estimate_width(&e.text, 13., true) + 24.).max(60.), 24.)
    }

    /// 可作为连线端点的元素（块状元素，从上往下找）。
    pub(super) fn hit_box(&self, p: Pt, except: u64) -> Option<&El> {
        let lk = self.doc.lookup();
        let tol = 10. / self.zoom;
        self.doc.els.iter().rev().find(|e| {
            e.id != except && e.is_box() && lk.visible(e.id) && super::seq::life_contains(e, p, tol).unwrap_or_else(|| e.rect().inflate(tol).contains(p))
        })
    }

    pub(super) fn selection_bounds(&self) -> Option<Rect> {
        let lk = self.doc.lookup();
        self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).map(|e| BoardDoc::bounds_in(&lk, e)).reduce(|a, b| a.union(&b))
    }

    pub(super) fn handles(r: Rect) -> [Pt; 8] {
        crate::canvas::math::handles8(&crate::canvas::math::corners(r.x, r.y, r.right(), r.bottom()))
    }

    pub(super) fn single(&self) -> Option<&El> {
        if self.sel.len() != 1 {
            return None;
        }
        self.doc.get(*self.sel.iter().next()?)
    }

    pub(super) fn single_resizable(&self) -> Option<&El> {
        // 思维导图节点的大小随文字自动变化
        self.single().filter(|e| !e.is_connector() && !matches!(e.kind, ElKind::Pen { .. }) && !e.locked && !e.is_mind())
    }

    /// 快捷创建圆点（世界坐标）：选中单个图形 / 便签时显示在四条边外侧。
    pub(super) fn quick_dots(&self) -> Vec<(Side, Pt)> {
        if !prefs().quick_create || self.editing.is_some() || self.drag.is_some() || self.tool != Tool::Select {
            return vec![];
        }
        let Some(e) = self.single().filter(|e| {
            matches!(e.kind, ElKind::Shape(_) | ElKind::Sticky | ElKind::Icon { .. } | ElKind::Image { .. })
                && !e.locked
                && !super::seq::is_life(e)
                && !super::seq::is_bar(e)
        }) else {
            return vec![];
        };
        let r = e.rect();
        // 思维导图节点：只在分支线和「+」没占用的边上显示，用来拖出关联线
        let sides: Vec<Side> = if e.is_mind() { self.mind_link_sides(e) } else { Side::ALL.to_vec() };
        let gap = if e.is_mind() { 14. } else { 18. } / self.zoom;
        sides
            .iter()
            .map(|s| {
                let (ax, ay) = s.anchor();
                (*s, pt(r.x + ax * r.w, r.y + ay * r.h).add(s.dir().scale(gap)))
            })
            .collect()
    }

    pub(super) fn expand_groups(&mut self) {
        if self.sel.len() != 1 || !self.single().is_some_and(|e| e.is_table()) {
            self.cell_sel = None;
        }
        let groups: HashSet<u64> = self.doc.els.iter().filter(|e| self.sel.contains(&e.id)).filter_map(|e| e.group).collect();
        if groups.is_empty() {
            return;
        }
        for e in &self.doc.els {
            if e.group.is_some_and(|g| groups.contains(&g)) {
                self.sel.insert(e.id);
            }
        }
    }

    // ───────────── 测试脚本 ─────────────

    /// 测试脚本用：从流程图形状库添加第 n 个形状。
    #[cfg(feature = "dev")]
    pub fn test_lib(&mut self, n: usize, text: &str, cx: &mut Context<Self>) {
        let (kind, _, w, h) = super::ui::FLOW_SHAPES[n.min(super::ui::FLOW_SHAPES.len() - 1)];
        self.add_shape(kind, w, h, text, cx);
    }

    #[cfg(feature = "dev")]
    pub fn test_layout(&mut self, cx: &mut Context<Self>) {
        self.snapshot();
        super::auto_layout(&mut self.doc);
        self.commit(cx);
        let b = self.doc.bounds();
        self.zoom_to(b);
    }

    #[cfg(feature = "dev")]
    pub fn test_rough(&mut self, cx: &mut Context<Self>) {
        self.doc.rough = !self.doc.rough;
        self.commit(cx);
    }

    /// 测试脚本：画一条波浪线（模拟画笔）。
    #[cfg(feature = "dev")]
    pub fn test_pen(&mut self, cx: &mut Context<Self>) {
        let c = self.to_world(self.canvas_bounds.center());
        let pts: Vec<[f32; 3]> = (0..80)
            .map(|i| {
                let x = c.x - 300. + i as f32 * 7.;
                [x, c.y + 160. + (i as f32 * 0.25).sin() * 30., 0.4 + 0.6 * ((i as f32 * 0.1).sin().abs())]
            })
            .collect();
        let mut el = self.doc.new_el(ElKind::Pen { pts }, Rect::default(), false);
        el.stroke_w = 3.;
        el.stroke = crate::canvas::Col::rgb(0xe5484d);
        self.doc.els.push(el);
        self.commit(cx);
    }

    /// 测试脚本：进入第 idx 个元素的文字编辑。
    #[cfg(feature = "dev")]
    pub fn test_edit(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.doc.els.get(idx).map(|e| e.id) {
            self.sel = [id].into();
            self.start_edit(id, window, cx);
        }
    }

    /// 测试脚本：打印当前工具、编辑框内容与各元素文字。
    #[cfg(feature = "dev")]
    pub fn test_dump(&self, window: &Window, cx: &App) {
        let editing = self.editing.as_ref().map(|e| (e.area.read(cx).text().to_string(), e.area.focus_handle(cx).is_focused(window)));
        println!(
            "BOARD tool={:?} editing={:?} sel={} texts={:?} kinds={:?}",
            self.tool,
            editing,
            self.sel.len(),
            self.doc.els.iter().map(|e| e.text.clone()).collect::<Vec<_>>(),
            self.doc.els.iter().map(|e| format!("{:?}", e.kind).chars().take(24).collect::<String>()).collect::<Vec<_>>()
        );
    }

    /// 测试脚本：打印每个元素的位置与连线端点。
    #[cfg(feature = "dev")]
    pub fn test_geo(&self) {
        for e in &self.doc.els {
            match &e.kind {
                ElKind::Connector { a, b, route, .. } => println!(
                    "GEO #{} conn {:?} a={:?} b={:?} pts={:?}",
                    e.id,
                    route,
                    a,
                    b,
                    self.doc.connector_points(e).iter().map(|p| (p.x.round(), p.y.round())).collect::<Vec<_>>()
                ),
                k => println!(
                    "GEO #{} {} ({:.0},{:.0},{:.0},{:.0}) text={:?}",
                    e.id,
                    format!("{k:?}").chars().take(18).collect::<String>(),
                    e.x,
                    e.y,
                    e.w,
                    e.h,
                    e.text
                ),
            }
        }
        println!("GEO undo={} sel={:?}", self.undo.len(), self.sel);
    }

    #[cfg(feature = "dev")]
    pub fn test_select(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(e) = self.doc.els.get(idx) {
            self.sel = [e.id].into();
        }
        cx.notify();
    }

    /// 测试脚本：在世界坐标处模拟一次点击 / 拖动（按工具语义），返回后打印状态。
    #[cfg(feature = "dev")]
    pub fn test_world_drag(&mut self, a: Pt, b: Pt, window: &mut Window, cx: &mut Context<Self>) {
        let v = self.view();
        let (pa, pb) = (v.p(a), v.p(b));
        let down = MouseDownEvent { button: MouseButton::Left, position: pa, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
        self.mouse_down(&down, window, cx);
        for i in 1..=6 {
            let t = i as f32 / 6.;
            let position = point(pa.x + (pb.x - pa.x) * t, pa.y + (pb.y - pa.y) * t);
            self.mouse_move(&MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }, window, cx);
        }
        self.mouse_up(&MouseUpEvent { button: MouseButton::Left, position: pb, modifiers: Modifiers::default(), click_count: 1 }, window, cx);
    }

    /// 测试脚本：在快捷创建圆点上单击（side: 0 上 1 右 2 下 3 左）。
    #[cfg(feature = "dev")]
    pub fn test_quick(&mut self, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        let dots = self.quick_dots();
        if let Some((_, p)) = dots.get(side) {
            let p = *p;
            self.test_world_drag(p, p, window, cx);
        }
    }

    #[cfg(feature = "dev")]
    pub fn test_set_tool(&mut self, name: &str, cx: &mut Context<Self>) {
        let tool = match name {
            "rect" => Tool::Shape(ShapeKind::RoundRect),
            "ellipse" => Tool::Shape(ShapeKind::Ellipse),
            "sticky" => Tool::Sticky,
            "text" => Tool::Text,
            "pen" => Tool::Pen,
            "conn" => Tool::Connector(Route::Elbow, true),
            "section" => Tool::Section(640., 360.),
            _ => Tool::Select,
        };
        self.set_tool(tool, cx);
    }

    #[cfg(feature = "dev")]
    pub fn test_popover(&mut self, name: &str, cx: &mut Context<Self>) {
        self.popover = match name {
            "fill" => Some(Popover::Fill),
            "border" => Some(Popover::Border),
            "more" => Some(Popover::More),
            "shape" => Some(Popover::ShapeKind),
            "route" => Some(Popover::Route),
            "end" => Some(Popover::EndArrow),
            "start" => Some(Popover::StartArrow),
            "linestyle" => Some(Popover::LineStyle),
            "linecolor" => Some(Popover::Border),
            "sticky" => Some(Popover::StickyColor),
            "settings" => Some(Popover::Settings),
            "arrange" => Some(Popover::Arrange),
            "textcolor" => Some(Popover::TextColor),
            "fontsize" => Some(Popover::FontSize),
            "format" => Some(Popover::Align),
            _ => None,
        };
        if name == "tableops" {
            self.popover = Some(Popover::TableOps);
        } else if name == "mindlayout" {
            self.popover = Some(Popover::MindLayout);
        }
        if self.popover.is_some() {
            // 下拉面板贴在属性栏下方、选区中间
            if let Some(b) = self.selection_bounds() {
                self.pop_at = self.view().p(b.center());
            }
        }
        self.flyout = match name {
            "fly-shapes" => Some(Flyout::Shapes),
            "fly-conn" => Some(Flyout::Connectors),
            "fly-sticky" => Some(Flyout::Sticky),
            "fly-section" => Some(Flyout::Section),
            "fly-table" => Some(Flyout::Table),
            "fly-mind" => Some(Flyout::Mind),
            "fly-more" => Some(Flyout::More),
            _ => None,
        };
        cx.notify();
    }

    #[cfg(feature = "dev")]
    pub fn test_menu(&mut self, cx: &mut Context<Self>) {
        let c = self.selection_bounds().map(|b| self.view().p(b.center())).unwrap_or(self.canvas_bounds.center());
        self.menu = Some((c, !self.sel.is_empty()));
        cx.notify();
    }

    #[cfg(feature = "dev")]
    pub fn test_pick(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let k = super::ui::BOARD_SHAPES[n.min(super::ui::BOARD_SHAPES.len() - 1)].0;
        self.picker_choose(Some(k), false, window, cx);
    }

    /// 测试脚本：生成 n 个图形 + n 条直角连线，打印一次场景构建与命中测试的耗时。
    #[cfg(feature = "dev")]
    pub fn test_stress(&mut self, n: usize, cx: &mut Context<Self>) {
        let mut prev = None;
        for i in 0..n {
            let (x, y) = ((i % 30) as f32 * 200., (i / 30) as f32 * 140.);
            let mut el = self.doc.new_el(ElKind::Shape(ShapeKind::RoundRect), crate::canvas::rect(x, y, 140., 72.), false);
            el.text = format!("节点 {i}");
            let id = el.id;
            self.doc.els.push(el);
            if let Some(p) = prev {
                let c = self.doc.new_el(
                    ElKind::Connector {
                        a: End::Bound { el: p, ax: 0.5, ay: 0.5 },
                        b: End::Bound { el: id, ax: 0.5, ay: 0.5 },
                        route: Route::Elbow,
                        start: crate::canvas::Arrow::None,
                        end: crate::canvas::Arrow::Filled,
                    },
                    Rect::default(),
                    false,
                );
                self.doc.els.push(c);
            }
            prev = Some(id);
        }
        let t0 = std::time::Instant::now();
        let scene = self.doc.to_scene(false);
        let t1 = t0.elapsed();
        let t0 = std::time::Instant::now();
        let h = self.hit(pt(-500., -500.));
        let t2 = t0.elapsed();
        println!(
            "STRESS els={} scene_items={} to_scene={:.2}ms hit_miss={:.2}ms {:?}",
            self.doc.els.len(),
            scene.items.len(),
            t1.as_secs_f64() * 1000.,
            t2.as_secs_f64() * 1000.,
            h
        );
        self.commit(cx);
    }

    /// 测试脚本：表格 / 泳道 / 思维导图 / 模板 / 搜索替换 / PlantUML / 格式刷 / 图形库。
    #[cfg(feature = "dev")]
    pub fn test_cmd(&mut self, parts: &[&str], window: &mut Window, cx: &mut Context<Self>) {
        let n = |i: usize| parts.get(i).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let center = self.to_world(self.canvas_bounds.center());
        match parts.first().copied().unwrap_or("") {
            "lane" => {
                self.set_tool(Tool::Lane(n(1) as u8), cx);
                self.test_world_drag(center, center, window, cx);
            }
            "mind" => {
                self.set_tool(Tool::Mind(n(1) as u8), cx);
                self.test_world_drag(center, center, window, cx);
            }
            "table" => self.insert_table(n(1).max(1), n(2).max(1), cx),
            "tpl" if parts.len() > 1 => self.insert_template(n(1), cx),
            // qtpl N：「从模板创建」面板预览第 N 个推荐模板
            "qtpl" => {
                self.quick_pick = Some(n(1));
                cx.notify();
            }
            "tpl" => self.open_templates(cx),
            "tplcat" => {
                self.templates = Some(n(1));
                cx.notify();
            }
            "lib" => {
                self.library = !self.library;
                cx.notify();
            }
            "libpick" => {
                // libpick 分类 序号
                if let Some(item) = super::library::CATEGORIES.get(n(1)).and_then(|c| c.1.get(n(2))) {
                    self.pick_library(item.0, cx);
                    self.test_world_drag(center, center, window, cx);
                }
            }
            "find" | "replace" => {
                self.open_find(window, cx);
                let q = parts.get(1).copied().unwrap_or("").to_string();
                if let Some(f) = self.find.as_mut() {
                    f.query = q.clone();
                    f.cur = 0;
                    let fq = f.q.clone();
                    fq.update(cx, |i, cx| i.set_text(&q, cx));
                }
                if parts[0] == "replace" {
                    let with = parts.get(2).copied().unwrap_or("").to_string();
                    if let Some(f) = self.find.as_mut() {
                        f.replace = true;
                        let r = f.r.clone();
                        r.update(cx, |i, cx| i.set_text(&with, cx));
                    }
                    if parts.get(3) == Some(&"one") {
                        self.replace_one(cx);
                    } else if parts.get(3) == Some(&"all") {
                        self.replace_all(cx);
                    }
                }
                cx.notify();
            }
            "findnext" => self.find_step(1, cx),
            // code [模板序号] / mermaid [模板序号]：打开代码绘图（选中代码分区时编辑它）
            "code" | "mermaid" => {
                let lang = if parts[0] == "mermaid" { super::code::CodeLang::Mermaid } else { super::code::CodeLang::Puml };
                let target = self.single().filter(|e| e.code.is_some()).map(|e| e.id);
                self.open_code(target, lang, window, cx);
                if parts.len() > 1 {
                    if let Some(d) = self.code_dlg.as_mut() {
                        let samples = d.lang.samples();
                        let i = n(1).min(samples.len() - 1);
                        d.tpl = i;
                        d.area.clone().update(cx, |a, cx| a.set_text(samples[i].1, cx));
                    }
                }
            }
            // codestyle 0|1：画板样式 / 经典样式
            "codestyle" => {
                if let Some(d) = self.code_dlg.as_mut() {
                    d.classic = n(1) == 1;
                }
                cx.notify();
            }
            "codeins" => self.insert_code(n(1) as u8, window, cx),
            "paint" => self.start_painter(parts.get(1) == Some(&"keep"), cx),
            "copystyle" => self.copy_style(cx),
            "pastestyle" => self.paste_style(cx),
            "cell" => {
                if let Some(id) = self.single_table().map(|e| e.id) {
                    self.cell_sel = Some((id, n(1)));
                    cx.notify();
                }
            }
            "celltext" => {
                // celltext 序号 文字：编辑单元格并提交
                if let Some(id) = self.single().map(|e| e.id) {
                    self.start_edit_part(id, Part::Cell(n(1)), window, cx);
                    let text = parts[2..].join(" ").replace("\\n", "\n");
                    if let Some(ed) = self.editing.as_ref() {
                        ed.area.clone().update(cx, |a, cx| a.set_text(&text, cx));
                    }
                    self.finish_edit(cx);
                }
            }
            "tableins" => self.table_insert(n(1) as u8, cx),
            // 文字样式：选中单元格时只改该单元格
            "font" => self.set_font(Some(n(1) as f32), cx),
            "bold" => self.toggle_bold(cx),
            "italic" => self.toggle_deco(crate::canvas::DECO_ITALIC, cx),
            "tcolor" => {
                self.set_text_color(crate::canvas::Col::rgb(u32::from_str_radix(parts.get(1).copied().unwrap_or("e5484d"), 16).unwrap_or(0xe5484d)), cx)
            }
            "talign" => self.set_text_align(n(1) as u8, cx),
            "valign" => self.set_v_align(n(1) as u8, cx),
            "settext" => {
                // settext 文字（\n 换行）：编辑选中元素的主文字并提交
                if let Some(id) = self.single().map(|e| e.id) {
                    self.start_edit_part(id, Part::Main, window, cx);
                    let text = parts[1..].join(" ").replace("\\n", "\n");
                    if let Some(ed) = self.editing.as_ref() {
                        ed.area.clone().update(cx, |a, cx| a.set_text(&text, cx));
                    }
                    self.finish_edit(cx);
                }
            }
            "icons" => self.toggle_icons(window, cx),
            "iconq" => {
                if let Some((q, _)) = self.icons.as_ref() {
                    let text = parts[1..].join(" ");
                    q.clone().update(cx, |i, cx| i.set_text(&text, cx));
                }
                cx.notify();
            }
            // iconpack <素材集 id | ->：进入素材集 / 回到素材首页
            "iconpack" => {
                let id = super::iconlib::pack(parts.get(1).copied().unwrap_or("")).map(|p| p.id.as_str());
                self.open_icon_pack(id, cx);
            }
            // icon <序号 | 图标名>：插入内置图标或任意素材（如 icon fluent:rocket）
            "icon" => {
                let by_name = parts.get(1).and_then(|k| super::iconlib::get(k)).map(|(_, d)| d.id.as_str());
                let name = by_name.or_else(|| super::icons::ICON_CATS.iter().flat_map(|(_, i)| i.iter()).nth(n(1)).map(|(n, _, _)| *n)).unwrap_or("star");
                let at = self.to_world(self.canvas_bounds.center());
                self.insert_icon(super::library::LibItem::Icon(name), at, cx);
            }
            "emoji" => {
                let at = self.to_world(self.canvas_bounds.center());
                self.insert_icon(super::library::LibItem::Emoji(super::icons::EMOJIS[n(1) % super::icons::EMOJIS.len()]), at, cx);
            }
            "tablehot" => {
                // tablehot row|col 序号：模拟悬停表格边界圆点
                self.table_hot = Some((parts.get(1) == Some(&"row"), n(2)));
                cx.notify();
            }
            "tabledel" => self.table_delete(parts.get(1) == Some(&"row"), cx),
            "mindchild" | "mindleft" | "mindsib" | "mindparent" => {
                // mindleft：模拟点根节点左侧的「+」
                if let Some(id) = self.single_mind().map(|e| e.id) {
                    match parts[0] {
                        "mindchild" => self.mind_add_child(id, 0, window, cx),
                        "mindleft" => self.mind_add_child(id, -1, window, cx),
                        "mindsib" => self.mind_add_sibling(id, window, cx),
                        _ => self.mind_add_parent(id, window, cx),
                    }
                    let text = parts[1..].join(" ");
                    if !text.is_empty() {
                        if let Some(ed) = self.editing.as_ref() {
                            ed.area.clone().update(cx, |a, cx| a.set_text(&text, cx));
                        }
                    }
                    self.finish_edit(cx);
                }
            }
            "mindfold" => self.mind_toggle_collapse(cx),
            "mindroot" => {
                // 选中当前节点所在导图的根节点
                if let Some(id) = self.single_mind().map(|e| e.id) {
                    self.sel = [self.doc.mind_root(id)].into();
                    cx.notify();
                }
            }
            "mindlayout" => self.mind_set_layout(n(1) as u8, cx),
            "selid" => {
                self.sel = [n(1) as u64].into();
                cx.notify();
            }
            "fit" => self.zoom_fit(cx),
            // home：缩放 100%、世界原点在画布中心；libat 分类 序号 X Y：在世界坐标处放置图形库里的图形
            "home" => {
                self.zoom_to(None);
                cx.notify();
            }
            "libat" => {
                if let Some(item) = super::library::CATEGORIES.get(n(1)).and_then(|c| c.1.get(n(2))) {
                    let f = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                    let at = pt(f(3), f(4));
                    self.pick_library(item.0, cx);
                    self.test_world_drag(at, at, window, cx);
                    self.finish_edit(cx);
                }
            }
            "setroute" => self.set_route([Route::Elbow, Route::RoundElbow, Route::Straight, Route::Curve][n(1).min(3)], cx),
            // hold X1 Y1 X2 Y2：按下并拖到 X2 Y2 但不松开（截图拖动中的状态）；release：松开
            "hold" => {
                let f = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let v = self.view();
                let (pa, pb) = (v.p(pt(f(1), f(2))), v.p(pt(f(3), f(4))));
                self.mouse_down(
                    &MouseDownEvent { button: MouseButton::Left, position: pa, modifiers: Modifiers::default(), click_count: 1, first_mouse: false },
                    window,
                    cx,
                );
                for i in 1..=6 {
                    let t = i as f32 / 6.;
                    let position = point(pa.x + (pb.x - pa.x) * t, pa.y + (pb.y - pa.y) * t);
                    self.mouse_move(&MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }, window, cx);
                }
            }
            "release" => {
                let position = self.view().p(self.mouse_world);
                self.mouse_up(&MouseUpEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1 }, window, cx);
            }
            // hover X Y：模拟鼠标悬停在世界坐标处
            "hover" => {
                let f = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
                let position = self.view().p(pt(f(1), f(2)));
                self.mouse_move(&MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }, window, cx);
            }
            _ => {}
        }
    }

    #[cfg(feature = "dev")]
    pub fn test_picker(&mut self, cx: &mut Context<Self>) {
        let at = self.canvas_bounds.center();
        self.picker = Some(Picker { at, world: self.to_world(at), conn: None });
        cx.notify();
    }
}

pub(super) use crate::canvas::math::dist_seg;

impl Render for BoardEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let dark = t.dark;
        let hide = self.editing.as_ref().map(|e| (e.id, e.part));
        let scene = Arc::new(self.doc.to_scene_opts(dark, hide));
        let view = self.view();
        let entity = cx.entity();
        let overlay = self.overlay_info(&t);
        let p = prefs();
        let grid = if dark { gpui::hsla(0., 0., 1., 0.08) } else { gpui::hsla(0., 0., 0., 0.1) };
        let bg = t.bg;
        let zoom = self.zoom;
        let offset = self.offset;
        let cursor = match self.tool {
            Tool::Hand => CursorStyle::OpenHand,
            Tool::Select if matches!(self.drag, Some(Drag::Pan { .. })) => CursorStyle::ClosedHand,
            Tool::Select => CursorStyle::Arrow,
            Tool::Text => CursorStyle::IBeam,
            _ => CursorStyle::Crosshair,
        };
        let cursor = match self.hover_line {
            Some(true) if self.tool == Tool::Select => CursorStyle::ResizeLeftRight,
            Some(false) if self.tool == Tool::Select => CursorStyle::ResizeUpDown,
            _ => cursor,
        };
        let cursor = match self
            .drag
            .as_ref()
            .map(|d| match d {
                Drag::Resize { handle, .. } => Some(*handle),
                _ => None,
            })
            .unwrap_or(self.hover_handle.filter(|_| self.tool == Tool::Select))
        {
            Some(h) => match h {
                0 | 4 => CursorStyle::ResizeUpLeftDownRight,
                2 | 6 => CursorStyle::ResizeUpRightDownLeft,
                1 | 5 => CursorStyle::ResizeUpDown,
                _ => CursorStyle::ResizeLeftRight,
            },
            None => cursor,
        };
        let cursor = match (&self.drag, self.hover_bend.filter(|_| self.tool == Tool::Select)) {
            (Some(Drag::GridLine { vertical: true, .. }), _) => CursorStyle::ResizeLeftRight,
            (Some(Drag::GridLine { vertical: false, .. }), _) => CursorStyle::ResizeUpDown,
            (Some(Drag::Bend { seg: true, orig, idx, .. }), _) if orig.len() > idx + 1 => {
                if (orig[*idx].x - orig[idx + 1].x).abs() < 0.5 {
                    CursorStyle::ResizeLeftRight
                } else {
                    CursorStyle::ResizeUpDown
                }
            }
            (None, Some(BendAt::Seg(_, true))) => CursorStyle::ResizeLeftRight,
            (None, Some(BendAt::Seg(_, false))) => CursorStyle::ResizeUpDown,
            (None, Some(_)) => CursorStyle::PointingHand,
            _ => cursor,
        };
        let cursor = if self.painting.is_some() { CursorStyle::PointingHand } else { cursor };
        let cursor = if self.space_down { CursorStyle::OpenHand } else { cursor };
        // 正在输入文字（原位编辑、代码弹窗、搜索框）时换一个按键上下文，单字母快捷键不抢输入法的按键
        let find_focused =
            self.find.as_ref().is_some_and(|f| f.focused(window, cx)) || self.icons.as_ref().is_some_and(|(q, _)| q.focus_handle(cx).is_focused(window));
        let editing = self.editing.is_some() || self.code_dlg.is_some() || find_focused;
        let empty = self.doc.els.is_empty();
        if !empty {
            self.quick_tpl = false;
        }
        let quick_preview = self.quick_visible().then(|| self.render_quick_preview(dark)).flatten();
        div()
            // 编辑文字时换一个按键上下文：单字母工具快捷键、方向键、删除等不再参与匹配，
            // 否则 GPUI 会在输入法拿到按键之前先把字母当成快捷键吃掉。
            .key_context(if editing { "BoardText" } else { "Board" })
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(bg)
            .text_color(t.text)
            .font_family("PingFang SC")
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::do_undo))
            .on_action(cx.listener(Self::do_redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::group))
            .on_action(cx.listener(Self::ungroup))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(|this, _: &BFront, _, cx| this.z_order(2, cx)))
            .on_action(cx.listener(|this, _: &BBack, _, cx| this.z_order(-2, cx)))
            .on_action(cx.listener(|this, _: &BLock, _, cx| this.toggle_lock(cx)))
            .on_action(cx.listener(|this, _: &BNudgeLeft, _, cx| this.nudge(-1., 0., cx)))
            .on_action(cx.listener(|this, _: &BNudgeRight, _, cx| this.nudge(1., 0., cx)))
            .on_action(cx.listener(|this, _: &BNudgeUp, _, cx| if this.quick_visible() { this.quick_step(-1, cx) } else { this.nudge(0., -1., cx) }))
            .on_action(cx.listener(|this, _: &BNudgeDown, _, cx| if this.quick_visible() { this.quick_step(1, cx) } else { this.nudge(0., 1., cx) }))
            .on_action(cx.listener(|this, _: &BNudgeLeftBig, _, cx| this.nudge(-10., 0., cx)))
            .on_action(cx.listener(|this, _: &BNudgeRightBig, _, cx| this.nudge(10., 0., cx)))
            .on_action(cx.listener(|this, _: &BNudgeUpBig, _, cx| this.nudge(0., -10., cx)))
            .on_action(cx.listener(|this, _: &BNudgeDownBig, _, cx| this.nudge(0., 10., cx)))
            .on_action(cx.listener(|this, _: &BZoomIn, _, cx| this.zoom_step(1.25, cx)))
            .on_action(cx.listener(|this, _: &BZoomOut, _, cx| this.zoom_step(0.8, cx)))
            .on_action(cx.listener(|this, _: &BZoomFit, _, cx| this.zoom_fit(cx)))
            .on_action(cx.listener(|this, _: &BZoomReset, _, cx| this.zoom_100(cx)))
            .on_action(cx.listener(|this, _: &BEdit, window, cx| {
                // 思维导图节点：↵ 添加兄弟节点；表格选中单元格：↵ 编辑单元格
                if let Some(k) = this.quick_pick.filter(|_| this.quick_visible()) {
                    this.quick_insert(k, cx);
                } else if let Some(id) = this.single_mind().filter(|e| !e.locked).map(|e| e.id) {
                    this.mind_add_sibling(id, window, cx);
                } else if let Some((id, i)) = this.cell_sel.filter(|(id, _)| this.single_table().is_some_and(|e| e.id == *id)) {
                    this.start_edit_part(id, Part::Cell(i), window, cx);
                } else if let Some(id) = this.single().filter(|e| !e.locked).map(|e| e.id) {
                    this.start_edit(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &BTab, window, cx| {
                if let Some(id) = this.single_mind().map(|e| e.id) {
                    this.mind_add_child(id, 0, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &BShiftTab, window, cx| {
                if let Some(id) = this.single_mind().map(|e| e.id) {
                    this.mind_add_parent(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &BCollapse, _, cx| this.mind_toggle_collapse(cx)))
            .on_action(cx.listener(|this, _: &BFind, window, cx| this.open_find(window, cx)))
            .on_action(cx.listener(|this, _: &BCopyStyle, _, cx| this.copy_style(cx)))
            .on_action(cx.listener(|this, _: &BPasteStyle, _, cx| this.paste_style(cx)))
            .on_action(cx.listener(|this, _: &BFlipH, _, cx| this.flip(true, cx)))
            .on_action(cx.listener(|this, _: &BBold, _, cx| this.toggle_bold(cx)))
            .on_action(cx.listener(|this, _: &BItalic, _, cx| this.toggle_deco(crate::canvas::DECO_ITALIC, cx)))
            .on_action(cx.listener(|this, _: &BUnderline, _, cx| this.toggle_deco(crate::canvas::DECO_UNDERLINE, cx)))
            .on_action(cx.listener(|this, _: &BStrike, _, cx| this.toggle_deco(crate::canvas::DECO_STRIKE, cx)))
            .on_action(cx.listener(|this, _: &BFlipV, _, cx| this.flip(false, cx)))
            .on_action(cx.listener(|this, _: &BCopyImage, _, cx| this.copy_as_image(cx)))
            .on_action(cx.listener(|this, _: &BLibrary, _, cx| {
                this.library = !this.library;
                this.icons = None;
                this.flyout = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BToolSelect, _, cx| this.set_tool(Tool::Select, cx)))
            .on_action(cx.listener(|this, _: &BToolHand, _, cx| this.set_tool(Tool::Hand, cx)))
            .on_action(cx.listener(|this, _: &BToolRect, _, cx| this.set_tool(Tool::Shape(ShapeKind::RoundRect), cx)))
            .on_action(cx.listener(|this, _: &BToolSquare, _, cx| this.set_tool(Tool::Shape(ShapeKind::Rect), cx)))
            .on_action(cx.listener(|this, _: &BToolEllipse, _, cx| this.set_tool(Tool::Shape(ShapeKind::Ellipse), cx)))
            .on_action(cx.listener(|this, _: &BToolDiamond, _, cx| this.set_tool(Tool::Shape(ShapeKind::Diamond), cx)))
            .on_action(cx.listener(|this, _: &BToolArrow, _, cx| {
                let (r, a) = this.last_conn;
                this.set_tool(Tool::Connector(r, a), cx)
            }))
            .on_action(cx.listener(|this, _: &BToolPen, _, cx| this.set_tool(Tool::Pen, cx)))
            .on_action(cx.listener(|this, _: &BToolText, _, cx| this.set_tool(Tool::Text, cx)))
            .on_action(cx.listener(|this, _: &BToolSticky, _, cx| this.set_tool(Tool::Sticky, cx)))
            .on_action(cx.listener(|this, _: &BToolEraser, _, cx| {
                this.pen.highlighter = false;
                this.set_tool(Tool::Eraser, cx)
            }))
            .on_action(cx.listener(|this, _: &BToolSection, _, cx| this.set_tool(Tool::Section(640., 360.), cx)))
            .on_action(cx.listener(|_, _: &BToggleGrid, _, cx| {
                let mut p = PREFS.lock().unwrap();
                p.grid = !p.grid;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BToggleQuick, _, cx| {
                {
                    let mut p = PREFS.lock().unwrap();
                    p.quick_create = !p.quick_create;
                }
                let _ = this;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "space" && !this.space_down && this.editing.is_none() {
                    this.space_down = true;
                    cx.notify();
                }
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                if ev.keystroke.key == "space" {
                    this.space_down = false;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, item: &super::library::LibDrag, window, cx| this.drop_library_item(*item, window, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                for p in paths.paths() {
                    if let Ok(bytes) = std::fs::read(p) {
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
                        if let Some(path) = crate::convert::save_image_bytes(&bytes, &ext, &crate::store::data_dir().join("assets")) {
                            this.add_image(path, cx);
                        }
                    }
                }
            }))
            .child(
                div()
                    .size_full()
                    .cursor(cursor)
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, _| {
                                    let first = this.canvas_bounds.size.width <= px(0.);
                                    this.canvas_bounds = bounds;
                                    if first && !this.fitted {
                                        this.fitted = true;
                                        let b = this.doc.bounds();
                                        this.zoom_to(b);
                                        if b.is_some() && this.zoom > 1. {
                                            let c = this.canvas_bounds.center();
                                            let f = 1. / this.zoom;
                                            this.zoom_at(c, f);
                                        }
                                    }
                                });
                                bounds
                            },
                            move |bounds, _, window, cx| {
                                // 网格背景（的点阵）
                                if p.grid {
                                    let mut step = 24. * zoom;
                                    while step < 12. {
                                        step *= 2.;
                                    }
                                    let ox = offset.x.rem_euclid(step);
                                    let oy = offset.y.rem_euclid(step);
                                    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                                    let d = (1.2 + zoom * 0.4).clamp(1.2, 2.2);
                                    let mut y = oy;
                                    while y < h {
                                        let mut x = ox;
                                        while x < w {
                                            window.paint_quad(
                                                fill(Bounds::new(point(bounds.origin.x + px(x), bounds.origin.y + px(y)), size(px(d), px(d))), grid)
                                                    .corner_radii(px(d / 2.)),
                                            );
                                            x += step;
                                        }
                                        y += step;
                                    }
                                }
                                paint_scene(&scene, view, bounds, window, cx);
                                overlay.paint(view, bounds, window, cx);
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::right_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::right_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::scroll)),
            )
            .when(empty && self.tool == Tool::Select && quick_preview.is_none(), |d| {
                // 空画板提示（「双击开始画图，或选择 模板」）
                d.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .text_color(t.text_muted)
                            .child(
                                div().flex().items_center().gap(px(6.)).text_size(px(18.)).child("双击开始画图，或选择").child(
                                    div()
                                        .id("empty-tpl")
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .text_color(t.accent)
                                        .cursor_pointer()
                                        .hover(|s| s.opacity(0.8))
                                        .child(crate::ui::icon("layout-template", 18., t.accent))
                                        .child("模板")
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                        .on_click(cx.listener(|this, _, _, cx| this.open_templates(cx))),
                                ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .opacity(0.8)
                                    .child("R 圆角矩形 · O 圆形 · D 菱形 · L 连线 · T 文本 · N 便签 · P 画笔 · ⇧S 分区 · M 更多图形"),
                            ),
                    ),
                )
            })
            .children(quick_preview)
            .children(self.render_edit_overlay(&t, cx))
            .children(self.render_selbar(&t, window, cx))
            .children(self.render_library(&t, cx))
            .children(self.render_icons(&t, cx))
            .child(self.render_left_toolbar(&t, cx))
            .children(self.render_find(&t, cx))
            .child(self.render_topbar(&t, cx))
            .children(self.render_pen_bar(&t, cx))
            .child(self.render_zoom_bar(&t, cx))
            .children(self.render_minimap(&t, cx))
            .children(self.render_menu(&t, cx))
            .children(self.render_picker(&t, cx))
            .children(self.render_quick_templates(&t, cx))
            .children(self.render_templates(&t, cx))
            .children(self.render_code_dlg(&t, window, cx))
    }
}
