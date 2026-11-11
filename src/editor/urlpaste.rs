//! 粘贴网址（D8）：先作为链接插入，并在旁边给出「链接 / 标题 / 卡片」三种展示方式。

use super::*;
use crate::linkcard::CardState;
use crate::model::{Block, BlockKind, Marks, RichText};
use crate::theme::Theme;

pub struct UrlPaste {
    pub slot: (usize, usize),
    pub start: usize,
    pub len: usize,
    pub url: String,
    /// 选了「标题」，正在等待抓取
    pub waiting: bool,
}

pub fn is_url(t: &str) -> bool {
    (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace) && t.len() > 10
}

impl Editor {
    /// 在光标处插入网址链接，返回 true 表示已处理。
    pub(super) fn paste_url(&mut self, url: &str, cx: &mut Context<Self>) -> bool {
        let h = self.core.head;
        if self.has_selection() || self.content.blocks[h.block].kind.is_void() || self.content.blocks[h.block].kind.is_source() || h.block == 0 {
            return false;
        }
        self.checkpoint(EditKind::Other);
        let marks = Marks { link: Some(url.to_string()), ..Default::default() };
        self.slot_mut(h.slot()).insert(h.offset, url, marks);
        self.sess.url_paste = Some(UrlPaste { slot: h.slot(), start: h.offset, len: url.len(), url: url.to_string(), waiting: false });
        self.core.head.offset += url.len();
        self.core.anchor = self.core.head;
        self.changed(cx);
        true
    }

    /// 链接文字仍在原处时返回其范围。
    fn url_paste_range(&self) -> Option<std::ops::Range<usize>> {
        let up = self.sess.url_paste.as_ref()?;
        let t = self.slot_text(up.slot)?;
        let r = up.start..up.start + up.len;
        (r.end <= t.len() && t.text.is_char_boundary(r.start) && t.text.is_char_boundary(r.end)).then_some(r)
    }

    pub fn url_paste_title(&mut self, cx: &mut Context<Self>) {
        let Some(up) = self.sess.url_paste.as_mut() else { return };
        let url = up.url.clone();
        match crate::linkcard::request(&url, cx) {
            CardState::Ready(c) if !c.title.trim().is_empty() => self.replace_url_text(&c.title, cx),
            CardState::Loading => {
                if let Some(up) = self.sess.url_paste.as_mut() {
                    up.waiting = true;
                }
                cx.notify();
            }
            _ => {
                cx.emit(EditorEvent::Toast(if crate::settings::network() {
                    "没能获取网页标题".into()
                } else {
                    "已关闭联网，无法获取标题（可在设置中开启）".into()
                }));
                self.sess.url_paste = None;
            }
        }
    }

    fn replace_url_text(&mut self, title: &str, cx: &mut Context<Self>) {
        let Some(r) = self.url_paste_range() else {
            self.sess.url_paste = None;
            return;
        };
        let Some(up) = self.sess.url_paste.take() else { return };
        self.checkpoint(EditKind::Other);
        let title = title.trim().replace('\n', " ");
        let slot = self.slot_mut(up.slot);
        slot.delete(r.clone());
        slot.insert(r.start, &title, Marks { link: Some(up.url.clone()), ..Default::default() });
        if self.core.head.slot() == up.slot && self.core.head.offset >= r.end {
            self.core.head.offset = self.core.head.offset - r.len() + title.len();
            self.core.anchor = self.core.head;
        }
        self.changed(cx);
    }

    pub fn url_paste_card(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.url_paste_range() else {
            self.sess.url_paste = None;
            return;
        };
        let Some(up) = self.sess.url_paste.take() else { return };
        self.checkpoint(EditKind::Other);
        let bi = up.slot.0;
        let card = Block::new(BlockKind::LinkCard { url: up.url.clone() }, RichText::default());
        let whole = up.slot.1 == 0
            && self.content.blocks[bi].text.text.trim() == self.content.blocks[bi].text.text[r.clone()].trim()
            && matches!(self.content.blocks[bi].kind, BlockKind::Text);
        if whole {
            // 段落里只有这个网址：整块变成卡片
            *self.block_mut(bi) = card;
            self.set_cursor(Pos::new(bi, 0, 0), false, cx);
        } else {
            self.slot_mut(up.slot).delete(r);
            self.insert_blocks(bi + 1, vec![card]);
            self.set_cursor(Pos::new(bi + 1, 0, 0), false, cx);
        }
        self.changed(cx);
    }

    /// 每帧检查：光标离开或抓取完成。
    pub(super) fn poll_url_paste(&mut self, cx: &mut Context<Self>) {
        let Some(up) = &self.sess.url_paste else { return };
        if self.url_paste_range().is_none() || self.core.head.slot() != up.slot {
            self.sess.url_paste = None;
            return;
        }
        if up.waiting {
            match crate::linkcard::request(&up.url.clone(), cx) {
                CardState::Ready(c) if !c.title.trim().is_empty() => self.replace_url_text(&c.title, cx),
                CardState::Loading => {}
                _ => {
                    self.sess.url_paste = None;
                    cx.emit(EditorEvent::Toast("没能获取网页标题".into()));
                }
            }
        }
    }

    pub(super) fn render_url_paste(&self, at: Point<Pixels>, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let up = self.sess.url_paste.as_ref()?;
        let waiting = up.waiting;
        let btn = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .px_2()
                .py(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |d| d.bg(t.active).text_color(t.accent))
                .when(!on, |d| d.text_color(t.text_secondary))
                .hover(|s| s.bg(t.hover))
                .child(label)
        };
        Some(
            deferred(
                anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .occlude()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.))
                        .p(px(3.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(7.))
                        .shadow_md()
                        .child(div().px_1().text_size(px(11.5)).text_color(t.text_muted).child("显示为"))
                        .child(btn("up-link", "链接", !waiting).on_click(cx.listener(|this, _, _, cx| {
                            this.sess.url_paste = None;
                            cx.notify();
                        })))
                        .child(
                            btn("up-title", if waiting { "标题…" } else { "标题" }, waiting).on_click(cx.listener(|this, _, _, cx| this.url_paste_title(cx))),
                        )
                        .child(btn("up-card", "卡片", false).on_click(cx.listener(|this, _, _, cx| this.url_paste_card(cx)))),
                ),
            )
            .with_priority(3),
        )
    }
}

impl Editor {
    /// 后台下载文档中的网络图片，完成后替换为本地路径。
    pub(super) fn fetch_remote_images(&mut self, cx: &mut Context<Self>) {
        if !crate::settings::network() {
            return;
        }
        let todo: Vec<(u64, String)> = self
            .content
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Image { path, .. } if path.starts_with("http") => Some((b.id, path.clone())),
                _ => None,
            })
            .collect();
        if todo.is_empty() {
            return;
        }
        let assets = self.store.assets_dir();
        let doc = self.doc_id.clone();
        for (id, url) in todo {
            let assets = assets.clone();
            let doc = doc.clone();
            let task = cx.background_executor().spawn(async move { crate::linkcard::download_image(&url, &assets).map(|p| (url, p)) });
            cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.doc_id != doc {
                        return;
                    }
                    let Some(ix) = this.content.blocks.iter().position(|b| b.id == id) else { return };
                    match result {
                        Some((url, local)) => {
                            if matches!(&this.content.blocks[ix].kind, BlockKind::Image { path, .. } if *path == url) {
                                if let BlockKind::Image { path, .. } = &mut this.block_mut(ix).kind {
                                    *path = local;
                                }
                                this.changed(cx);
                            }
                        }
                        None => cx.emit(EditorEvent::Toast("有网络图片下载失败".into())),
                    }
                })
                .ok();
            })
            .detach();
        }
    }
}
