//! 同步块（P6-3）：同一段内容出现在多篇文档里，任何一处修改都会同步到其他地方。
//! 文档里连续、`sync` 相同的块构成一个同步组；组内容另存在 sync_blocks 表中，
//! 保存文档时写回，打开文档时用最新版本替换。

use super::*;
use crate::model::{blocks_from_json, Block, BlockKind};
use crate::theme::Theme;
use crate::ui::icon;
use std::collections::HashMap;

/// 文档中的同步组：(同步 id, 块范围)。
pub fn sync_groups(blocks: &[Arc<Block>]) -> Vec<(String, Range<usize>)> {
    let mut out: Vec<(String, Range<usize>)> = vec![];
    for (i, b) in blocks.iter().enumerate() {
        let Some(id) = &b.sync else { continue };
        match out.last_mut() {
            Some((last, r)) if last == id && r.end == i => r.end = i + 1,
            _ => out.push((id.clone(), i..i + 1)),
        }
    }
    out
}

/// 组内容序列化（不含块 id，便于比较是否变化）。
fn group_json(blocks: &[Arc<Block>], r: &Range<usize>) -> String {
    // 折叠是各处自己的视图状态，不跟着同步
    let v: Vec<Block> = blocks[r.clone()]
        .iter()
        .map(|b| {
            let mut b = (**b).clone();
            b.collapsed = false;
            b
        })
        .collect();
    // 落盘用：序列化失败返回空串，`Store::save_sync` 会拒绝写入并上报
    crate::model::try_blocks_to_json(&v).unwrap_or_default()
}

impl Editor {
    /// 打开文档时：用同步表里的最新内容替换各同步组。
    pub(super) fn pull_sync(&mut self) {
        let groups = sync_groups(&self.content.blocks);
        if groups.is_empty() {
            self.sync_saved.clear();
            return;
        }
        let mut saved = HashMap::new();
        // 倒序替换，前面的范围不受影响
        for (id, r) in groups.into_iter().rev() {
            let cur = group_json(&self.content.blocks, &r);
            match self.store.load_sync(&id) {
                Some((json, _)) if json != cur => {
                    if let Some(mut fresh) = blocks_from_json(&json) {
                        for b in &mut fresh {
                            b.sync = Some(id.clone());
                        }
                        if !fresh.is_empty() {
                            self.content.blocks.splice(r, fresh.into_iter().map(Arc::new));
                        }
                    }
                    saved.insert(id, json);
                }
                Some(_) => {
                    saved.insert(id, cur);
                }
                None => {
                    // 同步表里没有（源已删除）：保留内容，下次保存时重新登记
                }
            }
        }
        self.sync_saved = saved;
    }

    /// 保存文档时：把有变化的同步组写回同步表。
    pub(super) fn push_sync(&mut self) {
        for (id, r) in sync_groups(&self.content.blocks) {
            let json = group_json(&self.content.blocks, &r);
            if self.sync_saved.get(&id) != Some(&json) {
                self.store.save_sync(&id, &json, &self.doc_id);
                self.sync_saved.insert(id, json);
            }
        }
    }

    /// 把光标所在块（或选中的多个块）设为新的同步块。
    pub fn make_sync(&mut self, cx: &mut Context<Self>) {
        self.pop.sync_menu = false;
        let (s, e) = self.sel();
        let range = s.block.max(1)..e.block.max(1) + 1;
        if range.start >= self.content.blocks.len() {
            return;
        }
        if self.content.blocks[range.clone()].iter().any(|b| b.sync.is_some()) {
            cx.emit(EditorEvent::Toast("所选内容已经是同步块".into()));
            return;
        }
        self.checkpoint(EditKind::Other);
        let id = crate::store::new_uuid();
        for i in range.clone() {
            self.block_mut(i).sync = Some(id.clone());
        }
        let json = group_json(&self.content.blocks, &range);
        self.store.save_sync(&id, &json, &self.doc_id);
        self.sync_saved.insert(id, json);
        self.changed(cx);
        cx.emit(EditorEvent::Toast("已创建同步块：用「/同步块」可以把它插入到其他文档".into()));
    }

    /// 在光标后插入一个已有的同步块。
    pub fn insert_sync(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pop.sync_menu = false;
        let Some((json, _)) = self.store.load_sync(id) else { return };
        let Some(mut blocks) = blocks_from_json(&json) else { return };
        for b in &mut blocks {
            b.sync = Some(id.to_string());
        }
        // 光标在空段落上时直接替换它
        let h = self.core.head.block;
        let replace_empty = h > 0 && matches!(self.content.blocks[h].kind, BlockKind::Text) && self.content.blocks[h].text.is_empty();
        if replace_empty {
            self.checkpoint(EditKind::Other);
            self.remove_blocks(h..h + 1);
            let n = blocks.len();
            self.insert_blocks(h, blocks);
            self.set_cursor(Pos::new(h + n - 1, 0, 0), false, cx);
            self.changed(cx);
        } else {
            self.insert_blocks_after_cursor(blocks, cx);
        }
        self.sync_saved.insert(id.to_string(), json);
    }

    /// 取消同步：该组在本文档中变成普通块（其他文档不受影响）。
    pub fn unsync_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(id) = self.content.blocks.get(ix).and_then(|b| b.sync.clone()) else { return };
        self.checkpoint(EditKind::Other);
        for (gid, r) in sync_groups(&self.content.blocks) {
            if gid == id && r.contains(&ix) {
                for i in r {
                    self.block_mut(i).sync = None;
                }
            }
        }
        self.sync_saved.remove(&id);
        self.pop.block_menu = None;
        self.changed(cx);
    }

    pub(super) fn render_sync_menu(&self, at: Point<Pixels>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.store.list_sync();
        let titles: HashMap<String, String> = self.store.list().into_iter().map(|d| (d.id, d.title)).collect();
        let row = |id: ElementId| div().id(id).px_3().py(px(7.)).rounded(px(6.)).cursor_pointer().hover(|s| s.bg(t.hover));
        deferred(
            anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("sync-menu")
                    .occlude()
                    .w(px(340.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .text_size(px(13.))
                    .child(
                        row("sync-new".into())
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(icon("link", 15., t.accent))
                            .child(div().flex_1().child(crate::i18n::tr("把当前块设为同步块")))
                            .on_click(cx.listener(|this, _, _, cx| this.make_sync(cx))),
                    )
                    .child(div().px_3().pt_2().pb_1().text_size(px(11.5)).text_color(t.text_muted).child(if list.is_empty() {
                        "还没有同步块"
                    } else {
                        "插入已有的同步块"
                    }))
                    .children(list.into_iter().enumerate().map(|(i, (id, src, preview, _))| {
                        let from = titles.get(&src).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "未命名文档".into());
                        row(("sync-it", i).into())
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_color(t.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(if preview.trim().is_empty() {
                                "（空）".to_string()
                            } else {
                                preview
                            }))
                            .child(div().text_size(px(11.)).text_color(t.text_muted).child(format!("来自《{from}》")))
                            .on_click(cx.listener(move |this, _, _, cx| this.insert_sync(&id, cx)))
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.pop.sync_menu = false;
                        cx.notify();
                    })),
            ),
        )
        .with_priority(3)
    }
}
