//! 阅读模式（思路来自 quiet.md 的 reading.rs）：不显示拖动手柄、浮动工具栏和光标，
//! 键入的文字不会进入文档，选中和复制照常可用。
//!
//! 三层防护：
//! 1. 文字输入 / 输入法直接跳过（`input.rs`）；
//! 2. `checkpoint` 不记撤销点；
//! 3. 所有修改最后都会走 `changed()`：发现文档和进入阅读模式时的快照不同，就整体回滚并提示。
//!    这样不必在每个编辑入口单独拦截，点复选框、拖动块、粘贴等也一并挡住。

use super::{Editor, EditorEvent};
use gpui::*;
use std::sync::Arc;

pub const READ_ONLY_HINT: &str = "当前为阅读模式，按 ⌥⌘R 或点顶栏「阅读模式」切换回编辑";

impl Editor {
    pub fn is_read_only(&self) -> bool {
        self.sess.read_only
    }

    pub fn set_read_only(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.sess.read_only == on {
            return;
        }
        self.sess.read_only = on;
        self.sess.ro_doc = on.then(|| self.content.blocks.clone());
        self.break_coalescing();
        self.core.marked = None;
        self.pop.slash = None;
        self.pop.block_menu = None;
        self.pop.lang_menu = None;
        self.pop.ordered_menu = None;
        self.pop.link_edit = None;
        self.pop.tb_menu = None;
        cx.notify();
    }

    /// 阅读模式下有修改漏进来：回滚到快照并提示。返回 true 表示已拦截（调用方不再走保存流程）。
    pub(super) fn reject_read_only_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.sess.read_only {
            return false;
        }
        let Some(snap) = self.sess.ro_doc.clone() else { return true };
        let same = snap.len() == self.content.blocks.len() && snap.iter().zip(&self.content.blocks).all(|(a, b)| Arc::ptr_eq(a, b));
        if !same {
            self.replace_all(snap);
            self.core.anchor = self.clamp(self.core.anchor);
            self.core.head = self.clamp(self.core.head);
            cx.emit(EditorEvent::Toast(READ_ONLY_HINT.into()));
        }
        cx.notify();
        true
    }
}
