//! 复制 / 剪切 / 粘贴 / 插入图片 / 拖放文件。

use super::*;
use crate::clipboard;
use crate::convert;
use crate::model::{blocks_from_json, blocks_to_json, Block, BlockKind, RichText};
use std::path::PathBuf;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff"];

impl Editor {
    /// 选区对应的块片段。单个文本槽内的部分选择返回一个「行内」正文块。
    pub fn selection_fragment(&self) -> Vec<Block> {
        let (s, e) = self.sel();
        if s == e {
            let b = &self.content.blocks[s.block];
            return if b.kind.is_void() { vec![(**b).clone()] } else { vec![] };
        }
        if s.slot() == e.slot() {
            let t = self.slot_text(s.slot()).map(|t| t.slice(s.offset..e.offset)).unwrap_or_default();
            return vec![Block::new(BlockKind::Text, t)];
        }
        // 表格：只取选区覆盖的矩形区域，而不是整张表
        if s.block == e.block {
            if let BlockKind::Table { cols, cells, merges, header_row, header_col, .. } = &self.content.blocks[s.block].kind {
                if s.cell != e.cell {
                    return vec![table_region(cells, (*cols).max(1), merges, *header_row, *header_col, s.cell.min(e.cell), s.cell.max(e.cell))];
                }
            }
        }
        let mut out = Vec::new();
        for bi in s.block..=e.block {
            let mut b = (*self.content.blocks[bi]).clone();
            if b.kind.is_textual() {
                let from = if bi == s.block { s.offset } else { 0 };
                let to = if bi == e.block { e.offset } else { b.text.len() };
                b.text = b.text.slice(from..to.max(from));
            }
            if b.kind == BlockKind::Title {
                b.kind = BlockKind::Heading { level: 1 };
            }
            out.push(b);
        }
        out
    }

    /// 整块复制（块菜单用，不依赖选区）。
    pub fn copy_block_at(&mut self, b: usize, cx: &mut Context<Self>) {
        let Some(blk) = self.content.blocks.get(b).map(|b| (**b).clone()) else { return };
        let refs = vec![&blk];
        let text = convert::blocks_to_md(&refs);
        let html = convert::blocks_to_html(&refs);
        clipboard::write(&text, Some(&html), Some(&blocks_to_json(std::slice::from_ref(&blk))));
        cx.emit(EditorEvent::Toast("已复制".into()));
        cx.notify();
    }

    /// 整块剪切（块菜单用）。
    pub fn cut_block_at(&mut self, b: usize, cx: &mut Context<Self>) {
        self.copy_block_at(b, cx);
        self.checkpoint(EditKind::Other);
        self.delete_block_at(b, cx);
    }

    pub fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let frag = self.selection_fragment();
        if frag.is_empty() {
            return;
        }
        let inline = frag.len() == 1 && frag[0].kind == BlockKind::Text;
        let refs: Vec<&Block> = frag.iter().collect();
        let text = if inline { frag[0].text.plain_with_math() } else { convert::blocks_to_md(&refs) };
        let html = if inline { convert::rich_to_html(&frag[0].text) } else { convert::blocks_to_html(&refs) };
        clipboard::write(&text, Some(&html), Some(&blocks_to_json(&frag)));
        let _ = cx;
    }

    pub fn cut(&mut self, a: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let _ = a;
        self.copy(&Copy, window, cx);
        let h = self.core.head;
        if self.has_selection() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_inner();
            self.changed(cx);
        } else if self.content.blocks[h.block].kind.is_void() {
            self.delete_block_at(h.block, cx);
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_inner(false, cx);
    }

    pub fn paste_plain(&mut self, _: &PastePlain, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_inner(true, cx);
    }

    fn paste_inner(&mut self, plain: bool, cx: &mut Context<Self>) {
        let clip = clipboard::read();
        // 选中文字时粘贴网址：直接变成链接
        if let Some(t) = clip.text.as_deref().map(str::trim) {
            let (s, e) = self.sel();
            if !plain && s != e && s.slot() == e.slot() && !t.contains(char::is_whitespace) && (t.starts_with("http://") || t.starts_with("https://")) {
                self.apply_link(s.slot(), s.offset..e.offset, t, cx);
                return;
            }
        }
        // 单独一个网址：插入为链接，并提供「链接 / 标题 / 卡片」选择
        if !plain && clip.stonebrook.is_none() && clip.image.is_none() {
            if let Some(t) = clip.text.as_deref().map(str::trim).filter(|t| super::urlpaste::is_url(t)) {
                let html_is_link = clip.html.as_deref().is_none_or(|h| {
                    let text = scraper::Html::parse_fragment(h).root_element().text().collect::<String>();
                    text.trim() == t || text.trim().is_empty()
                });
                if html_is_link && self.paste_url(t, cx) {
                    return;
                }
            }
        }
        let assets = self.store.assets_dir();
        let mut source = "";
        let mut blocks: Vec<Block> = Vec::new();
        if !plain {
            if let Some(j) = clip.stonebrook.as_deref().and_then(blocks_from_json) {
                blocks = j;
                source = "stonebrook";
                // 嵌入对象（画板 / 导图 / 表格）粘贴为独立副本，避免两处共用同一份数据
                for b in &mut blocks {
                    if let BlockKind::Embed { id, .. } = &mut b.kind {
                        if let Some(copy) = crate::objects::duplicate(id) {
                            *id = copy;
                        }
                    }
                }
            } else if let Some(h) = clip.html.as_deref() {
                blocks = convert::html_to_blocks(h, &assets);
                source = "html";
            }
            if blocks.is_empty() {
                if let Some((bytes, ext)) = &clip.image {
                    if let Some(path) = convert::save_image_bytes(bytes, ext, &assets) {
                        blocks = vec![Block::new(BlockKind::image(path), RichText::default())];
                        source = "image";
                    }
                }
            }
        }
        if blocks.is_empty() {
            let Some(t) = clip.text else { return };
            let t = t.replace("\r\n", "\n");
            if !plain && convert::looks_like_markdown(&t) {
                blocks = convert::md_to_blocks(&t);
                source = "markdown";
            } else if t.contains('\n') && !self.content.blocks[self.core.head.block].kind.enter_is_newline() {
                blocks = convert::plain_to_blocks(&t);
            } else {
                blocks = vec![Block::new(BlockKind::Text, RichText::plain(t))];
            }
        }
        if plain {
            let text = blocks.iter().map(|b| b.plain_text()).collect::<Vec<_>>().join("\n");
            blocks = vec![Block::new(BlockKind::Text, RichText::plain(text))];
        }
        let n = blocks.len();
        let remote = blocks.iter().any(|b| matches!(&b.kind, BlockKind::Image { path, .. } if path.starts_with("http")));
        self.insert_fragment(blocks, cx);
        if remote {
            self.fetch_remote_images(cx);
        }
        if n > 1 {
            let msg = match source {
                "html" => format!("已从富文本还原 {n} 个块"),
                "markdown" => format!("已将 Markdown 转换为 {n} 个块"),
                _ => format!("已粘贴 {n} 个块"),
            };
            cx.emit(EditorEvent::Toast(msg));
        }
    }

    /// 在光标处插入块片段。
    pub fn insert_fragment(&mut self, mut blocks: Vec<Block>, cx: &mut Context<Self>) {
        if blocks.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        if self.has_selection() {
            self.delete_selection_inner();
        }
        let h = self.core.head;
        let target = self.content.blocks[h.block].kind.clone();
        let inline = blocks.len() == 1 && blocks[0].kind == BlockKind::Text;
        let flat = matches!(target, BlockKind::Title | BlockKind::Code { .. }) || target.is_multi();
        if inline || flat {
            let mut rich = if inline { blocks.remove(0).text } else { RichText::plain(blocks.iter().map(|b| b.plain_text()).collect::<Vec<_>>().join("\n")) };
            if matches!(target, BlockKind::Code { .. }) {
                rich = RichText::plain(rich.text);
            }
            if target == BlockKind::Title {
                rich = RichText::plain(rich.text.replace('\n', " "));
            }
            if target.is_void() {
                self.insert_blocks(h.block + 1, vec![Block::new(BlockKind::Text, rich.clone())]);
                self.core.head = Pos::new(h.block + 1, 0, rich.len());
            } else {
                self.slot_mut(h.slot()).insert_rich(h.offset, &rich);
                self.core.head.offset += rich.len();
            }
            self.core.anchor = self.core.head;
            self.changed(cx);
            self.reveal_head();
            return;
        }
        for b in &mut blocks {
            if b.kind == BlockKind::Title {
                b.kind = BlockKind::Heading { level: 1 };
            }
        }
        if target.is_void() {
            let n = blocks.len();
            self.insert_blocks(h.block + 1, blocks);
            self.core.head = self.slot_end(h.block + n);
            self.core.anchor = self.core.head;
            self.changed(cx);
            self.reveal_head();
            return;
        }
        let tail = self.slot_mut(h.slot()).split_off(h.offset);
        let first = blocks.remove(0);
        let mut rest = blocks;
        let cur_empty = self.content.blocks[h.block].text.is_empty() && self.content.blocks[h.block].kind == BlockKind::Text;
        // 只有「正文」才并入当前段落；标题 / 列表等保持独立成块
        if first.kind.is_textual() && (first.kind == BlockKind::Text || cur_empty) {
            let cur = &self.content.blocks[h.block];
            if cur.text.is_empty() && cur.kind == BlockKind::Text {
                let indent = cur.indent;
                let b = self.block_mut(h.block);
                b.kind = first.kind;
                b.indent = first.indent.max(indent);
                b.text = first.text;
            } else {
                self.block_mut(h.block).text.append(&first.text);
            }
        } else {
            rest.insert(0, first);
        }
        let n = rest.len();
        self.insert_blocks(h.block + 1, rest);
        let last = h.block + n;
        let last_kind = self.content.blocks[last].kind.clone();
        if last_kind.is_textual() {
            let len = self.content.blocks[last].text.len();
            if !tail.is_empty() {
                self.block_mut(last).text.append(&tail);
            }
            self.core.head = Pos::new(last, 0, len);
        } else if !tail.is_empty() || last + 1 >= self.content.blocks.len() {
            self.insert_blocks(last + 1, vec![Block::new(BlockKind::Text, tail)]);
            self.core.head = Pos::new(last + 1, 0, 0);
        } else {
            self.core.head = self.slot_end(last);
        }
        // 原块被清空且原本就是空正文（例如在空行粘贴了图片），把它删掉
        if self.content.blocks[h.block].kind == BlockKind::Text && self.content.blocks[h.block].text.is_empty() && h.block > 0 && n > 0 {
            self.remove_blocks(h.block..h.block + 1);
            self.core.head.block -= 1;
        }
        self.core.head = self.clamp(self.core.head);
        self.core.anchor = self.core.head;
        self.changed(cx);
        self.reveal_head();
    }

    pub fn insert_image(&mut self, _: &InsertImage, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_images(window, cx);
    }

    /// 斜杠菜单「图片」：选择框只能选图片，选中后直接插入图片块。
    pub fn pick_images(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("插入图片".into()),
            allowed_extensions: Some(IMAGE_EXTS.iter().map(|e| (*e).into()).collect()),
        });
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |this, cx| this.insert_images(paths, cx)).ok();
            }
        })
        .detach();
    }

    /// 斜杠菜单「视频或文件」：任意文件，一律作为附件块插入（图片也是文件卡片）。
    pub fn pick_files(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("插入".into()), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |this, cx| this.insert_attachments(paths, cx)).ok();
            }
        })
        .detach();
    }

    /// 插入图片块（只认图片，其他文件忽略）。
    pub fn insert_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let assets = self.store.assets_dir();
        let blocks: Vec<Block> = paths
            .iter()
            .filter_map(|p| {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                if !IMAGE_EXTS.contains(&ext.as_str()) {
                    return None;
                }
                let path = convert::save_image_bytes(&std::fs::read(p).ok()?, &ext, &assets)?;
                Some(Block::new(BlockKind::image(path), RichText::default()))
            })
            .collect();
        if blocks.is_empty() {
            cx.emit(EditorEvent::Toast("没有可以插入的图片".into()));
            return;
        }
        self.insert_fragment(blocks, cx);
    }

    /// 插入「视频或文件」块：视频 → 视频块，其余（含图片、Markdown）→ 文件块。
    /// 先插入块（指向 assets 里的目标路径），再后台拷贝并显示上传进度。
    pub fn insert_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        use crate::model::{FILE_VIEW_CARD, FILE_VIEW_PREVIEW};
        let assets = self.store.assets_dir();
        let mut blocks = Vec::new();
        let mut uploads = Vec::new();
        for p in paths {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("文件").to_string();
            let Some(dst) = reserve_asset(&p, &assets) else { continue };
            uploads.push((p.clone(), dst.clone()));
            if crate::media::VIDEO_EXTS.contains(&ext.as_str()) {
                blocks.push(Block::new(BlockKind::Video { path: dst, name, view: FILE_VIEW_PREVIEW, width: None }, RichText::default()));
            } else {
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                // 图片作为文件时默认卡片视图；其他文件默认预览视图（不支持的类型显示「无法预览」占位）
                let view = if super::attach::is_image_name(&name) { FILE_VIEW_CARD } else { FILE_VIEW_PREVIEW };
                blocks.push(Block::new(BlockKind::File { path: dst, name, size, view }, RichText::default()));
            }
        }
        if blocks.is_empty() {
            cx.emit(EditorEvent::Toast("没有可以插入的文件".into()));
            return;
        }
        self.insert_fragment(blocks, cx);
        for (src, dst) in uploads {
            self.start_upload(src, dst, cx);
        }
    }

    /// 拖入 / 粘贴本地文件：图片 → 图片块；Markdown / 文本 / 飞书 DocxXML → 解析后插入；视频与其他文件 → 附件块。
    pub fn insert_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // draw.io 文件导入成画板（.drawio / .dio，或内容是 mxfile 的 .xml）
        let (drawio, paths): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| is_drawio_file(p));
        if !drawio.is_empty() {
            self.import_drawio(drawio, cx);
        }
        if paths.is_empty() {
            return;
        }
        // 能解析成 DocxXML 的 .xml 才当文档内容，其余 .xml 仍是附件
        let docxml = |p: &PathBuf| -> Option<Vec<Block>> {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            (ext == "xml").then(|| std::fs::read_to_string(p).ok().and_then(|s| crate::docxml::import(&s).ok())).flatten()
        };
        let (inline, rest): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            IMAGE_EXTS.contains(&ext.as_str()) || matches!(ext.as_str(), "md" | "markdown" | "txt") || docxml(p).is_some()
        });
        if !rest.is_empty() {
            self.insert_attachments(rest, cx);
        }
        if inline.is_empty() {
            return;
        }
        let assets = self.store.assets_dir();
        let mut blocks = Vec::new();
        for p in inline {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if IMAGE_EXTS.contains(&ext.as_str()) {
                if let Some(path) = std::fs::read(&p).ok().and_then(|b| convert::save_image_bytes(&b, &ext, &assets)) {
                    blocks.push(Block::new(BlockKind::image(path), RichText::default()));
                }
            } else if let Some(mut doc) = docxml(&p) {
                // 文件的标题变成一级标题，正文接在后面
                let title = doc.remove(0);
                if !title.text.is_empty() {
                    blocks.push(Block::new(BlockKind::Heading { level: 1 }, title.text));
                }
                blocks.extend(doc);
            } else if let Ok(s) = std::fs::read_to_string(&p) {
                blocks.extend(if ext == "txt" { convert::plain_to_blocks(&s) } else { convert::md_to_blocks(&s) });
            }
        }
        if blocks.is_empty() {
            cx.emit(EditorEvent::Toast("没有可以插入的文件".into()));
            return;
        }
        // 保证不会被当作「行内」片段
        if blocks.len() == 1 && blocks[0].kind == BlockKind::Text {
            blocks.push(Block::text(""));
        }
        self.insert_fragment(blocks, cx);
    }
}

/// 在 assets 里为文件预留一个不重名的位置（只建目录，不拷贝）。
fn reserve_asset(src: &std::path::Path, assets: &std::path::Path) -> Option<String> {
    let name = src.file_name()?.to_string_lossy().to_string();
    let dir = assets.join(&crate::store::new_uuid()[..8]);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name).to_string_lossy().to_string())
}

fn is_drawio_file(p: &std::path::Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "drawio" | "dio" => true,
        "xml" => std::fs::read_to_string(p).is_ok_and(|s| crate::board::drawio::is_drawio(&s)),
        _ => false,
    }
}

impl Editor {
    /// 斜杠「导入 draw.io」：选文件。
    pub(super) fn pick_drawio(&mut self, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("导入".into()), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |this, cx| this.import_drawio(paths, cx)).ok();
            }
        })
        .detach();
    }

    /// draw.io 文件 → 每页一个画板插到光标后；多页时每页前加一行页名。
    pub fn import_drawio(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let dark = crate::theme::theme(cx).dark;
        let mut blocks = vec![];
        let (mut simplified, mut errors) = (0, vec![]);
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("{name}：{e}"));
                    continue;
                }
            };
            match crate::board::drawio::import(&text, dark) {
                Ok(pages) => {
                    let multi = pages.len() > 1;
                    for page in pages {
                        simplified += page.simplified;
                        if multi {
                            blocks.push(Block::new(BlockKind::Heading { level: 3 }, RichText::plain(page.name)));
                        }
                        let id = crate::objects::create(crate::objects::ObjKind::Board, Some(crate::objects::ObjData::Board(std::sync::Arc::new(page.doc))));
                        blocks.push(Block::new(BlockKind::Embed { kind: "board".into(), id }, RichText::default()));
                    }
                }
                Err(e) => errors.push(format!("{name}：{e}")),
            }
        }
        if !blocks.is_empty() {
            let n = blocks.iter().filter(|b| matches!(b.kind, BlockKind::Embed { .. })).count();
            self.insert_blocks_after_cursor(blocks, cx);
            let mut msg = format!("已导入 {n} 个 draw.io 画板，可双击编辑");
            if simplified > 0 {
                msg.push_str(&format!("（{simplified} 个不常见形状已简化为矩形）"));
            }
            cx.emit(EditorEvent::Toast(msg));
        }
        if !errors.is_empty() {
            cx.emit(EditorEvent::Toast(format!("导入失败：{}", errors.join("；"))));
        }
    }
}

/// 把文件拷贝进 assets 目录（保留原文件名，避免重名加前缀）。
pub(super) fn copy_asset(src: &std::path::Path, assets: &std::path::Path) -> Option<String> {
    let name = src.file_name()?.to_string_lossy().to_string();
    let dir = assets.join(&crate::store::new_uuid()[..8]);
    std::fs::create_dir_all(&dir).ok()?;
    let dst = dir.join(name);
    crate::assets::import_copy(src, &dst).ok()?;
    Some(dst.to_string_lossy().to_string())
}

/// 从表格里截取「单元格区间」→ 一张子表（保留裁剪后的合并信息）。
fn table_region(cells: &[RichText], cols: usize, merges: &[crate::model::CellMerge], header_row: bool, header_col: bool, a: usize, z: usize) -> Block {
    let (r1, c1, r2, c2) = crate::model::table_rect(cols, merges, a, z);
    let (nr, nc) = (r2 - r1 + 1, c2 - c1 + 1);
    let mut sub_cells = Vec::with_capacity(nr * nc);
    for r in r1..=r2 {
        for c in c1..=c2 {
            let covered = matches!(crate::model::merge_at(merges, r, c), Some(m) if (m.row, m.col) != (r, c));
            let i = r * cols + c;
            sub_cells.push(if covered { RichText::default() } else { cells.get(i).cloned().unwrap_or_default() });
        }
    }
    let mut sub_merges = Vec::new();
    for m in merges {
        let rr1 = m.row.max(r1);
        let rr2 = (m.row + m.row_span).min(r2 + 1);
        let cc1 = m.col.max(c1);
        let cc2 = (m.col + m.col_span).min(c2 + 1);
        let (rs, cs) = (rr2.saturating_sub(rr1), cc2.saturating_sub(cc1));
        if rs == 0 || cs == 0 || (rs == 1 && cs == 1) {
            continue;
        }
        sub_merges.push(crate::model::CellMerge { row: rr1 - r1, col: cc1 - c1, row_span: rs, col_span: cs });
    }
    Block::new(
        BlockKind::Table {
            rows: nr,
            cols: nc,
            cells: sub_cells,
            header_row: header_row && r1 == 0,
            header_col: header_col && c1 == 0,
            widths: None,
            merges: sub_merges,
        },
        RichText::default(),
    )
}
