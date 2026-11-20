//! 文档格式往返测试（思路来自 quiet.md 的 all-blocks.xml）。
//!
//! `tests/fixtures/all-blocks.json` 是一篇包含**每一种块**、各种行内样式和非默认字段的文档，
//! 也包含一个来自「未来版本」的未知块。要求：
//!
//! 1. 读入再写出与样例完全一致（格式稳定，旧数据不会被悄悄改写）；
//! 2. 样例覆盖 `BlockKind` 的全部变体：新增块类型时编译器会在 `kind_name` 处报错，
//!    提醒把它的规范写法加进 `all_blocks()` 并重新生成样例：
//!    `STONEBROOK_WRITE_FIXTURE=1 cargo test all_blocks_fixture`
//! 3. 代码里构造的 `all_blocks()` 序列化后与样例一致：改了序列化逻辑（字段名、默认值省略规则）会被发现。

use crate::model::*;
use std::collections::BTreeSet;

const FIXTURE: &str = include_str!("../tests/fixtures/all-blocks.json");
const FIXTURE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/all-blocks.json");

/// 每种块的名字。故意不写通配分支：新增变体时这里编译不过。
fn kind_name(k: &BlockKind) -> &'static str {
    match k {
        BlockKind::Title => "title",
        BlockKind::Text => "text",
        BlockKind::Heading { .. } => "heading",
        BlockKind::Bullet => "bullet",
        BlockKind::Ordered => "ordered",
        BlockKind::Todo { .. } => "todo",
        BlockKind::Quote => "quote",
        BlockKind::Code { .. } => "code",
        BlockKind::Callout { .. } => "callout",
        BlockKind::Divider => "divider",
        BlockKind::Notes => "notes",
        BlockKind::Image { .. } => "image",
        BlockKind::Table { .. } => "table",
        BlockKind::Columns { .. } => "columns",
        BlockKind::File { .. } => "file",
        BlockKind::Toc => "toc",
        BlockKind::Countdown { .. } => "countdown",
        BlockKind::Button { .. } => "button",
        BlockKind::Timeline { .. } => "timeline",
        BlockKind::Math => "math",
        BlockKind::Diagram { .. } => "diagram",
        BlockKind::Html { .. } => "html",
        BlockKind::LinkCard { .. } => "link_card",
        BlockKind::Video { .. } => "video",
        BlockKind::Embed { .. } => "embed",
        BlockKind::Unknown { .. } => "unknown",
    }
}

const ALL_KINDS: &[&str] = &[
    "title",
    "text",
    "heading",
    "bullet",
    "ordered",
    "todo",
    "quote",
    "code",
    "callout",
    "divider",
    "notes",
    "image",
    "table",
    "columns",
    "file",
    "toc",
    "countdown",
    "button",
    "timeline",
    "math",
    "diagram",
    "html",
    "link_card",
    "video",
    "embed",
    "unknown",
];

fn styled(parts: &[(&str, Marks)]) -> RichText {
    let mut t = RichText::default();
    for (s, m) in parts {
        t.text.push_str(s);
        t.spans.push(Span { len: s.len(), marks: m.clone() });
    }
    t.fixup();
    t
}

fn m(f: impl FnOnce(&mut Marks)) -> Marks {
    let mut x = Marks::default();
    f(&mut x);
    x
}

fn with(mut b: Block, f: impl FnOnce(&mut Block)) -> Block {
    f(&mut b);
    b
}

/// 每种块的规范写法：尽量把可选字段都设成非默认值。
fn all_blocks() -> Vec<Block> {
    let p = RichText::plain;
    let mut inline_math = p("公式 ");
    inline_math.insert_math(inline_math.len(), "a^2+b^2=c^2");
    let mut cell_align = p("居中");
    cell_align.align = 1;
    cell_align.valign = 2;
    vec![
        Block::new(BlockKind::Title, p("全部块样例")),
        Block::new(
            BlockKind::Text,
            styled(&[
                ("普通", Marks::default()),
                ("粗", m(|x| x.bold = true)),
                ("斜", m(|x| x.italic = true)),
                ("下划", m(|x| x.underline = true)),
                ("删除", m(|x| x.strike = true)),
                ("code", m(|x| x.code = true)),
                ("红字", m(|x| x.color = 3)),
                ("黄底", m(|x| x.bg = 5)),
                ("链接", m(|x| x.link = Some("https://example.com".into()))),
            ]),
        ),
        Block::new(BlockKind::Text, inline_math),
        with(Block::text("居中缩进"), |b| {
            b.align = 1;
            b.indent = 2;
        }),
        Block::new(BlockKind::Heading { level: 3 }, p("三级标题")),
        with(Block::new(BlockKind::Bullet, p("无序")), |b| b.indent = 1),
        Block::new(BlockKind::Ordered, p("有序")),
        with(Block::new(BlockKind::Ordered, p("指定编号")), |b| b.num = Some(7)),
        Block::new(BlockKind::Todo { checked: true }, p("待办")),
        Block::new(BlockKind::Quote, p("引用")),
        Block::new(BlockKind::Code { lang: "rust".into(), wrap: true, height: None }, p("fn main() {}\n")),
        Block::new(BlockKind::Callout { emoji: "💡".into(), color: 4 }, p("高亮块")),
        Block::new(BlockKind::Divider, RichText::default()),
        Block::new(BlockKind::Notes, p("演讲者备注")),
        Block::new(
            BlockKind::Image { path: "assets/a.png".into(), alt: "描述".into(), width: Some(320.), crop: Some((0.1, 0.2, 0.5, 0.5)) },
            RichText::default(),
        ),
        Block::new(
            BlockKind::Table {
                rows: 2,
                cols: 3,
                cells: vec![p("a"), p("b"), p("c"), cell_align, RichText::default(), RichText::default()],
                header_row: false,
                header_col: true,
                widths: Some(vec![1., 2., 1.]),
                merges: vec![CellMerge { row: 1, col: 1, row_span: 1, col_span: 2 }],
            },
            RichText::default(),
        ),
        Block::new(BlockKind::Columns { cols: vec![p("左"), styled(&[("右", m(|x| x.bold = true))])] }, RichText::default()),
        Block::new(BlockKind::File { path: "assets/b.pdf".into(), name: "b.pdf".into(), size: 2048, view: FILE_VIEW_CARD }, RichText::default()),
        Block::new(BlockKind::Toc, RichText::default()),
        Block::new(BlockKind::Countdown { target: 1_800_000_000_000, color: 4, notify: true, dur: Some(3600) }, RichText::default()),
        Block::new(BlockKind::Button { action: "open_url".into(), url: "https://example.com".into(), style: 3 }, p("按钮")),
        Block::new(
            BlockKind::Timeline {
                items: vec![TimelineItem { title: p("立项"), detail: p("详情"), time: p("2026-01") }, TimelineItem { title: p("上线"), ..Default::default() }],
                vertical: true,
                same_side: true,
                hide: TL_HIDE_DETAIL,
            },
            RichText::default(),
        ),
        Block::new(BlockKind::Math, p("\\int_0^1 x\\,dx")),
        Block::new(BlockKind::Diagram { view: 2, offsets: [("A".to_string(), (10., -5.))].into_iter().collect() }, p("flowchart LR\n  A --> B")),
        Block::new(BlockKind::Html { scripts: true, file: "assets/page.html".into(), network: false }, p("<b>hi</b>")),
        Block::new(BlockKind::LinkCard { url: "https://example.com/post".into() }, RichText::default()),
        Block::new(BlockKind::Video { path: "assets/v.mp4".into(), name: "v.mp4".into(), view: FILE_VIEW_TEXT, width: Some(480.) }, RichText::default()),
        Block::new(BlockKind::Embed { kind: "sheet".into(), id: "obj-1".into() }, RichText::default()),
        with(Block::text("同步块里的段落"), |b| b.sync = Some("sync-1".into())),
        Block::from_value(serde_json::json!({ "type": "from_the_future", "payload": { "x": 1 }, "text": { "text": "未来版本的块" } })),
    ]
}

fn value(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("样例不是合法 JSON")
}

#[test]
fn all_blocks_fixture() {
    let built = DocContent::from_blocks(all_blocks()).to_json();
    if std::env::var("STONEBROOK_WRITE_FIXTURE").is_ok() {
        let pretty = serde_json::to_string_pretty(&value(&built)).unwrap();
        std::fs::create_dir_all(std::path::Path::new(FIXTURE_PATH).parent().unwrap()).unwrap();
        std::fs::write(FIXTURE_PATH, pretty + "\n").unwrap();
        return;
    }

    // 1. 覆盖全部块类型
    let doc = DocContent::try_from_json(FIXTURE).expect("样例读不进来");
    let seen: BTreeSet<&str> = doc.blocks.iter().map(|b| kind_name(&b.kind)).collect();
    let missing: Vec<_> = ALL_KINDS.iter().filter(|k| !seen.contains(*k)).collect();
    assert!(missing.is_empty(), "样例缺少这些块类型：{missing:?}（加进 all_blocks() 后用 STONEBROOK_WRITE_FIXTURE=1 重新生成）");

    // 2. 读入再写出与样例一致
    assert_eq!(value(&doc.to_json()), value(FIXTURE), "样例读入再写出后变了：读写逻辑改动破坏了格式稳定性");

    // 3. 代码构造的规范写法与样例一致
    assert_eq!(value(&built), value(FIXTURE), "all_blocks() 的序列化与样例不一致：改了序列化规则？确认无误后用 STONEBROOK_WRITE_FIXTURE=1 重新生成");
}

#[test]
fn all_kinds_list_matches_kind_name() {
    let names: BTreeSet<&str> = all_blocks().iter().map(|b| kind_name(&b.kind)).collect();
    let listed: BTreeSet<&str> = ALL_KINDS.iter().copied().collect();
    assert_eq!(names, listed, "ALL_KINDS 与 all_blocks() 覆盖的块类型不一致");
}
