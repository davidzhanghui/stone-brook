//! 双向链接（思路来自 quiet.md 的 links.rs）：正文里输入 `@` 选一篇文档，插入指向它的链接；
//! ⌘ + 点击链接跳到那篇文档；被引用的文档在右侧栏列出「反向链接」（见 workspace）。
//!
//! 存储上就是普通的链接样式，地址为 `stonebrook://doc/<文档 ID>`：不改文档格式，
//! 旧版本当作普通链接显示，导出 Markdown 也是普通链接。

use super::{EditKind, Editor, EditorEvent, Pos};
use crate::model::{BlockKind, Marks};
use crate::theme::Theme;
use gpui::{prelude::FluentBuilder, *};

pub const DOC_LINK_PREFIX: &str = "stonebrook://doc/";

/// 链接地址指向的文档 ID。
pub fn doc_link_target(url: &str) -> Option<&str> {
    url.strip_prefix(DOC_LINK_PREFIX).filter(|id| !id.is_empty())
}

pub fn doc_link(id: &str) -> String {
    format!("{DOC_LINK_PREFIX}{id}")
}

/// 音频时间戳链接：`stonebrook://seek/<秒>?<音频文件名>`。块 id 不落盘，所以用资源文件名定位音频块。
pub const SEEK_LINK_PREFIX: &str = "stonebrook://seek/";

pub fn seek_link(secs: f64, file: &str) -> String {
    let name = std::path::Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    format!("{SEEK_LINK_PREFIX}{:.1}?{name}", secs.max(0.))
}

/// 解析时间戳链接：(秒, 音频文件名)。
pub fn seek_link_target(url: &str) -> Option<(f64, &str)> {
    let rest = url.strip_prefix(SEEK_LINK_PREFIX)?;
    let (t, name) = rest.split_once('?')?;
    let t: f64 = t.parse().ok().filter(|t: &f64| t.is_finite() && *t >= 0.)?;
    (!name.is_empty()).then_some((t, name))
}

/// 时间戳显示文字：`▶ 1:02:03` / `▶ 12:34`。
pub fn stamp_label(secs: f64) -> String {
    let s = secs.max(0.) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("▶ {h}:{m:02}:{s:02}")
    } else {
        format!("▶ {m}:{s:02}")
    }
}

/// `@` 文档选择菜单。
pub struct MentionMenu {
    pub block: usize,
    pub cell: usize,
    /// `@` 所在的字节偏移
    pub start: usize,
    pub selected: usize,
}

/// 文档库里可以被引用的文档：(ID, 标题)，按最近修改排在前面。由工作区在文档列表变化时设置。
#[derive(Clone, Default)]
pub struct DocList(pub std::rc::Rc<Vec<(String, String)>>);

const MAX_ITEMS: usize = 8;

impl Editor {
    pub fn set_doc_list(&mut self, list: Vec<(String, String)>) {
        self.doc_list = DocList(std::rc::Rc::new(list));
    }

    /// 刚输入了 `@`：可以写正文的地方打开文档菜单。
    pub(super) fn open_mention(&mut self) {
        let h = self.core.head;
        let Some(kind) = self.content.blocks.get(h.block).map(|b| b.kind.clone()) else { return };
        if h.block == 0 || kind.is_source() || matches!(kind, BlockKind::Code { .. }) || kind.is_void() || self.doc_list.0.is_empty() {
            return;
        }
        let Some(t) = self.slot_text(h.slot()) else { return };
        let Some(ch) = t.text[..h.offset].chars().next_back() else { return };
        if !matches!(ch, '@' | '＠') {
            return;
        }
        self.pop.mention = Some(MentionMenu { block: h.block, cell: h.cell, start: h.offset - ch.len_utf8(), selected: 0 });
    }

    pub fn mention_query(&self) -> Option<String> {
        let m = self.pop.mention.as_ref()?;
        let t = self.slot_text((m.block, m.cell))?;
        let first = t.text.get(m.start..)?.chars().next()?.len_utf8();
        t.text.get(m.start + first..self.core.head.offset).map(str::to_string)
    }

    /// 匹配的文档（不含当前文档），最多 MAX_ITEMS 篇。
    pub fn mention_items(&self) -> Vec<(String, String)> {
        let q = self.mention_query().unwrap_or_default().to_lowercase();
        self.doc_list
            .0
            .iter()
            .filter(|(id, title)| *id != self.doc_id && (q.is_empty() || title.to_lowercase().contains(&q)))
            .take(MAX_ITEMS)
            .cloned()
            .collect()
    }

    /// 光标离开 `@…` 或查询里出现空白、没有匹配时关闭菜单。
    pub(super) fn update_mention(&mut self) {
        let Some(m) = &self.pop.mention else { return };
        let h = self.core.head;
        let valid = h.block == m.block
            && h.cell == m.cell
            && h.offset > m.start
            && self.slot_text(h.slot()).and_then(|t| t.text.get(m.start..)).is_some_and(|s| s.starts_with(['@', '＠']));
        let q = if valid { self.mention_query() } else { None };
        match q {
            Some(q) if !q.contains(char::is_whitespace) && q.chars().count() <= 20 && !self.mention_items().is_empty() => {
                let n = self.mention_items().len();
                if let Some(m) = self.pop.mention.as_mut() {
                    m.selected = m.selected.min(n.saturating_sub(1));
                }
            }
            _ => self.pop.mention = None,
        }
    }

    pub(super) fn mention_move(&mut self, d: i32, cx: &mut Context<Self>) {
        let n = self.mention_items().len().max(1) as i32;
        if let Some(m) = self.pop.mention.as_mut() {
            m.selected = (m.selected as i32 + d).rem_euclid(n) as usize;
        }
        cx.notify();
    }

    /// 把 `@查询` 替换成指向文档的链接（文字为文档标题），后面补一个空格方便接着写。
    pub fn apply_mention(&mut self, ix: usize, cx: &mut Context<Self>) {
        // 菜单打开后块可能被移动 / 删除过：先确认 `@…` 还在光标前，否则不插入
        self.update_mention();
        let Some((id, title)) = self.mention_items().get(ix).cloned() else { return };
        let Some(m) = self.pop.mention.take() else { return };
        let end = self.core.head.offset;
        self.checkpoint(EditKind::Other);
        let title = if title.trim().is_empty() { "未命名文档".to_string() } else { title.replace('\n', " ") };
        let link = Marks { link: Some(doc_link(&id)), ..Default::default() };
        let slot = self.slot_mut((m.block, m.cell));
        slot.delete(m.start..end);
        slot.insert(m.start, &title, link);
        slot.insert(m.start + title.len(), " ", Marks::default());
        self.core.head = Pos::new(m.block, m.cell, m.start + title.len() + 1);
        self.core.anchor = self.core.head;
        self.core.pending_marks = None;
        self.changed(cx);
    }

    /// ⌘ + 点击链接：文档链接在应用内打开，其他链接交给系统。
    pub(super) fn open_link(&mut self, url: &str, cx: &mut Context<Self>) {
        if let Some((secs, name)) = seek_link_target(url) {
            let name = name.to_string();
            self.seek_to_stamp(secs, &name, cx);
            return;
        }
        match doc_link_target(url) {
            Some(id) => cx.emit(EditorEvent::OpenDoc(id.to_string())),
            None => cx.open_url(url),
        }
    }

    /// 本文档里文件名为 `name` 的音频块：(块序号, 块 id, 文件路径)。
    fn audio_block(&self, name: &str) -> Option<(usize, u64, String)> {
        self.content.blocks.iter().enumerate().find_map(|(i, b)| match &b.kind {
            BlockKind::File { path, .. } | BlockKind::Video { path, .. }
                if std::path::Path::new(path).file_name().is_some_and(|n| n.to_string_lossy() == name) =>
            {
                Some((i, b.id, path.clone()))
            }
            _ => None,
        })
    }

    /// 点击时间戳：滚到音频块、跳到该时刻并播放。
    pub(super) fn seek_to_stamp(&mut self, secs: f64, name: &str, cx: &mut Context<Self>) {
        let Some((ix, id, path)) = self.audio_block(name) else {
            cx.emit(EditorEvent::Toast("这段音频已不在本文档中".into()));
            return;
        };
        let Some(play) = super::audio::play_path(&path, cx) else {
            cx.emit(EditorEvent::Toast("音频还在解析，请稍后再试".into()));
            return;
        };
        crate::media::pause_others(id);
        crate::media::with(id, &play, |pl| {
            pl.seek(secs);
            if !pl.playing {
                pl.play();
            }
        });
        self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        cx.notify();
    }

    /// 正在播放（或最近暂停）的音频块：(文件路径, 当前位置)。
    fn active_audio(&self, cx: &mut App) -> Option<(String, f64)> {
        let mut best: Option<(String, f64, bool)> = None;
        for b in &self.content.blocks {
            let path = match &b.kind {
                BlockKind::File { path, name, .. } if super::attach::preview_kind(name) == super::attach::Preview::Audio => path,
                _ => continue,
            };
            if !crate::media::exists(b.id) {
                continue;
            }
            let Some(play) = super::audio::play_path(path, cx) else { continue };
            let (playing, pos) = crate::media::with(b.id, &play, |pl| (pl.playing, pl.position())).unwrap_or((false, 0.));
            if best.as_ref().is_none_or(|(_, _, p)| playing && !*p) {
                best = Some((path.clone(), pos, playing));
            }
        }
        best.map(|(p, t, _)| (p, t))
    }

    /// ⌥⌘T：给正在播放的音频插入时间戳。
    pub fn insert_timestamp_action(&mut self, _: &super::InsertTimestamp, _: &mut Window, cx: &mut Context<Self>) {
        match self.active_audio(cx) {
            Some((path, secs)) => self.insert_timestamp(&path, secs, cx),
            None => cx.emit(EditorEvent::Toast("先播放文档里的一段音频".into())),
        }
    }

    /// 在光标处插入音频时间戳链接；光标不在可写文字的位置时，在音频块后新起一段。
    pub fn insert_timestamp(&mut self, file: &str, secs: f64, cx: &mut Context<Self>) {
        if self.sess.read_only {
            return;
        }
        let label = stamp_label(secs);
        let link = Marks { link: Some(seek_link(secs, file)), ..Default::default() };
        let h = self.core.head;
        let writable = self
            .content
            .blocks
            .get(h.block)
            .is_some_and(|b| h.block > 0 && !b.kind.is_void() && !b.kind.is_source() && !matches!(b.kind, BlockKind::Code { .. }));
        self.checkpoint(EditKind::Other);
        if writable {
            if self.has_selection() {
                self.delete_selection_inner();
            }
            let h = self.core.head;
            let before = self.slot_text(h.slot()).and_then(|t| t.text[..h.offset].chars().next_back());
            let mut at = h.offset;
            let slot = self.slot_mut(h.slot());
            if before.is_some_and(|c| !c.is_whitespace()) {
                slot.insert(at, " ", Marks::default());
                at += 1;
            }
            slot.insert(at, &label, link);
            slot.insert(at + label.len(), " ", Marks::default());
            self.core.head = Pos::new(h.block, h.cell, at + label.len() + 1);
        } else {
            let name = std::path::Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let at = self.audio_block(&name).map_or(h.block + 1, |(i, _, _)| i + 1).min(self.content.blocks.len());
            let mut text = crate::model::RichText::default();
            text.insert(0, &label, link);
            text.insert(label.len(), " ", Marks::default());
            let n = text.len();
            self.insert_blocks(at, vec![crate::model::Block::new(BlockKind::Text, text)]);
            self.core.head = Pos::new(at, 0, n);
        }
        self.core.anchor = self.core.head;
        self.core.pending_marks = None;
        self.reveal_head();
        self.changed(cx);
    }

    pub(super) fn render_mention(&self, at: Point<Pixels>, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.mention_items();
        let selected = self.pop.mention.as_ref().map(|m| m.selected).unwrap_or(0);
        deferred(
            anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("mention-menu")
                    .occlude()
                    .w(px(280.))
                    .p_1()
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .text_size(px(13.))
                    .child(div().px_2().py_1().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("链接到文档")))
                    .children(items.into_iter().enumerate().map(|(i, (_, title))| {
                        div()
                            .id(("mention", i))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py(px(6.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .when(i == selected, |d| d.bg(t.active))
                            .hover(|s| s.bg(t.hover))
                            .child(crate::ui::icon("file", 14., t.text_muted))
                            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(t.text).child(if title.trim().is_empty() {
                                "未命名文档".to_string()
                            } else {
                                title
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.apply_mention(i, cx);
                                }),
                            )
                    })),
            ),
        )
        .with_priority(3)
    }
}

#[cfg(test)]
mod tests {
    use super::{doc_link, doc_link_target, seek_link, seek_link_target, stamp_label, DOC_LINK_PREFIX};

    #[test]
    fn seek_links_round_trip() {
        let url = seek_link(754.26, "/Users/x/Stonebrook/assets/ab12.m4a");
        assert_eq!(url, "stonebrook://seek/754.3?ab12.m4a");
        assert_eq!(seek_link_target(&url), Some((754.3, "ab12.m4a")));
        assert_eq!(seek_link_target("stonebrook://seek/abc?x.m4a"), None);
        assert_eq!(seek_link_target("stonebrook://seek/12?"), None);
        assert_eq!(stamp_label(754.9), "▶ 12:34");
        assert_eq!(stamp_label(3723.), "▶ 1:02:03");
    }

    #[test]
    fn doc_links_round_trip_through_the_url() {
        assert_eq!(doc_link_target(&doc_link("abc-123")), Some("abc-123"));
        assert_eq!(doc_link_target("https://example.com"), None);
        assert_eq!(doc_link_target(DOC_LINK_PREFIX), None);
    }
}
