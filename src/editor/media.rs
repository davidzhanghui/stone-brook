//! 图片块操作与评论：缩放、裁剪、重置裁剪、对齐、描述、评论。

use super::render::content_w;
use super::*;
use crate::model::BlockKind;
use crate::store::Comment;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use std::collections::HashMap;

/// 裁剪框最小边长（占原图比例），低于这个就不再缩
const MIN_CROP: f32 = 0.06;

impl Editor {
    // ───────────── 图片 ─────────────

    /// 图片块的原图尺寸。
    pub fn image_natural(&self, ix: usize) -> Option<(f32, f32)> {
        let BlockKind::Image { path, .. } = &self.content.blocks.get(ix)?.kind else { return None };
        self.image_size(path).map(|(w, h)| (w.max(1) as f32, h.max(1) as f32))
    }

    /// 图片当前显示框 (宽, 高)。
    pub fn image_display_box(&self, ix: usize) -> Option<(f32, f32)> {
        let blk = self.content.blocks.get(ix)?;
        if !matches!(blk.kind, BlockKind::Image { .. }) {
            return None;
        }
        let natural = self.image_natural(ix)?;
        blk.kind.image_box((natural.0 as u32, natural.1 as u32), self.image_max_w(ix))
    }

    /// 图片可用的最大宽度：正文实际宽度（窗口变窄时小于 content_w）减去缩进。
    pub fn image_max_w(&self, ix: usize) -> f32 {
        let indent = self.content.blocks.get(ix).map_or(0, |b| b.indent) as f32;
        (self.avail_w.get().min(content_w()) - indent * super::render::indent_w()).max(80.)
    }

    // ───────────── 裁剪模式 ─────────────

    /// 进入裁剪模式：从当前裁剪框（没有则整图）开始。
    pub fn start_crop(&mut self, ix: usize, cx: &mut Context<Self>) {
        let rect = match &self.content.blocks[ix].kind {
            BlockKind::Image { crop, .. } => crop.unwrap_or((0., 0., 1., 1.)),
            _ => return,
        };
        self.sess.image_drag = None;
        self.sess.image_crop = Some(CropEdit { block: ix, rect, drag: None });
        self.core.anchor = Pos::new(ix, 0, 0);
        self.core.head = self.core.anchor;
        cx.notify();
    }

    pub fn crop_begin_drag(&mut self, handle: CropHandle, at: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(c) = self.sess.image_crop.as_mut() {
            c.drag = Some(CropDrag { handle, start: at, rect0: c.rect });
            cx.notify();
        }
    }

    /// 拖动裁剪框：按住的部位决定移动哪条边；整体移动时保持大小。
    pub fn crop_drag_move(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(c) = self.sess.image_crop.as_ref() else { return };
        let Some(d) = c.drag.as_ref() else { return };
        let Some(b) = self.frame.borrow().images.get(&c.block).copied() else { return };
        let (bw, bh) = (f32::from(b.size.width).max(1.), f32::from(b.size.height).max(1.));
        let (dx, dy) = (f32::from(at.x - d.start.x) / bw, f32::from(at.y - d.start.y) / bh);
        let rect = crop_drag_rect(d.handle, d.rect0, dx, dy);
        if let Some(c) = self.sess.image_crop.as_mut() {
            c.rect = rect;
        }
        cx.notify();
    }

    /// 完成裁剪：框等于整图时视为「不裁剪」。
    pub fn crop_commit(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.sess.image_crop.take() else { return };
        let full = c.rect.0 < 0.002 && c.rect.1 < 0.002 && c.rect.2 > 0.998 && c.rect.3 > 0.998;
        let new = (!full).then_some(c.rect);
        let old = match &self.content.blocks.get(c.block).map(|b| &b.kind) {
            Some(BlockKind::Image { crop, .. }) => *crop,
            _ => return cx.notify(),
        };
        if old != new {
            self.set_image_crop(c.block, new, cx);
        } else {
            cx.notify();
        }
    }

    pub fn crop_cancel(&mut self, cx: &mut Context<Self>) {
        if self.sess.image_crop.take().is_some() {
            cx.notify();
        }
    }

    /// 裁剪框还原为整图（仍在裁剪模式里，点「完成」才生效）。
    pub fn crop_reset_full(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.sess.image_crop.as_mut() {
            c.rect = (0., 0., 1., 1.);
            cx.notify();
        }
    }

    // ───────────── 查看大图 ─────────────

    pub fn open_image_preview(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(BlockKind::Image { path, .. }) = self.content.blocks.get(ix).map(|b| &b.kind) {
            self.sess.image_preview = Some(path.clone());
            cx.notify();
        }
    }

    /// 把图片宽度设为 `w`（等比，高度自动）。w 为 None 表示恢复自适应。
    pub fn set_image_width(&mut self, ix: usize, w: Option<f32>, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Image { width, .. } | BlockKind::Video { width, .. } = &mut self.block_mut(ix).kind {
            *width = w.map(|v| v.clamp(48., 2400.));
        }
        self.changed(cx);
    }

    /// 设置裁剪框（比例坐标）。
    pub fn set_image_crop(&mut self, ix: usize, crop: Option<(f32, f32, f32, f32)>, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        if let BlockKind::Image { crop: c, .. } = &mut self.block_mut(ix).kind {
            *c = crop.map(|(x, y, w, h)| {
                let x = x.clamp(0., 1. - MIN_CROP);
                let y = y.clamp(0., 1. - MIN_CROP);
                (x, y, w.clamp(MIN_CROP, 1. - x), h.clamp(MIN_CROP, 1. - y))
            });
        }
        self.changed(cx);
    }

    pub fn reset_image_crop(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.set_image_crop(ix, None, cx);
        cx.emit(EditorEvent::Toast("已撤销裁剪".into()));
    }

    /// 设置块对齐：0 左、1 居中、2 右。
    pub fn set_align(&mut self, ix: usize, align: u8, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        self.block_mut(ix).align = align.min(2);
        self.changed(cx);
    }

    /// 打开「图片描述」编辑框。
    pub fn edit_image_alt(&mut self, ix: usize, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let cur = match &self.content.blocks[ix].kind {
            BlockKind::Image { alt, .. } => alt.clone(),
            _ => return,
        };
        self.open_prop_edit(ix, "image_alt", &cur, "输入图片描述（替代文本）", at, window, cx);
    }

    // ───────────── 评论 ─────────────

    /// 重新从库里读评论，并按序号映射到当前块的 id。读之前先把编辑中挪动过的位置写回去。
    pub fn refresh_comments(&mut self) {
        self.sync_comment_blocks();
        self.comments = self.store.comments(&self.doc_id);
        for c in &mut self.comments {
            c.block_id = self.content.blocks.get(c.block).map_or(0, |b| b.id);
        }
    }

    /// 把评论的块序号更新为它所跟随的块的当前位置（保存时调用）。块被删掉时保持原序号，撤销后还能找回。
    pub fn sync_comment_blocks(&mut self) {
        if self.comments.is_empty() {
            return;
        }
        let pos: HashMap<u64, usize> = self.content.blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        for c in &mut self.comments {
            if let Some(&ix) = pos.get(&c.block_id) {
                if ix != c.block {
                    self.store.set_comment_block(&c.id, ix);
                    c.block = ix;
                }
            }
        }
    }

    /// 评论当前所在的块；原块已被删除时为 None。
    pub fn comment_block_ix(&self, c: &Comment) -> Option<usize> {
        self.content.blocks.iter().position(|b| b.id == c.block_id)
    }

    /// 某块上的评论数。
    pub fn comments_on(&self, ix: usize) -> usize {
        let Some(id) = self.content.blocks.get(ix).map(|b| b.id) else { return 0 };
        self.comments.iter().filter(|c| c.block_id == id).count()
    }

    /// 某块上未解决的评论数（右侧气泡用）。
    pub fn open_comments_on(&self, ix: usize) -> usize {
        let Some(id) = self.content.blocks.get(ix).map(|b| b.id) else { return 0 };
        self.comments.iter().filter(|c| c.block_id == id && !c.resolved).count()
    }

    pub fn comments_of(&self, ix: usize) -> Vec<Comment> {
        let Some(id) = self.content.blocks.get(ix).map(|b| b.id) else { return vec![] };
        self.comments.iter().filter(|c| c.block_id == id).cloned().collect()
    }

    /// 给正在编辑的块（或选区所在块）加评论。锚点存选中的文字快照。
    pub fn start_comment(&mut self, ix: usize, _at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let ix = ix.min(self.content.blocks.len().saturating_sub(1));
        let anchor = {
            let (s, e) = self.sel();
            if s.block == ix && s.slot() == e.slot() && s.offset != e.offset {
                self.slot_text(s.slot()).map(|t| t.text[s.offset.min(t.len())..e.offset.min(t.len())].to_string()).unwrap_or_default()
            } else {
                let t = self.content.blocks[ix].plain_text();
                t.chars().take(40).collect()
            }
        };
        let input = cx.new(|cx| LineInput::new("写下评论，回车提交", cx));
        let anchor_for_sub = anchor.clone();
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => {
                let body = input.read(cx).text().trim().to_string();
                if !body.is_empty() {
                    this.add_comment(ix, &anchor_for_sub, &body, cx);
                }
                this.sess.comment_input = None;
                this.sess.comment_open = Some(ix);
                window.focus(&this.focus);
                cx.notify();
            }
            InputEvent::Cancel => {
                this.sess.comment_input = None;
                if this.comments_on(ix) == 0 {
                    this.sess.comment_open = None;
                }
                window.focus(&this.focus);
                cx.notify();
            }
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.sess.comment_open = Some(ix);
        self.sess.comment_input = Some(CommentInput { block: ix, anchor, input, _sub: sub });
        cx.notify();
    }

    fn add_comment(&mut self, ix: usize, anchor: &str, body: &str, cx: &mut Context<Self>) {
        let author = crate::settings::author();
        // 先把其它评论的位置写回，新评论用当前序号入库，再统一重读
        self.sync_comment_blocks();
        self.store.add_comment(&self.doc_id, ix, anchor, body, &author);
        self.refresh_comments();
        self.sess.comment_open = Some(ix);
        // 评论单独存表，正文没变：只提示，不触发文档变更 / 保存
        cx.emit(EditorEvent::Toast("已添加评论".into()));
        cx.notify();
    }

    pub fn toggle_comment_open(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.sess.comment_open == Some(ix) {
            self.close_comment_panel(cx);
        } else {
            self.open_comment_panel(ix, cx);
        }
    }

    /// 打开某块的评论列表。重置悬停状态，避免列表刚出现就被鼠标移出逻辑收掉。
    pub fn open_comment_panel(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.sess.comment_open = Some(ix);
        self.refresh_comments();
        cx.notify();
    }

    pub fn close_comment_panel(&mut self, cx: &mut Context<Self>) {
        self.sess.comment_open = None;
        cx.notify();
    }

    /// 关闭所有评论浮层（输入框 + 列表）。浮层只在点击浮层外部时关闭。
    pub(crate) fn dismiss_comments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sess.comment_input.is_none() && self.sess.comment_open.is_none() {
            return;
        }
        // 只有焦点还在浮层输入框上时才收回，避免抢走用户正在点击的其它控件焦点
        let input_focused = self.sess.comment_input.as_ref().map(|ci| ci.input.read(cx).focus_handle(cx).is_focused(window)).unwrap_or(false);
        self.sess.comment_input = None;
        self.sess.comment_open = None;
        if input_focused {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    pub fn resolve_comment(&mut self, id: &str, resolved: bool, cx: &mut Context<Self>) {
        self.store.set_comment_resolved(id, resolved);
        self.refresh_comments();
        cx.notify();
    }

    pub fn remove_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.store.delete_comment(id);
        self.refresh_comments();
        cx.notify();
    }

    /// 跳到某个块并把光标放上去（评论侧栏点击用）。
    pub fn jump_to_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        let ix = ix.min(self.content.blocks.len().saturating_sub(1));
        self.core.anchor = Pos::new(ix, 0, 0);
        self.core.head = self.core.anchor;
        self.sess.comment_open = Some(ix);
        self.refresh_comments();
        self.scroll_to_block(ix, cx);
        cx.notify();
    }

    /// 自测脚本用：直接在某块上加一条评论。
    #[cfg(feature = "dev")]
    pub fn test_add_comment(&mut self, ix: usize, body: &str, cx: &mut Context<Self>) {
        let ix = ix.min(self.content.blocks.len().saturating_sub(1));
        let anchor = self.content.blocks[ix].plain_text().chars().take(40).collect::<String>();
        self.add_comment(ix, &anchor, body, cx);
    }

    /// 自测脚本用：打开某块的评论输入框。
    #[cfg(feature = "dev")]
    pub fn test_open_comment_input(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let ix = ix.min(self.content.blocks.len().saturating_sub(1));
        self.start_comment(ix, point(px(320.), px(160.)), window, cx);
    }

    /// 自测脚本用：把输入法事件直接送进评论输入框。
    /// `sub` 为 `ime`（组字）/ `commit`（上屏）/ `raw`（指定 utf16 range 的 marked text）。
    #[cfg(feature = "dev")]
    pub fn test_comment_ime(&mut self, sub: &str, rest: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ci) = self.sess.comment_input.as_ref() else {
            eprintln!("cinput: 评论输入框没打开");
            return;
        };
        let input = ci.input.clone();
        match sub {
            "ime" => {
                input.update(cx, |i, cx| i.replace_and_mark_text_in_range(None, rest, None, window, cx));
            }
            "commit" => {
                input.update(cx, |i, cx| i.replace_text_in_range(None, rest, window, cx));
            }
            "raw" => {
                // raw <start> <end> <文字>
                let mut it = rest.splitn(3, ' ');
                let s = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                let e = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                let text = it.next().unwrap_or("").to_string();
                input.update(cx, |i, cx| i.replace_and_mark_text_in_range(Some(s..e), &text, None, window, cx));
            }
            _ => {}
        }
    }

    // ───────────── 渲染辅助 ─────────────

    /// 图片下方的描述：有描述时始终显示；没有时只在选中图片时给出「添加图片描述」入口。
    pub(super) fn render_image_caption(&self, ix: usize, path: &str, selected: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let alt = match &self.content.blocks.get(ix).map(|b| &b.kind) {
            Some(BlockKind::Image { alt, .. }) => alt.clone(),
            _ => String::new(),
        };
        let empty = alt.trim().is_empty();
        // 网络图片 / 缺失文件时给一句说明，别让用户以为图挂了
        let missing = if path.starts_with("http") {
            if crate::settings::network() {
                None
            } else {
                Some("网络图片（已关闭联网，未下载）")
            }
        } else if self.image_size(path).is_none() {
            Some("图片文件不存在")
        } else {
            None
        };
        let text = match (missing, empty) {
            (Some(m), _) => m.to_string(),
            (None, true) if selected => "添加图片描述".to_string(),
            (None, true) => return div().into_any_element(),
            (None, false) => alt,
        };
        div()
            .id(("img-cap", ix))
            .w_full()
            .pt(px(6.))
            .text_size(px(12.5))
            .text_color(if empty { t.placeholder } else { t.text_muted })
            .text_align(gpui::TextAlign::Center)
            .when(selected && missing.is_none(), |d| {
                d.cursor(CursorStyle::IBeam).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.edit_image_alt(ix, ev.position, window, cx);
                    }),
                )
            })
            .child(text)
            .into_any_element()
    }

    /// 查看大图：盖住编辑区的暗色浮层，图片按窗口等比放大，点击任意处或 Esc 关闭。
    pub(super) fn render_image_preview(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let path = self.sess.image_preview.clone()?;
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width) - 120., f32::from(vp.height) - 120.);
        let (nw, nh) = self.image_size(&path).map(|(w, h)| (w.max(1) as f32, h.max(1) as f32)).unwrap_or((800., 600.));
        let k = (vw / nw).min(vh / nh).min(2.);
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id("img-preview")
                        .w(vp.width)
                        .h(vp.height)
                        .occlude()
                        .bg(gpui::black().opacity(0.86))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .cursor_pointer()
                        .child(img(std::path::PathBuf::from(path.clone())).w(px(nw * k)).h(px(nh * k)).rounded(px(4.)))
                        .child(
                            div().text_size(px(12.)).text_color(gpui::white().opacity(0.7)).child(format!("{:.0} × {:.0} · 点击任意处或按 Esc 关闭", nw, nh)),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.sess.image_preview = None;
                                cx.notify();
                            }),
                        ),
                ),
            )
            .with_priority(9)
            .into_any_element(),
        )
    }
}

/// 按拖动的部位与位移（原图比例）算出新的裁剪框。整体移动保持大小；拖边角时不越界、不小于最小尺寸。
pub(crate) fn crop_drag_rect(h: CropHandle, r0: (f32, f32, f32, f32), dx: f32, dy: f32) -> (f32, f32, f32, f32) {
    use CropHandle as H;
    let (x, y, w, hh) = r0;
    if h == H::Move {
        return ((x + dx).clamp(0., 1. - w), (y + dy).clamp(0., 1. - hh), w, hh);
    }
    let (mut l, mut t, mut r, mut b) = (x, y, x + w, y + hh);
    if matches!(h, H::W | H::NW | H::SW) {
        l = (l + dx).clamp(0., r - MIN_CROP);
    }
    if matches!(h, H::E | H::NE | H::SE) {
        r = (r + dx).clamp(l + MIN_CROP, 1.);
    }
    if matches!(h, H::N | H::NW | H::NE) {
        t = (t + dy).clamp(0., b - MIN_CROP);
    }
    if matches!(h, H::S | H::SW | H::SE) {
        b = (b + dy).clamp(t + MIN_CROP, 1.);
    }
    (l, t, r - l, b - t)
}

/// 时间戳 → 「今天 14:03」这种短格式。
pub fn fmt_comment_time(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(dt) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let now = Local::now();
    if dt.date_naive() == now.date_naive() {
        format!("今天 {}", dt.format("%H:%M"))
    } else if dt.year() == now.year() {
        dt.format("%m-%d %H:%M").to_string()
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

use chrono::Datelike;

// ───────────── 评论浮层 ─────────────

impl Editor {
    /// 鼠标在评论浮层外按下：关闭浮层。落在任一评论浮层内都不关。
    pub(crate) fn comment_outside(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let inside = {
            let f = self.frame.borrow();
            f.comment_input_bounds.is_some_and(|b| b.contains(&pos)) || f.comment_panel_bounds.is_some_and(|b| b.contains(&pos))
        };
        if !inside {
            self.dismiss_comments(window, cx);
        }
    }

    /// 评论浮层：贴在被评论块的右侧边距，放不下时落到块下方；
    /// 上面是该块的评论列表，底部是回复框（点一下就地输入，↵ 发送、Esc 收起）。
    pub(super) fn render_comment_popover(&self, anchor: Option<Bounds<Pixels>>, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        const W: f32 = 320.;
        let ix = self.sess.comment_open.or(self.sess.comment_input.as_ref().map(|c| c.block))?;
        let list = self.comments_of(ix);
        let input = self.sess.comment_input.as_ref().filter(|c| c.block == ix);
        if list.is_empty() && input.is_none() {
            return None;
        }
        // 被评论的块滚出视野：卡片跟着隐藏（正在输入时保留，免得打字打到一半消失）
        if anchor.is_none() && input.is_none() {
            return None;
        }
        let vp = self.list.viewport_bounds();
        let at = match anchor {
            // 右侧边距放得下：与块顶部对齐
            Some(b) if vp.right() - b.right() >= px(W + 12.) => point(b.right() + px(4.), b.top()),
            // 放不下：落在块下方，右缘与正文对齐
            Some(b) => point((b.right() - px(64. + W)).max(b.left() + px(64.)), b.bottom() + px(4.)),
            None => point(vp.right() - px(W + 16.), vp.top() + px(16.)),
        };
        let open_count = list.iter().filter(|c| !c.resolved).count();
        let quote = input.map(|c| c.anchor.clone()).filter(|a| !a.is_empty()).or_else(|| list.first().map(|c| c.anchor.clone()).filter(|a| !a.is_empty()));
        let frame = self.frame.clone();
        let small_btn = |id: SharedString, label: &'static str, color: Hsla| {
            div().id(id).px(px(5.)).py(px(1.)).rounded(px(4.)).cursor_pointer().text_color(color).hover(|s| s.bg(t.panel_bg)).child(label)
        };
        let card = div()
            .id("comment-panel")
            .relative()
            .occlude()
            .w(px(W))
            .max_h(px(440.))
            .overflow_y_scroll()
            .rounded(px(10.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .p(px(10.))
            .flex()
            .flex_col()
            .gap_2()
            // 记录浮层区域，供「点外部关闭」判断
            .child(
                canvas(
                    move |bounds, _, _| {
                        frame.borrow_mut().comment_panel_bounds = Some(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.comment_outside(ev.position, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(icon("comment", 13., t.text_muted))
                    .child(div().flex_1().text_size(px(12.)).text_color(t.text_muted).child(if list.is_empty() {
                        "添加评论".to_string()
                    } else {
                        format!("{} 条评论{}", list.len(), if open_count == 0 { " · 已全部解决" } else { "" })
                    }))
                    .child(
                        div()
                            .id("cm-close")
                            .size(px(20.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("x", 13., t.text_muted))
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss_comments(window, cx))),
                    ),
            )
            .when_some(quote, |d, q| {
                d.child(
                    div()
                        .pl(px(8.))
                        .border_l_2()
                        .border_color(gpui::hsla(0.12, 0.9, 0.5, 0.8))
                        .text_size(px(12.))
                        .text_color(t.text_secondary)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(q.chars().take(60).collect::<String>()),
                )
            })
            .children(list.into_iter().map(|c| {
                let id = c.id.clone();
                let id2 = c.id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.))
                    .bg(t.hover)
                    .when(c.resolved, |d| d.opacity(0.6))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(t.text_muted)
                            .child(
                                div()
                                    .size(px(18.))
                                    .rounded_full()
                                    .bg(t.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(10.))
                                    .text_color(gpui::white())
                                    .child(c.author.chars().next().map(String::from).unwrap_or_default()),
                            )
                            .child(div().flex_1().child(format!("{} · {}", c.author, fmt_comment_time(c.created))))
                            .child(
                                small_btn(
                                    SharedString::from(format!("cm-r-{id}")),
                                    if c.resolved { "重新打开" } else { "解决" },
                                    if c.resolved { t.text_muted } else { t.accent },
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let cur = this.comments.iter().find(|x| x.id == id).map(|x| x.resolved).unwrap_or(false);
                                    this.resolve_comment(&id, !cur, cx);
                                })),
                            )
                            .child(small_btn(SharedString::from(format!("cm-d-{id2}")), "删除", t.text_muted).on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_comment(&id2, cx);
                            }))),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(if c.resolved { t.text_muted } else { t.text })
                            .when(c.resolved, |d| d.line_through())
                            .child(c.body.clone()),
                    )
            }))
            .child(match input {
                Some(ci) => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div().rounded(px(6.)).border_1().border_color(t.accent).px_2().py_1().text_size(px(13.)).line_height(px(22.)).child(ci.input.clone()),
                    )
                    .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("↵ 发送 · Esc 取消")))
                    .into_any_element(),
                None => div()
                    .id("cm-reply")
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border)
                    .px_2()
                    .py(px(5.))
                    .text_size(px(13.))
                    .text_color(t.placeholder)
                    .cursor(CursorStyle::IBeam)
                    .hover(|s| s.border_color(t.accent))
                    .child(crate::i18n::tr("回复…"))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.start_comment(ix, ev.position(), window, cx);
                    }))
                    .into_any_element(),
            });
        Some(deferred(anchored().position(at).snap_to_window_with_margin(px(10.)).child(card)).with_priority(5).into_any_element())
    }
}

#[cfg(test)]
mod crop_tests {
    use super::{crop_drag_rect, CropHandle, MIN_CROP};

    #[test]
    fn move_keeps_size_and_stays_inside() {
        let r = crop_drag_rect(CropHandle::Move, (0.2, 0.2, 0.5, 0.5), 0.9, -0.9);
        assert_eq!(r, (0.5, 0., 0.5, 0.5));
    }

    #[test]
    fn corner_drag_clamps_to_min_size() {
        let r = crop_drag_rect(CropHandle::SE, (0.1, 0.1, 0.5, 0.5), -2., -2.);
        assert!((r.2 - MIN_CROP).abs() < 1e-6 && (r.3 - MIN_CROP).abs() < 1e-6);
        let r = crop_drag_rect(CropHandle::NW, (0.1, 0.1, 0.5, 0.5), -1., -1.);
        assert_eq!((r.0, r.1), (0., 0.));
        assert!((r.2 - 0.6).abs() < 1e-6);
    }
}
