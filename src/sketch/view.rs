//! 草图全屏编辑器：状态、快捷键、整体渲染。
//!
//! - `interact.rs`：鼠标交互（创建、选择、移动、缩放、旋转、绑定、橡皮）
//! - `ops.rs`：命令（撤销、剪贴板、成组、层级、对齐、导入导出、文字编辑）
//! - `ui.rs`：工具栏、属性面板、菜单、缩放栏

use super::geom::{self, v, R, V};
use super::model::Element;
use super::model::*;
use super::paint;
use crate::canvas::paint::View;
use crate::canvas::{pt, Pt};
use crate::text_area::TextArea;
use crate::theme::theme;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

actions!(
    sketch,
    [
        SDelete,
        SUndo,
        SRedo,
        SSelectAll,
        SDuplicate,
        SCopy,
        SCut,
        SPaste,
        SGroup,
        SUngroup,
        SLock,
        SForward,
        SBackward,
        SFront,
        SBack,
        SEscape,
        SEnter,
        SNudgeLeft,
        SNudgeRight,
        SNudgeUp,
        SNudgeDown,
        SNudgeLeftBig,
        SNudgeRightBig,
        SNudgeUpBig,
        SNudgeDownBig,
        SZoomIn,
        SZoomOut,
        SZoomFit,
        SZoomSel,
        SZoomReset,
        SToggleGrid,
        SToolLock,
        SCopyPng,
        SOpen,
        SSaveAs,
        SAlignLeft,
        SAlignRight,
        SAlignTop,
        SAlignBottom,
        SToolSelect,
        SToolHand,
        SToolRect,
        SToolDiamond,
        SToolEllipse,
        SToolArrow,
        SToolLine,
        SToolDraw,
        SToolText,
        SToolImage,
        SToolEraser,
        SToolFrame,
        SToolSticky,
        SToolLaser,
        SToolBucket,
        SToolShapeDraw,
        SLineEdit,
        SToggleView
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Sketch");
    cx.bind_keys([
        KeyBinding::new("backspace", SDelete, c),
        KeyBinding::new("delete", SDelete, c),
        KeyBinding::new("cmd-z", SUndo, c),
        KeyBinding::new("cmd-shift-z", SRedo, c),
        KeyBinding::new("cmd-y", SRedo, c),
        KeyBinding::new("cmd-a", SSelectAll, c),
        KeyBinding::new("cmd-d", SDuplicate, c),
        KeyBinding::new("cmd-c", SCopy, c),
        KeyBinding::new("cmd-x", SCut, c),
        KeyBinding::new("cmd-v", SPaste, c),
        KeyBinding::new("cmd-g", SGroup, c),
        KeyBinding::new("cmd-shift-g", SUngroup, c),
        KeyBinding::new("cmd-shift-l", SLock, c),
        KeyBinding::new("cmd-]", SForward, c),
        KeyBinding::new("cmd-[", SBackward, c),
        KeyBinding::new("cmd-shift-]", SFront, c),
        KeyBinding::new("cmd-shift-[", SBack, c),
        KeyBinding::new("cmd-alt-]", SFront, c),
        KeyBinding::new("cmd-alt-[", SBack, c),
        KeyBinding::new("escape", SEscape, c),
        KeyBinding::new("enter", SEnter, c),
        KeyBinding::new("left", SNudgeLeft, c),
        KeyBinding::new("right", SNudgeRight, c),
        KeyBinding::new("up", SNudgeUp, c),
        KeyBinding::new("down", SNudgeDown, c),
        KeyBinding::new("shift-left", SNudgeLeftBig, c),
        KeyBinding::new("shift-right", SNudgeRightBig, c),
        KeyBinding::new("shift-up", SNudgeUpBig, c),
        KeyBinding::new("shift-down", SNudgeDownBig, c),
        KeyBinding::new("cmd-=", SZoomIn, c),
        KeyBinding::new("cmd-+", SZoomIn, c),
        KeyBinding::new("cmd--", SZoomOut, c),
        KeyBinding::new("cmd-0", SZoomReset, c),
        KeyBinding::new("shift-1", SZoomFit, c),
        KeyBinding::new("shift-2", SZoomSel, c),
        KeyBinding::new("cmd-'", SToggleGrid, c),
        KeyBinding::new("ctrl-'", SToggleGrid, c),
        KeyBinding::new("q", SToolLock, c),
        KeyBinding::new("cmd-shift-c", SCopyPng, c),
        KeyBinding::new("cmd-o", SOpen, c),
        KeyBinding::new("cmd-shift-s", SSaveAs, c),
        KeyBinding::new("cmd-s", SSaveAs, c),
        KeyBinding::new("cmd-shift-left", SAlignLeft, c),
        KeyBinding::new("cmd-shift-right", SAlignRight, c),
        KeyBinding::new("cmd-shift-up", SAlignTop, c),
        KeyBinding::new("cmd-shift-down", SAlignBottom, c),
        // 工具：与官网一致（字母 + 数字）
        KeyBinding::new("v", SToolSelect, c),
        KeyBinding::new("1", SToolSelect, c),
        KeyBinding::new("h", SToolHand, c),
        KeyBinding::new("r", SToolRect, c),
        KeyBinding::new("2", SToolRect, c),
        KeyBinding::new("d", SToolDiamond, c),
        KeyBinding::new("3", SToolDiamond, c),
        KeyBinding::new("o", SToolEllipse, c),
        KeyBinding::new("4", SToolEllipse, c),
        KeyBinding::new("a", SToolArrow, c),
        KeyBinding::new("5", SToolArrow, c),
        KeyBinding::new("l", SToolLine, c),
        KeyBinding::new("6", SToolLine, c),
        KeyBinding::new("p", SToolDraw, c),
        KeyBinding::new("7", SToolDraw, c),
        KeyBinding::new("t", SToolText, c),
        KeyBinding::new("8", SToolText, c),
        KeyBinding::new("i", SToolImage, c),
        KeyBinding::new("9", SToolImage, c),
        KeyBinding::new("e", SToolEraser, c),
        KeyBinding::new("0", SToolEraser, c),
        KeyBinding::new("f", SToolFrame, c),
        KeyBinding::new("n", SToolSticky, c),
        KeyBinding::new("k", SToolLaser, c),
        KeyBinding::new("b", SToolBucket, c),
        KeyBinding::new("shift-x", SToolShapeDraw, c),
        KeyBinding::new("cmd-enter", SLineEdit, c),
        KeyBinding::new("alt-r", SToggleView, c),
    ]);
    // 查看模式：只留缩放和退出，编辑类快捷键一律不响应
    let vc = Some("SketchView");
    cx.bind_keys([
        KeyBinding::new("alt-r", SToggleView, vc),
        KeyBinding::new("escape", SToggleView, vc),
        KeyBinding::new("cmd-=", SZoomIn, vc),
        KeyBinding::new("cmd-+", SZoomIn, vc),
        KeyBinding::new("cmd--", SZoomOut, vc),
        KeyBinding::new("cmd-0", SZoomReset, vc),
        KeyBinding::new("shift-1", SZoomFit, vc),
        KeyBinding::new("cmd-shift-c", SCopyPng, vc),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Hand,
    Rect,
    Diamond,
    Ellipse,
    Arrow,
    Line,
    Draw,
    Text,
    Image,
    Eraser,
    Frame,
    /// 便签（N）
    Sticky,
    /// 套索选择
    Lasso,
    /// 激光笔（K）：会消失的红色轨迹，不进文档
    Laser,
    /// 油漆桶（B）：点元素填色
    Bucket,
    /// 画完自动识别成形状（⇧X）
    ShapeDraw,
}

impl Tool {
    #[cfg(feature = "dev")]
    pub fn parse(s: &str) -> Option<Tool> {
        Some(match s {
            "select" => Tool::Select,
            "hand" => Tool::Hand,
            "rect" | "rectangle" => Tool::Rect,
            "diamond" => Tool::Diamond,
            "ellipse" => Tool::Ellipse,
            "arrow" => Tool::Arrow,
            "line" => Tool::Line,
            "draw" | "freedraw" => Tool::Draw,
            "text" => Tool::Text,
            "image" => Tool::Image,
            "eraser" => Tool::Eraser,
            "frame" => Tool::Frame,
            "sticky" | "stickynote" | "note" => Tool::Sticky,
            "lasso" => Tool::Lasso,
            "laser" => Tool::Laser,
            "bucket" => Tool::Bucket,
            "shape" | "drawshape" => Tool::ShapeDraw,
            _ => return None,
        })
    }
    pub fn kind(self) -> Option<&'static str> {
        Some(match self {
            Tool::Rect => "rectangle",
            Tool::Diamond => "diamond",
            Tool::Ellipse => "ellipse",
            Tool::Arrow => "arrow",
            Tool::Line => "line",
            Tool::Draw => "freedraw",
            Tool::Text => "text",
            Tool::Image => "image",
            Tool::Frame => "frame",
            Tool::Sticky => "stickynote",
            Tool::ShapeDraw => "freedraw",
            _ => return None,
        })
    }
}

/// 新元素使用的当前样式（官方 appState.currentItem*）。
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStyle {
    pub stroke: String,
    pub bg: String,
    pub fill: String,
    pub stroke_width: f64,
    pub stroke_style: String,
    pub roughness: f64,
    pub opacity: f64,
    pub round: bool,
    pub font_size: f64,
    pub font_family: i64,
    pub text_align: String,
    pub start_arrow: Option<String>,
    pub end_arrow: Option<String>,
    /// 箭头类型：sharp / round / elbow
    pub arrow_type: &'static str,
    /// 便签自己的配色（官方 currentItemStickynote*）
    pub sticky_bg: String,
    pub sticky_ink: String,
}

impl Default for ItemStyle {
    fn default() -> Self {
        ItemStyle {
            stroke: DEFAULT_STROKE.into(),
            bg: TRANSPARENT.into(),
            fill: "solid".into(),
            stroke_width: 2.,
            stroke_style: "solid".into(),
            roughness: 1.,
            opacity: 100.,
            round: true,
            font_size: DEFAULT_FONT_SIZE,
            font_family: FONT_HAND,
            text_align: "left".into(),
            start_arrow: None,
            end_arrow: Some("arrow".into()),
            arrow_type: "round",
            sticky_bg: STICKY_BG.into(),
            sticky_ink: DEFAULT_STROKE.into(),
        }
    }
}

pub(super) enum Drag {
    Move {
        start: V,
        orig: Vec<(String, f64, f64)>,
        moved: bool,
        /// 按在选区外框内的空白处（没点中元素本身）：没拖动就松开 = 取消选择（官网行为）
        from_box: bool,
    },
    Resize {
        handle: usize,
        orig: Vec<Element>,
        bounds: R,
        single: bool,
    },
    Rotate {
        center: V,
        start: f64,
        orig: Vec<Element>,
    },
    Create {
        id: String,
        start: V,
    },
    Linear {
        id: String,
        start: V,
    },
    Point {
        id: String,
        idx: usize,
    },
    Marquee {
        start: V,
        cur: V,
    },
    Pan {
        start: Point<Pixels>,
        orig: Pt,
    },
    Draw {
        id: String,
    },
    Erase {
        last: V,
    },
    Lasso {
        pts: Vec<V>,
    },
    Laser,
    Crop {
        handle: usize,
        orig: Element,
    },
    /// 拖透明度滑块
    Slider,
}

/// 原位文字编辑
pub(super) struct Editing {
    pub id: String,
    /// 编辑的是框架名称
    pub frame_name: bool,
    pub area: Entity<TextArea>,
    pub _sub: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popover {
    Menu,
    Export,
    /// 工具栏「更多工具」
    Tools,
    /// 颜色选择器
    Color(PickTarget),
    /// 起点 / 终点端点样式（true = 起点）
    Head(bool),
    /// 更多字体
    Fonts,
}

/// 颜色选择器作用于哪种颜色
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickTarget {
    Stroke,
    Bg,
}

/// 浮动输入：链接 / 十六进制颜色
pub(super) struct InputPrompt {
    pub kind: PromptKind,
    pub input: Entity<crate::line_input::LineInput>,
    pub _sub: Subscription,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PromptKind {
    /// 元素链接
    Link(Vec<String>),
    /// 颜色选择器里的十六进制
    Hex(PickTarget),
}

/// Mermaid 至草图对话框
pub(super) struct MermaidDlg {
    pub area: Entity<TextArea>,
    pub error: Option<String>,
    /// 右侧预览（随输入防抖刷新）
    pub preview: Option<MermaidPreview>,
    /// 预览对应的源码：输入框光标闪烁也会通知，源码没变就不重算
    pub preview_src: String,
    pub preview_gen: u64,
    pub _sub: Subscription,
}

pub(super) enum MermaidPreview {
    Doc(Arc<SketchDoc>),
    Image(std::path::PathBuf),
    Error(String),
}

pub enum SketchEvent {
    Close,
}

impl EventEmitter<SketchEvent> for SketchEditor {}

pub struct SketchEditor {
    pub obj_id: String,
    pub(super) doc: SketchDoc,
    pub(super) undo: VecDeque<SketchDoc>,
    pub(super) redo: Vec<SketchDoc>,
    /// 世界原点在画布里的位置（像素）
    pub(super) offset: Pt,
    pub(super) zoom: f32,
    pub(super) tool: Tool,
    pub(super) tool_lock: bool,
    pub(super) sel: HashSet<String>,
    pub(super) drag: Option<Drag>,
    /// 正在逐点绘制的线 / 箭头
    pub(super) multi: Option<String>,
    pub(super) editing: Option<Editing>,
    pub(super) focus: FocusHandle,
    pub(super) canvas_bounds: Bounds<Pixels>,
    /// 拖线时吸附的绑定目标
    pub(super) bind_hint: Option<String>,
    /// 拖箭头时起点所在的形状与按下位置：起点实时吸到轮廓上（官网同款）
    pub(super) link_start: Option<(String, V)>,
    /// 橡皮擦经过、松开后删除的元素
    pub(super) erase: Vec<String>,
    pub(super) mouse_world: V,
    pub(super) space_down: bool,
    pub(super) fitted: bool,
    pub(super) grid: bool,
    pub(super) cur: ItemStyle,
    pub(super) popover: Option<Popover>,
    /// 右键菜单（窗口坐标）
    pub(super) menu: Option<Point<Pixels>>,
    pub(super) toast: Option<(String, u64)>,
    pub(super) toast_gen: u64,
    /// 悬停的缩放 / 旋转手柄（光标形状用）
    pub(super) hover_handle: Option<usize>,
    /// 选择工具悬停在可拖动的元素 / 选区外框上：光标显示四向箭头
    pub(super) hover_move: bool,
    /// 查看模式（官网 ⌥R）：只读，拖动平移，点带链接的元素打开链接
    pub(super) view_mode: bool,
    /// 查看模式下悬停在带链接的元素上（光标变手指）
    pub(super) hover_link: bool,
    /// 线编辑模式：显示全部节点与中点，可增删节点
    pub(super) line_edit: Option<String>,
    /// 线编辑模式里选中的节点
    pub(super) line_pt: Option<usize>,
    /// 图片裁剪模式
    pub(super) crop_edit: Option<String>,
    /// 激光笔轨迹（世界坐标 + 时刻），约 1 秒后淡出
    pub(super) laser: Vec<(V, std::time::Instant)>,
    pub(super) laser_anim: bool,
    /// 吸管：下一次点画布取色
    pub(super) eyedropper: Option<PickTarget>,
    pub(super) prompt: Option<InputPrompt>,
    pub(super) mermaid: Option<MermaidDlg>,
    /// 透明度滑块轨道位置（窗口坐标）
    pub(super) slider_bounds: Bounds<Pixels>,
    /// 颜色选择器里展示的色阶（0..5），None = 按当前颜色推断
    pub(super) shade: Option<usize>,
}

impl Focusable for SketchEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SketchEditor {
    pub fn new(obj_id: String, cx: &mut Context<Self>) -> Self {
        let doc = crate::objects::sketch(&obj_id).map(|d| (*d).clone()).unwrap_or_default();
        let grid = doc.app_state.get("gridModeEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
        SketchEditor {
            obj_id,
            doc,
            undo: VecDeque::new(),
            redo: vec![],
            offset: pt(0., 0.),
            zoom: 1.,
            tool: Tool::Select,
            tool_lock: false,
            sel: HashSet::new(),
            drag: None,
            multi: None,
            editing: None,
            focus: cx.focus_handle(),
            canvas_bounds: Bounds::default(),
            bind_hint: None,
            link_start: None,
            erase: vec![],
            mouse_world: V::default(),
            space_down: false,
            fitted: false,
            grid,
            cur: ItemStyle::default(),
            popover: None,
            menu: None,
            toast: None,
            toast_gen: 0,
            hover_handle: None,
            hover_move: false,
            view_mode: false,
            hover_link: false,
            line_edit: None,
            line_pt: None,
            crop_edit: None,
            laser: vec![],
            laser_anim: false,
            eyedropper: None,
            prompt: None,
            mermaid: None,
            slider_bounds: Bounds::default(),
            shade: None,
        }
    }

    pub(super) fn view(&self) -> View {
        View { origin: point(self.canvas_bounds.origin.x + px(self.offset.x), self.canvas_bounds.origin.y + px(self.offset.y)), scale: self.zoom }
    }

    pub(super) fn to_world(&self, p: Point<Pixels>) -> V {
        let w = self.view().inv(p);
        v(w.x as f64, w.y as f64)
    }

    pub(super) fn to_screen(&self, p: V) -> Point<Pixels> {
        self.view().p(pt(p.x as f32, p.y as f32))
    }

    pub(super) fn snapshot(&mut self) {
        self.undo.push_back(self.doc.clone());
        if self.undo.len() > 200 {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// 写回对象存储（写线程后台保存）。
    pub(super) fn commit(&mut self, cx: &mut Context<Self>) {
        self.doc.app_state.insert("gridModeEnabled".into(), serde_json::Value::Bool(self.grid));
        crate::objects::update(&self.obj_id, crate::objects::ObjData::Sketch(Arc::new(self.doc.clone())));
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(id) = self.multi.clone() {
            self.finish_linear(&id, cx);
        }
        self.commit(cx);
        cx.emit(SketchEvent::Close);
    }

    pub(super) fn show_toast(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.toast_gen += 1;
        let g = self.toast_gen;
        self.toast = Some((msg.into(), g));
        cx.spawn(async move |this: WeakEntity<SketchEditor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(2200)).await;
            this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|(_, gg)| *gg == g) {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn set_tool(&mut self, t: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(id) = self.multi.clone() {
            self.finish_linear(&id, cx);
        }
        self.tool = t;
        self.bind_hint = None;
        self.link_start = None;
        self.line_edit = None;
        self.line_pt = None;
        self.crop_edit = None;
        self.eyedropper = None;
        if !matches!(t, Tool::Select | Tool::Lasso) {
            self.sel.clear();
        }
        self.popover = None;
        self.menu = None;
        if t == Tool::Image {
            self.pick_image(None, window, cx);
            self.tool = Tool::Select;
        }
        cx.notify();
    }

    // ───────────── 选区 ─────────────

    /// 选中的元素（文档顺序），不含容器 / 箭头上的绑定文字
    pub(super) fn selected(&self) -> Vec<&Element> {
        self.doc.live().filter(|e| self.sel.contains(&e.id)).collect()
    }

    pub(super) fn single(&self) -> Option<&Element> {
        if self.sel.len() != 1 {
            return None;
        }
        self.doc.get(self.sel.iter().next()?)
    }

    pub(super) fn selection_bounds(&self) -> Option<R> {
        geom::bounds_of(self.selected())
    }

    /// 点在选区（未锁定）的公共外框里：官网可从外框内任意处拖动已选元素
    pub(super) fn in_sel_box(&self, p: V) -> bool {
        if self.line_edit.is_some() || self.crop_edit.is_some() || self.selected().iter().all(|e| e.locked) {
            return false;
        }
        self.selection_bounds().is_some_and(|r| r.inflate(4. / self.zoom as f64).contains(p))
    }

    /// 点中元素时实际选中的集合：最外层分组的全部成员；绑定文字换成它的容器。
    pub(super) fn select_target(&self, id: &str) -> Vec<String> {
        let Some(e) = self.doc.get(id) else { return vec![] };
        let e = match e.container_id().and_then(|c| self.doc.get(c)) {
            Some(c) => c,
            None => e,
        };
        match e.group_ids.last() {
            Some(g) => self.doc.live().filter(|x| x.group_ids.contains(g)).map(|x| x.id.clone()).collect(),
            None => vec![e.id.clone()],
        }
    }

    /// 最上层被点中的元素（锁定的不算；容器文字算它的容器）
    pub(super) fn hit_at(&self, p: V) -> Option<String> {
        let tol = 6. / self.zoom as f64;
        // 先找非框架元素，框架只在点到边框 / 标题时命中
        self.doc
            .live()
            .rev()
            .filter(|e| !e.locked && !e.is_frame())
            .find(|e| geom::hit(e, p, tol))
            .or_else(|| self.doc.live().rev().filter(|e| !e.locked && e.is_frame()).find(|e| geom::hit(e, p, tol)))
            .map(|e| e.id.clone())
    }

    /// 选区包括绑定文字（复制、删除时一起带上）
    pub(super) fn sel_with_bound(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.sel.iter().cloned().collect();
        for e in self.doc.live() {
            if let Some(c) = e.container_id() {
                if self.sel.contains(c) && !ids.contains(&e.id) {
                    ids.push(e.id.clone());
                }
            }
            if e.is_frame() && self.sel.contains(&e.id) {
                for k in self.doc.live().filter(|k| k.frame_id() == Some(&e.id)) {
                    if !ids.contains(&k.id) {
                        ids.push(k.id.clone());
                    }
                }
            }
        }
        ids
    }

    // ───────────── 视图 ─────────────

    pub(super) fn zoom_at(&mut self, screen: Point<Pixels>, factor: f32) {
        let w = self.view().inv(screen);
        self.zoom = (self.zoom * factor).clamp(0.1, 30.);
        let local = (f32::from(screen.x - self.canvas_bounds.origin.x), f32::from(screen.y - self.canvas_bounds.origin.y));
        self.offset = pt(local.0 - w.x * self.zoom, local.1 - w.y * self.zoom);
    }

    pub(super) fn zoom_to(&mut self, r: Option<R>, max: f32) {
        let size = self.canvas_bounds.size;
        let (vw, vh) = (f32::from(size.width).max(200.), f32::from(size.height).max(200.));
        match r {
            Some(r) => {
                let r = r.inflate(60.);
                self.zoom = (vw / r.w() as f32).min(vh / r.h() as f32).clamp(0.1, max);
                let c = r.center();
                self.offset = pt(vw / 2. - c.x as f32 * self.zoom, vh / 2. - c.y as f32 * self.zoom);
            }
            None => {
                // 空画布：原点放在左上三分之一处，新画的内容不会挤到右下角
                self.zoom = 1.;
                self.offset = pt(vw * 0.3, vh * 0.3);
            }
        }
    }

    pub(super) fn zoom_step(&mut self, f: f32, cx: &mut Context<Self>) {
        let c = self.canvas_bounds.center();
        self.zoom_at(c, f);
        cx.notify();
    }

    pub(super) fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        let r = geom::bounds_of(self.doc.live());
        self.zoom_to(r, 1.);
        cx.notify();
    }

    pub(super) fn zoom_sel(&mut self, cx: &mut Context<Self>) {
        let r = self.selection_bounds().or_else(|| geom::bounds_of(self.doc.live()));
        self.zoom_to(r, 4.);
        cx.notify();
    }

    pub(super) fn zoom_100(&mut self, cx: &mut Context<Self>) {
        let c = self.canvas_bounds.center();
        let f = 1. / self.zoom;
        self.zoom_at(c, f);
        cx.notify();
    }

    pub(super) fn snap(&self, p: V) -> V {
        if !self.grid {
            return p;
        }
        let g = self.doc.grid_size();
        v((p.x / g).round() * g, (p.y / g).round() * g)
    }
}

/// 叠加层：选区框、手柄、框选、绑定高亮（画在场景之上）。
pub(super) struct Overlay {
    pub sel_boxes: Vec<[V; 4]>,
    pub group_box: Option<[V; 4]>,
    pub handles: Vec<V>,
    pub rot_handle: Option<V>,
    pub points: Vec<V>,
    pub marquee: Option<R>,
    /// 绑定目标的真实轮廓（闭合折线）
    pub bind: Vec<V>,
    /// 端点将吸附到的轮廓点（实心小圆点）
    pub bind_dot: Option<V>,
    /// 从形状内部起拖：按下处（空心圆）→ 轮廓上的起点（虚线）
    pub bind_press: Option<(V, V)>,
    pub accent: Hsla,
    /// 绑定高亮色（官网是浅蓝）
    pub bind_color: Hsla,
    pub bg: Hsla,
    /// 线段中点（拖动可弯曲 / 加点）
    pub mids: Vec<V>,
    /// 线编辑里选中的节点
    pub sel_pt: Option<V>,
    /// 套索路径
    pub lasso: Vec<V>,
    /// 裁剪模式：整张原图的轮廓
    pub crop_full: Option<[V; 4]>,
    /// 激光笔轨迹（点 + 剩余不透明度）
    pub laser: Vec<(V, f32)>,
}

impl Overlay {
    fn paint(&self, view: View, window: &mut Window) {
        let sp = |p: V| view.p(pt(p.x as f32, p.y as f32));
        let outline = |q: &[V; 4], w: f32, color: Hsla, window: &mut Window| {
            let mut b = PathBuilder::stroke(px(w));
            b.move_to(sp(q[0]));
            for p in &q[1..] {
                b.line_to(sp(*p));
            }
            b.close();
            if let Ok(p) = b.build() {
                window.paint_path(p, color);
            }
        };
        for q in &self.sel_boxes {
            outline(q, 1., self.accent, window);
        }
        if let Some(q) = &self.group_box {
            let mut b = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
            b.move_to(sp(q[0]));
            for p in &q[1..] {
                b.line_to(sp(*p));
            }
            b.close();
            if let Ok(p) = b.build() {
                window.paint_path(p, self.accent);
            }
        }
        if self.bind.len() >= 3 {
            let mut b = PathBuilder::stroke(px(2.5));
            b.move_to(sp(self.bind[0]));
            for p in &self.bind[1..] {
                b.line_to(sp(*p));
            }
            b.close();
            if let Ok(p) = b.build() {
                window.paint_path(p, self.bind_color);
            }
        }
        if let Some((a, b)) = self.bind_press {
            let mut pb = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
            pb.move_to(sp(a));
            pb.line_to(sp(b));
            if let Ok(p) = pb.build() {
                window.paint_path(p, self.accent.opacity(0.7));
            }
            let c = sp(a);
            let bb = Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.)));
            window.paint_quad(fill(bb, self.bg).corner_radii(px(5.)).border_widths(px(1.5)).border_color(self.accent));
        }
        if let Some(d) = self.bind_dot {
            let c = sp(d);
            let bb = Bounds::new(point(c.x - px(4.5), c.y - px(4.5)), size(px(9.), px(9.)));
            window.paint_quad(fill(bb, self.bind_color.opacity(1.)).corner_radii(px(4.5)));
        }
        for h in &self.handles {
            let c = sp(*h);
            let b = Bounds::new(point(c.x - px(4.5), c.y - px(4.5)), size(px(9.), px(9.)));
            window.paint_quad(fill(b, self.bg).corner_radii(px(2.)).border_widths(px(1.)).border_color(self.accent));
        }
        if let Some(h) = self.rot_handle {
            let c = sp(h);
            let b = Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.)));
            window.paint_quad(fill(b, self.bg).corner_radii(px(5.)).border_widths(px(1.)).border_color(self.accent));
        }
        for p in &self.points {
            let c = sp(*p);
            let b = Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.)));
            window.paint_quad(fill(b, self.bg).corner_radii(px(5.)).border_widths(px(1.5)).border_color(self.accent));
        }
        for m in &self.mids {
            let c = sp(*m);
            let b = Bounds::new(point(c.x - px(4.), c.y - px(4.)), size(px(8.), px(8.)));
            window.paint_quad(fill(b, self.accent.opacity(0.45)).corner_radii(px(4.)));
        }
        if let Some(p) = self.sel_pt {
            let c = sp(p);
            let b = Bounds::new(point(c.x - px(8.), c.y - px(8.)), size(px(16.), px(16.)));
            window.paint_quad(fill(b, self.accent.opacity(0.25)).corner_radii(px(8.)));
        }
        if let Some(q) = &self.crop_full {
            let mut b = PathBuilder::stroke(px(1.)).dash_array(&[px(5.), px(4.)]);
            b.move_to(sp(q[0]));
            for p in &q[1..] {
                b.line_to(sp(*p));
            }
            b.close();
            if let Ok(p) = b.build() {
                window.paint_path(p, self.accent.opacity(0.8));
            }
        }
        if self.lasso.len() >= 2 {
            let mut b = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
            b.move_to(sp(self.lasso[0]));
            for p in &self.lasso[1..] {
                b.line_to(sp(*p));
            }
            if let Ok(p) = b.build() {
                window.paint_path(p, self.accent);
            }
        }
        // 激光笔：越早的点越淡、越细
        for w in self.laser.windows(2) {
            let (a, b) = (w[0], w[1]);
            let alpha = a.1.min(b.1);
            if alpha <= 0.02 {
                continue;
            }
            let mut pb = PathBuilder::stroke(px(1.5 + 3.5 * alpha));
            pb.move_to(sp(a.0));
            pb.line_to(sp(b.0));
            if let Ok(p) = pb.build() {
                window.paint_path(p, hsla(0., 0.95, 0.55, alpha));
            }
        }
        if let Some(r) = self.marquee {
            let a = sp(v(r.x0, r.y0));
            let b = sp(v(r.x1, r.y1));
            let bb = Bounds::from_corners(a, b);
            window.paint_quad(fill(bb, self.accent.opacity(0.08)).border_widths(px(1.)).border_color(self.accent.opacity(0.7)));
        }
    }
}

impl Render for SketchEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let dark = t.dark;
        let doc = Arc::new(self.doc.clone());
        let view = self.view();
        let entity = cx.entity();
        let overlay = self.overlay(&t);
        let hide = self.editing.as_ref().filter(|e| !e.frame_name).map(|e| e.id.clone());
        let erase = self.erase.clone();
        let grid = self.grid;
        let gsize = self.doc.grid_size() as f32;
        let zoom = self.zoom;
        let offset = self.offset;
        let bg = paint::bg_hsla(&self.doc, dark);
        let grid_col = if dark { hsla(0., 0., 1., 0.07) } else { hsla(0., 0., 0., 0.07) };
        let cursor = match (&self.drag, self.tool) {
            (Some(Drag::Pan { .. }), _) => CursorStyle::ClosedHand,
            _ if self.view_mode && self.hover_link => CursorStyle::PointingHand,
            _ if self.view_mode => CursorStyle::OpenHand,
            (Some(Drag::Move { .. }), _) => CursorStyle::Move,
            _ if self.space_down => CursorStyle::OpenHand,
            (_, Tool::Hand) => CursorStyle::OpenHand,
            // 系统光标藏起来，画自己的吸管图标（render_eyedropper）
            _ if self.eyedropper.is_some() => CursorStyle::None,
            (_, Tool::Text) => CursorStyle::IBeam,
            (_, Tool::Bucket) => CursorStyle::PointingHand,
            (_, Tool::Lasso) => CursorStyle::Arrow,
            (_, Tool::Select) => match self.hover_handle {
                Some(0) | Some(4) => CursorStyle::ResizeUpLeftDownRight,
                Some(2) | Some(6) => CursorStyle::ResizeUpRightDownLeft,
                Some(1) | Some(5) => CursorStyle::ResizeUpDown,
                Some(3) | Some(7) => CursorStyle::ResizeLeftRight,
                Some(_) => CursorStyle::PointingHand,
                None if self.hover_move => CursorStyle::Move,
                None => CursorStyle::Arrow,
            },
            _ => CursorStyle::Crosshair,
        };
        let editing = self.editing.is_some();
        let typing = editing || self.prompt.is_some() || self.mermaid.is_some();
        let picking = matches!(self.popover, Some(Popover::Color(_)));
        let empty = self.doc.is_empty();
        div()
            // 编辑文字 / 输入框时换按键上下文：单字母工具快捷键不抢输入法的按键；
            // 颜色选择器打开时字母键用来选色
            .key_context(if self.view_mode {
                "SketchView"
            } else if typing {
                "SketchText"
            } else if picking {
                "SketchPicker"
            } else {
                "Sketch"
            })
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(bg)
            .text_color(t.text)
            .font_family("PingFang SC")
            .on_action(cx.listener(|this, _: &SDelete, _, cx| this.delete_sel(cx)))
            .on_action(cx.listener(|this, _: &SUndo, _, cx| this.do_undo(cx)))
            .on_action(cx.listener(|this, _: &SRedo, _, cx| this.do_redo(cx)))
            .on_action(cx.listener(|this, _: &SSelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &SDuplicate, _, cx| this.duplicate(cx)))
            .on_action(cx.listener(|this, _: &SCopy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &SCut, _, cx| this.cut(cx)))
            .on_action(cx.listener(|this, _: &SPaste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &SGroup, _, cx| this.group(cx)))
            .on_action(cx.listener(|this, _: &SUngroup, _, cx| this.ungroup(cx)))
            .on_action(cx.listener(|this, _: &SLock, _, cx| this.toggle_lock(cx)))
            .on_action(cx.listener(|this, _: &SForward, _, cx| this.z_order(1, cx)))
            .on_action(cx.listener(|this, _: &SBackward, _, cx| this.z_order(-1, cx)))
            .on_action(cx.listener(|this, _: &SFront, _, cx| this.z_order(2, cx)))
            .on_action(cx.listener(|this, _: &SBack, _, cx| this.z_order(-2, cx)))
            .on_action(cx.listener(|this, _: &SEscape, w, cx| this.escape(w, cx)))
            .on_action(cx.listener(|this, _: &SEnter, w, cx| this.enter(w, cx)))
            .on_action(cx.listener(|this, _: &SNudgeLeft, _, cx| this.nudge(-1., 0., cx)))
            .on_action(cx.listener(|this, _: &SNudgeRight, _, cx| this.nudge(1., 0., cx)))
            .on_action(cx.listener(|this, _: &SNudgeUp, _, cx| this.nudge(0., -1., cx)))
            .on_action(cx.listener(|this, _: &SNudgeDown, _, cx| this.nudge(0., 1., cx)))
            .on_action(cx.listener(|this, _: &SNudgeLeftBig, _, cx| this.nudge(-10., 0., cx)))
            .on_action(cx.listener(|this, _: &SNudgeRightBig, _, cx| this.nudge(10., 0., cx)))
            .on_action(cx.listener(|this, _: &SNudgeUpBig, _, cx| this.nudge(0., -10., cx)))
            .on_action(cx.listener(|this, _: &SNudgeDownBig, _, cx| this.nudge(0., 10., cx)))
            .on_action(cx.listener(|this, _: &SZoomIn, _, cx| this.zoom_step(1.25, cx)))
            .on_action(cx.listener(|this, _: &SZoomOut, _, cx| this.zoom_step(0.8, cx)))
            .on_action(cx.listener(|this, _: &SZoomFit, _, cx| this.zoom_fit(cx)))
            .on_action(cx.listener(|this, _: &SZoomSel, _, cx| this.zoom_sel(cx)))
            .on_action(cx.listener(|this, _: &SZoomReset, _, cx| this.zoom_100(cx)))
            .on_action(cx.listener(|this, _: &SToggleGrid, _, cx| {
                this.grid = !this.grid;
                this.commit(cx);
            }))
            .on_action(cx.listener(|this, _: &SToolLock, _, cx| {
                this.tool_lock = !this.tool_lock;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SCopyPng, _, cx| this.copy_png(cx)))
            .on_action(cx.listener(|this, _: &SOpen, _, cx| this.open_file(cx)))
            .on_action(cx.listener(|this, _: &SSaveAs, _, cx| this.save_as(cx)))
            .on_action(cx.listener(|this, _: &SAlignLeft, _, cx| this.align(0, cx)))
            .on_action(cx.listener(|this, _: &SAlignRight, _, cx| this.align(2, cx)))
            .on_action(cx.listener(|this, _: &SAlignTop, _, cx| this.align(3, cx)))
            .on_action(cx.listener(|this, _: &SAlignBottom, _, cx| this.align(5, cx)))
            .on_action(cx.listener(|this, _: &SToolSelect, w, cx| this.set_tool(Tool::Select, w, cx)))
            .on_action(cx.listener(|this, _: &SToolHand, w, cx| this.set_tool(Tool::Hand, w, cx)))
            .on_action(cx.listener(|this, _: &SToolRect, w, cx| this.set_tool(Tool::Rect, w, cx)))
            .on_action(cx.listener(|this, _: &SToolDiamond, w, cx| this.set_tool(Tool::Diamond, w, cx)))
            .on_action(cx.listener(|this, _: &SToolEllipse, w, cx| this.set_tool(Tool::Ellipse, w, cx)))
            .on_action(cx.listener(|this, _: &SToolArrow, w, cx| this.set_tool(Tool::Arrow, w, cx)))
            .on_action(cx.listener(|this, _: &SToolLine, w, cx| this.set_tool(Tool::Line, w, cx)))
            .on_action(cx.listener(|this, _: &SToolDraw, w, cx| this.set_tool(Tool::Draw, w, cx)))
            .on_action(cx.listener(|this, _: &SToolText, w, cx| this.set_tool(Tool::Text, w, cx)))
            .on_action(cx.listener(|this, _: &SToolImage, w, cx| this.set_tool(Tool::Image, w, cx)))
            .on_action(cx.listener(|this, _: &SToolEraser, w, cx| this.set_tool(Tool::Eraser, w, cx)))
            .on_action(cx.listener(|this, _: &SToolFrame, w, cx| this.set_tool(Tool::Frame, w, cx)))
            .on_action(cx.listener(|this, _: &SToolSticky, w, cx| this.set_tool(Tool::Sticky, w, cx)))
            .on_action(cx.listener(|this, _: &SToolLaser, w, cx| this.set_tool(Tool::Laser, w, cx)))
            .on_action(cx.listener(|this, _: &SToolBucket, w, cx| this.set_tool(Tool::Bucket, w, cx)))
            .on_action(cx.listener(|this, _: &SToolShapeDraw, w, cx| this.set_tool(Tool::ShapeDraw, w, cx)))
            .on_action(cx.listener(|this, _: &SLineEdit, _, cx| this.toggle_line_edit(cx)))
            .on_action(cx.listener(|this, _: &SToggleView, _, cx| this.toggle_view_mode(cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if this.picker_key(ev, cx) {
                    cx.stop_propagation();
                    return;
                }
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
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if !this.view_mode {
                    this.drop_paths(paths.paths().to_vec(), cx)
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
                                        let r = geom::bounds_of(this.doc.live());
                                        this.zoom_to(r, 1.);
                                    }
                                });
                                bounds
                            },
                            move |bounds, _, window, cx| {
                                if grid {
                                    let mut step = gsize * zoom;
                                    while step < 10. {
                                        step *= 5.;
                                    }
                                    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                                    let mut x = offset.x.rem_euclid(step);
                                    while x < w {
                                        window.paint_quad(fill(
                                            Bounds::new(point(bounds.origin.x + px(x), bounds.origin.y), size(px(1.), bounds.size.height)),
                                            grid_col,
                                        ));
                                        x += step;
                                    }
                                    let mut y = offset.y.rem_euclid(step);
                                    while y < h {
                                        window.paint_quad(fill(
                                            Bounds::new(point(bounds.origin.x, bounds.origin.y + px(y)), size(bounds.size.width, px(1.))),
                                            grid_col,
                                        ));
                                        y += step;
                                    }
                                }
                                paint::paint_doc(&doc, view, bounds, dark, hide.as_deref(), &erase, window, cx);
                                overlay.paint(view, window);
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
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::scroll)),
            )
            .when(empty && self.tool == Tool::Select && !editing && !self.view_mode, |d| {
                d.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .text_color(t.text_muted)
                            .child(div().text_size(px(18.)).font_family(super::fonts::HAND_FAMILY).child("选择工具开始画，或双击写字"))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .opacity(0.85)
                                    .child("R 矩形 · D 菱形 · O 椭圆 · A 箭头 · L 直线 · P 画笔 · T 文本 · N 便签 · E 橡皮 · K 激光笔"),
                            )
                            .child(div().text_size(px(12.5)).opacity(0.85).child("⌘O 打开 .excalidraw · ⌘V 粘贴官网复制的图形")),
                    ),
                )
            })
            .children(self.render_edit_overlay(&t, cx))
            .children(self.render_link_badges(&t, cx))
            .when(!self.view_mode, |d| d.child(self.render_toolbar(&t, cx)))
            .when(self.view_mode, |d| d.child(self.render_view_badge(&t, cx)))
            .child(self.render_topbar(&t, cx))
            .when(!self.view_mode, |d| d.children(self.render_props(&t, cx)))
            .child(self.render_zoombar(&t, cx))
            .children(self.render_popover(&t, cx))
            .children(self.render_menu(&t, cx))
            .children(self.render_prompt(&t, cx))
            .children(self.render_mermaid(&t, cx))
            .children(self.render_toast(&t))
            .children(self.render_eyedropper(&t))
    }
}

impl SketchEditor {
    fn overlay(&self, t: &crate::theme::Theme) -> Overlay {
        let mut o = Overlay {
            sel_boxes: vec![],
            group_box: None,
            handles: vec![],
            rot_handle: None,
            points: vec![],
            marquee: None,
            bind: vec![],
            bind_dot: None,
            bind_press: None,
            accent: t.accent,
            bind_color: hsla(207. / 360., 0.9, 0.62, 0.75),
            bg: t.bg,
            mids: vec![],
            sel_pt: None,
            lasso: vec![],
            crop_full: None,
            laser: vec![],
        };
        if let Some(Drag::Marquee { start, cur }) = &self.drag {
            o.marquee = R::from_pts([*start, *cur]);
        }
        if let Some(Drag::Lasso { pts }) = &self.drag {
            o.lasso = pts.clone();
        }
        let now = std::time::Instant::now();
        o.laser = self.laser.iter().map(|(p, t0)| (*p, 1. - now.duration_since(*t0).as_millis() as f32 / super::tools::LASER_MS as f32)).collect();
        if let Some(t) = self.bind_hint.as_ref().and_then(|id| self.doc.get(id)) {
            o.bind = geom::outline_poly(t);
            o.bind_dot = self.bind_dot(t);
        }
        // 从形状内部起拖的箭头：标出按下处，并用虚线连到轮廓上的起点
        if let (Some(Drag::Linear { id, .. }), Some((sid, press))) = (&self.drag, &self.link_start) {
            if let (Some(a), Some(st)) = (self.doc.get(id), self.doc.get(sid)) {
                let p0 = geom::world_points(a)[0];
                if geom::inside_shape(st, *press) && p0.dist(*press) > 1. {
                    o.bind_press = Some((*press, p0));
                }
            }
        }
        if self.editing.is_some() || matches!(self.drag, Some(Drag::Create { .. }) | Some(Drag::Draw { .. }) | Some(Drag::Linear { .. })) {
            return o;
        }
        let pad = 4. / self.zoom as f64;
        let sel = self.selected();
        if sel.is_empty() || self.view_mode {
            return o;
        }
        if let Some(e) = self.single() {
            if self.crop_edit.as_deref() == Some(e.id.as_str()) {
                let q = geom::corners(e);
                o.sel_boxes.push(q);
                o.crop_full = Some(self.crop_full_quad(e));
                o.handles = handles_of(&q);
                return o;
            }
            if e.is_linear() && e.pts().len() >= 2 {
                let pts = geom::world_points(e);
                let n = pts.len();
                let editing = self.line_edit.as_deref() == Some(e.id.as_str());
                if e.elbowed() {
                    o.points = vec![pts[0], pts[n - 1]];
                    let r = geom::bounds(e).inflate(pad);
                    o.sel_boxes.push([v(r.x0, r.y0), v(r.x1, r.y0), v(r.x1, r.y1), v(r.x0, r.y1)]);
                    return o;
                }
                if n > 2 && !editing {
                    // 多点线：外框可缩放旋转，同时显示节点（可直接拖）
                    o.points = pts.clone();
                } else if n == 2 || editing {
                    if editing {
                        let r = geom::bounds(e).inflate(pad);
                        o.sel_boxes.push([v(r.x0, r.y0), v(r.x1, r.y0), v(r.x1, r.y1), v(r.x0, r.y1)]);
                        o.sel_pt = self.line_pt.and_then(|i| pts.get(i).copied());
                    }
                    if !e.locked {
                        o.mids = geom::segment_mids(e);
                    }
                    o.points = pts;
                    return o;
                }
            }
            let mut e2 = e.clone();
            e2.x -= pad;
            e2.y -= pad;
            e2.width += pad * 2.;
            e2.height += pad * 2.;
            if e.kind == "freedraw" || e.is_linear() {
                let r = geom::bounds(e).inflate(pad);
                e2.x = r.x0;
                e2.y = r.y0;
                e2.width = r.w();
                e2.height = r.h();
                e2.angle = 0.;
            }
            let q = geom::corners(&e2);
            o.sel_boxes.push(q);
            if !e.locked {
                o.handles = handles_of(&q);
                o.rot_handle = Some(rot_handle_of(&q, 18. / self.zoom as f64));
            }
            return o;
        }
        for e in &sel {
            let mut e2 = (*e).clone();
            if e.points.is_some() {
                let r = geom::bounds(e);
                e2.x = r.x0;
                e2.y = r.y0;
                e2.width = r.w();
                e2.height = r.h();
                e2.angle = 0.;
            }
            o.sel_boxes.push(geom::corners(&e2));
        }
        if let Some(r) = self.selection_bounds() {
            let r = r.inflate(pad * 2.);
            let q = [v(r.x0, r.y0), v(r.x1, r.y0), v(r.x1, r.y1), v(r.x0, r.y1)];
            o.group_box = Some(q);
            if !sel.iter().all(|e| e.locked) {
                o.handles = handles_of(&q);
                o.rot_handle = Some(rot_handle_of(&q, 18. / self.zoom as f64));
            }
        }
        o
    }
}

/// 8 个缩放手柄：左上、上、右上、右、右下、下、左下、左（与 corners 顺序一致）
pub(super) fn handles_of(q: &[V; 4]) -> Vec<V> {
    crate::canvas::math::handles8(q).to_vec()
}

/// 旋转手柄：上边中点再往外 d
pub(super) fn rot_handle_of(q: &[V; 4], d: f64) -> V {
    let top = v((q[0].x + q[1].x) / 2., (q[0].y + q[1].y) / 2.);
    let c = v((q[0].x + q[2].x) / 2., (q[0].y + q[2].y) / 2.);
    let dir = top.sub(c);
    let l = dir.len().max(1e-6);
    top.add(dir.scale(d / l))
}
