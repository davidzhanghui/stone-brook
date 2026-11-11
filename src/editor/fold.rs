//! 折叠与聚焦（Workflowy 式大纲）：
//! - 折叠：列表项（以及任何带缩进的块）收起缩进更深的后续块；标题收起到下一个同级或更高级标题之前的内容。
//!   折叠状态存在块上（`Block::collapsed`），属于视图状态：同步块与模板不带它（见 sync.rs / 模板保存）。
//! - 聚焦：只显示某一项及其子项，上方显示面包屑；`Esc` 或点面包屑返回。聚焦不落盘。
//!
//! 隐藏的块照常在列表里，只是渲染成零高度；光标移动跳过它们，光标落进隐藏区域（查找、撤销等）时自动展开。

use super::*;
use crate::model::BlockKind;

/// `i` 之后属于它的子块区间（可能为空）。
pub fn subtree(blocks: &[Arc<Block>], i: usize) -> Range<usize> {
    let Some(b) = blocks.get(i) else { return i + 1..i + 1 };
    let mut end = i + 1;
    match b.kind {
        BlockKind::Title => return end..end,
        BlockKind::Heading { level } => {
            while let Some(n) = blocks.get(end) {
                if matches!(n.kind, BlockKind::Heading { level: l } if l <= level) {
                    break;
                }
                end += 1;
            }
        }
        _ => {
            while let Some(n) = blocks.get(end) {
                if n.indent <= b.indent || matches!(n.kind, BlockKind::Heading { .. } | BlockKind::Title) {
                    break;
                }
                end += 1;
            }
        }
    }
    i + 1..end
}

/// 每个块是否隐藏：被折叠的祖先收起，或在聚焦子树之外。
pub fn hidden_mask(blocks: &[Arc<Block>], focus: Option<usize>) -> Vec<bool> {
    let n = blocks.len();
    let mut hidden = vec![false; n];
    if let Some(root) = focus.filter(|r| *r < n) {
        let sub = subtree(blocks, root);
        for (i, h) in hidden.iter_mut().enumerate() {
            // 标题行保留：聚焦时它显示为面包屑
            *h = i != 0 && i != root && !sub.contains(&i);
        }
    }
    let mut i = 0;
    while i < n {
        if blocks[i].collapsed && !hidden[i] {
            let sub = subtree(blocks, i);
            for h in &mut hidden[sub.clone()] {
                *h = true;
            }
            i = sub.end.max(i + 1);
        } else {
            i += 1;
        }
    }
    hidden
}

impl Editor {
    /// 改折叠状态。阅读模式下同步改快照：折叠是视图状态，不能被「回滚只读修改」还原。
    fn set_collapsed(&mut self, ix: usize, v: bool) {
        self.block_mut(ix).collapsed = v;
        if let Some(b) = self.sess.ro_doc.as_mut().and_then(|d| d.get_mut(ix)) {
            Arc::make_mut(b).collapsed = v;
        }
    }

    /// 聚焦根所在的块序号。
    pub(super) fn focus_ix(&self) -> Option<usize> {
        let id = self.sess.focus_root?;
        self.content.blocks.iter().position(|b| b.id == id)
    }

    pub(super) fn hidden(&self) -> Rc<Vec<bool>> {
        // 块的增删改都会清掉缓存（见 mod.rs 的修改原语）；版本号只是双保险
        let key = (self.version, self.sess.focus_root);
        if let Some((k, m)) = self.caches.fold_cache.borrow().as_ref() {
            if *k == key {
                return m.clone();
            }
        }
        let m = Rc::new(hidden_mask(&self.content.blocks, self.focus_ix()));
        *self.caches.fold_cache.borrow_mut() = Some((key, m.clone()));
        m
    }

    pub fn is_hidden(&self, ix: usize) -> bool {
        self.hidden().get(ix).copied().unwrap_or(false)
    }

    /// 块是否有可折叠的子项。
    pub(super) fn has_children(&self, ix: usize) -> bool {
        !subtree(&self.content.blocks, ix).is_empty()
    }

    /// 展开包含 `ix` 的所有折叠祖先（光标要去那里）。聚焦外的块：退出聚焦。返回是否改动了文档。
    pub(super) fn reveal_block(&mut self, ix: usize) -> bool {
        if !self.is_hidden(ix) {
            return false;
        }
        if let Some(root) = self.focus_ix() {
            if ix != root && !subtree(&self.content.blocks, root).contains(&ix) {
                self.sess.focus_root = None;
            }
        }
        let mut changed = false;
        for a in (0..ix).rev() {
            if self.content.blocks[a].collapsed && subtree(&self.content.blocks, a).contains(&ix) {
                self.set_collapsed(a, false);
                changed = true;
            }
        }
        if changed {
            self.version += 1;
            self.dirty = true;
        }
        changed
    }

    /// 折叠 / 展开某块（没有子项时作用于最近的有子项的祖先）。
    pub fn toggle_fold_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        // 聚焦时只能折叠聚焦子树里的块（折叠聚焦项的祖先会把自己藏起来）
        let in_focus = |a: usize| self.focus_ix().is_none_or(|r| a == r || subtree(&self.content.blocks, r).contains(&a));
        let target = (0..=ix).rev().find(|&a| in_focus(a) && self.has_children(a) && (a == ix || subtree(&self.content.blocks, a).contains(&ix)));
        let Some(a) = target.filter(|a| *a > 0) else { return };
        let collapsed = !self.content.blocks[a].collapsed;
        self.set_collapsed(a, collapsed);
        // 光标在被收起的子项里：移到父项末尾
        if collapsed && subtree(&self.content.blocks, a).contains(&self.core.head.block) {
            let len = self.slot_len((a, 0));
            self.core.head = Pos::new(a, 0, len);
            self.core.anchor = self.core.head;
        }
        // 折叠不进撤销栈（视图状态），但要保存
        self.version += 1;
        self.dirty = true;
        self.schedule_save(cx);
        self.list.reset(self.content.blocks.len());
        cx.notify();
    }

    pub fn toggle_fold(&mut self, _: &ToggleFold, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.core.head.block;
        self.toggle_fold_at(ix, cx);
    }

    /// 聚焦到某一项（再次对同一项执行则退出）。
    pub fn focus_on(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(b) = self.content.blocks.get(ix).filter(|_| ix > 0) else { return };
        let id = b.id;
        if self.sess.focus_root == Some(id) {
            self.sess.focus_root = None;
        } else {
            self.sess.focus_root = Some(id);
            if b.collapsed {
                self.set_collapsed(ix, false);
                self.version += 1;
                self.dirty = true;
                self.schedule_save(cx);
            }
            if self.core.head.block != ix && !subtree(&self.content.blocks, ix).contains(&self.core.head.block) {
                let len = self.slot_len((ix, 0));
                self.core.head = Pos::new(ix, 0, len);
                self.core.anchor = self.core.head;
            }
        }
        self.list.reset(self.content.blocks.len());
        self.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        cx.notify();
    }

    pub fn focus_block(&mut self, _: &FocusBlock, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.focus_ix().filter(|r| *r == self.core.head.block).unwrap_or(self.core.head.block);
        self.focus_on(ix, cx);
    }

    pub fn exit_focus(&mut self, cx: &mut Context<Self>) -> bool {
        if self.sess.focus_root.take().is_none() {
            return false;
        }
        // 聚焦项可能在折叠的祖先里：退出后把光标所在处展开
        self.caches.fold_cache.get_mut().take();
        if self.reveal_block(self.core.head.block) {
            self.schedule_save(cx);
        }
        self.list.reset(self.content.blocks.len());
        self.reveal_head();
        cx.notify();
        true
    }

    /// 聚焦路径：从顶层到聚焦项的各级祖先 (块序号, 文字)。
    pub fn focus_path(&self) -> Vec<(usize, String)> {
        let Some(root) = self.focus_ix() else { return vec![] };
        let mut path = vec![];
        for a in 1..=root {
            if a == root || subtree(&self.content.blocks, a).contains(&root) {
                let b = &self.content.blocks[a];
                let s: String =
                    if b.kind.is_textual() { b.text.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(20).collect() } else { String::new() };
                path.push((a, if s.is_empty() { b.kind.label().to_string() } else { s }));
            }
        }
        path
    }

    /// 聚焦模式下标题行的位置显示面包屑：文档标题 › 祖先 › 当前项。
    pub(super) fn render_focus_crumbs(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> AnyElement {
        let title = self.content.title();
        let mut items: Vec<(Option<usize>, String)> = vec![(None, if title.trim().is_empty() { "未命名文档".into() } else { title })];
        items.extend(self.focus_path().into_iter().map(|(i, s)| (Some(i), s)));
        let n = items.len();
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(render::content_w() + render::GUTTER * 2.))
                    .px(px(render::GUTTER))
                    .pt(px(18.))
                    .pb(px(10.))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(13.))
                    .children(items.into_iter().enumerate().map(|(k, (ix, label))| {
                        let last = k + 1 == n;
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                div()
                                    .id(("crumb", k))
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_color(if last { t.text } else { t.text_muted })
                                    .when(!last, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                                    .child(label)
                                    .when(!last, |d| {
                                        d.on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                match ix {
                                                    Some(i) => this.focus_on(i, cx),
                                                    None => {
                                                        this.exit_focus(cx);
                                                    }
                                                }
                                            }),
                                        )
                                    }),
                            )
                            .when(!last, |d| d.child(crate::ui::icon("chevron-right", 12., t.text_muted)))
                    }))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("crumb-exit")
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .text_color(t.accent)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(crate::i18n::tr("退出聚焦 Esc"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.exit_focus(cx);
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// 跳过隐藏块：从 `ix` 起朝 `down` 方向找第一个可见块。
    pub(super) fn visible_from(&self, ix: usize, down: bool) -> Option<usize> {
        let h = self.hidden();
        if down {
            (ix..self.content.blocks.len()).find(|i| !h[*i])
        } else {
            (0..=ix.min(self.content.blocks.len().saturating_sub(1))).rev().find(|i| !h[*i])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{hidden_mask, subtree};
    use crate::model::{Block, BlockKind, RichText};
    use std::sync::Arc;

    fn li(t: &str, indent: u8) -> Arc<Block> {
        let mut b = Block::new(BlockKind::Bullet, RichText::plain(t));
        b.indent = indent;
        Arc::new(b)
    }

    fn h(t: &str, level: u8) -> Arc<Block> {
        Arc::new(Block::new(BlockKind::Heading { level }, RichText::plain(t)))
    }

    #[test]
    fn subtree_by_indent_and_heading() {
        let title = Arc::new(Block::new(BlockKind::Title, RichText::plain("t")));
        let v = vec![title, h("一", 1), li("a", 0), li("a1", 1), li("a2", 2), li("b", 0), h("二", 2), li("c", 0), h("三", 1)];
        assert_eq!(subtree(&v, 2), 3..5);
        assert_eq!(subtree(&v, 3), 4..5);
        assert_eq!(subtree(&v, 5), 6..6);
        assert_eq!(subtree(&v, 1), 2..8, "一级标题收到下一个一级标题为止，二级标题算它的子项");
        assert_eq!(subtree(&v, 6), 7..8);
        assert_eq!(subtree(&v, 0), 1..1);
    }

    #[test]
    fn mask_folds_and_focus() {
        let title = Arc::new(Block::new(BlockKind::Title, RichText::plain("t")));
        let mut a = (*li("a", 0)).clone();
        a.collapsed = true;
        let v = vec![title, Arc::new(a), li("a1", 1), li("a2", 2), li("b", 0), li("b1", 1)];
        assert_eq!(hidden_mask(&v, None), vec![false, false, true, true, false, false]);
        // 聚焦 b：只显示 b 和它的子项
        assert_eq!(hidden_mask(&v, Some(4)), vec![false, true, true, true, false, false]);
    }
}
