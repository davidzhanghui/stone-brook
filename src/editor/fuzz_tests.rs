//! 编辑内核随机测试（思路来自 quiet.md 的 fuzz_tests.rs）。
//!
//! 用 GPUI 的无头 TestAppContext 驱动真实的 `Editor`：从一篇覆盖各种块的种子文档出发，
//! 每一步先随机放光标 / 选区，再随机做一种编辑操作，然后检查不变量：
//!
//! - 第 0 块是标题，其余块不是标题，至少还有一个正文块；块 id 唯一；
//! - 每段富文本的样式区间长度之和等于文本长度、没有空区间、边界落在字符边界上；
//!   行内公式区间里只有占位符 U+FFFC；
//! - 表格单元格数 = 行 × 列，列宽数 = 列数，合并单元格在界内且互不重叠；分栏至少一栏；
//! - 光标与选区两端落在合法的文本槽和字符边界上；
//! - 存盘（JSON）再读回结果不变；
//! - 阅读模式下任何操作都改不了文档；
//! - 撤销到底回到种子文档，重做到头回到最终文档。
//!
//! 结果是确定的：失败时打印种子、步号和最近的操作，便于复现。
//! `STONEBROOK_FUZZ_SEEDS=n` 跑 0..n 个种子（默认 40），`STONEBROOK_FUZZ_STEPS=m` 每个种子 m 步（默认 150，需小于撤销上限 500），
//! `STONEBROOK_FUZZ_SEED=k` 只跑第 k 个种子。

use super::{
    Backspace, Bold, Copy, Cut, Delete, DeleteWordBack, DocEnd, DocStart, DuplicateBlock, Editor, Enter, InlineCode, Italic, LineEnd, LineStart, MoveBlockDown,
    MoveBlockUp, MoveLeft, MoveRight, Paste, PastePlain, Pos, Redo, SelectAll, SelectLeft, SelectRight, SelectWordLeft, ShiftTab, SoftEnter, Strike, Tab,
    ToggleTodo, Underline, Undo, WordLeft, WordRight,
};
use crate::model::{fix_merges, Block, BlockKind, CellMerge, DocContent, Marks, RichText, Span, TimelineItem};
use gpui::{TestAppContext, Window};
use std::collections::HashSet;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn rich(parts: &[(&str, Marks)]) -> RichText {
    let mut t = RichText::default();
    for (s, m) in parts {
        t.text.push_str(s);
        t.spans.push(Span { len: s.len(), marks: m.clone() });
    }
    t.fixup();
    t
}

fn bold() -> Marks {
    Marks { bold: true, ..Default::default() }
}

/// 覆盖常见块类型、样式、缩进、表格合并、分栏、行内公式的种子文档。
fn seed_doc() -> DocContent {
    let mut inline_math = RichText::plain("质能方程 ");
    inline_math.insert_math(inline_math.len(), "E=mc^2");
    inline_math.insert(inline_math.len(), " 结束", Marks::default());
    let mut table = Block::table(3, 3);
    if let BlockKind::Table { cells, merges, widths, .. } = &mut table.kind {
        for (i, c) in cells.iter_mut().enumerate() {
            *c = RichText::plain(format!("格{i}"));
        }
        merges.push(CellMerge { row: 1, col: 1, row_span: 2, col_span: 2 });
        for i in [5, 7, 8] {
            cells[i] = RichText::default();
        }
        *widths = Some(vec![1., 2., 1.]);
    }
    let mut cols = Block::columns(2);
    if let BlockKind::Columns { cols } = &mut cols.kind {
        cols[0] = RichText::plain("左栏 left");
        cols[1] = rich(&[("右栏 ", Marks::default()), ("粗", bold())]);
    }
    let mut timeline = Block::new(
        BlockKind::Timeline { items: vec![TimelineItem::default(), TimelineItem::default()], vertical: false, same_side: false, hide: 0 },
        RichText::default(),
    );
    timeline.fixup();
    let mut nested = Block::new(BlockKind::Bullet, RichText::plain("二级列表"));
    nested.indent = 1;
    let mut todo2 = Block::new(BlockKind::Todo { checked: true }, RichText::plain("已完成"));
    todo2.indent = 1;
    DocContent::from_blocks(vec![
        Block::new(BlockKind::Title, RichText::plain("随机测试")),
        Block::new(BlockKind::Heading { level: 1 }, RichText::plain("第一章 Intro")),
        Block::new(
            BlockKind::Text,
            rich(&[
                ("普通 ", Marks::default()),
                ("粗体", bold()),
                (" 与 ", Marks::default()),
                ("code", Marks { code: true, ..Default::default() }),
                ("，再见。", Marks::default()),
            ]),
        ),
        Block::new(BlockKind::Text, inline_math),
        Block::new(BlockKind::Bullet, RichText::plain("一级列表 item")),
        nested,
        Block::new(BlockKind::Ordered, RichText::plain("有序一")),
        Block::new(BlockKind::Ordered, RichText::plain("有序二")),
        Block::new(BlockKind::Todo { checked: false }, RichText::plain("待办 todo")),
        todo2,
        Block::new(BlockKind::Quote, RichText::plain("引用 quote")),
        Block::new(BlockKind::Code { lang: "rust".into() }, RichText::plain("fn main() {\n    println!(\"hi\");\n}")),
        Block::new(BlockKind::Callout { emoji: "💡".into(), color: 2 }, RichText::plain("高亮块")),
        Block::new(BlockKind::Divider, RichText::default()),
        table,
        cols,
        Block::new(BlockKind::Math, RichText::plain("\\frac{a}{b}")),
        timeline,
        Block::new(BlockKind::Heading { level: 2 }, RichText::plain("第二节")),
        Block::text("最后一段 the end"),
    ])
}

// ───────────── 不变量 ─────────────

fn check_text(t: &RichText, at: &str) -> Result<(), String> {
    let total: usize = t.spans.iter().map(|s| s.len).sum();
    if total != t.text.len() {
        return Err(format!("{at}: 样式区间总长 {total} ≠ 文本长度 {}", t.text.len()));
    }
    let mut pos = 0;
    for sp in &t.spans {
        if sp.len == 0 {
            return Err(format!("{at}: 有长度为 0 的样式区间"));
        }
        if !t.text.is_char_boundary(pos) || !t.text.is_char_boundary(pos + sp.len) {
            return Err(format!("{at}: 样式区间 {pos}..{} 不在字符边界上", pos + sp.len));
        }
        if sp.marks.math.is_some() && t.text[pos..pos + sp.len].chars().any(|c| c != '\u{FFFC}') {
            return Err(format!("{at}: 行内公式区间里有普通文字 {:?}", &t.text[pos..pos + sp.len]));
        }
        pos += sp.len;
    }
    Ok(())
}

fn check_block(b: &Block, i: usize) -> Result<(), String> {
    let at = format!("块 {i}（{}）", b.kind.label());
    check_text(&b.text, &at)?;
    match &b.kind {
        BlockKind::Table { rows, cols, cells, widths, merges, .. } => {
            if cells.len() != rows * cols {
                return Err(format!("{at}: 单元格数 {} ≠ {rows}×{cols}", cells.len()));
            }
            if let Some(w) = widths {
                if w.len() != *cols || w.iter().any(|v| !v.is_finite() || *v <= 0.) {
                    return Err(format!("{at}: 列宽 {w:?} 与列数 {cols} 不符"));
                }
            }
            for (k, c) in cells.iter().enumerate() {
                check_text(c, &format!("{at} 格 {k}"))?;
            }
            let mut fixed = merges.clone();
            fix_merges(&mut fixed, *rows, *cols);
            if fixed != *merges {
                return Err(format!("{at}: 合并单元格越界或重叠 {merges:?}"));
            }
        }
        BlockKind::Columns { cols } => {
            if cols.is_empty() {
                return Err(format!("{at}: 分栏没有栏"));
            }
            for (k, c) in cols.iter().enumerate() {
                check_text(c, &format!("{at} 栏 {k}"))?;
            }
        }
        BlockKind::Timeline { items, .. } => {
            for (k, it) in items.iter().enumerate() {
                for (n, t) in [&it.title, &it.detail, &it.time].into_iter().enumerate() {
                    check_text(t, &format!("{at} 节点 {k}.{n}"))?;
                }
            }
        }
        BlockKind::Heading { level } if !(1..=6).contains(level) => return Err(format!("{at}: 标题级别 {level}")),
        _ => {}
    }
    Ok(())
}

fn check_pos(ed: &Editor, p: Pos, name: &str) -> Result<(), String> {
    let blocks = &ed.content.blocks;
    if p.block >= blocks.len() {
        return Err(format!("{name} {p:?} 超出块数 {}", blocks.len()));
    }
    if p.cell >= ed.stops(p.block) {
        return Err(format!("{name} {p:?} 超出文本槽数 {}", ed.stops(p.block)));
    }
    match blocks[p.block].slot(p.cell) {
        Some(t) if p.offset > t.len() || !t.text.is_char_boundary(p.offset) => Err(format!("{name} {p:?} 不在字符边界上（文本 {:?}）", t.text)),
        None if p.offset != 0 => Err(format!("{name} {p:?} 落在没有文本的块上却有偏移")),
        _ => Ok(()),
    }
}

fn check(ed: &Editor) -> Result<(), String> {
    let blocks = &ed.content.blocks;
    if ed.is_hidden(ed.core.head.block) {
        return Err(format!("光标停在被折叠 / 聚焦隐藏的块 {} 上", ed.core.head.block));
    }
    if blocks.len() < 2 {
        return Err(format!("只剩 {} 个块", blocks.len()));
    }
    if blocks[0].kind != BlockKind::Title {
        return Err("第 0 块不是标题".into());
    }
    let mut ids = HashSet::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 && b.kind == BlockKind::Title {
            return Err(format!("块 {i} 也是标题"));
        }
        if !ids.insert(b.id) {
            return Err(format!("块 {i} 的 id {} 重复", b.id));
        }
        check_block(b, i)?;
    }
    check_pos(ed, ed.core.anchor, "anchor")?;
    check_pos(ed, ed.core.head, "head")?;
    Ok(())
}

/// 存盘再读回：JSON 必须完全一致（块 id 不进 JSON）。
fn round_trip(ed: &Editor) -> Result<(), String> {
    let json = ed.content.to_json();
    let back = DocContent::try_from_json(&json).map_err(|e| format!("读回失败：{e}"))?;
    let again = back.to_json();
    if again != json {
        return Err(format!("存盘再读回结果变了：\n{}", diff(&json, &again)));
    }
    Ok(())
}

fn diff(a: &str, b: &str) -> String {
    let p = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let p = (0..=p).rev().find(|&i| a.is_char_boundary(i) && b.is_char_boundary(i)).unwrap_or(0);
    let from = (0..=p.saturating_sub(80)).rev().find(|&i| a.is_char_boundary(i) && b.is_char_boundary(i)).unwrap_or(0);
    let cut = |s: &str| s[from..].chars().take(200).collect::<String>();
    format!("  之前：…{}\n  之后：…{}", cut(a), cut(b))
}

// ───────────── 随机操作 ─────────────

const SNIPPETS: &[&str] = &[
    "a", "x", " ", "中", "文字", "Hello", "@", "＠", "**粗**", "`码`", "~~删~~", "$x^2$", "# ", "## ", "- ", "1. ", "[] ", "> ", "```", "---", "😀", "é", "\n",
    "—", "/",
];

fn random_pos(ed: &Editor, rng: &mut Rng) -> Pos {
    let block = rng.below(ed.content.blocks.len());
    let cell = rng.below(ed.stops(block));
    let len = ed.content.blocks[block].slot(cell).map(|t| t.len()).unwrap_or(0);
    ed.clamp(Pos::new(block, cell, rng.below(len + 1)))
}

fn random_kind(rng: &mut Rng) -> BlockKind {
    match rng.below(9) {
        0 => BlockKind::Text,
        1 => BlockKind::Heading { level: 1 + rng.below(6) as u8 },
        2 => BlockKind::Bullet,
        3 => BlockKind::Ordered,
        4 => BlockKind::Todo { checked: rng.chance(50) },
        5 => BlockKind::Quote,
        6 => BlockKind::Code { lang: String::new() },
        7 => BlockKind::Callout { emoji: "📌".into(), color: rng.below(5) as u8 },
        _ => BlockKind::Text,
    }
}

fn blocks_of(ed: &Editor, f: impl Fn(&BlockKind) -> bool) -> Vec<usize> {
    ed.content.blocks.iter().enumerate().filter(|(_, b)| f(&b.kind)).map(|(i, _)| i).collect()
}

/// 做一步随机操作，返回操作描述（失败时打印）。
fn step(ed: &mut Editor, rng: &mut Rng, window: &mut Window, cx: &mut gpui::Context<Editor>) -> String {
    // 先放光标：多数时候保持原位，偶尔跳到随机位置或拉出选区
    let r = rng.below(100);
    if r < 25 {
        let p = random_pos(ed, rng);
        ed.set_cursor(p, false, cx);
    } else if r < 35 {
        let p = random_pos(ed, rng);
        ed.set_cursor(p, true, cx);
    }
    let sel = format!("[{:?}→{:?}] ", ed.core.anchor, ed.core.head);
    if ed.sess.read_only {
        return sel + &read_only_step(ed, rng, window, cx);
    }
    let n = ed.content.blocks.len();
    let op = match rng.below(46) {
        0..=7 => {
            let s = *rng.pick(SNIPPETS);
            ed.insert_text(s, cx);
            // 输入 / 会打开斜杠菜单，随后的回车会执行菜单项（图片、文件会弹系统对话框，无头平台不支持）
            ed.pop.slash = None;
            // 输入 @ 打开文档菜单：一半时候直接选一篇插入链接
            if ed.pop.mention.is_some() && rng.chance(50) {
                let ix = rng.below(3);
                ed.apply_mention(ix, cx);
                return format!("{sel}输入 {s:?} 并选第 {ix} 篇文档");
            }
            format!("输入 {s:?}")
        }
        8..=10 => {
            ed.backspace(&Backspace, window, cx);
            "退格".into()
        }
        11 => {
            ed.delete(&Delete, window, cx);
            "向后删除".into()
        }
        12 => {
            ed.delete_word_back(&DeleteWordBack, window, cx);
            "删除前一个词".into()
        }
        13..=14 => {
            ed.enter(&Enter, window, cx);
            "回车".into()
        }
        15 => {
            ed.soft_enter(&SoftEnter, window, cx);
            "软回车".into()
        }
        16 => {
            ed.tab(&Tab, window, cx);
            "Tab".into()
        }
        17 => {
            ed.shift_tab(&ShiftTab, window, cx);
            "Shift-Tab".into()
        }
        18 => {
            match rng.below(5) {
                0 => ed.bold(&Bold, window, cx),
                1 => ed.italic(&Italic, window, cx),
                2 => ed.underline(&Underline, window, cx),
                3 => ed.strike(&Strike, window, cx),
                _ => ed.inline_code(&InlineCode, window, cx),
            }
            "切换行内样式".into()
        }
        19 => {
            let (fg, bg) = (rng.chance(50).then(|| rng.below(7) as u8), rng.chance(50).then(|| rng.below(15) as u8));
            ed.set_color(fg, bg, cx);
            format!("颜色 {fg:?}/{bg:?}")
        }
        20 => {
            ed.clear_format(cx);
            "清除格式".into()
        }
        21 => {
            let k = random_kind(rng);
            let label = k.label();
            ed.set_kind(k, cx);
            format!("改块类型为 {label}")
        }
        22 => {
            ed.set_sel_align(rng.below(3) as u8, cx);
            "对齐".into()
        }
        23 => {
            ed.toggle_todo(&ToggleTodo, window, cx);
            "勾选待办".into()
        }
        24 => {
            ed.duplicate_block(&DuplicateBlock, window, cx);
            "复制块".into()
        }
        25 => {
            if rng.chance(50) {
                ed.move_block_up(&MoveBlockUp, window, cx);
            } else {
                ed.move_block_down(&MoveBlockDown, window, cx);
            }
            "移动块".into()
        }
        26 => {
            let (from, to) = (1 + rng.below(n - 1), 1 + rng.below(n));
            ed.move_block_to(from, to, cx);
            format!("拖动块 {from}→{to}")
        }
        27 => {
            let b = 1 + rng.below(n - 1);
            ed.delete_block_at(b, cx);
            format!("删除块 {b}")
        }
        28 => {
            ed.copy(&Copy, window, cx);
            "复制".into()
        }
        29 => {
            ed.cut(&Cut, window, cx);
            "剪切".into()
        }
        30..=31 => {
            if rng.chance(70) {
                ed.paste(&Paste, window, cx);
                "粘贴".into()
            } else {
                ed.paste_plain(&PastePlain, window, cx);
                "粘贴纯文本".into()
            }
        }
        32 => {
            ed.undo(&Undo, window, cx);
            "撤销".into()
        }
        33 => {
            ed.redo(&Redo, window, cx);
            "重做".into()
        }
        34 => {
            ed.select_all(&SelectAll, window, cx);
            "全选".into()
        }
        35 => {
            match rng.below(10) {
                0 => ed.move_left(&MoveLeft, window, cx),
                1 => ed.move_right(&MoveRight, window, cx),
                2 => ed.select_left(&SelectLeft, window, cx),
                3 => ed.select_right(&SelectRight, window, cx),
                4 => ed.word_left(&WordLeft, window, cx),
                5 => ed.word_right(&WordRight, window, cx),
                6 => ed.select_word_left(&SelectWordLeft, window, cx),
                7 => ed.line_start(&LineStart, window, cx),
                8 => ed.line_end(&LineEnd, window, cx),
                _ => {
                    if rng.chance(50) {
                        ed.doc_start(&DocStart, window, cx)
                    } else {
                        ed.doc_end(&DocEnd, window, cx)
                    }
                }
            }
            "移动光标".into()
        }
        36 => table_step(ed, rng, cx),
        37 => {
            let ols = blocks_of(ed, |k| *k == BlockKind::Ordered);
            match ols.get(rng.below(ols.len().max(1))) {
                Some(&b) => {
                    let num = match rng.below(3) {
                        0 => None,
                        1 => Some(1),
                        _ => Some(1 + rng.below(20) as u32),
                    };
                    ed.set_ordered_number(b, num, cx);
                    format!("有序列表 {b} 编号设为 {num:?}")
                }
                None => {
                    ed.set_kind(BlockKind::Ordered, cx);
                    "改为有序列表".into()
                }
            }
        }
        38 => {
            let cols = blocks_of(ed, |k| matches!(k, BlockKind::Columns { .. }));
            match cols.first() {
                Some(&b) => {
                    let add = rng.chance(50);
                    ed.columns_op(b, add, cx);
                    format!("分栏 {b} {}", if add { "加一栏" } else { "减一栏" })
                }
                None => {
                    ed.insert_fragment(vec![Block::columns(2)], cx);
                    "插入分栏".into()
                }
            }
        }
        39 => {
            // 阅读模式：大约一半时间开着，验证所有操作都改不了文档
            let on = !ed.sess.read_only;
            ed.set_read_only(on, cx);
            format!("{}阅读模式", if on { "进入" } else { "退出" })
        }
        40..=41 => {
            let b = 1 + rng.below(n.saturating_sub(1).max(1));
            ed.toggle_fold_at(b.min(n - 1), cx);
            format!("折叠 / 展开块 {b}")
        }
        42 => {
            let b = 1 + rng.below(n.saturating_sub(1).max(1));
            ed.focus_on(b.min(n - 1), cx);
            format!("聚焦块 {b}")
        }
        43 => {
            ed.exit_focus(cx);
            "退出聚焦".into()
        }
        _ => {
            let (r, c) = (1 + rng.below(3), 1 + rng.below(3));
            ed.insert_fragment(vec![Block::table(r, c)], cx);
            format!("插入 {r}×{c} 表格")
        }
    };
    sel + &op
}

/// 阅读模式下用户还能触达的操作：键盘只剩移动 / 选择 / 复制（其余动作没注册），
/// 鼠标还能点待办复选框（会被回滚并提示）。
fn read_only_step(ed: &mut Editor, rng: &mut Rng, window: &mut Window, cx: &mut gpui::Context<Editor>) -> String {
    match rng.below(10) {
        0 => {
            ed.set_read_only(false, cx);
            "退出阅读模式".into()
        }
        1 => {
            let todos = blocks_of(ed, |k| matches!(k, BlockKind::Todo { .. }));
            match todos.get(rng.below(todos.len().max(1))) {
                Some(&b) => {
                    ed.toggle_todo_at(b, cx);
                    format!("（阅读模式）点待办 {b}")
                }
                None => "（阅读模式）没有待办".into(),
            }
        }
        2 => {
            ed.copy(&Copy, window, cx);
            "（阅读模式）复制".into()
        }
        3 => {
            ed.select_all(&SelectAll, window, cx);
            "（阅读模式）全选".into()
        }
        4 => {
            ed.select_right(&SelectRight, window, cx);
            "（阅读模式）向右选择".into()
        }
        5 => {
            ed.word_left(&WordLeft, window, cx);
            "（阅读模式）按词左移".into()
        }
        _ => {
            let p = random_pos(ed, rng);
            ed.set_cursor(p, rng.chance(30), cx);
            "（阅读模式）点击".into()
        }
    }
}

fn table_step(ed: &mut Editor, rng: &mut Rng, cx: &mut gpui::Context<Editor>) -> String {
    let tables = blocks_of(ed, |k| matches!(k, BlockKind::Table { .. }));
    let Some(&b) = tables.get(rng.below(tables.len().max(1))) else { return "（没有表格）".into() };
    let BlockKind::Table { rows, cols, .. } = ed.content.blocks[b].kind else { unreachable!() };
    // 和界面一样：光标先进表格（偶尔拉出多格选区），再走工具栏 / 右键菜单的入口 table_op
    let a = rng.below(rows * cols);
    ed.set_cursor(Pos::new(b, a, 0), false, cx);
    let h = if rng.chance(40) { rng.below(rows * cols) } else { a };
    if h != a {
        ed.set_cursor(Pos::new(b, h, 0), true, cx);
    }
    let op = *rng.pick(&[
        "row_above",
        "row_below",
        "col_left",
        "col_right",
        "del_row",
        "del_col",
        "merge",
        "merge",
        "split",
        "header_row",
        "header_col",
        "even_widths",
        "widths",
    ]);
    if op == "widths" {
        // 拖动列宽松手时的路径（end_table_col_drag）
        let w: Vec<f32> = (0..cols).map(|_| 0.5 + rng.below(4) as f32).collect();
        ed.checkpoint(super::EditKind::Other);
        ed.table_set_col_widths(b, w, cx);
    } else {
        ed.table_op(b, op, cx);
    }
    format!("表格 {b} 格 {a}..{h} {op}")
}

// ───────────── 驱动 ─────────────

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// 文档 JSON 去掉折叠状态：折叠是视图状态，不进撤销栈，比较文档内容时忽略。
fn norm(json: &str) -> String {
    json.replace(",\"collapsed\":true", "")
}

fn run_seed(seed: u64, steps: usize, store: &crate::store::Store, cx: &mut TestAppContext) {
    // 剪贴板在种子之间共享：清空，保证每个种子单独跑和连着跑结果一致
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(String::new())));
    let win = cx.add_window(|_, cx| Editor::new(store.clone(), format!("fuzz-{seed}"), seed_doc(), cx));
    win.update(cx, |ed, window, cx| {
        ed.set_doc_list(vec![("doc-a".into(), "文档甲".into()), ("doc-b".into(), "Doc 乙".into()), (format!("fuzz-{seed}"), "自己".into())]);
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let start = ed.content.to_json();
        let mut log: Vec<String> = Vec::new();
        let fail = |log: &[String], ed: &Editor, why: String| -> ! {
            let tail = log.iter().rev().take(12).rev().cloned().collect::<Vec<_>>().join("\n  ");
            panic!("种子 {seed}，第 {} 步：{why}\n最近的操作：\n  {tail}\n文档：{}", log.len(), ed.content.to_json());
        };
        if let Err(e) = check(ed) {
            fail(&log, ed, format!("种子文档本身不合法：{e}"));
        }
        for _ in 0..steps {
            let before = ed.content.to_json();
            let was_read_only = ed.sess.read_only;
            let op = step(ed, &mut rng, window, cx);
            let undo_redo = op.ends_with("撤销") || op.ends_with("重做");
            log.push(op);
            if let Err(e) = check(ed).and_then(|_| round_trip(ed)) {
                fail(&log, ed, e);
            }
            if was_read_only && ed.sess.read_only && norm(&ed.content.to_json()) != norm(&before) {
                fail(&log, ed, format!("阅读模式下文档被改了：\n{}", diff(&before, &ed.content.to_json())));
            }
            // 改了文档就必须记过撤销点（checkpoint 会清空重做栈），否则这一步撤销不了，重做还会丢掉它
            if !undo_redo && !ed.core.redo_stack.is_empty() && norm(&ed.content.to_json()) != norm(&before) {
                fail(&log, ed, format!("这一步改了文档却没有记录撤销点：\n{}", diff(&before, &ed.content.to_json())));
            }
        }
        // 撤销到底 → 种子文档；重做到头 → 最终文档
        ed.set_read_only(false, cx);
        let end = ed.content.to_json();
        ed.break_coalescing();
        // 最后几步之前若做过撤销且之后没改文档，重做栈里会合法地留着旧记录；清掉它，重做到头才应等于最终文档
        ed.core.redo_stack.clear();
        let mut guard = 0;
        while !ed.core.undo_stack.is_empty() && guard < 1000 {
            ed.undo(&Undo, window, cx);
            guard += 1;
            if let Err(e) = check(ed) {
                fail(&log, ed, format!("撤销第 {guard} 步后：{e}"));
            }
        }
        if norm(&ed.content.to_json()) != norm(&start) {
            fail(&log, ed, format!("撤销到底没有回到种子文档：\n{}", diff(&start, &ed.content.to_json())));
        }
        while !ed.core.redo_stack.is_empty() {
            ed.redo(&Redo, window, cx);
            if let Err(e) = check(ed) {
                fail(&log, ed, format!("重做后：{e}"));
            }
        }
        if norm(&ed.content.to_json()) != norm(&end) {
            fail(&log, ed, format!("重做到头没有回到最终文档：\n{}", diff(&end, &ed.content.to_json())));
        }
    })
    .unwrap();
}

#[gpui::test]
fn random_edits_keep_the_document_valid(cx: &mut TestAppContext) {
    let _env = crate::store::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("stonebrook-fuzz-{}", std::process::id()));
    std::env::set_var(crate::store::DATA_DIR_ENV, &dir);
    let store = crate::store::Store::open().unwrap();
    cx.update(|cx| cx.set_global(crate::theme::Theme::light()));
    let seeds = env_usize("STONEBROOK_FUZZ_SEEDS", 40) as u64;
    let steps = env_usize("STONEBROOK_FUZZ_STEPS", 150).min(480);
    // STONEBROOK_FUZZ_SEED=k 只跑第 k 个种子（复现失败用）
    let range = match std::env::var("STONEBROOK_FUZZ_SEED").ok().and_then(|v| v.parse::<u64>().ok()) {
        Some(k) => k..k + 1,
        None => 0..seeds,
    };
    for seed in range {
        run_seed(seed, steps, &store, cx);
    }
    store.flush();
    std::env::remove_var(crate::store::DATA_DIR_ENV);
    let _ = std::fs::remove_dir_all(&dir);
}
