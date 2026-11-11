//! 全屏嵌入对象编辑器（画板 / 草图 / 电子表格 / 多维表格）的统一接口。
//!
//! 工作区只通过这个 trait 打开、关闭它们：新增一种嵌入对象只要实现 [`EmbedEditor`]，
//! 再在 [`crate::objects::ObjKind`] 里加一个分支，不用复制一遍「新建视图 → 订阅关闭事件 → 抢焦点」的流程。

use gpui::{Context, EventEmitter, Focusable, Render, Window};

pub trait EmbedEditor: Render + Focusable + EventEmitter<Self::Event> + Sized + 'static {
    type Event: 'static;

    /// 打开对象 `obj_id`（数据从 `objects` 缓存里取）。
    fn open(obj_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// 这个事件是否表示「关闭全屏编辑器」。
    fn closes(ev: &Self::Event) -> bool;
}

impl EmbedEditor for crate::board::view::BoardEditor {
    type Event = crate::board::view::BoardEvent;
    fn open(obj_id: String, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(obj_id, cx)
    }
    fn closes(_: &Self::Event) -> bool {
        true
    }
}

impl EmbedEditor for crate::sketch::view::SketchEditor {
    type Event = crate::sketch::view::SketchEvent;
    fn open(obj_id: String, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(obj_id, cx)
    }
    fn closes(_: &Self::Event) -> bool {
        true
    }
}

impl EmbedEditor for crate::sheet::SheetEditor {
    type Event = crate::sheet::SheetEvent;
    fn open(obj_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(obj_id, window, cx)
    }
    fn closes(ev: &Self::Event) -> bool {
        matches!(ev, crate::sheet::SheetEvent::Close)
    }
}

impl EmbedEditor for crate::base::editor::BaseEditor {
    type Event = crate::base::editor::BaseEvent;
    fn open(obj_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(obj_id, window, cx)
    }
    fn closes(ev: &Self::Event) -> bool {
        matches!(ev, crate::base::editor::BaseEvent::Close)
    }
}
