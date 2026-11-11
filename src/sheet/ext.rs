//! 单元格扩展对象：数据验证（下拉列表 / 复选框 / 日期 / 数字 / 文本长度）、日期提醒、备注、评论、
//! 单元格图片、附件、迷你图、筛选视图。
//!
//! IronCalc 只管值、公式、样式；这些「单元格上的对象」整体序列化成一段 JSON，存在 `UserModel::ext`
//!（vendored ironcalc_base 的 [stonebrook patch]）。修改走 `set_ext`，进入 IronCalc 的撤销历史，
//! 和同时发生的单元格修改用 `begin_batch / end_batch` 合成一步撤销。
//! 按工作表的 `sheet_id`（不随排序 / 删除变化）分组；插入、删除行列时由 `shift` 同步平移。

use super::{funcs, SheetEditor};
use gpui::Context;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct Ext {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sheets: BTreeMap<u32, SheetX>,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct SheetX {
    /// 数据验证规则（后加的优先）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dv: Vec<Dv>,
    /// 单元格对象
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<CellX>,
    /// 筛选视图
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<View>,
    /// 文本换行「截断」的区域 [r0, c0, r1, c1]（IronCalc 的对齐里没有这一项）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip: Vec<[i32; 4]>,
    /// 跨表引用：从其他电子表格拉取的数据块
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub xrefs: Vec<XRef>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct XRef {
    /// 放置位置（左上角）
    pub r: i32,
    pub c: i32,
    /// 来源电子表格对象 id
    pub obj: String,
    /// 来源区域（空 = 第一个工作表的整个数据区）
    #[serde(default)]
    pub range: String,
}

/// 一条数据验证：区域 + 规则。
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Dv {
    pub r0: i32,
    pub c0: i32,
    pub r1: i32,
    pub c1: i32,
    pub rule: Rule,
    /// 输入非法值时拒绝（否则只标红提示）
    #[serde(default)]
    pub reject: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "t")]
pub enum Rule {
    /// 下拉列表：选项、多选、选项是否带颜色；`src` 非空时选项来自该区域
    List {
        opts: Vec<Opt>,
        #[serde(default)]
        multi: bool,
        #[serde(default)]
        colored: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        src: String,
    },
    /// 复选框（值为 TRUE / FALSE）
    Check,
    /// 日期选项：点击弹出日历；`time` 时带时间
    Date {
        #[serde(default)]
        time: bool,
    },
    /// 数字范围：op = between / notBetween / eq / ne / gt / lt / ge / le
    Num {
        op: String,
        a: f64,
        #[serde(default)]
        b: f64,
    },
    /// 文本长度
    Len {
        op: String,
        a: f64,
        #[serde(default)]
        b: f64,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Opt {
    pub text: String,
    /// 颜色序号（`OPT_COLORS`）
    #[serde(default)]
    pub color: u8,
    /// 从色板自选的底色（#rrggbb，优先于 `color`）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hex: String,
}

impl Opt {
    /// (底色, 文字色)
    pub fn colors(&self) -> (u32, u32) {
        opt_colors(self.color, &self.hex)
    }
}

/// 选项颜色：自选底色时文字色按明暗自动取（浅底配同色系深字，深底配白字）。
pub fn opt_colors(color: u8, hex: &str) -> (u32, u32) {
    match u32::from_str_radix(hex.trim_start_matches('#'), 16).ok().filter(|_| hex.len() == 7) {
        Some(bg) => {
            let ch = |s: u32| ((bg >> s) & 255) as f32;
            let lum = (0.299 * ch(16) + 0.587 * ch(8) + 0.114 * ch(0)) / 255.;
            let dark = |s: u32| ((ch(s) * 0.38).round() as u32) << s;
            (bg, if lum > 0.6 { dark(16) | dark(8) | dark(0) } else { 0xffffff })
        }
        None => OPT_COLORS[color as usize % OPT_COLORS.len()],
    }
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct CellX {
    pub r: i32,
    pub c: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// 单元格图片（资源目录里的文件路径）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remind: Option<Remind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spark: Option<Spark>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Cmt>,
}

impl CellX {
    fn is_empty(&self) -> bool {
        self.note.is_empty() && self.image.is_empty() && self.files.is_empty() && self.remind.is_none() && self.spark.is_none() && self.comments.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct FileRef {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub size: u64,
}

/// 日期提醒：提醒时间 = 单元格日期 - `before` 分钟。
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Remind {
    /// 提前多少分钟（0 = 事件发生时）
    #[serde(default)]
    pub before: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// 已经提醒过的时间点（毫秒），避免重复通知
    #[serde(default)]
    pub fired: i64,
}

/// 迷你图（折线 / 柱状，主题、颜色、圆角 / 渐变、标记高点低点等）。
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Spark {
    pub src: String,
    /// 0 折线 1 柱状 2 盈亏
    #[serde(default)]
    pub kind: u8,
    /// 主题序号（`SPARK_THEMES`）
    #[serde(default)]
    pub theme: u8,
    /// 自定义颜色（空 = 主题默认）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// 折线：渐变填充；柱状：圆角
    #[serde(default = "yes")]
    pub fancy: bool,
    /// 标记：1 高点 2 低点 4 负点 8 首点 16 尾点 32 所有数据点
    #[serde(default)]
    pub marks: u8,
    /// 各标记的颜色（按位序号 0..6；空 = 主题默认）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mark_colors: Vec<String>,
    /// 折线粗细（px）
    #[serde(default = "two")]
    pub lw: u8,
    /// 空单元格：0 空距 1 零值 2 用直线连接数据点
    #[serde(default)]
    pub empty: u8,
    /// 显示横坐标轴
    #[serde(default)]
    pub axis: bool,
    /// 从右到左绘制
    #[serde(default)]
    pub rtl: bool,
    /// 迷你图组（一次创建的为一组，可整组删除）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
}

impl Spark {
    pub fn new(src: String, group: &str) -> Self {
        Spark {
            src,
            kind: 0,
            theme: 0,
            color: String::new(),
            fancy: true,
            marks: 0,
            mark_colors: vec![],
            lw: 2,
            empty: 0,
            axis: false,
            rtl: false,
            group: group.to_string(),
        }
    }

    /// 第 bit 位标记的颜色（自定义 > 主题强调色）
    pub fn mark_color(&self, bit: usize) -> Option<&str> {
        self.mark_colors.get(bit).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }
}

fn two() -> u8 {
    2
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Cmt {
    pub author: String,
    pub text: String,
    pub ts: i64,
}

/// 筛选视图：一组保存下来的筛选条件（不影响其他视图看到的数据）。
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct View {
    pub name: String,
    pub head: i32,
    pub last: i32,
    pub c0: i32,
    pub c1: i32,
    #[serde(default)]
    pub excl: BTreeMap<i32, Vec<String>>,
}

/// 下拉选项颜色（同类：浅底深字）：(底色, 文字色)
pub const OPT_COLORS: [(u32, u32); 10] = [
    (0xbacefd, 0x1d3a8a),
    (0xfed4a4, 0x7a3a00),
    (0xa9e6fa, 0x0b4f66),
    (0xb7edb1, 0x1f5c18),
    (0xfbbfbc, 0x7a1d1a),
    (0xcdb2fa, 0x3f1d80),
    (0xffe28f, 0x6b5000),
    (0xf9b7d9, 0x7a1048),
    (0xdee0e3, 0x373c43),
    (0x9fe8d8, 0x0d5245),
];

/// 迷你图主题：(名称, 主色, 强调色)
pub const SPARK_THEMES: [(&str, u32, u32); 5] =
    [("标准", 0x3370ff, 0xf54a45), ("专业", 0x1f3a93, 0xd83931), ("鲜明", 0x5b3df5, 0xf2b136), ("柔和", 0x4e9a8f, 0x6aa84f), ("清新", 0x2ec4b6, 0xffbf00)];

impl Dv {
    pub fn contains(&self, r: i32, c: i32) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }

    /// 校验值是否合法（空值总是合法）。
    pub fn check(&self, shown: &str, num: Option<f64>) -> bool {
        if shown.is_empty() {
            return true;
        }
        let cmp = |op: &str, v: f64, a: f64, b: f64| match op {
            "between" => v >= a.min(b) && v <= a.max(b),
            "notBetween" => v < a.min(b) || v > a.max(b),
            "eq" => (v - a).abs() < 1e-9,
            "ne" => (v - a).abs() >= 1e-9,
            "gt" => v > a,
            "lt" => v < a,
            "ge" => v >= a,
            "le" => v <= a,
            _ => true,
        };
        match &self.rule {
            Rule::List { opts, multi, .. } => {
                if *multi {
                    split_multi(shown).iter().all(|v| opts.iter().any(|o| &o.text == v))
                } else {
                    opts.iter().any(|o| o.text == shown)
                }
            }
            Rule::Check => matches!(shown.to_uppercase().as_str(), "TRUE" | "FALSE"),
            Rule::Date { .. } => num.is_some(),
            Rule::Num { op, a, b } => num.is_some_and(|v| cmp(op, v, *a, *b)),
            Rule::Len { op, a, b } => cmp(op, shown.chars().count() as f64, *a, *b),
        }
    }

    pub fn describe(&self) -> String {
        let op = |op: &str, a: f64, b: f64| {
            let (a, b) = (funcs::fmt_num(a), funcs::fmt_num(b));
            match op {
                "between" => format!("介于 {a} 和 {b} 之间"),
                "notBetween" => format!("不介于 {a} 和 {b} 之间"),
                "eq" => format!("等于 {a}"),
                "ne" => format!("不等于 {a}"),
                "gt" => format!("大于 {a}"),
                "lt" => format!("小于 {a}"),
                "ge" => format!("大于或等于 {a}"),
                _ => format!("小于或等于 {a}"),
            }
        };
        match &self.rule {
            Rule::List { opts, .. } => format!("请从下拉列表中选择：{}", opts.iter().map(|o| o.text.as_str()).collect::<Vec<_>>().join("、")),
            Rule::Check => "请输入 TRUE 或 FALSE".into(),
            Rule::Date { .. } => "请输入日期".into(),
            Rule::Num { op: o, a, b } => format!("请输入{}的数字", op(o, *a, *b)),
            Rule::Len { op: o, a, b } => format!("文本长度需{}", op(o, *a, *b)),
        }
    }
}

/// 多选下拉的值：逗号分隔。
pub fn split_multi(v: &str) -> Vec<String> {
    v.split([',', '，']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// 行（或列）插入 / 删除后平移一个区间：`at` 起插入 `n` 行（n < 0 为删除 -n 行）。返回 None 表示整段被删掉。
fn shift_span(a: i32, b: i32, at: i32, n: i32) -> Option<(i32, i32)> {
    if n >= 0 {
        let f = |x: i32| if x >= at { x + n } else { x };
        return Some((f(a), f(b)));
    }
    let (d0, d1) = (at, at - n - 1);
    let f = |x: i32| {
        if x > d1 {
            x + n
        } else if x >= d0 {
            d0
        } else {
            x
        }
    };
    if a >= d0 && b <= d1 {
        return None;
    }
    let na = f(a);
    let nb = if b > d1 {
        b + n
    } else if b >= d0 {
        d0 - 1
    } else {
        b
    };
    (nb >= na).then_some((na, nb))
}

/// 平移区域引用文本（迷你图数据源）。
fn shift_ref(s: &str, rows: bool, at: i32, n: i32) -> String {
    let Some((r0, c0, r1, c1)) = funcs::parse_range(s) else { return s.to_string() };
    let (r0, c0, r1, c1) = if rows {
        let Some((a, b)) = shift_span(r0, r1, at, n) else { return s.to_string() };
        (a, c0, b, c1)
    } else {
        let Some((a, b)) = shift_span(c0, c1, at, n) else { return s.to_string() };
        (r0, a, r1, b)
    };
    let a = |r: i32, c: i32| format!("{}{}", super::col_name(c), r);
    if (r0, c0) == (r1, c1) {
        a(r0, c0)
    } else {
        format!("{}:{}", a(r0, c0), a(r1, c1))
    }
}

impl SheetX {
    pub fn cell(&self, r: i32, c: i32) -> Option<&CellX> {
        self.cells.iter().find(|x| x.r == r && x.c == c)
    }

    /// 取（必要时新建）单元格对象。
    pub fn cell_mut(&mut self, r: i32, c: i32) -> &mut CellX {
        if let Some(i) = self.cells.iter().position(|x| x.r == r && x.c == c) {
            return &mut self.cells[i];
        }
        self.cells.push(CellX { r, c, ..Default::default() });
        self.cells.last_mut().unwrap()
    }

    pub fn gc(&mut self) {
        self.cells.retain(|x| !x.is_empty());
    }

    /// 覆盖该格的数据验证（后加的优先）。
    pub fn dv_at(&self, r: i32, c: i32) -> Option<&Dv> {
        self.dv.iter().rev().find(|d| d.contains(r, c))
    }

    /// 从区域里去掉数据验证：完全被覆盖的规则删除，部分重叠的拆成剩余的矩形。
    pub fn clear_dv(&mut self, r0: i32, c0: i32, r1: i32, c1: i32) {
        let mut out = vec![];
        for d in self.dv.drain(..) {
            if d.r1 < r0 || d.r0 > r1 || d.c1 < c0 || d.c0 > c1 {
                out.push(d);
                continue;
            }
            let mut piece = |a: i32, b: i32, c: i32, e: i32| {
                if a <= b && c <= e {
                    out.push(Dv { r0: a, r1: b, c0: c, c1: e, ..d.clone() });
                }
            };
            piece(d.r0, r0 - 1, d.c0, d.c1);
            piece(r1 + 1, d.r1, d.c0, d.c1);
            piece(d.r0.max(r0), d.r1.min(r1), d.c0, c0 - 1);
            piece(d.r0.max(r0), d.r1.min(r1), c1 + 1, d.c1);
        }
        self.dv = out;
    }

    pub fn is_clip(&self, r: i32, c: i32) -> bool {
        self.clip.iter().rev().any(|k| r >= k[0] && r <= k[2] && c >= k[1] && c <= k[3])
    }

    /// 设置 / 取消区域的「截断」：先把区域从已有记录里挖掉，再按需加回。
    pub fn set_clip(&mut self, r0: i32, c0: i32, r1: i32, c1: i32, on: bool) {
        let mut out = vec![];
        for k in self.clip.drain(..) {
            if k[2] < r0 || k[0] > r1 || k[3] < c0 || k[1] > c1 {
                out.push(k);
                continue;
            }
            for p in
                [[k[0], k[1], r0 - 1, k[3]], [r1 + 1, k[1], k[2], k[3]], [k[0].max(r0), k[1], k[2].min(r1), c0 - 1], [k[0].max(r0), c1 + 1, k[2].min(r1), k[3]]]
            {
                if p[0] <= p[2] && p[1] <= p[3] {
                    out.push(p);
                }
            }
        }
        if on {
            out.push([r0, c0, r1, c1]);
        }
        self.clip = out;
    }

    /// 插入 / 删除行列后同步平移（`rows` 为行，否则为列）。
    pub fn shift(&mut self, rows: bool, at: i32, n: i32) {
        self.clip.retain_mut(|k| {
            let (i, j) = if rows { (0, 2) } else { (1, 3) };
            match shift_span(k[i], k[j], at, n) {
                Some((a, b)) => {
                    (k[i], k[j]) = (a, b);
                    true
                }
                None => false,
            }
        });
        self.dv.retain_mut(|d| {
            let (a, b) = if rows { (d.r0, d.r1) } else { (d.c0, d.c1) };
            match shift_span(a, b, at, n) {
                Some((a, b)) => {
                    if rows {
                        (d.r0, d.r1) = (a, b);
                    } else {
                        (d.c0, d.c1) = (a, b);
                    }
                    true
                }
                None => false,
            }
        });
        self.cells.retain_mut(|x| {
            let v = if rows { x.r } else { x.c };
            match shift_span(v, v, at, n) {
                Some((a, _)) => {
                    if rows {
                        x.r = a;
                    } else {
                        x.c = a;
                    }
                    if let Some(s) = &mut x.spark {
                        s.src = shift_ref(&s.src, rows, at, n);
                    }
                    true
                }
                None => false,
            }
        });
        self.views.retain_mut(|v| {
            if rows {
                match shift_span(v.head, v.last, at, n) {
                    Some((a, b)) => {
                        (v.head, v.last) = (a, b);
                        true
                    }
                    None => false,
                }
            } else {
                match shift_span(v.c0, v.c1, at, n) {
                    Some((a, b)) => {
                        (v.c0, v.c1) = (a, b);
                        v.excl = std::mem::take(&mut v.excl).into_iter().filter_map(|(c, e)| shift_span(c, c, at, n).map(|(c, _)| (c, e))).collect();
                        true
                    }
                    None => false,
                }
            }
        });
    }

    /// 区域内的对象复制到另一个位置（粘贴 / 填充用）：平移 (dr, dc)。
    pub fn copy_block(&mut self, src: &SheetX, r0: i32, c0: i32, r1: i32, c1: i32, dr: i32, dc: i32) {
        for x in src.cells.iter().filter(|x| x.r >= r0 && x.r <= r1 && x.c >= c0 && x.c <= c1) {
            let mut y = x.clone();
            y.r += dr;
            y.c += dc;
            self.cells.retain(|z| !(z.r == y.r && z.c == y.c));
            self.cells.push(y);
        }
        for d in src.dv.iter() {
            let (a0, b0, a1, b1) = (d.r0.max(r0), d.c0.max(c0), d.r1.min(r1), d.c1.min(c1));
            if a0 <= a1 && b0 <= b1 {
                self.clear_dv(a0 + dr, b0 + dc, a1 + dr, b1 + dc);
                self.dv.push(Dv { r0: a0 + dr, c0: b0 + dc, r1: a1 + dr, c1: b1 + dc, ..d.clone() });
            }
        }
    }
}

impl SheetEditor {
    /// 当前工作簿的扩展数据（按 `UserModel::ext` 的内容缓存解析结果）。
    pub(crate) fn ext(&self) -> std::rc::Rc<Ext> {
        let m = self.model.borrow();
        let raw = m.get_ext();
        let mut cache = self.ext_cache.borrow_mut();
        if cache.0 != raw {
            cache.0 = raw.to_string();
            cache.1 = std::rc::Rc::new(serde_json::from_str(raw).unwrap_or_default());
        }
        cache.1.clone()
    }

    pub(crate) fn sheet_id(&self) -> u32 {
        self.model.borrow().get_worksheets_properties().get(self.sheet as usize).map(|s| s.sheet_id).unwrap_or(1)
    }

    /// 当前工作表的扩展数据（只读）。
    pub(crate) fn sx(&self) -> SheetX {
        self.ext().sheets.get(&self.sheet_id()).cloned().unwrap_or_default()
    }

    /// 修改当前工作表的扩展数据（进入撤销历史；不调用 `changed`，由调用方决定）。
    pub(crate) fn edit_sx(&mut self, f: impl FnOnce(&mut SheetX)) {
        let id = self.sheet_id();
        let mut e = (*self.ext()).clone();
        let s = e.sheets.entry(id).or_default();
        f(s);
        s.gc();
        if s == &SheetX::default() {
            e.sheets.remove(&id);
        }
        let json = if e == Ext::default() {
            String::new()
        } else {
            match serde_json::to_string(&e) {
                Ok(j) => j,
                Err(err) => {
                    // 序列化失败时保留原扩展数据，不能写空串（会清掉所有下拉 / 备注 / 图片）
                    self.toast = Some(format!("修改未生效：扩展数据序列化失败（{err}）").into());
                    return;
                }
            }
        };
        self.model.borrow_mut().set_ext(json);
    }

    /// 修改并刷新。
    pub(crate) fn update_sx(&mut self, f: impl FnOnce(&mut SheetX), cx: &mut Context<Self>) {
        self.edit_sx(f);
        self.changed(cx);
    }

    /// 把若干修改合并为一个撤销步骤。
    pub(crate) fn batch<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.model.borrow_mut().begin_batch();
        let r = f(self);
        self.model.borrow_mut().end_batch();
        r
    }
}
