//! 文档模型：块（Block）+ 富文本（RichText）。
//!
//! RichText 用「一份纯文本 + 一组按顺序排列的样式区间」表示，
//! 插入 / 删除只需调整区间长度，渲染时直接映射为 GPUI 的 TextRun。

use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// 行内样式。color / bg 为调色板下标，0 表示默认。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strike: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub code: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub color: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bg: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// 行内公式：该区间是一个占位字符 U+FFFC，值为 LaTeX 源码
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// 表格标题行默认开启（保持旧文档「首行加粗」的观感）
fn is_true_default() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}
fn is_zero(v: &u8) -> bool {
    *v == 0
}

impl Marks {
    pub fn is_plain(&self) -> bool {
        *self == Marks::default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub len: usize,
    #[serde(default, skip_serializing_if = "Marks::is_plain")]
    pub marks: Marks,
}

/// 富文本：text 为 UTF-8 字符串，spans 覆盖整个 text（长度之和 == text.len()）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
    /// 文字对齐：0 左、1 居中、2 右（仅表格单元格 / 分栏使用，普通块用 Block::align）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub align: u8,
    /// 垂直对齐：0 顶部、1 居中、2 底部（仅表格单元格使用）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub valign: u8,
}

impl RichText {
    pub fn plain(s: impl Into<String>) -> Self {
        let text: String = s.into();
        let spans = if text.is_empty() { vec![] } else { vec![Span { len: text.len(), marks: Marks::default() }] };
        RichText { text, spans, align: 0, valign: 0 }
    }

    /// 整段文字都是同一个链接。
    pub fn styled_link(text: &str, url: &str) -> Self {
        let mut r = Self::plain(text);
        if let Some(sp) = r.spans.first_mut() {
            sp.marks.link = Some(url.to_string());
        }
        r
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 反序列化后修正 spans（老数据 / 外部数据可能缺失 spans）。
    pub fn fixup(&mut self) {
        let total: usize = self.spans.iter().map(|s| s.len).sum();
        if total != self.text.len() {
            self.spans = vec![Span { len: self.text.len(), marks: Marks::default() }];
        }
        self.normalize();
    }

    fn normalize(&mut self) {
        let mut out: Vec<Span> = Vec::with_capacity(self.spans.len());
        for sp in self.spans.drain(..) {
            if sp.len == 0 {
                continue;
            }
            if let Some(last) = out.last_mut() {
                if last.marks == sp.marks {
                    last.len += sp.len;
                    continue;
                }
            }
            out.push(sp);
        }
        self.spans = out;
    }

    /// 在 offset 处切开 span，返回切点所在的 span 下标（切点之后的第一个 span）。
    fn split_at(&mut self, offset: usize) -> usize {
        let mut pos = 0;
        for i in 0..self.spans.len() {
            if pos == offset {
                return i;
            }
            let end = pos + self.spans[i].len;
            if offset < end {
                let right = Span { len: end - offset, marks: self.spans[i].marks.clone() };
                self.spans[i].len = offset - pos;
                self.spans.insert(i + 1, right);
                return i + 1;
            }
            pos = end;
        }
        self.spans.len()
    }

    /// offset 处「继承」的样式：取前一个字符的样式（行内代码 / 链接不向外延伸）。
    pub fn marks_at(&self, offset: usize) -> Marks {
        let mut pos = 0;
        let mut found: Option<&Marks> = None;
        for sp in &self.spans {
            let end = pos + sp.len;
            if offset > pos && offset <= end {
                found = Some(&sp.marks);
                break;
            }
            pos = end;
        }
        match found {
            Some(m) => {
                let mut m = m.clone();
                if offset == self.span_end_containing(offset) {
                    m.link = None;
                }
                m.math = None;
                m
            }
            None => self
                .spans
                .first()
                .map(|s| {
                    let mut m = s.marks.clone();
                    m.link = None;
                    m.code = false;
                    m.math = None;
                    m
                })
                .unwrap_or_default(),
        }
    }

    fn span_end_containing(&self, offset: usize) -> usize {
        let mut pos = 0;
        for sp in &self.spans {
            let end = pos + sp.len;
            if offset > pos && offset <= end {
                return end;
            }
            pos = end;
        }
        usize::MAX
    }

    pub fn insert(&mut self, offset: usize, s: &str, marks: Marks) {
        if s.is_empty() {
            return;
        }
        let offset = offset.min(self.text.len());
        self.text.insert_str(offset, s);
        let ix = self.split_at_for_insert(offset);
        self.spans.insert(ix, Span { len: s.len(), marks });
        self.normalize();
    }

    fn split_at_for_insert(&mut self, offset: usize) -> usize {
        // 注意：此时 text 已插入，但 spans 仍对应旧文本
        let mut pos = 0;
        for i in 0..self.spans.len() {
            let end = pos + self.spans[i].len;
            if offset == pos {
                return i;
            }
            if offset < end {
                let right = Span { len: end - offset, marks: self.spans[i].marks.clone() };
                self.spans[i].len = offset - pos;
                self.spans.insert(i + 1, right);
                return i + 1;
            }
            pos = end;
        }
        self.spans.len()
    }

    pub fn insert_rich(&mut self, offset: usize, other: &RichText) {
        if other.is_empty() {
            return;
        }
        let offset = offset.min(self.text.len());
        let ix = self.split_at(offset);
        self.text.insert_str(offset, &other.text);
        for (k, sp) in other.spans.iter().enumerate() {
            self.spans.insert(ix + k, sp.clone());
        }
        self.normalize();
    }

    pub fn delete(&mut self, range: Range<usize>) {
        let range = range.start.min(self.len())..range.end.min(self.len());
        if range.is_empty() {
            return;
        }
        let a = self.split_at(range.start);
        let b = self.split_at(range.end);
        self.spans.drain(a..b);
        self.text.replace_range(range, "");
        self.normalize();
    }

    pub fn slice(&self, range: Range<usize>) -> RichText {
        let mut out = RichText { text: self.text[range.clone()].to_string(), spans: vec![], align: 0, valign: 0 };
        let mut pos = 0;
        for sp in &self.spans {
            let end = pos + sp.len;
            let s = pos.max(range.start);
            let e = end.min(range.end);
            if s < e {
                out.spans.push(Span { len: e - s, marks: sp.marks.clone() });
            }
            pos = end;
        }
        out.normalize();
        out
    }

    pub fn split_off(&mut self, offset: usize) -> RichText {
        let tail = self.slice(offset..self.len());
        self.delete(offset..self.len());
        tail
    }

    pub fn append(&mut self, other: &RichText) {
        let len = self.len();
        self.insert_rich(len, other);
    }

    /// 对区间应用样式修改。
    pub fn update_marks(&mut self, range: Range<usize>, f: impl Fn(&mut Marks)) {
        if range.is_empty() {
            return;
        }
        let a = self.split_at(range.start);
        let b = self.split_at(range.end);
        for sp in &mut self.spans[a..b] {
            f(&mut sp.marks);
        }
        self.normalize();
    }

    /// 区间内是否所有字符都满足条件。
    pub fn all_marks(&self, range: Range<usize>, f: impl Fn(&Marks) -> bool) -> bool {
        if range.is_empty() {
            return false;
        }
        let mut pos = 0;
        for sp in &self.spans {
            let end = pos + sp.len;
            if end > range.start && pos < range.end && !f(&sp.marks) {
                return false;
            }
            pos = end;
        }
        true
    }

    /// 遍历 (区间, 样式)。
    pub fn runs(&self) -> impl Iterator<Item = (Range<usize>, &Marks)> {
        let mut pos = 0;
        self.spans.iter().map(move |sp| {
            let r = pos..pos + sp.len;
            pos += sp.len;
            (r, &sp.marks)
        })
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().filter(|c| !c.is_whitespace()).count()
    }

    /// 行内公式：(起始偏移, LaTeX)。每个公式在文本中占一个 U+FFFC（3 字节）。
    pub fn math_spans(&self) -> Vec<(usize, String)> {
        let mut out = vec![];
        for (r, m) in self.runs() {
            if let Some(tex) = &m.math {
                let mut o = r.start;
                while o + 3 <= r.end {
                    out.push((o, tex.clone()));
                    o += 3;
                }
            }
        }
        out
    }

    /// 插入一个行内公式。
    pub fn insert_math(&mut self, offset: usize, tex: &str) {
        self.insert(offset, MATH_CHAR, Marks { math: Some(tex.to_string()), ..Default::default() });
    }

    /// 纯文本形式（行内公式还原为 $...$）。
    pub fn plain_with_math(&self) -> String {
        let mut out = String::new();
        for (r, m) in self.runs() {
            match &m.math {
                Some(tex) => {
                    for _ in 0..(r.end - r.start) / 3 {
                        out.push_str(&format!("${tex}$"));
                    }
                }
                None => out.push_str(&self.text[r]),
            }
        }
        out
    }
}

/// 行内公式在文本中的占位字符
pub const MATH_CHAR: &str = "\u{FFFC}";

/// 表格合并区：记录左上角与跨度（1 表示不跨）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellMerge {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
}

/// 落在 (r, c) 上的合并区（若该格被合并覆盖）。
pub fn merge_at(merges: &[CellMerge], r: usize, c: usize) -> Option<&CellMerge> {
    merges.iter().find(|m| r >= m.row && r < m.row + m.row_span && c >= m.col && c < m.col + m.col_span)
}

/// 该格所属合并区的 (左上角行, 左上角列, 行跨度, 列跨度)。
pub fn merge_origin(merges: &[CellMerge], r: usize, c: usize) -> (usize, usize, usize, usize) {
    match merge_at(merges, r, c) {
        Some(m) => (m.row, m.col, m.row_span, m.col_span),
        None => (r, c, 1, 1),
    }
}

/// 两个单元格（线性下标）围成的矩形 (r1, c1, r2, c2)，含端点，并扩展到完整覆盖相交的合并区。
///
/// 选区的 anchor / head 按线性下标排序，列可能反向（从右上拖到左下），这里按行、列分别取 min / max。
pub fn table_rect(cols: usize, merges: &[CellMerge], a: usize, b: usize) -> (usize, usize, usize, usize) {
    let cols = cols.max(1);
    let (ra, ca, rb, cb) = (a / cols, a % cols, b / cols, b % cols);
    let (mut r1, mut c1, mut r2, mut c2) = (ra.min(rb), ca.min(cb), ra.max(rb), ca.max(cb));
    // 合并区可能把矩形往外撑，撑完可能又碰到新的合并区，直到稳定
    loop {
        let mut grown = false;
        for m in merges {
            let (mr2, mc2) = (m.row + m.row_span - 1, m.col + m.col_span - 1);
            if m.row <= r2 && mr2 >= r1 && m.col <= c2 && mc2 >= c1 && (m.row < r1 || mr2 > r2 || m.col < c1 || mc2 > c2) {
                (r1, c1, r2, c2) = (r1.min(m.row), c1.min(m.col), r2.max(mr2), c2.max(mc2));
                grown = true;
            }
        }
        if !grown {
            return (r1, c1, r2, c2);
        }
    }
}

/// 清理合并区：裁掉越界部分、去掉 1×1、丢弃重叠项。
pub fn fix_merges(merges: &mut Vec<CellMerge>, rows: usize, cols: usize) {
    let mut used = vec![false; rows.max(1) * cols.max(1)];
    let mut out: Vec<CellMerge> = Vec::with_capacity(merges.len());
    for m in merges.drain(..) {
        if m.row >= rows || m.col >= cols {
            continue;
        }
        let rs = m.row_span.max(1).min(rows - m.row);
        let cs = m.col_span.max(1).min(cols - m.col);
        if rs == 1 && cs == 1 {
            continue;
        }
        let mut free = true;
        for r in m.row..m.row + rs {
            for c in m.col..m.col + cs {
                if used[r * cols + c] {
                    free = false;
                }
            }
        }
        if !free {
            continue;
        }
        for r in m.row..m.row + rs {
            for c in m.col..m.col + cs {
                used[r * cols + c] = true;
            }
        }
        out.push(CellMerge { row: m.row, col: m.col, row_span: rs, col_span: cs });
    }
    *merges = out;
}

/// 按钮颜色（上排浅色、下排深色各 8 个）：(名称, 背景, 文字)
pub const BUTTON_COLORS: [(&str, u32, u32); 16] = [
    ("浅红色", 0xfde2e2, 0xd83931),
    ("浅橙色", 0xfeead2, 0xc25e00),
    ("浅绿色", 0xd9f5d6, 0x2a8c1e),
    ("浅青色", 0xd9f3fd, 0x0b84b0),
    ("浅蓝色", 0xe1eaff, 0x245bdb),
    ("浅粉色", 0xfcdcef, 0xc11f7d),
    ("浅紫色", 0xece2fe, 0x6425d0),
    ("浅灰色", 0xeff0f1, 0x1f2329),
    ("红色", 0xf54a45, 0xffffff),
    ("橙色", 0xff8800, 0xffffff),
    ("绿色", 0x34c724, 0xffffff),
    ("青色", 0x14b4f0, 0xffffff),
    ("蓝色", 0x3370ff, 0xffffff),
    ("粉色", 0xe8479a, 0xffffff),
    ("紫色", 0x7f3bf5, 0xffffff),
    ("黑色", 0x1f2329, 0xffffff),
];
/// 新建按钮默认浅蓝色
pub const BUTTON_DEFAULT_COLOR: usize = 4;

/// 按钮 style → 颜色序号。0 / 1 是旧版的「实心 / 描边」，≥2 为 2 + 序号。
pub fn button_color(style: u8) -> usize {
    match style {
        0 => 12,
        1 => 4,
        s => (s as usize - 2) % BUTTON_COLORS.len(),
    }
}

pub fn button_style(color: usize) -> u8 {
    color as u8 + 2
}

/// 时间轴节点（标题 / 详情 / 时间）。旧版的 body 读作标题、date 读作时间。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    #[serde(default, alias = "body")]
    pub title: RichText,
    #[serde(default)]
    pub detail: RichText,
    #[serde(default, alias = "date")]
    pub time: RichText,
}

impl TimelineItem {
    pub fn slot(&self, k: usize) -> &RichText {
        match k {
            0 => &self.title,
            1 => &self.detail,
            _ => &self.time,
        }
    }

    pub fn slot_mut(&mut self, k: usize) -> &mut RichText {
        match k {
            0 => &mut self.title,
            1 => &mut self.detail,
            _ => &mut self.time,
        }
    }
}

/// 时间轴每个节点占 3 个文本槽：标题、详情、时间。
pub const TL_SLOTS: usize = 3;
/// 时间轴可隐藏的字段位：标题 / 详情 / 时间
pub const TL_HIDE_TITLE: u8 = 1;
pub const TL_HIDE_DETAIL: u8 = 2;
pub const TL_HIDE_TIME: u8 = 4;

/// 时间轴节点配色（按顺序循环：蓝、绿、青、紫……）：(卡片底色, 圆点色)
pub const TIMELINE_COLORS: [(u32, u32); 7] =
    [(0xe1eaff, 0x3370ff), (0xd9f5d6, 0x34c724), (0xd9f3fd, 0x14b4f0), (0xece2fe, 0x7f3bf5), (0xfeead2, 0xff8800), (0xfcdcef, 0xe8479a), (0xfde2e2, 0xf54a45)];

/// 倒计时颜色（ 7 色）：(名称, 色值)
pub const COUNTDOWN_COLORS: [(&str, u32); 7] =
    [("红色", 0xf54a45), ("橙色", 0xff8800), ("黄色", 0xffc60a), ("绿色", 0x34c724), ("蓝色", 0x3370ff), ("紫色", 0x7f3bf5), ("灰色", 0x8f959e)];

fn countdown_default_color() -> u8 {
    1
}

/// 「视频或文件」块的三种视图：预览视图 / 卡片视图 / 文本视图。
pub const FILE_VIEW_PREVIEW: u8 = 0;
pub const FILE_VIEW_CARD: u8 = 1;
pub const FILE_VIEW_TEXT: u8 = 2;

fn is_countdown_default_color(v: &u8) -> bool {
    *v == 1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    Title,
    Text,
    Heading {
        level: u8,
    },
    Bullet,
    Ordered,
    Todo {
        checked: bool,
    },
    Quote,
    Code {
        lang: String,
    },
    Callout {
        emoji: String,
        color: u8,
    },
    Divider,
    /// 演讲者备注：正文里显示为浅色备注框，演示时隐藏（演讲者视图里显示）
    Notes,
    Image {
        path: String,
        /// 图片描述（替代文本）
        #[serde(default, skip_serializing_if = "String::is_empty")]
        alt: String,
        /// 显示宽度（逻辑像素）；None 表示按原图自动
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f32>,
        /// 裁剪区域，按原图比例 (x, y, w, h)，取值 0..1；None 表示不裁剪
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crop: Option<(f32, f32, f32, f32)>,
    },
    Table {
        rows: usize,
        cols: usize,
        cells: Vec<RichText>,
        /// 首行是否作为标题行（加粗 + 底色）
        #[serde(default = "is_true_default", skip_serializing_if = "is_true")]
        header_row: bool,
        /// 首列是否作为标题列
        #[serde(default, skip_serializing_if = "is_false")]
        header_col: bool,
        /// 各列宽度比例；None 表示均分
        #[serde(default, skip_serializing_if = "Option::is_none")]
        widths: Option<Vec<f32>>,
        /// 合并单元格（只记录左上角与跨度）
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        merges: Vec<CellMerge>,
    },
    Columns {
        cols: Vec<RichText>,
    },
    // ───── v2 新增 ─────
    /// 文件附件（已拷贝到 assets）；view 见 [`FILE_VIEW_PREVIEW`] 等
    File {
        path: String,
        name: String,
        size: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        view: u8,
    },
    /// 文档内目录
    Toc,
    /// 倒计时，target 为毫秒时间戳
    Countdown {
        target: i64,
        /// 颜色序号，见 [`COUNTDOWN_COLORS`]
        #[serde(default = "countdown_default_color", skip_serializing_if = "is_countdown_default_color")]
        color: u8,
        /// 倒计时结束时显示气泡提醒
        #[serde(default, skip_serializing_if = "is_false")]
        notify: bool,
        /// 按「时长」设置时记下的秒数（再次打开设置时回显）；按具体日期设置时为 None
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dur: Option<i64>,
    },
    /// 按钮：action = "open_url" | "duplicate_doc"；style 见 [`button_color`]；按钮文字存在块文本里（浮层中编辑，不可直接输入）
    Button {
        action: String,
        url: String,
        style: u8,
    },
    /// 时间轴：槽 3i / 3i+1 / 3i+2 依次为标题、详情、时间
    Timeline {
        items: Vec<TimelineItem>,
        /// 垂直排列（默认水平）
        #[serde(default, skip_serializing_if = "is_false")]
        vertical: bool,
        /// 卡片都在轴的同一侧（默认交替）
        #[serde(default, skip_serializing_if = "is_false")]
        same_side: bool,
        /// 隐藏的字段，见 `TL_HIDE_*`
        #[serde(default, skip_serializing_if = "is_zero")]
        hide: u8,
    },
    /// 块级公式（源码在 text 槽中，LaTeX）
    Math,
    /// 文本绘图（源码在 text 槽中，Mermaid 风格语法）；offsets 为拖动微调的节点偏移
    Diagram {
        #[serde(default)]
        view: u8,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        offsets: std::collections::BTreeMap<String, (f32, f32)>,
    },
    /// HTML 块（源码在 text 槽中）
    /// file 非空：上传的整页 HTML 文件（assets 下），原样渲染；network 控制是否允许联网
    Html {
        #[serde(default)]
        scripts: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        file: String,
        #[serde(default = "is_true_default", skip_serializing_if = "is_true")]
        network: bool,
    },
    /// 网页卡片（元数据缓存在 link_cards 表）
    LinkCard {
        url: String,
    },
    /// 视频（「视频或文件」：预览 / 卡片 / 文本三种视图，预览可拖动调整宽度）
    Video {
        path: String,
        name: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        view: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f32>,
    },
    /// 嵌入对象：画板 / 电子表格 / 思维导图（数据在 objects 表）
    Embed {
        kind: String,
        id: String,
    },
    /// 无法识别的块（来自更新版本），原样保留
    Unknown {
        #[serde(skip)]
        raw: Option<serde_json::Value>,
    },
}

impl BlockKind {
    /// 新建图片块（无描述、自适应宽度、不裁剪）。
    pub fn image(path: impl Into<String>) -> Self {
        BlockKind::Image { path: path.into(), alt: String::new(), width: None, crop: None }
    }

    /// 按原图尺寸 + 裁剪框 + 手动宽度算出显示框。
    /// 返回 (框宽, 框高)；`max_w` 是当前正文可用宽度。
    pub fn image_box(&self, natural: (u32, u32), max_w: f32) -> Option<(f32, f32)> {
        let BlockKind::Image { width, crop, .. } = self else { return None };
        let (nw, nh) = (natural.0.max(1) as f32, natural.1.max(1) as f32);
        // 截图通常是 2x 像素，按一半宽度显示更接近原始观感
        let base = if nw > 1200. { nw * 0.5 } else { nw };
        let (cx, cy, cw, ch) = crop.map(|(x, y, w, h)| (x.clamp(0., 1.), y.clamp(0., 1.), w.clamp(0.02, 1.), h.clamp(0.02, 1.))).unwrap_or((0., 0., 1., 1.));
        let cw = cw.min(1. - cx).max(0.02);
        let ch = ch.min(1. - cy).max(0.02);
        // 裁掉一部分后，显示宽度按比例缩小，观感与「裁掉边缘」一致
        let want = width.map(|w| w.max(24.)).unwrap_or(base * cw);
        let dw = want.min(max_w).max(24.);
        let aspect = (cw * nw) / (ch * nh);
        Some((dw, (dw / aspect).max(8.)))
    }

    /// 只有一个文本槽、可与相邻块合并的块。
    pub fn is_textual(&self) -> bool {
        matches!(
            self,
            BlockKind::Title
                | BlockKind::Text
                | BlockKind::Heading { .. }
                | BlockKind::Bullet
                | BlockKind::Ordered
                | BlockKind::Todo { .. }
                | BlockKind::Quote
                | BlockKind::Code { .. }
                | BlockKind::Callout { .. }
                | BlockKind::Notes
        )
    }

    /// 没有可编辑文本、整体选中的块。
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            BlockKind::Divider
                | BlockKind::Image { .. }
                | BlockKind::File { .. }
                | BlockKind::Toc
                | BlockKind::Countdown { .. }
                | BlockKind::Button { .. }
                | BlockKind::LinkCard { .. }
                | BlockKind::Video { .. }
                | BlockKind::Embed { .. }
                | BlockKind::Unknown { .. }
        )
    }

    /// 有多个文本槽的块。
    pub fn is_multi(&self) -> bool {
        matches!(self, BlockKind::Table { .. } | BlockKind::Columns { .. } | BlockKind::Timeline { .. })
    }

    /// 文本槽里放的是源码（公式 / 绘图 / HTML）。
    pub fn is_source(&self) -> bool {
        matches!(self, BlockKind::Math | BlockKind::Diagram { .. } | BlockKind::Html { .. })
    }

    /// 块内 Enter 插入换行而不是新建块。
    pub fn enter_is_newline(&self) -> bool {
        matches!(self, BlockKind::Code { .. } | BlockKind::Table { .. } | BlockKind::Columns { .. } | BlockKind::Timeline { .. }) || self.is_source()
    }

    pub fn is_list(&self) -> bool {
        matches!(self, BlockKind::Bullet | BlockKind::Ordered | BlockKind::Todo { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            BlockKind::Title => "标题",
            BlockKind::Text => "正文",
            BlockKind::Heading { level } => match level {
                1 => "一级标题",
                2 => "二级标题",
                3 => "三级标题",
                4 => "四级标题",
                5 => "五级标题",
                _ => "六级标题",
            },
            BlockKind::Bullet => "无序列表",
            BlockKind::Ordered => "有序列表",
            BlockKind::Todo { .. } => "待办",
            BlockKind::Quote => "引用",
            BlockKind::Code { .. } => "代码块",
            BlockKind::Callout { .. } => "高亮块",
            BlockKind::Divider => "分割线",
            BlockKind::Notes => "演讲者备注",
            BlockKind::Image { .. } => "图片",
            BlockKind::Table { .. } => "表格",
            BlockKind::Columns { .. } => "分栏",
            BlockKind::File { .. } => "文件",
            BlockKind::Toc => "目录",
            BlockKind::Countdown { .. } => "倒计时",
            BlockKind::Button { .. } => "按钮",
            BlockKind::Timeline { .. } => "时间轴",
            BlockKind::Math => "公式",
            BlockKind::Diagram { .. } => "文本绘图",
            BlockKind::Html { .. } => "HTML",
            BlockKind::LinkCard { .. } => "网页卡片",
            BlockKind::Video { .. } => "视频",
            BlockKind::Embed { kind, .. } => match kind.as_str() {
                "board" => "画板",
                "flow" => "流程图",
                "sheet" => "电子表格",
                "base" => "多维表格",
                "sketch" => "草图",
                _ => "嵌入对象",
            },
            BlockKind::Unknown { .. } => "不支持的块",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    #[serde(skip, default = "next_id")]
    pub id: u64,
    #[serde(flatten)]
    pub kind: BlockKind,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indent: u8,
    /// 对齐：0 左、1 居中、2 右
    #[serde(default, skip_serializing_if = "is_zero")]
    pub align: u8,
    #[serde(default)]
    pub text: RichText,
    /// 所属同步块（跨文档共享内容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<String>,
    /// 有序列表项的指定编号：从这一项起按此值继续计数（「开始新列表」为 1、「修改编号值」为输入值）。
    /// 其他类型的块忽略；None 表示沿用自动编号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num: Option<u32>,
    /// 折叠：子项（缩进更深的后续块 / 标题下的内容）不显示。属于视图状态，不进同步块与模板
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// 当前版本不认识的字段（更新版本给已有块加的新字段）：读入时收集，写回时原样带上，不会被旧版本静默丢掉。
    #[serde(skip)]
    pub extra: Option<Box<serde_json::Map<String, serde_json::Value>>>,
}

impl Block {
    pub fn new(kind: BlockKind, text: RichText) -> Self {
        Block { id: next_id(), kind, indent: 0, align: 0, text, sync: None, num: None, collapsed: false, extra: None }
    }

    pub fn text(s: &str) -> Self {
        Self::new(BlockKind::Text, RichText::plain(s))
    }

    pub fn table(rows: usize, cols: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self::new(
            BlockKind::Table {
                rows,
                cols,
                cells: vec![RichText::default(); rows * cols],
                header_row: true,
                header_col: false,
                widths: None,
                merges: Vec::new(),
            },
            RichText::default(),
        )
    }

    pub fn columns(n: usize) -> Self {
        Self::new(BlockKind::Columns { cols: vec![RichText::default(); n] }, RichText::default())
    }

    /// 文本槽数量（普通块 1 个，表格为单元格数，分栏为栏数，空块 0 个）。
    pub fn slot_count(&self) -> usize {
        match &self.kind {
            BlockKind::Table { cells, .. } => cells.len(),
            BlockKind::Columns { cols } => cols.len(),
            BlockKind::Timeline { items, .. } => items.len() * TL_SLOTS,
            k if k.is_void() => 0,
            _ => 1,
        }
    }

    pub fn slot(&self, cell: usize) -> Option<&RichText> {
        match &self.kind {
            BlockKind::Table { cells, .. } => cells.get(cell),
            BlockKind::Columns { cols } => cols.get(cell),
            BlockKind::Timeline { items, .. } => items.get(cell / TL_SLOTS).map(|it| it.slot(cell % TL_SLOTS)),
            k if k.is_void() => None,
            _ => Some(&self.text),
        }
    }

    pub fn slot_mut(&mut self, cell: usize) -> Option<&mut RichText> {
        match &mut self.kind {
            BlockKind::Table { cells, .. } => cells.get_mut(cell),
            BlockKind::Columns { cols } => cols.get_mut(cell),
            BlockKind::Timeline { items, .. } => items.get_mut(cell / TL_SLOTS).map(|it| it.slot_mut(cell % TL_SLOTS)),
            k if k.is_void() => None,
            _ => Some(&mut self.text),
        }
    }

    /// 文本槽是否被隐藏（时间轴关掉了该字段），光标移动时跳过。
    pub fn slot_hidden(&self, cell: usize) -> bool {
        match &self.kind {
            BlockKind::Timeline { hide, .. } => hide & (1 << (cell % TL_SLOTS)) != 0,
            _ => false,
        }
    }

    pub fn plain_text(&self) -> String {
        match &self.kind {
            BlockKind::Table { cols, cells, .. } => {
                cells.chunks(*cols.max(&1)).map(|row| row.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")
            }
            BlockKind::Columns { cols } => cols.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n"),
            BlockKind::Timeline { items, .. } => items
                .iter()
                .map(|it| [&it.time.text, &it.title.text, &it.detail.text].iter().filter(|s| !s.is_empty()).map(|s| s.as_str()).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("\n"),
            BlockKind::File { name, .. } => name.clone(),
            BlockKind::LinkCard { url } => url.clone(),
            BlockKind::Video { name, .. } => name.clone(),
            _ => self.text.text.clone(),
        }
    }

    pub fn char_count(&self) -> usize {
        match &self.kind {
            BlockKind::Table { cells, .. } => cells.iter().map(|c| c.char_count()).sum(),
            BlockKind::Columns { cols } => cols.iter().map(|c| c.char_count()).sum(),
            BlockKind::Timeline { items, .. } => items.iter().map(|i| i.title.char_count() + i.detail.char_count() + i.time.char_count()).sum(),
            k if k.is_source() => 0,
            _ => self.text.char_count(),
        }
    }

    pub fn fixup(&mut self) {
        self.text.fixup();
        match &mut self.kind {
            BlockKind::Table { rows, cols, cells, widths, merges, .. } => {
                *cols = (*cols).max(1);
                *rows = (*rows).max(1);
                cells.resize(*rows * *cols, RichText::default());
                cells.iter_mut().for_each(|c| c.fixup());
                if let Some(w) = widths {
                    w.resize(*cols, 1.);
                    if w.iter().any(|v| !v.is_finite() || *v <= 0.) {
                        *widths = None;
                    }
                }
                fix_merges(merges, *rows, *cols);
            }
            BlockKind::Columns { cols } => {
                if cols.is_empty() {
                    cols.push(RichText::default());
                }
                cols.iter_mut().for_each(|c| c.fixup());
            }
            BlockKind::Timeline { items, hide, .. } => {
                if items.is_empty() {
                    items.push(TimelineItem::default());
                }
                for it in items.iter_mut() {
                    it.title.fixup();
                    it.detail.fixup();
                    it.time.fixup();
                }
                // 至少留一个字段可见
                if *hide & 7 == 7 {
                    *hide = TL_HIDE_TITLE | TL_HIDE_DETAIL;
                }
            }
            BlockKind::Heading { level } => *level = (*level).clamp(1, 6),
            _ => {}
        }
    }

    /// 序列化为 JSON 值（未知块原样写回）。
    /// 序列化为 JSON 值；失败时返回错误（落盘路径用，不能把块写成 null）。
    pub fn try_to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        if let BlockKind::Unknown { raw: Some(raw) } = &self.kind {
            return Ok(raw.clone());
        }
        let mut v = serde_json::to_value(self)?;
        if let (Some(extra), Some(obj)) = (&self.extra, v.as_object_mut()) {
            for (k, x) in extra.iter() {
                obj.entry(k.clone()).or_insert_with(|| x.clone());
            }
        }
        Ok(v)
    }

    /// 从 JSON 值反序列化；不认识的块降级为 Unknown 并保留原始数据，认识的块里不认识的字段收进 `extra`。
    pub fn from_value(v: serde_json::Value) -> Block {
        match serde_json::from_value::<Block>(v.clone()) {
            Ok(mut b) => {
                b.extra = unknown_fields(&b, &v);
                b.fixup();
                b
            }
            Err(_) => Block::new(BlockKind::Unknown { raw: Some(v) }, RichText::default()),
        }
    }
}

/// 原始 JSON 里有、但重新序列化后没有的顶层字段。
/// 只认「重新序列化里不存在的键」：值为默认而被 `skip_serializing_if` 省掉的已知字段也会落进来，
/// 写回时用 `or_insert` 合并，已知字段的当前值始终优先，因此不会把旧值写回去。
fn unknown_fields(b: &Block, raw: &serde_json::Value) -> Option<Box<serde_json::Map<String, serde_json::Value>>> {
    let raw = raw.as_object()?;
    let known = serde_json::to_value(b).ok()?;
    let known = known.as_object()?;
    let extra: serde_json::Map<String, serde_json::Value> =
        raw.iter().filter(|(k, _)| !known.contains_key(*k) && !KNOWN_OPTIONAL_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    (!extra.is_empty()).then(|| Box::new(extra))
}

/// `Block` 自身可能被 `skip_serializing_if` 省略的字段：读入时值为默认，不算未知字段。
const KNOWN_OPTIONAL_KEYS: &[&str] = &["indent", "align", "text", "sync", "num", "collapsed"];

/// 文档内容：块序列，第 0 块固定为 Title。块用 Arc 共享，撤销快照几乎零成本。
#[derive(Clone, Debug, Default)]
pub struct DocContent {
    pub blocks: Vec<Arc<Block>>,
}

/// 当前文档格式版本。v2 起新增块类型；旧版本读到新块时降级为「不支持的块」但不丢数据。
pub const DOC_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct DocJsonRaw {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    blocks: Vec<serde_json::Value>,
}

impl DocContent {
    pub fn new_empty(title: &str) -> Self {
        DocContent { blocks: vec![Arc::new(Block::new(BlockKind::Title, RichText::plain(title))), Arc::new(Block::text(""))] }
    }

    pub fn title(&self) -> String {
        self.blocks.first().map(|b| b.text.text.clone()).unwrap_or_default()
    }

    #[cfg(test)]
    pub fn to_json(&self) -> String {
        self.try_to_json().unwrap_or_default()
    }

    /// 序列化文档；失败时返回错误（落盘路径必须用这个，失败就不写，不能把空串写进数据库）。
    pub fn try_to_json(&self) -> Result<String, serde_json::Error> {
        encode_blocks(self.blocks.iter().map(|b| &**b))
    }

    /// 解析文档 JSON。整体格式错误时返回错误（不能悄悄当成空文档，否则下次保存会把原数据覆盖掉）。
    pub fn try_from_json(s: &str) -> Result<Self, serde_json::Error> {
        let mut d: DocJsonRaw = serde_json::from_str(s)?;
        d.blocks.iter_mut().for_each(crate::assets::refs_in);
        Ok(Self::from_blocks(d.blocks.into_iter().map(Block::from_value).collect()))
    }

    /// 解析失败时打开的说明文档：告诉用户原始数据备份在哪里。
    pub fn corrupt_notice(backup: &str) -> Self {
        let mut tip =
            Block::new(BlockKind::Callout { emoji: "⚠️".into(), color: 0 }, RichText::plain("这篇文档的数据无法解析，原始内容已完整备份，未做任何修改。"));
        tip.indent = 0;
        Self::from_blocks(vec![Block::new(BlockKind::Title, RichText::plain("（文档数据损坏）")), tip, Block::text(&format!("备份文件：{backup}"))])
    }

    #[cfg(test)]
    pub fn from_json(s: &str) -> Self {
        Self::try_from_json(s).unwrap_or_else(|_| Self::from_blocks(vec![]))
    }

    pub fn from_blocks(mut blocks: Vec<Block>) -> Self {
        if blocks.first().map(|b| b.kind != BlockKind::Title).unwrap_or(true) {
            blocks.insert(0, Block::new(BlockKind::Title, RichText::default()));
        }
        for b in blocks.iter_mut().skip(1) {
            if b.kind == BlockKind::Title {
                b.kind = BlockKind::Heading { level: 1 };
            }
        }
        if blocks.len() == 1 {
            blocks.push(Block::text(""));
        }
        DocContent { blocks: blocks.into_iter().map(Arc::new).collect() }
    }

    pub fn plain_text(&self) -> String {
        self.blocks.iter().map(|b| b.plain_text()).collect::<Vec<_>>().join("\n")
    }

    pub fn char_count(&self) -> usize {
        self.blocks.iter().skip(1).map(|b| b.char_count()).sum()
    }
}

fn encode_blocks<'a>(blocks: impl Iterator<Item = &'a Block>) -> Result<String, serde_json::Error> {
    let mut raw = DocJsonRaw { version: DOC_VERSION, blocks: blocks.map(|b| b.try_to_value()).collect::<Result<_, _>>()? };
    raw.blocks.iter_mut().for_each(crate::assets::refs_out);
    serde_json::to_string(&raw)
}

/// 剪贴板片段（块数组）的序列化。
pub fn blocks_to_json(blocks: &[Block]) -> String {
    encode_blocks(blocks.iter()).unwrap_or_default()
}

/// 块数组序列化（落盘用：模板、同步块）。
pub fn try_blocks_to_json(blocks: &[Block]) -> Result<String, serde_json::Error> {
    encode_blocks(blocks.iter())
}

pub fn blocks_from_json(s: &str) -> Option<Vec<Block>> {
    let mut d: DocJsonRaw = serde_json::from_str(s).ok()?;
    d.blocks.iter_mut().for_each(crate::assets::refs_in);
    Some(d.blocks.into_iter().map(Block::from_value).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_block_fields_survive_round_trip() {
        use super::*;
        // 更新版本给段落块加了新字段 `future`；旧版本读进来再写回，不能丢
        let raw = serde_json::json!({"type": "text", "text": {"text": "hi"}, "future": {"x": 1}});
        let mut b = Block::from_value(raw);
        assert!(matches!(b.kind, BlockKind::Text));
        b.indent = 2; // 已知字段照常修改
        let back = b.try_to_value().unwrap();
        assert_eq!(back["future"], serde_json::json!({"x": 1}));
        assert_eq!(back["indent"], 2);
        // 没有未知字段时不带 extra
        let plain = Block::from_value(Block::text("a").try_to_value().unwrap());
        assert!(plain.extra.is_none());
    }

    #[test]
    fn table_rect_normalizes_reversed_columns_and_grows_over_merges() {
        // 3 列表格：从 (0,2) 拖到 (1,0)，线性下标 2 → 3，列是反的
        assert_eq!(table_rect(3, &[], 2, 3), (0, 0, 1, 2));
        // 碰到跨行合并区时矩形扩展到把它整个包住
        let m = vec![CellMerge { row: 1, col: 1, row_span: 2, col_span: 1 }];
        assert_eq!(table_rect(3, &m, 0, 4), (0, 0, 2, 1));
    }

    use super::*;

    #[test]
    fn old_timeline_and_countdown_still_load() {
        // 旧版时间轴：date + body 两个字段 → 时间 + 标题
        let v = serde_json::json!({"type": "timeline", "items": [{"date": {"text": "2024-01-01"}, "body": {"text": "启动"}}]});
        let b = Block::from_value(v);
        let BlockKind::Timeline { items, vertical, same_side, hide } = &b.kind else { panic!("{:?}", b.kind) };
        assert_eq!((items[0].time.text.as_str(), items[0].title.text.as_str(), items[0].detail.text.as_str()), ("2024-01-01", "启动", ""));
        assert_eq!((*vertical, *same_side, *hide), (false, false, 0));
        assert_eq!(b.slot_count(), 3);
        assert_eq!(b.slot(2).unwrap().text, "2024-01-01");
        // 旧版倒计时只有 target：默认橙色、不提醒、按日期
        let b = Block::from_value(serde_json::json!({"type": "countdown", "target": 1}));
        assert_eq!(b.kind, BlockKind::Countdown { target: 1, color: 1, notify: false, dur: None });
        // 新字段写出后能读回
        let round = Block::from_value(b.try_to_value().unwrap());
        assert_eq!(round.kind, b.kind);
    }

    #[test]
    fn timeline_never_hides_every_field() {
        let mut b = Block::new(BlockKind::Timeline { items: vec![], vertical: true, same_side: true, hide: 7 }, RichText::default());
        b.fixup();
        let BlockKind::Timeline { items, hide, .. } = &b.kind else { unreachable!() };
        assert_eq!((items.len(), *hide), (1, TL_HIDE_TITLE | TL_HIDE_DETAIL));
        assert!(b.slot_hidden(0) && b.slot_hidden(1) && !b.slot_hidden(2));
    }

    fn bold() -> Marks {
        Marks { bold: true, ..Default::default() }
    }

    #[test]
    fn insert_and_delete_keep_spans_consistent() {
        let mut r = RichText::plain("hello world");
        r.update_marks(0..5, |m| m.bold = true);
        assert_eq!(r.spans.len(), 2);
        r.insert(5, "!!", bold());
        assert_eq!(r.text, "hello!! world");
        assert_eq!(r.spans[0].len, 7);
        r.delete(3..9);
        assert_eq!(r.text, "helorld");
        assert_eq!(r.spans.iter().map(|s| s.len).sum::<usize>(), r.len());
        let tail = r.split_off(3);
        assert_eq!(r.text, "hel");
        assert_eq!(tail.text, "orld");
        assert!(!tail.spans[0].marks.bold);
    }

    #[test]
    fn marks_inherit_from_previous_char() {
        let mut r = RichText::plain("ab");
        r.update_marks(0..1, |m| m.bold = true);
        assert!(r.marks_at(1).bold);
        assert!(!r.marks_at(2).bold);
        assert!(r.marks_at(0).bold); // 行首继承第一个字符
    }

    #[test]
    fn unknown_blocks_survive_roundtrip() {
        let json = r#"{"version":9,"blocks":[{"type":"title","text":{"text":"T"}},{"type":"hologram","depth":3,"text":{"text":"x"}}]}"#;
        let d = DocContent::from_json(json);
        assert_eq!(d.blocks.len(), 2);
        assert!(matches!(d.blocks[1].kind, BlockKind::Unknown { .. }));
        let back = d.to_json();
        assert!(back.contains("hologram") && back.contains("\"depth\":3"));
    }

    #[test]
    fn json_roundtrip() {
        let mut d = DocContent::new_empty("测试");
        Arc::make_mut(&mut d.blocks[1]).text = {
            let mut r = RichText::plain("粗体");
            r.update_marks(0..6, |m| *m = bold());
            r
        };
        d.blocks.push(Arc::new(Block::table(2, 2)));
        let back = DocContent::from_json(&d.to_json());
        assert_eq!(back.blocks.len(), 3);
        assert_eq!(back.title(), "测试");
        assert!(back.blocks[1].text.spans[0].marks.bold);
    }
}
