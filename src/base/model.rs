//! 多维表格数据模型：多维表格（Base）→ 数据表（Table）→ 字段 / 记录 / 视图。
//!
//! 所有修改都走 `put_* / del_*`：先记下修改前的状态，`commit` 时合并成一步撤销，
//! 并把修改后的状态逐行写进 SQLite（见 `db.rs`）。撤销 / 重做就是把实体恢复到前 / 后状态。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}{}", &crate::store::new_uuid()[..10])
}

pub fn now() -> i64 {
    crate::store::now_ms()
}

// ───────────── 字段 ─────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Number,
    Select,
    MultiSelect,
    Date,
    Checkbox,
    Person,
    Url,
    Phone,
    Email,
    Attachment,
    Progress,
    Currency,
    Rating,
    Stage,
    Link,
    DuplexLink,
    Lookup,
    Formula,
    AutoNumber,
    CreatedTime,
    ModifiedTime,
    /// 按钮：点击打开链接 / 修改字段 / 触发自动化（R16）
    Button,
}

/// 字段类型菜单（分组，「字段类型」面板）。
pub const TYPE_MENU: &[(&str, &[FieldType])] = &[
    (
        "基础",
        &[
            FieldType::Text,
            FieldType::Select,
            FieldType::MultiSelect,
            FieldType::Person,
            FieldType::Date,
            FieldType::Attachment,
            FieldType::Number,
            FieldType::Checkbox,
            FieldType::Url,
        ],
    ),
    (
        "业务",
        &[
            FieldType::Stage,
            FieldType::AutoNumber,
            FieldType::Phone,
            FieldType::Email,
            FieldType::Progress,
            FieldType::Currency,
            FieldType::Rating,
            FieldType::Button,
        ],
    ),
    ("关系与计算", &[FieldType::Formula, FieldType::Lookup, FieldType::Link, FieldType::DuplexLink]),
    ("系统", &[FieldType::CreatedTime, FieldType::ModifiedTime]),
];

impl FieldType {
    pub fn label(self) -> &'static str {
        use FieldType::*;
        match self {
            Text => "文本",
            Number => "数字",
            Select => "单选",
            MultiSelect => "多选",
            Date => "日期",
            Checkbox => "复选框",
            Person => "人员",
            Url => "超链接",
            Phone => "电话号码",
            Email => "Email",
            Attachment => "附件",
            Progress => "进度",
            Currency => "货币",
            Rating => "评分",
            Stage => "流程",
            Link => "单向关联",
            DuplexLink => "双向关联",
            Lookup => "查找引用",
            Formula => "公式",
            AutoNumber => "自动编号",
            CreatedTime => "创建时间",
            ModifiedTime => "最后更新时间",
            Button => "按钮",
        }
    }

    pub fn icon(self) -> &'static str {
        use FieldType::*;
        match self {
            Text => "f-text",
            Number => "f-number",
            Select => "f-select",
            MultiSelect => "f-multi",
            Date => "calendar",
            Checkbox => "square-check",
            Person => "f-person",
            Url => "link",
            Phone => "f-phone",
            Email => "f-mail",
            Attachment => "paperclip",
            Progress => "f-progress",
            Currency => "f-currency",
            Rating => "f-star",
            Stage => "f-stage",
            Link => "f-link",
            DuplexLink => "f-duplex",
            Lookup => "search",
            Formula => "sigma",
            AutoNumber => "list-ordered",
            CreatedTime => "clock",
            ModifiedTime => "f-history",
            Button => "f-button",
        }
    }

    /// 由系统计算、不能手填的字段。
    pub fn computed(self) -> bool {
        matches!(self, FieldType::Lookup | FieldType::Formula | FieldType::AutoNumber | FieldType::CreatedTime | FieldType::ModifiedTime | FieldType::Button)
    }

    pub fn has_options(self) -> bool {
        matches!(self, FieldType::Select | FieldType::MultiSelect | FieldType::Stage)
    }

    pub fn is_link(self) -> bool {
        matches!(self, FieldType::Link | FieldType::DuplexLink)
    }

    /// 可作为看板分组依据的字段（人员、单选、多选、复选框、流程、评分、单向 / 双向关联）。
    pub fn kanban_groupable(self) -> bool {
        use FieldType::*;
        matches!(self, Select | MultiSelect | Person | Checkbox | Stage | Rating | Link | DuplexLink)
    }

    pub fn numeric(self) -> bool {
        matches!(self, FieldType::Number | FieldType::Progress | FieldType::Currency | FieldType::Rating | FieldType::AutoNumber)
    }

    pub fn is_date(self) -> bool {
        matches!(self, FieldType::Date | FieldType::CreatedTime | FieldType::ModifiedTime)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelOpt {
    pub id: String,
    pub name: String,
    pub color: u8,
}

/// 字段属性：不同类型只用到其中一部分。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prop {
    /// 单选 / 多选 / 流程的选项
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelOpt>,
    /// 格式序号：数字 / 货币小数位、日期格式、进度格式
    #[serde(skip_serializing_if = "is_zero")]
    pub fmt: u8,
    /// 货币符号 / 评分图标
    #[serde(skip_serializing_if = "String::is_empty")]
    pub symbol: String,
    /// 评分上限 / 进度上限
    #[serde(skip_serializing_if = "is_zero_f")]
    pub max: f64,
    /// 日期：新记录自动填入创建时间
    #[serde(skip_serializing_if = "is_false")]
    pub auto_fill: bool,
    /// 关联：目标数据表
    #[serde(skip_serializing_if = "String::is_empty")]
    pub table: String,
    /// 双向关联：对面表里的回链字段
    #[serde(skip_serializing_if = "String::is_empty")]
    pub back: String,
    /// 关联：只允许一条
    #[serde(skip_serializing_if = "is_false")]
    pub single: bool,
    /// 查找引用：经由哪个关联字段
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
    /// 查找引用：目标表里的哪个字段
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// 查找引用：汇总方式
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agg: String,
    /// 公式表达式
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expr: String,
    /// 自动编号前缀与位数
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub digits: u8,
    /// 查找引用「按条件查找」：目标数据表（非空即为条件模式，R3）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lookup_table: String,
    /// 查找引用的条件 / 选项联动的引用条件（最多 5 条，全部满足）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conds: Vec<LookupCond>,
    /// 字段默认值（新建记录、表单预填，R6）；日期可以是 "today"
    #[serde(skip_serializing_if = "Value::is_null")]
    pub default: Value,
    /// 选项引用：选项来自某表某字段的去重值（R6）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub opt_table: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub opt_field: String,
    /// 数字单位后缀
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// 公式 / 查找引用的结果格式：""自动 num 数字 cur 货币 pct 百分比 date 日期（R4）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub out: String,
    /// 按钮：文字、颜色、打开的链接（可含 {{[字段]}}）、要修改的字段、触发的自动化
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<FieldSet>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub auto: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub color: u8,
}

/// 查找引用 / 选项联动的一条条件：目标表字段 · 运算符 · 当前表字段（或固定值）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LookupCond {
    pub field: String,
    pub op: Op,
    /// 当前表的字段；为空时用 `value`
    #[serde(default)]
    pub rhs: String,
    #[serde(default)]
    pub value: Value,
}

/// 「把某字段设为某值」（按钮、自动化动作共用）；值可以是 "today" / "now" / "{{[字段]}}"。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldSet {
    pub field: String,
    #[serde(default)]
    pub value: Value,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}
fn is_zero_f(v: &f64) -> bool {
    *v == 0.
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub id: String,
    pub name: String,
    pub kind: FieldType,
    #[serde(default)]
    pub prop: Prop,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desc: String,
}

impl Field {
    pub fn new(name: &str, kind: FieldType) -> Self {
        let mut f = Field { id: new_id("fld"), name: name.into(), kind, prop: Prop::default(), desc: String::new() };
        f.prop = default_prop(kind);
        f
    }

    pub fn opt(&self, id: &str) -> Option<&SelOpt> {
        self.prop.options.iter().find(|o| o.id == id)
    }

    pub fn opt_by_name(&self, name: &str) -> Option<&SelOpt> {
        self.prop.options.iter().find(|o| o.name == name)
    }

    /// 选项不存在时新建（颜色轮换），返回选项 id。
    pub fn ensure_opt(&mut self, name: &str) -> String {
        if let Some(o) = self.opt_by_name(name) {
            return o.id.clone();
        }
        let id = new_id("opt");
        let color = (self.prop.options.len() % OPT_COLORS) as u8;
        self.prop.options.push(SelOpt { id: id.clone(), name: name.into(), color });
        id
    }
}

/// 字段默认值（新记录 / 表单预填）；日期的 "today" 换成今天。
pub fn default_value(f: &Field) -> Option<Value> {
    if f.kind.computed() || f.kind.is_link() || is_empty_value(Some(&f.prop.default)) && f.kind != FieldType::Checkbox || f.prop.default.is_null() {
        return None;
    }
    match (&f.kind, &f.prop.default) {
        (FieldType::Date, Value::String(s)) if s == "today" => Some(Value::from(super::formula::today())),
        (_, v) => Some(v.clone()),
    }
}

/// 选项可用的颜色数（见 `ui::tag_color`）。
pub const OPT_COLORS: usize = 12;

pub fn default_prop(kind: FieldType) -> Prop {
    let mut p = Prop::default();
    match kind {
        FieldType::Rating => {
            p.max = 5.;
            p.symbol = "star".into();
        }
        FieldType::Progress => p.max = 1.,
        FieldType::Currency => {
            p.symbol = "¥".into();
            p.fmt = 2;
        }
        FieldType::Stage => {
            for (i, n) in ["未开始", "进行中", "已完成"].iter().enumerate() {
                p.options.push(SelOpt { id: new_id("opt"), name: (*n).into(), color: [11, 0, 3][i] });
            }
        }
        FieldType::AutoNumber => p.digits = 0,
        _ => {}
    }
    p
}

// ───────────── 记录 ─────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    #[serde(default)]
    pub cells: HashMap<String, Value>,
    /// 手动顺序（分数索引：插到两条之间取中点）
    #[serde(default)]
    pub rank: f64,
    /// 自动编号
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    /// 图片缩略图（画册封面用）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumb: String,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        let n = self.name.to_lowercase();
        [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".tif", ".tiff", ".heic"].iter().any(|e| n.ends_with(e))
    }
    pub fn preview(&self) -> &str {
        if self.thumb.is_empty() {
            &self.path
        } else {
            &self.thumb
        }
    }
}

pub fn attachments(v: Option<&Value>) -> Vec<Attachment> {
    v.and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

pub fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    }
}

pub fn is_empty_value(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Bool(b)) => !*b,
        _ => false,
    }
}

// ───────────── 视图 ─────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewKind {
    Grid,
    Kanban,
    Gallery,
    Calendar,
    Gantt,
    Form,
}

impl ViewKind {
    pub const ALL: [ViewKind; 6] = [ViewKind::Grid, ViewKind::Kanban, ViewKind::Gallery, ViewKind::Gantt, ViewKind::Calendar, ViewKind::Form];

    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Grid => "表格视图",
            ViewKind::Kanban => "看板视图",
            ViewKind::Gallery => "画册视图",
            ViewKind::Calendar => "日历视图",
            ViewKind::Gantt => "甘特视图",
            ViewKind::Form => "表单视图",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            ViewKind::Grid => "表格",
            ViewKind::Kanban => "看板",
            ViewKind::Gallery => "画册",
            ViewKind::Calendar => "日历",
            ViewKind::Gantt => "甘特",
            ViewKind::Form => "表单",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            ViewKind::Grid => "v-grid",
            ViewKind::Kanban => "v-kanban",
            ViewKind::Gallery => "v-gallery",
            ViewKind::Calendar => "calendar",
            ViewKind::Gantt => "gantt",
            ViewKind::Form => "v-form",
        }
    }
    pub fn color(self) -> u32 {
        match self {
            ViewKind::Grid => 0x3370ff,
            ViewKind::Kanban => 0x2ea121,
            ViewKind::Gallery => 0x7f3bf5,
            ViewKind::Calendar => 0xe5484d,
            ViewKind::Gantt => 0xf14c9d,
            ViewKind::Form => 0xf5a339,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Is,
    IsNot,
    Contains,
    NotContains,
    Empty,
    NotEmpty,
    Gt,
    Ge,
    Lt,
    Le,
    /// 介于（数字 / 日期，值为 [下限, 上限]）
    Between,
    /// 复选框：已勾选 / 未勾选（不需要值）
    Checked,
    Unchecked,
}

impl Op {
    pub fn label(self, kind: FieldType) -> &'static str {
        match self {
            Op::Is => "等于",
            Op::IsNot => "不等于",
            Op::Contains => "包含",
            Op::NotContains => "不包含",
            Op::Empty => "为空",
            Op::NotEmpty => "不为空",
            Op::Gt if kind.is_date() => "晚于",
            Op::Lt if kind.is_date() => "早于",
            Op::Gt => "大于",
            Op::Ge => "大于等于",
            Op::Lt => "小于",
            Op::Le => "小于等于",
            Op::Between => "介于",
            Op::Checked => "已勾选",
            Op::Unchecked => "未勾选",
        }
    }
    pub fn needs_value(self) -> bool {
        !matches!(self, Op::Empty | Op::NotEmpty | Op::Checked | Op::Unchecked)
    }
    pub const ALL: [Op; 13] =
        [Op::Is, Op::IsNot, Op::Contains, Op::NotContains, Op::Empty, Op::NotEmpty, Op::Gt, Op::Ge, Op::Lt, Op::Le, Op::Between, Op::Checked, Op::Unchecked];
}

/// 某类字段可用的筛选运算符。
pub fn ops_for(kind: FieldType) -> &'static [Op] {
    use FieldType::*;
    use Op::*;
    match kind {
        Number | Progress | Currency | Rating | AutoNumber => &[Is, IsNot, Gt, Ge, Lt, Le, Between, Empty, NotEmpty],
        Date | CreatedTime | ModifiedTime => &[Is, Lt, Gt, Between, Empty, NotEmpty],
        Checkbox => &[Checked, Unchecked],
        Formula | Lookup => &[Is, IsNot, Contains, NotContains, Gt, Lt, Empty, NotEmpty],
        Select | Stage => &[Is, IsNot, Contains, NotContains, Empty, NotEmpty],
        MultiSelect | Person => &[Contains, NotContains, Is, Empty, NotEmpty],
        Attachment => &[Empty, NotEmpty],
        _ => &[Is, IsNot, Contains, NotContains, Empty, NotEmpty],
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cond {
    pub field: String,
    pub op: Op,
    #[serde(default)]
    pub value: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// true：满足任一条件；false：满足所有条件
    #[serde(default)]
    pub any: bool,
    #[serde(default)]
    pub conds: Vec<Cond>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortRule {
    pub field: String,
    #[serde(default)]
    pub desc: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    #[serde(default)]
    pub filter: Filter,
    #[serde(default)]
    pub sorts: Vec<SortRule>,
    /// 表格视图分组（最多 3 级）
    #[serde(default)]
    pub groups: Vec<SortRule>,
    #[serde(default)]
    pub hidden: Vec<String>,
    /// 视图内的字段顺序（缺的字段按数据表顺序补在后面）
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub widths: HashMap<String, f32>,
    /// 行高：0 低 1 中 2 高 3 超高
    #[serde(default)]
    pub row_h: u8,
    /// 冻结到第几列（含主字段，至少 1）
    #[serde(default = "one")]
    pub frozen: u8,
    /// 底部统计：字段 → 统计方式
    #[serde(default)]
    pub stats: HashMap<String, String>,
    /// 看板分组字段
    #[serde(default)]
    pub group_field: Option<String>,
    /// 卡片封面（附件字段）
    #[serde(default)]
    pub cover: Option<String>,
    /// 封面裁切填满（false 为完整显示）
    #[serde(default = "yes")]
    pub cover_fill: bool,
    #[serde(default)]
    pub compact: bool,
    /// 折叠的分组 / 看板列
    #[serde(default)]
    pub collapsed: Vec<String>,
    /// 日历 / 甘特：开始日期字段
    #[serde(default)]
    pub date_field: Option<String>,
    /// 甘特：结束日期字段
    #[serde(default)]
    pub end_field: Option<String>,
    /// 甘特：时间刻度 0 日 1 周 2 月
    #[serde(default)]
    pub scale: u8,
    /// 视图锁定：禁止修改视图配置
    #[serde(default)]
    pub locked: bool,
    /// 填色规则（R2，按顺序排优先级，最多 50 条）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<ColorRule>,
    /// 日历：0 月 1 周 2 日（R9）
    #[serde(default)]
    pub cal_mode: u8,
    /// 日历 / 甘特：颜色跟随的单选字段（空为默认色）
    #[serde(default)]
    pub color_field: Option<String>,
    /// 甘特：进度字段（R10）
    #[serde(default)]
    pub progress_field: Option<String>,
    /// 甘特：只算工作日；`workdays` 为每周的工作日（0 周一 … 6 周日，空为周一到周五）
    #[serde(default)]
    pub workdays_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workdays: Vec<u8>,
    /// 表单配置（R8）
    #[serde(default)]
    pub form: FormCfg,
}

/// 填色规则：0 单元格 1 整行 2 整列。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorRule {
    pub id: String,
    pub kind: u8,
    pub field: String,
    #[serde(default)]
    pub cond: Option<Cond>,
    pub color: u8,
}

pub const MAX_COLOR_RULES: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormCfg {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
    /// 封面图路径
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    /// 字段 → 填写说明
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub notes: HashMap<String, String>,
    /// 字段 → 默认值（覆盖字段默认值）
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub defaults: HashMap<String, Value>,
    /// 字段 → 显示条件（引用前面的题）
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub show_if: HashMap<String, Cond>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub done_text: String,
}

fn one() -> u8 {
    1
}
fn yes() -> bool {
    true
}

impl View {
    pub fn new(name: &str, kind: ViewKind) -> Self {
        View {
            id: new_id("viw"),
            name: name.into(),
            kind,
            filter: Filter::default(),
            sorts: vec![],
            groups: vec![],
            hidden: vec![],
            order: vec![],
            widths: HashMap::new(),
            row_h: 0,
            frozen: 1,
            stats: HashMap::new(),
            group_field: None,
            cover: None,
            cover_fill: true,
            compact: false,
            collapsed: vec![],
            date_field: None,
            end_field: None,
            scale: 0,
            locked: false,
            colors: vec![],
            cal_mode: 0,
            color_field: None,
            progress_field: None,
            workdays_only: false,
            workdays: vec![],
            form: FormCfg::default(),
        }
    }

    pub fn row_height(&self) -> f32 {
        [34., 56., 88., 128.][self.row_h.min(3) as usize]
    }

    /// 是否为工作日（day_index：0 周一 … 6 周日）。
    pub fn is_workday(&self, day_index: u8) -> bool {
        if self.workdays.is_empty() {
            day_index < 5
        } else {
            self.workdays.contains(&day_index)
        }
    }
}

// ───────────── 数据表与多维表格 ─────────────

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BaseMeta {
    pub name: String,
    pub tables: Vec<String>,
    #[serde(default)]
    pub active: String,
    /// 仪表盘（R14，和数据表平级显示在侧栏）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboards: Vec<Dashboard>,
    /// 本地自动化（R15）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automations: Vec<Automation>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

/// 仪表盘组件：metric 指标卡 / bar 柱状图 / hbar 条形图 / line 折线图 / pie 饼图 / pivot 透视表 / text 文本。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Widget {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub table: String,
    /// 只统计该视图里的记录（空为全表）
    pub view: String,
    /// 维度（分组）字段；透视表的列维度为 `dim2`
    pub dim: String,
    pub dim2: String,
    /// 指标字段（空为记录数）与汇总方式 count / sum / avg / max / min
    pub field: String,
    pub agg: String,
    pub filter: Filter,
    /// 0 不限
    pub top: usize,
    pub desc: bool,
    pub text: String,
    /// 网格位置（12 列）
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub on: bool,
    /// create 新增记录 / match 记录满足条件 / change 字段被修改 / schedule 定时 / button 点击按钮
    pub trigger: String,
    pub table: String,
    pub cond: Filter,
    pub field: String,
    /// 定时："daily HH:MM" / "weekly N HH:MM"（N 为 1-7）/ "monthly D HH:MM"
    pub schedule: String,
    pub actions: Vec<AutoAction>,
}

/// 动作：update 更新当前记录 / create 新增记录 / notify 系统通知。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoAction {
    pub kind: String,
    pub table: String,
    pub sets: Vec<FieldSet>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableMeta {
    pub id: String,
    pub name: String,
    /// 字段顺序（第一个为主字段 / 索引列）
    pub fields: Vec<String>,
    pub views: Vec<String>,
    #[serde(default)]
    pub active_view: String,
    #[serde(default)]
    pub next_seq: u64,
    /// 记录层级：父记录字段（本表单向关联、只允许一条，R1）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_field: Option<String>,
    /// 字段编组（所有视图共用，R6）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<FieldGroup>,
    /// 记录详情布局：0 默认单列 1 多列 2 按字段编组（R7）
    #[serde(default)]
    pub detail_layout: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

pub const MAX_FIELD_GROUPS: usize = 300;
/// 父 + 4 级子记录
pub const MAX_DEPTH: usize = 5;

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub meta: TableMeta,
    pub fields: HashMap<String, Field>,
    pub records: HashMap<String, Record>,
    /// 记录按 rank 排好的顺序
    pub order: Vec<String>,
    pub views: HashMap<String, View>,
    order_dirty: bool,
    /// 当前最大 rank（同一次提交里连续新建记录时也能排在最后）
    max_rank: f64,
    /// 顺序变过，提交时检查一次 rank 是否需要重排
    rank_check: bool,
}

impl Default for TableMeta {
    fn default() -> Self {
        TableMeta {
            id: new_id("tbl"),
            name: String::new(),
            fields: vec![],
            views: vec![],
            active_view: String::new(),
            next_seq: 1,
            parent_field: None,
            groups: vec![],
            detail_layout: 0,
        }
    }
}

impl Table {
    pub fn assemble(meta: TableMeta, fields: Vec<Field>, records: Vec<Record>, views: Vec<View>) -> Table {
        let mut t = Table {
            meta,
            fields: fields.into_iter().map(|f| (f.id.clone(), f)).collect(),
            records: records.into_iter().map(|r| (r.id.clone(), r)).collect(),
            order: vec![],
            views: views.into_iter().map(|v| (v.id.clone(), v)).collect(),
            order_dirty: true,
            max_rank: 0.,
            rank_check: false,
        };
        // 字段 / 视图顺序里缺失的条目（异常数据）去掉，多出来的补上
        t.meta.fields.retain(|f| t.fields.contains_key(f));
        for f in t.fields.keys() {
            if !t.meta.fields.contains(f) {
                t.meta.fields.push(f.clone());
            }
        }
        t.meta.views.retain(|v| t.views.contains_key(v));
        t.reorder();
        t
    }

    pub fn field_list(&self) -> Vec<&Field> {
        self.meta.fields.iter().filter_map(|f| self.fields.get(f)).collect()
    }
    pub fn primary(&self) -> Option<&Field> {
        self.meta.fields.first().and_then(|f| self.fields.get(f))
    }
    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.values().find(|f| f.name == name)
    }
    pub fn records_in_order(&self) -> impl Iterator<Item = &Record> {
        self.order.iter().filter_map(|id| self.records.get(id))
    }
    fn reorder(&mut self) {
        let mut v: Vec<(&String, f64)> = self.records.iter().map(|(k, r)| (k, r.rank)).collect();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(b.0)));
        self.max_rank = v.last().map(|x| x.1).unwrap_or(0.);
        self.order = v.into_iter().map(|(k, _)| k.clone()).collect();
        self.order_dirty = false;
        self.rank_check = true;
    }
    /// 视图里的字段顺序：视图顺序 + 新增字段补齐；主字段固定在最前。
    pub fn view_fields(&self, v: &View) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        if let Some(p) = self.meta.fields.first() {
            out.push(p.clone());
        }
        for f in v.order.iter().chain(self.meta.fields.iter()) {
            if self.fields.contains_key(f) && !out.contains(f) {
                out.push(f.clone());
            }
        }
        out
    }
    /// 视图中可见的字段（主字段总是可见）。
    pub fn visible_fields(&self, v: &View) -> Vec<String> {
        let all = self.view_fields(v);
        all.into_iter().enumerate().filter(|(i, f)| *i == 0 || !v.hidden.contains(f)).map(|(_, f)| f).collect()
    }
    pub fn last_rank(&self) -> f64 {
        self.max_rank
    }
}

/// 两个 rank 之间取中点。
pub fn rank_between(a: Option<f64>, b: Option<f64>) -> f64 {
    match (a, b) {
        (Some(a), Some(b)) => (a + b) / 2.,
        (Some(a), None) => a + 1.,
        (None, Some(b)) => b - 1.,
        (None, None) => 1.,
    }
}

#[derive(Clone, Debug)]
pub enum Change {
    Meta(BaseMeta, BaseMeta),
    Table(String, Option<TableMeta>, Option<TableMeta>),
    Field(String, String, Option<Field>, Option<Field>),
    Record(String, String, Option<Record>, Option<Record>),
    View(String, String, Option<View>, Option<View>),
}

impl Change {
    fn key(&self) -> (u8, String, String) {
        match self {
            Change::Meta(..) => (0, String::new(), String::new()),
            Change::Table(t, ..) => (1, t.clone(), String::new()),
            Change::Field(t, f, ..) => (2, t.clone(), f.clone()),
            Change::Record(t, r, ..) => (3, t.clone(), r.clone()),
            Change::View(t, v, ..) => (4, t.clone(), v.clone()),
        }
    }
    fn merge_after(&mut self, later: &Change) {
        match (self, later) {
            (Change::Meta(_, a), Change::Meta(_, b)) => *a = b.clone(),
            (Change::Table(_, _, a), Change::Table(_, _, b)) => *a = b.clone(),
            (Change::Field(_, _, _, a), Change::Field(_, _, _, b)) => *a = b.clone(),
            (Change::Record(_, _, _, a), Change::Record(_, _, _, b)) => *a = b.clone(),
            (Change::View(_, _, _, a), Change::View(_, _, _, b)) => *a = b.clone(),
            _ => {}
        }
    }
    fn is_noop(&self) -> bool {
        match self {
            Change::Meta(a, b) => a == b,
            Change::Table(_, a, b) => a == b,
            Change::Field(_, _, a, b) => a == b,
            Change::Record(_, _, a, b) => a == b,
            Change::View(_, _, a, b) => a == b,
        }
    }
}

struct UndoStep {
    changes: Vec<Change>,
    merge: Option<String>,
    at: i64,
}

pub struct Base {
    pub id: String,
    pub meta: BaseMeta,
    pub tables: HashMap<String, Table>,
    pending: Vec<Change>,
    undo: std::collections::VecDeque<UndoStep>,
    redo: Vec<UndoStep>,
    /// 每次提交 +1，界面与计算缓存据此失效
    pub ver: u64,
    /// 是否写数据库（单元测试里关掉）
    pub persist: bool,
    pub(crate) cache: RefCell<super::calc::Cache>,
    /// 自动化运行日志（最近 100 条，运行期）
    pub auto_log: std::collections::VecDeque<AutoLog>,
    /// 自动化产生的系统通知，界面层取走后弹出
    pub notices: Vec<(String, String)>,
    /// 定时自动化上次运行的分钟
    sched_last: HashMap<String, i64>,
    /// 最近若干次提交改了什么（视图结果据此增量更新，见 `calc::refresh_rows`）
    pub log: std::collections::VecDeque<CommitInfo>,
    /// 装载时读不出来的数据表 id：界面上不显示，但保存元数据时原样保留（不能因为读失败就把表从元数据里抹掉）
    pub unreadable: Vec<String>,
}

/// 一次提交的摘要：改动的记录、是否改了字段定义、哪些计算列被整列失效。
#[derive(Clone, Debug, Default)]
pub struct CommitInfo {
    pub ver: u64,
    pub structural: bool,
    pub recs: HashMap<String, Vec<String>>,
    pub cols: HashSet<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct AutoLog {
    pub at: i64,
    pub name: String,
    pub ok: bool,
    pub msg: String,
}

/// 自动化单次触发最多级联的层数（防止 A 改 B、B 又改 A 死循环）
const AUTO_DEPTH: usize = 3;

impl Base {
    pub fn empty(id: &str) -> Self {
        Base {
            id: id.into(),
            meta: BaseMeta::default(),
            tables: HashMap::new(),
            pending: vec![],
            undo: Default::default(),
            redo: vec![],
            ver: 1,
            persist: true,
            cache: RefCell::new(Default::default()),
            auto_log: Default::default(),
            notices: vec![],
            sched_last: HashMap::new(),
            log: Default::default(),
            unreadable: vec![],
        }
    }

    pub fn from_parts(id: &str, meta: BaseMeta, tables: Vec<Table>) -> Self {
        let mut b = Base::empty(id);
        b.meta = meta;
        for mut t in tables {
            t.reorder();
            b.tables.insert(t.meta.id.clone(), t);
        }
        b
    }

    pub fn table(&self, tid: &str) -> Option<&Table> {
        self.tables.get(tid)
    }

    /// 落盘用的元数据：把装载时读不出来的数据表补回去，保证读失败不会改动持久化的表清单。
    pub fn meta_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        if self.unreadable.is_empty() {
            return crate::assets::encode(&self.meta);
        }
        let mut meta = self.meta.clone();
        for t in &self.unreadable {
            if !meta.tables.contains(t) {
                meta.tables.push(t.clone());
            }
        }
        crate::assets::encode(&meta)
    }

    pub fn table_list(&self) -> Vec<&Table> {
        self.meta.tables.iter().filter_map(|t| self.tables.get(t)).collect()
    }

    pub fn field(&self, tid: &str, fid: &str) -> Option<&Field> {
        self.tables.get(tid)?.fields.get(fid)
    }

    pub fn record(&self, tid: &str, rid: &str) -> Option<&Record> {
        self.tables.get(tid)?.records.get(rid)
    }

    pub fn view(&self, tid: &str, vid: &str) -> Option<&View> {
        self.tables.get(tid)?.views.get(vid)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // ── 基本修改 ──

    pub fn set_meta(&mut self, m: BaseMeta) {
        let before = self.meta.clone();
        self.meta = m.clone();
        self.pending.push(Change::Meta(before, m));
    }

    pub fn put_table(&mut self, m: TableMeta) {
        let before = self.tables.get(&m.id).map(|t| t.meta.clone());
        // 改名会影响跨表公式，层级字段影响视图结构：这两种才需要失效
        let structural = before.as_ref().is_none_or(|b| b.name != m.name || b.parent_field != m.parent_field);
        let t = self.tables.entry(m.id.clone()).or_default();
        t.meta = m.clone();
        let tid = m.id.clone();
        self.pending.push(Change::Table(m.id.clone(), before, Some(m)));
        if structural {
            self.invalidate_table(&tid);
        }
    }

    /// 删除整张数据表（连同字段、记录、视图，撤销时一并恢复）。
    pub fn del_table(&mut self, tid: &str) {
        let Some(t) = self.tables.get(tid) else { return };
        let fids: Vec<String> = t.fields.keys().cloned().collect();
        let rids: Vec<String> = t.records.keys().cloned().collect();
        let vids: Vec<String> = t.views.keys().cloned().collect();
        for r in rids {
            self.del_record(tid, &r);
        }
        for f in fids {
            self.del_field(tid, &f);
        }
        for v in vids {
            self.del_view(tid, &v);
        }
        let before = self.tables.remove(tid).map(|t| t.meta);
        self.pending.push(Change::Table(tid.into(), before, None));
    }

    pub fn put_field(&mut self, tid: &str, f: Field) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.fields.insert(f.id.clone(), f.clone());
        self.pending.push(Change::Field(tid.into(), f.id.clone(), before, Some(f)));
        self.invalidate_table(tid);
    }

    pub fn del_field(&mut self, tid: &str, fid: &str) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.fields.remove(fid);
        self.pending.push(Change::Field(tid.into(), fid.into(), before, None));
        self.invalidate_table(tid);
    }

    pub fn put_record(&mut self, tid: &str, r: Record) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.records.insert(r.id.clone(), r.clone());
        if before.as_ref().map(|b| b.rank) != Some(r.rank) {
            t.order_dirty = true;
        }
        t.max_rank = t.max_rank.max(r.rank);
        let rid = r.id.clone();
        let changed = before.as_ref().map(|b| cell_diff(b, &r));
        self.pending.push(Change::Record(tid.into(), r.id.clone(), before, Some(r)));
        self.invalidate_record(tid, &rid, changed);
    }

    pub fn del_record(&mut self, tid: &str, rid: &str) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.records.remove(rid);
        t.order_dirty = true;
        self.pending.push(Change::Record(tid.into(), rid.into(), before, None));
        self.invalidate_record(tid, rid, None);
    }

    pub fn put_view(&mut self, tid: &str, v: View) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.views.insert(v.id.clone(), v.clone());
        self.pending.push(Change::View(tid.into(), v.id.clone(), before, Some(v)));
    }

    pub fn del_view(&mut self, tid: &str, vid: &str) {
        let Some(t) = self.tables.get_mut(tid) else { return };
        let before = t.views.remove(vid);
        self.pending.push(Change::View(tid.into(), vid.into(), before, None));
    }

    /// 提交本次修改：合并成一步撤销并写库。
    pub fn commit(&mut self) {
        self.commit_merge(None);
    }

    fn log_commit(&mut self, changes: &[Change]) {
        let mut info = CommitInfo { ver: self.ver, ..Default::default() };
        for c in changes {
            match c {
                Change::Field(..) => info.structural = true,
                Change::Record(t, r, ..) => info.recs.entry(t.clone()).or_default().push(r.clone()),
                _ => {}
            }
        }
        info.cols = std::mem::take(&mut self.cache.borrow_mut().dropped);
        self.log.push_back(info);
        if self.log.len() > 64 {
            self.log.pop_front();
        }
    }

    /// 只写库、不进撤销栈（切换视图 / 数据表这类界面状态）：不能清掉重做。
    pub fn commit_silent(&mut self) {
        let changes = coalesce(std::mem::take(&mut self.pending));
        self.finish_order();
        if changes.is_empty() {
            return;
        }
        self.write(&changes, true);
        self.ver += 1;
        self.log_commit(&changes);
    }

    /// `merge` 相同且间隔很短的连续提交合并成一步撤销（连续打字）。
    pub fn commit_merge(&mut self, merge: Option<&str>) {
        let t0 = std::time::Instant::now();
        self.finish_order();
        self.run_automations();
        self.finish_order();
        self.rebalance();
        let changes = coalesce(std::mem::take(&mut self.pending));
        if changes.is_empty() {
            return;
        }
        self.write(&changes, true);
        self.ver += 1;
        self.log_commit(&changes);
        self.redo.clear();
        let t = now();
        if let (Some(m), Some(last)) = (merge, self.undo.back_mut()) {
            if last.merge.as_deref() == Some(m) && t - last.at < 2500 {
                let mut all = std::mem::take(&mut last.changes);
                all.extend(changes);
                last.changes = coalesce(all);
                last.at = t;
                return;
            }
        }
        self.undo.push_back(UndoStep { changes, merge: merge.map(String::from), at: t });
        if self.undo.len() > 200 {
            self.undo.pop_front();
        }
        super::stress::record(&super::stress::COMMIT_US, t0);
    }

    fn finish_order(&mut self) {
        for t in self.tables.values_mut() {
            if t.order_dirty {
                t.reorder();
            }
        }
    }

    /// 分数索引反复取中点会耗尽浮点精度：相邻两条的 rank 太近时按当前顺序整表重排为 1, 2, 3…
    fn rebalance(&mut self) {
        let tids: Vec<String> = self
            .tables
            .iter()
            .filter(|(_, t)| t.rank_check && t.order.windows(2).any(|w| t.records[&w[1]].rank - t.records[&w[0]].rank < 1e-6))
            .map(|(k, _)| k.clone())
            .collect();
        for t in self.tables.values_mut() {
            t.rank_check = false;
        }
        for tid in tids {
            let recs: Vec<Record> = self.tables[&tid].records_in_order().cloned().collect();
            for (i, mut r) in recs.into_iter().enumerate() {
                r.rank = (i + 1) as f64;
                self.put_record(&tid, r);
            }
            self.finish_order();
        }
    }

    pub fn undo(&mut self) -> bool {
        self.pending.clear();
        let Some(step) = self.undo.pop_back() else { return false };
        for c in step.changes.iter().rev() {
            self.apply(c, false);
        }
        self.write(&step.changes, false);
        self.finish_order();
        self.ver += 1;
        self.log_commit(&step.changes);
        self.redo.push(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.pending.clear();
        let Some(step) = self.redo.pop() else { return false };
        for c in step.changes.iter() {
            self.apply(c, true);
        }
        self.write(&step.changes, true);
        self.finish_order();
        self.ver += 1;
        self.log_commit(&step.changes);
        self.undo.push_back(step);
        true
    }

    /// 把实体恢复到修改前（after=false）或修改后的状态。
    fn apply(&mut self, c: &Change, after: bool) {
        let changed = |a: &Option<Record>, b: &Option<Record>| match (a, b) {
            (Some(a), Some(b)) => Some(cell_diff(a, b)),
            _ => None,
        };
        match c {
            Change::Table(tid, ..) | Change::Field(tid, ..) => self.invalidate_table(tid),
            Change::Record(tid, rid, a, b) => self.invalidate_record(tid, rid, changed(a, b)),
            _ => {}
        }
        self.apply_raw(c, after);
        match c {
            Change::Table(tid, ..) | Change::Field(tid, ..) => self.invalidate_table(tid),
            Change::Record(tid, rid, a, b) => self.invalidate_record(tid, rid, changed(a, b)),
            _ => {}
        }
    }

    fn apply_raw(&mut self, c: &Change, after: bool) {
        fn pick<T: Clone>(a: &Option<T>, b: &Option<T>, after: bool) -> Option<T> {
            if after {
                b.clone()
            } else {
                a.clone()
            }
        }
        match c {
            Change::Meta(a, b) => self.meta = if after { b.clone() } else { a.clone() },
            Change::Table(tid, a, b) => match pick(a, b, after) {
                Some(m) => self.tables.entry(tid.clone()).or_default().meta = m,
                None => {
                    self.tables.remove(tid);
                }
            },
            Change::Field(tid, fid, a, b) => {
                let t = self.tables.entry(tid.clone()).or_default();
                match pick(a, b, after) {
                    Some(f) => {
                        t.fields.insert(fid.clone(), f);
                    }
                    None => {
                        t.fields.remove(fid);
                    }
                }
            }
            Change::Record(tid, rid, a, b) => {
                let t = self.tables.entry(tid.clone()).or_default();
                t.order_dirty = true;
                match pick(a, b, after) {
                    Some(r) => {
                        t.records.insert(rid.clone(), r);
                    }
                    None => {
                        t.records.remove(rid);
                    }
                }
            }
            Change::View(tid, vid, a, b) => {
                let t = self.tables.entry(tid.clone()).or_default();
                match pick(a, b, after) {
                    Some(v) => {
                        t.views.insert(vid.clone(), v);
                    }
                    None => {
                        t.views.remove(vid);
                    }
                }
            }
        }
    }

    fn write(&self, changes: &[Change], after: bool) {
        if self.persist {
            super::db::write(&self.id, changes, after);
        }
    }

    /// 首次创建 / 复制后整库写入。
    pub fn save_all(&self) {
        if !self.persist {
            return;
        }
        let mut changes = vec![];
        for t in self.tables.values() {
            changes.push(Change::Table(t.meta.id.clone(), None, Some(t.meta.clone())));
            for f in t.fields.values() {
                changes.push(Change::Field(t.meta.id.clone(), f.id.clone(), None, Some(f.clone())));
            }
            for r in t.records.values() {
                changes.push(Change::Record(t.meta.id.clone(), r.id.clone(), None, Some(r.clone())));
            }
            for v in t.views.values() {
                changes.push(Change::View(t.meta.id.clone(), v.id.clone(), None, Some(v.clone())));
            }
        }
        super::db::write(&self.id, &changes, true);
    }

    /// 深拷贝成一个新的多维表格（id 不变的实体放在新 base_id 下即可）。
    pub fn duplicate(&self, new_id: &str) -> Base {
        let tables = self
            .tables
            .values()
            .map(|t| Table {
                meta: t.meta.clone(),
                fields: t.fields.clone(),
                records: t.records.clone(),
                order: t.order.clone(),
                views: t.views.clone(),
                order_dirty: false,
                max_rank: t.max_rank,
                rank_check: false,
            })
            .collect();
        let mut b = Base::from_parts(new_id, self.meta.clone(), tables);
        b.persist = self.persist;
        b
    }

    // ── 高层操作 ──

    /// 新建一条记录（自动编号、自动填日期），插到 `after_rank` 之后 / 末尾。
    pub fn new_record(&mut self, tid: &str, rank: Option<f64>) -> Option<Record> {
        let t = self.tables.get(tid)?;
        let mut meta = t.meta.clone();
        let seq = meta.next_seq.max(1);
        meta.next_seq = seq + 1;
        let ts = now();
        let mut r = Record { id: new_id("rec"), cells: HashMap::new(), rank: rank.unwrap_or_else(|| t.last_rank() + 1.), seq, created: ts, updated: ts };
        for f in t.fields.values() {
            if f.kind == FieldType::Date && f.prop.auto_fill {
                r.cells.insert(f.id.clone(), Value::from(ts));
            } else if let Some(v) = default_value(f) {
                r.cells.insert(f.id.clone(), v);
            }
        }
        self.put_table(meta);
        self.put_record(tid, r.clone());
        Some(r)
    }

    /// 设置单元格（双向关联会同步对面表的回链字段）。
    pub fn set_cell(&mut self, tid: &str, rid: &str, fid: &str, v: Value) {
        let Some(f) = self.field(tid, fid).cloned() else { return };
        if f.kind.computed() {
            return;
        }
        let Some(mut r) = self.record(tid, rid).cloned() else { return };
        let old = r.cells.get(fid).cloned();
        let v = if f.kind.is_link() && f.prop.single {
            match v {
                Value::Array(a) => Value::Array(a.into_iter().rev().take(1).collect()),
                v => v,
            }
        } else {
            v
        };
        if is_empty_value(Some(&v)) && f.kind != FieldType::Checkbox {
            r.cells.remove(fid);
        } else {
            r.cells.insert(fid.into(), v.clone());
        }
        if old.as_ref() == r.cells.get(fid) {
            return;
        }
        r.updated = now();
        self.put_record(tid, r);
        if f.kind == FieldType::DuplexLink && !f.prop.back.is_empty() {
            let before: HashSet<String> = str_list(old.as_ref()).into_iter().collect();
            let after: HashSet<String> = str_list(Some(&v)).into_iter().collect();
            for other in after.symmetric_difference(&before) {
                let add = after.contains(other);
                let Some(mut o) = self.record(&f.prop.table, other).cloned() else { continue };
                let mut list = str_list(o.cells.get(&f.prop.back));
                list.retain(|x| x != rid);
                if add {
                    list.push(rid.into());
                }
                if list.is_empty() {
                    o.cells.remove(&f.prop.back);
                } else {
                    o.cells.insert(f.prop.back.clone(), Value::from(list));
                }
                o.updated = now();
                self.put_record(&f.prop.table, o);
            }
        }
    }

    /// 修改字段（含重命名）：本表公式里的 `[旧名]` 一并改成 `[新名]`（改名不会让公式失效）。
    pub fn rename_field(&mut self, tid: &str, f: Field) {
        let old = self.field(tid, &f.id).map(|x| x.name.clone());
        let new_name = f.name.clone();
        let fid = f.id.clone();
        self.put_field(tid, f);
        let Some(old) = old.filter(|o| *o != new_name) else { return };
        let formulas: Vec<Field> =
            self.tables.get(tid).map(|t| t.fields.values().filter(|x| x.kind == FieldType::Formula && x.id != fid).cloned().collect()).unwrap_or_default();
        for mut x in formulas {
            let e = super::formula::rename_ref(&x.prop.expr, &old, &new_name);
            if e != x.prop.expr {
                x.prop.expr = e;
                self.put_field(tid, x);
            }
        }
    }

    /// 复制一条记录插到它后面（双向关联的对面也会多一条回链），返回新记录 id。
    pub fn duplicate_record(&mut self, tid: &str, rid: &str) -> Option<String> {
        let src = self.record(tid, rid)?.clone();
        let t = self.table(tid)?;
        let i = t.order.iter().position(|r| r == rid)?;
        let next = t.order.get(i + 1).and_then(|r| t.records.get(r)).map(|r| r.rank);
        let r = self.new_record(tid, Some(rank_between(Some(src.rank), next)))?;
        let fields: Vec<Field> = self.table(tid)?.fields.values().filter(|f| !f.kind.computed()).cloned().collect();
        for f in fields {
            if let Some(v) = src.cells.get(&f.id) {
                self.set_cell(tid, &r.id, &f.id, v.clone());
            }
        }
        Some(r.id)
    }

    /// 删除记录：双向关联对面的回链同时去掉，其他表指向它的单向关联也清掉，
    /// 子记录挂到被删记录的父记录下（整个操作是同一步撤销）。
    pub fn delete_record(&mut self, tid: &str, rid: &str) {
        if let Some(pf) = self.table(tid).and_then(|t| t.meta.parent_field.clone()) {
            let up = self.parent_of(tid, rid);
            for c in self.children_of(tid, rid) {
                self.set_cell(tid, &c, &pf, up.as_ref().map(|p| Value::from(vec![p.clone()])).unwrap_or(Value::Null));
            }
        }
        let refs: Vec<(String, String, String)> = self
            .tables
            .values()
            .flat_map(|t| {
                t.fields
                    .values()
                    .filter(|f| f.kind == FieldType::Link && f.prop.table == tid && t.meta.parent_field.as_ref() != Some(&f.id))
                    .map(move |f| (t.meta.id.clone(), f.id.clone()))
            })
            .flat_map(|(t, f)| {
                self.tables[&t]
                    .records
                    .values()
                    .filter(|r| str_list(r.cells.get(&f)).iter().any(|x| x == rid))
                    .map(|r| (t.clone(), r.id.clone(), f.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (t, r, f) in refs {
            let left: Vec<String> = str_list(self.record(&t, &r).and_then(|x| x.cells.get(&f))).into_iter().filter(|x| x != rid).collect();
            self.set_cell(&t, &r, &f, Value::from(left));
        }
        let duplex: Vec<String> = self
            .tables
            .get(tid)
            .map(|t| t.fields.values().filter(|f| f.kind == FieldType::DuplexLink && !f.prop.back.is_empty()).map(|f| f.id.clone()).collect())
            .unwrap_or_default();
        for fid in duplex {
            if self.record(tid, rid).is_some_and(|r| r.cells.contains_key(&fid)) {
                self.set_cell(tid, rid, &fid, Value::Array(vec![]));
            }
        }
        self.del_record(tid, rid);
    }

    /// 开启记录层级：新建「父记录」字段（本表单向关联、只允许一条）。返回字段 id。
    pub fn enable_hierarchy(&mut self, tid: &str) -> Option<String> {
        let mut m = self.table(tid)?.meta.clone();
        if let Some(p) = m.parent_field.clone().filter(|p| self.field(tid, p).is_some()) {
            return Some(p);
        }
        let mut f = Field::new("父记录", FieldType::Link);
        if self.table(tid)?.field_by_name("父记录").is_some() {
            f.name = "父记录（层级）".into();
        }
        f.prop.table = tid.into();
        f.prop.single = true;
        let fid = f.id.clone();
        m.fields.push(fid.clone());
        m.parent_field = Some(fid.clone());
        self.put_field(tid, f);
        self.put_table(m);
        Some(fid)
    }

    /// 设置父记录：检查不能成环、层级不超过 5 层。
    pub fn set_parent(&mut self, tid: &str, rid: &str, parent: Option<&str>) -> Result<(), String> {
        let pf = self.table(tid).and_then(|t| t.meta.parent_field.clone()).ok_or("还没有开启记录层级")?;
        if let Some(p) = parent {
            if p == rid {
                return Err("不能把记录设为自己的子记录".into());
            }
            let mut cur = Some(p.to_string());
            while let Some(c) = cur {
                if c == rid {
                    return Err("不能把记录移到它自己的子记录下面".into());
                }
                cur = self.parent_of(tid, &c);
            }
            if self.depth_of(tid, p) + 1 + self.subtree_height(tid, rid) >= MAX_DEPTH {
                return Err(format!("最多支持 {MAX_DEPTH} 层（父记录 + 4 级子记录）"));
            }
        }
        self.set_cell(tid, rid, &pf, parent.map(|p| Value::from(vec![p.to_string()])).unwrap_or(Value::Null));
        Ok(())
    }

    /// 按「字段 = 值」批量设置（按钮、自动化）；值支持 today / now / {{[字段]}} 引用本记录字段。
    pub fn apply_sets(&mut self, tid: &str, rid: &str, sets: &[FieldSet]) {
        for s in sets {
            let Some(mut f) = self.field(tid, &s.field).cloned() else { continue };
            let v = match &s.value {
                Value::String(x) if x == "today" => Value::from(super::formula::today()),
                Value::String(x) if x == "now" => Value::from(now()),
                Value::String(x) if x.contains("{{") => {
                    let text = self.template(tid, rid, x);
                    super::io::text_to_value(&mut f, &text).unwrap_or(Value::Null)
                }
                Value::String(x) if !matches!(f.kind, FieldType::Text | FieldType::Url | FieldType::Phone | FieldType::Email) => {
                    super::io::text_to_value(&mut f, x).unwrap_or(Value::Null)
                }
                v => v.clone(),
            };
            if f.kind.has_options() {
                self.put_field(tid, f);
            }
            self.set_cell(tid, rid, &s.field, v);
        }
    }

    /// 把文本里的 `{{[字段]}}` 换成本记录该字段的显示文本。
    pub fn template(&self, tid: &str, rid: &str, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("{{") {
            out.push_str(&rest[..i]);
            let Some(j) = rest[i..].find("}}") else {
                rest = &rest[i..];
                break;
            };
            let name = rest[i + 2..i + j].trim().trim_start_matches('[').trim_end_matches(']').trim();
            if let Some(f) = self.table(tid).and_then(|t| t.field_by_name(name)) {
                out.push_str(&self.text(tid, rid, &f.id.clone()));
            }
            rest = &rest[i + j + 2..];
        }
        out.push_str(rest);
        out
    }

    // ── 自动化（R15） ──

    /// 本次提交里的记录修改触发自动化；动作产生的修改并进同一步撤销，最多级联 3 层。
    fn run_automations(&mut self) {
        let autos: Vec<Automation> =
            self.meta.automations.iter().filter(|a| a.on && matches!(a.trigger.as_str(), "create" | "match" | "change")).cloned().collect();
        if autos.is_empty() {
            return;
        }
        let mut from = 0;
        for _ in 0..AUTO_DEPTH {
            let batch = coalesce(self.pending[from..].to_vec());
            from = self.pending.len();
            let mut fired = vec![];
            for ch in &batch {
                let Change::Record(tid, rid, before, Some(after)) = ch else { continue };
                for a in autos.iter().filter(|a| &a.table == tid) {
                    let ok = |b: &Base| a.cond.conds.is_empty() || b.record_matches(tid, rid, &a.cond);
                    let hit = match a.trigger.as_str() {
                        "create" => before.is_none() && ok(self),
                        "change" => before.as_ref().is_some_and(|b| b.cells.get(&a.field) != after.cells.get(&a.field)) && ok(self),
                        _ => {
                            // 记录「变成」满足条件时触发：用修改前的记录再判断一次
                            let now = !a.cond.conds.is_empty() && ok(self);
                            now && match before {
                                None => true,
                                Some(b) => !self.matches_as(tid, b, &a.cond),
                            }
                        }
                    };
                    if hit {
                        fired.push((a.clone(), tid.clone(), rid.clone()));
                    }
                }
            }
            if fired.is_empty() {
                break;
            }
            for (a, tid, rid) in fired {
                self.run_actions(&a, &tid, Some(&rid));
            }
        }
    }

    /// 用记录的另一个版本判断是否满足筛选（判断完恢复）。
    fn matches_as(&mut self, tid: &str, r: &Record, filter: &Filter) -> bool {
        let Some(t) = self.tables.get_mut(tid) else { return false };
        let cur = t.records.insert(r.id.clone(), r.clone());
        self.invalidate_record(tid, &r.id, None);
        let ok = self.record_matches(tid, &r.id, filter);
        if let (Some(t), Some(c)) = (self.tables.get_mut(tid), cur) {
            t.records.insert(r.id.clone(), c);
        }
        self.invalidate_record(tid, &r.id, None);
        ok
    }

    /// 执行一条自动化的动作。`rid` 为触发的记录（定时触发时为空：更新动作作用于所有满足条件的记录）。
    pub fn run_actions(&mut self, a: &Automation, tid: &str, rid: Option<&str>) {
        let mut msgs = vec![];
        for act in &a.actions {
            match act.kind.as_str() {
                "update" => {
                    let targets: Vec<String> = match rid {
                        Some(r) => vec![r.to_string()],
                        None => self
                            .table(tid)
                            .map(|t| t.order.iter().filter(|r| a.cond.conds.is_empty() || self.record_matches(tid, r, &a.cond)).cloned().collect())
                            .unwrap_or_default(),
                    };
                    for r in &targets {
                        self.apply_sets(tid, r, &act.sets);
                    }
                    msgs.push(format!("更新了 {} 条记录", targets.len()));
                }
                "create" => {
                    let t2 = if act.table.is_empty() { tid.to_string() } else { act.table.clone() };
                    let Some(nr) = self.new_record(&t2, None) else {
                        msgs.push("目标数据表不存在".into());
                        continue;
                    };
                    for st in &act.sets {
                        let Some(mut f) = self.field(&t2, &st.field).cloned() else { continue };
                        let v = match (&st.value, rid) {
                            (Value::String(x), Some(r)) if x.contains("{{") => Value::from(self.template(tid, r, x)),
                            (v, _) => v.clone(),
                        };
                        let v = match v {
                            Value::String(x) if x == "today" => Value::from(super::formula::today()),
                            Value::String(x) if !matches!(f.kind, FieldType::Text | FieldType::Url | FieldType::Phone | FieldType::Email) => {
                                super::io::text_to_value(&mut f, &x).unwrap_or(Value::Null)
                            }
                            v => v,
                        };
                        if f.kind.has_options() {
                            self.put_field(&t2, f);
                        }
                        self.set_cell(&t2, &nr.id, &st.field, v);
                    }
                    msgs.push("新增了 1 条记录".into());
                }
                "notify" => {
                    let text = match rid {
                        Some(r) => self.template(tid, r, &act.text),
                        None => act.text.clone(),
                    };
                    self.notices.push((a.name.clone(), text.clone()));
                    msgs.push(format!("通知：{text}"));
                }
                _ => {}
            }
        }
        self.auto_log.push_back(AutoLog { at: now(), name: a.name.clone(), ok: true, msg: msgs.join("；") });
        if self.auto_log.len() > 100 {
            self.auto_log.pop_front();
        }
    }

    /// 定时自动化：到点的执行一次（同一分钟不重复）。返回是否有修改。
    pub fn run_schedule(&mut self, now_ms: i64) -> bool {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let Some(t) = Local.timestamp_millis_opt(now_ms).single() else { return false };
        let minute = now_ms / 60_000;
        let autos: Vec<Automation> = self.meta.automations.iter().filter(|a| a.on && a.trigger == "schedule").cloned().collect();
        let mut ran = false;
        for a in autos {
            let parts: Vec<&str> = a.schedule.split_whitespace().collect();
            let hm = |s: &str| s.split_once(':').and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)));
            let due = match parts.as_slice() {
                ["daily", at] => hm(at) == Some((t.hour(), t.minute())),
                ["weekly", d, at] => d.parse::<u32>().ok() == Some(t.weekday().number_from_monday()) && hm(at) == Some((t.hour(), t.minute())),
                ["monthly", d, at] => d.parse::<u32>().ok() == Some(t.day()) && hm(at) == Some((t.hour(), t.minute())),
                _ => false,
            };
            if due && self.sched_last.get(&a.id) != Some(&minute) {
                self.sched_last.insert(a.id.clone(), minute);
                let tid = a.table.clone();
                self.run_actions(&a, &tid, None);
                ran = true;
            }
        }
        if ran {
            self.commit();
        }
        ran
    }

    /// 点击按钮字段：修改字段、触发「点击按钮」自动化；返回要打开的链接。
    pub fn press_button(&mut self, tid: &str, rid: &str, fid: &str) -> Option<String> {
        let f = self.field(tid, fid)?.clone();
        self.apply_sets(tid, rid, &f.prop.sets);
        if !f.prop.auto.is_empty() {
            if let Some(a) = self.meta.automations.iter().find(|a| a.id == f.prop.auto && a.on).cloned() {
                self.run_actions(&a, tid, Some(rid));
            }
        }
        let url = self.template(tid, rid, &f.prop.url);
        (!url.trim().is_empty()).then(|| if url.contains("://") { url } else { format!("https://{url}") })
    }

    /// 新建数据表：默认四个字段、一个表格视图、若干空记录。
    pub fn add_table(&mut self, name: &str, kind: ViewKind, rows: usize) -> String {
        let mut meta = TableMeta { name: name.into(), ..Default::default() };
        let tid = meta.id.clone();
        let mut fields = vec![
            Field::new("文本", FieldType::Text),
            Field::new("单选", FieldType::Select),
            Field::new("日期", FieldType::Date),
            Field::new("附件", FieldType::Attachment),
        ];
        if kind == ViewKind::Kanban {
            fields[1].name = "状态".into();
            for n in ["待处理", "进行中", "已完成"] {
                fields[1].ensure_opt(n);
            }
            fields[1].prop.options[0].color = 11;
            fields[1].prop.options[1].color = 0;
            fields[1].prop.options[2].color = 3;
        }
        meta.fields = fields.iter().map(|f| f.id.clone()).collect();
        let mut views = vec![View::new(kind.label(), kind)];
        if kind != ViewKind::Grid {
            views.insert(0, View::new(ViewKind::Grid.label(), ViewKind::Grid));
        }
        let main = views.iter().find(|v| v.kind == kind).map(|v| v.id.clone()).unwrap_or_default();
        for v in views.iter_mut() {
            match v.kind {
                ViewKind::Kanban => v.group_field = Some(fields[1].id.clone()),
                ViewKind::Gallery => v.cover = Some(fields[3].id.clone()),
                _ => {}
            }
        }
        meta.views = views.iter().map(|v| v.id.clone()).collect();
        meta.active_view = main;
        self.tables.insert(tid.clone(), Table::default());
        self.put_table(meta);
        for f in fields {
            self.put_field(&tid, f);
        }
        for v in views {
            self.put_view(&tid, v);
        }
        for _ in 0..rows {
            self.new_record(&tid, None);
        }
        let mut m = self.meta.clone();
        m.tables.push(tid.clone());
        m.active = tid.clone();
        self.set_meta(m);
        tid
    }

    /// 预设模板（斜杠菜单「任务看板」「日程」）：字段和视图都配好的多维表格。
    /// - `tasks`：标题 / 状态 / 截止日期 / 负责人；看板（按状态分列）+ 表格 + 日历（按截止日期）
    /// - `schedule`：事项 / 日期 / 类型 / 备注；日历 + 表格
    pub fn new_preset(id: &str, preset: &str) -> Base {
        let mut b = Base::empty(id);
        let opts = |f: &mut Field, names: &[&str], colors: &[u8]| {
            for (n, c) in names.iter().zip(colors) {
                f.ensure_opt(n);
                if let Some(o) = f.prop.options.last_mut() {
                    o.color = *c;
                }
            }
        };
        let (name, table, fields, views, rows): (&str, &str, Vec<Field>, Vec<ViewKind>, usize) = match preset {
            "schedule" => {
                let mut kind = Field::new("类型", FieldType::Select);
                opts(&mut kind, &["会议", "里程碑", "提醒", "其他"], &[0, 3, 11, 7]);
                (
                    "日程",
                    "日程",
                    vec![Field::new("事项", FieldType::Text), Field::new("日期", FieldType::Date), kind, Field::new("备注", FieldType::Text)],
                    vec![ViewKind::Calendar, ViewKind::Grid],
                    0,
                )
            }
            _ => {
                let mut status = Field::new("状态", FieldType::Select);
                opts(&mut status, &["待办", "进行中", "完成"], &[11, 0, 3]);
                (
                    "任务看板",
                    "任务",
                    vec![Field::new("标题", FieldType::Text), status, Field::new("截止日期", FieldType::Date), Field::new("负责人", FieldType::Person)],
                    vec![ViewKind::Kanban, ViewKind::Grid, ViewKind::Calendar],
                    3,
                )
            }
        };
        b.meta.name = name.into();
        let mut meta = TableMeta { name: table.into(), ..Default::default() };
        let tid = meta.id.clone();
        let select = fields.iter().find(|f| f.kind == FieldType::Select).map(|f| f.id.clone());
        let date = fields.iter().find(|f| f.kind == FieldType::Date).map(|f| f.id.clone());
        let mut vs: Vec<View> = views.iter().map(|k| View::new(k.label(), *k)).collect();
        for v in vs.iter_mut() {
            match v.kind {
                ViewKind::Kanban => v.group_field = select.clone(),
                ViewKind::Calendar => {
                    v.date_field = date.clone();
                    v.color_field = select.clone();
                }
                _ => {}
            }
        }
        meta.fields = fields.iter().map(|f| f.id.clone()).collect();
        meta.views = vs.iter().map(|v| v.id.clone()).collect();
        meta.active_view = vs[0].id.clone();
        b.tables.insert(tid.clone(), Table::default());
        b.put_table(meta);
        for f in fields {
            b.put_field(&tid, f);
        }
        for v in vs {
            b.put_view(&tid, v);
        }
        for _ in 0..rows {
            b.new_record(&tid, None);
        }
        let mut m = b.meta.clone();
        m.tables.push(tid.clone());
        m.active = tid;
        b.set_meta(m);
        b.pending.clear();
        b.finish_order();
        b
    }

    /// 一个全新的多维表格（斜杠菜单「表格 / 看板 / 画册」）。
    pub fn new_default(id: &str, kind: ViewKind) -> Base {
        let mut b = Base::empty(id);
        b.meta.name = "多维表格".into();
        b.add_table("数据表", kind, if kind == ViewKind::Grid { 5 } else { 3 });
        b.pending.clear();
        b.finish_order();
        b
    }
}

/// 两个版本的记录之间改动了哪些字段（rank 变化不算字段改动）。
fn cell_diff(a: &Record, b: &Record) -> Vec<String> {
    let mut out: Vec<String> = a.cells.iter().filter(|(k, v)| b.cells.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
    out.extend(b.cells.keys().filter(|k| !a.cells.contains_key(*k)).cloned());
    out
}

/// 同一实体的多次修改合并为一条（保留第一次的「前」和最后一次的「后」），并去掉无变化的条目。
fn coalesce(changes: Vec<Change>) -> Vec<Change> {
    let mut out: Vec<Change> = vec![];
    let mut ix: HashMap<(u8, String, String), usize> = HashMap::new();
    for c in changes {
        let k = c.key();
        if let Some(&i) = ix.get(&k) {
            out[i].merge_after(&c);
        } else {
            ix.insert(k, out.len());
            out.push(c);
        }
    }
    out.retain(|c| !c.is_noop());
    out
}

#[cfg(test)]
mod tests {
    use super::{Base, BaseMeta, FieldType, Value, ViewKind};

    #[test]
    fn unreadable_tables_stay_in_persisted_meta() {
        let mut b = base();
        b.unreadable = vec!["lost".into()];
        let meta: BaseMeta = serde_json::from_slice(&b.meta_bytes().unwrap()).unwrap();
        assert!(meta.tables.contains(&"lost".to_string()), "读不出来的表不能从元数据里抹掉");
        assert!(!b.meta.tables.contains(&"lost".to_string()), "但界面上不显示");
    }

    fn base() -> Base {
        let mut b = Base::new_default("t", ViewKind::Grid);
        b.persist = false;
        b
    }

    #[test]
    fn presets_wire_views_to_fields() {
        for (p, first) in [("tasks", ViewKind::Kanban), ("schedule", ViewKind::Calendar)] {
            let mut b = Base::new_preset("t", p);
            b.persist = false;
            let t = b.table(&b.meta.active).unwrap();
            assert_eq!(t.meta.fields.len(), 4);
            let v = &t.views[&t.meta.active_view];
            assert_eq!(v.kind, first);
            let sel = t.meta.fields.iter().find(|f| t.fields[*f].kind == FieldType::Select).unwrap();
            assert!(t.fields[sel].prop.options.len() >= 3);
            for v in t.views.values() {
                match v.kind {
                    ViewKind::Kanban => assert_eq!(v.group_field.as_ref(), Some(sel)),
                    ViewKind::Calendar => assert!(v.date_field.as_ref().is_some_and(|d| t.fields[d].kind == FieldType::Date)),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn default_base_has_table_fields_views() {
        let b = base();
        let t = b.table(&b.meta.active).unwrap();
        assert_eq!(t.meta.fields.len(), 4);
        assert_eq!(t.records.len(), 5);
        assert_eq!(t.order.len(), 5);
        assert_eq!(t.views.len(), 1);
    }

    #[test]
    fn records_created_in_one_commit_keep_order() {
        let mut b = base();
        let tid = b.meta.active.clone();
        let ids: Vec<String> = (0..4).map(|_| b.new_record(&tid, None).unwrap().id).collect();
        b.commit();
        let order = &b.table(&tid).unwrap().order;
        assert_eq!(&order[order.len() - 4..], &ids[..]);
    }

    #[test]
    fn undo_redo_cell() {
        let mut b = base();
        let tid = b.meta.active.clone();
        let rid = b.table(&tid).unwrap().order[0].clone();
        let fid = b.table(&tid).unwrap().meta.fields[0].clone();
        b.set_cell(&tid, &rid, &fid, Value::from("你好"));
        b.commit();
        assert_eq!(b.record(&tid, &rid).unwrap().cells[&fid], Value::from("你好"));
        assert!(b.undo());
        assert!(!b.record(&tid, &rid).unwrap().cells.contains_key(&fid));
        assert!(b.redo());
        assert_eq!(b.record(&tid, &rid).unwrap().cells[&fid], Value::from("你好"));
    }

    #[test]
    fn typing_merges_into_one_undo_step() {
        let mut b = base();
        let tid = b.meta.active.clone();
        let rid = b.table(&tid).unwrap().order[0].clone();
        let fid = b.table(&tid).unwrap().meta.fields[0].clone();
        for s in ["a", "ab", "abc"] {
            b.set_cell(&tid, &rid, &fid, Value::from(s));
            b.commit_merge(Some("cell"));
        }
        assert!(b.undo());
        assert!(!b.record(&tid, &rid).unwrap().cells.contains_key(&fid));
        assert!(!b.can_undo());
    }

    #[test]
    fn delete_table_undo_restores_everything() {
        let mut b = base();
        let tid = b.meta.active.clone();
        let n = b.table(&tid).unwrap().records.len();
        b.del_table(&tid);
        let mut m = b.meta.clone();
        m.tables.retain(|t| t != &tid);
        b.set_meta(m);
        b.commit();
        assert!(b.table(&tid).is_none());
        b.undo();
        let t = b.table(&tid).unwrap();
        assert_eq!(t.records.len(), n);
        assert_eq!(t.order.len(), n);
        assert_eq!(t.fields.len(), 4);
    }

    #[test]
    fn duplex_link_syncs_back_field() {
        let mut b = base();
        let t1 = b.meta.active.clone();
        let t2 = b.add_table("客户", ViewKind::Grid, 2);
        let mut f1 = super::Field::new("客户", FieldType::DuplexLink);
        let mut f2 = super::Field::new("订单", FieldType::DuplexLink);
        f1.prop.table = t2.clone();
        f1.prop.back = f2.id.clone();
        f2.prop.table = t1.clone();
        f2.prop.back = f1.id.clone();
        b.put_field(&t1, f1.clone());
        b.put_field(&t2, f2.clone());
        b.commit();
        let r1 = b.table(&t1).unwrap().order[0].clone();
        let c1 = b.table(&t2).unwrap().order[1].clone();
        b.set_cell(&t1, &r1, &f1.id, Value::from(vec![c1.clone()]));
        b.commit();
        assert_eq!(super::str_list(b.record(&t2, &c1).unwrap().cells.get(&f2.id)), vec![r1.clone()]);
        b.set_cell(&t1, &r1, &f1.id, Value::from(Vec::<String>::new()));
        b.commit();
        assert!(super::str_list(b.record(&t2, &c1).unwrap().cells.get(&f2.id)).is_empty());
    }
}
