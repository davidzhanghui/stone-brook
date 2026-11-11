//! 画布引擎（P8-1）。
//!
//! 文本绘图、流程图、UML、画板、思维导图共用同一套「场景」描述：
//! 一组形状 / 连线 / 文字 / 路径 / 图片，坐标为世界坐标（缩放 1 时等于像素）。
//! - `paint.rs`：用 GPUI 绘制场景（视口裁剪、缩放、手绘风格）
//! - `svg.rs`：导出 SVG（再经 resvg 得到 PNG）
//! - `geom.rs`：形状轮廓、箭头、曲线等几何计算

pub mod geom;
pub mod math;
pub mod paint;
pub mod rich;
pub mod svg;

pub use rich::{Fmt, Span};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

pub fn pt(x: f32, y: f32) -> Pt {
    Pt { x, y }
}

impl Pt {
    pub fn dist(self, o: Pt) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
    pub fn add(self, o: Pt) -> Pt {
        pt(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: Pt) -> Pt {
        pt(self.x - o.x, self.y - o.y)
    }
    pub fn scale(self, s: f32) -> Pt {
        pt(self.x * s, self.y * s)
    }
    pub fn lerp(self, o: Pt, t: f32) -> Pt {
        pt(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Rect {
    pub fn center(&self) -> Pt {
        pt(self.x + self.w / 2., self.y + self.h / 2.)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        rect(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn inflate(&self, d: f32) -> Rect {
        rect(self.x - d, self.y - d, self.w + 2. * d, self.h + 2. * d)
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn from_points(a: Pt, b: Pt) -> Rect {
        rect(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs())
    }
}

/// 颜色：0xRRGGBBAA
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Col(pub u32);

impl Col {
    pub fn rgb(hex: u32) -> Col {
        Col((hex << 8) | 0xff)
    }
    pub fn with_alpha(self, a: f32) -> Col {
        Col((self.0 & 0xffff_ff00) | ((a.clamp(0., 1.) * 255.) as u32))
    }
    pub fn alpha(self) -> f32 {
        (self.0 & 0xff) as f32 / 255.
    }
    pub fn hsla(self) -> gpui::Hsla {
        gpui::rgba(self.0).into()
    }
    pub fn hex(self) -> String {
        format!("#{:06x}", self.0 >> 8)
    }
    pub fn from_hsla(c: gpui::Hsla) -> Col {
        let r = c.to_rgb();
        Col(((r.r * 255.) as u32) << 24 | ((r.g * 255.) as u32) << 16 | ((r.b * 255.) as u32) << 8 | ((r.a * 255.) as u32))
    }
    /// 解析 `#rgb` / `#rrggbb` / `rgb(r,g,b)` / 常见颜色名
    pub fn parse(s: &str) -> Option<Col> {
        let s = s.trim().to_lowercase();
        if let Some(h) = s.strip_prefix('#') {
            let v = u32::from_str_radix(h, 16).ok()?;
            return match h.len() {
                3 => Some(Col::rgb(((v >> 8) & 0xf) * 0x110000 + ((v >> 4) & 0xf) * 0x1100 + (v & 0xf) * 0x11)),
                6 => Some(Col::rgb(v)),
                8 => Some(Col(v)),
                _ => None,
            };
        }
        if let Some(inner) = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")) {
            let parts: Vec<f32> = inner.trim_end_matches(')').split(',').filter_map(|p| p.trim().parse().ok()).collect();
            if parts.len() >= 3 {
                let a = parts.get(3).copied().unwrap_or(1.);
                return Some(Col::rgb(((parts[0] as u32) << 16) | ((parts[1] as u32) << 8) | parts[2] as u32).with_alpha(a));
            }
        }
        let named = match s.as_str() {
            "red" => 0xe5484d,
            "green" => 0x2ea121,
            "blue" => 0x3b6cf6,
            "yellow" => 0xf5c518,
            "orange" => 0xf5a339,
            "purple" => 0x8b5cf6,
            "pink" => 0xec4899,
            "gray" | "grey" => 0x8f959e,
            "black" => 0x000000,
            "white" => 0xffffff,
            "lightblue" => 0xe1eaff,
            "lightgreen" => 0xd9f5d6,
            "lightyellow" => 0xfff5b8,
            _ => return None,
        };
        Some(Col::rgb(named))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShapeKind {
    #[default]
    Rect,
    RoundRect,
    Stadium,
    Ellipse,
    Circle,
    DoubleCircle,
    Diamond,
    Hexagon,
    Parallelogram,
    ParallelogramAlt,
    Trapezoid,
    TrapezoidAlt,
    Cylinder,
    Document,
    Subroutine,
    Flag,
    Note,
    /// 带标签页的框（时序图 loop/alt、子图）
    Frame,
    /// 实心条（状态图 fork/join）
    Bar,
    /// 实心圆点（开始状态）
    StartDot,
    /// 圆环 + 实心圆（结束状态）
    EndDot,
    /// 小人（时序图参与者 actor）
    Actor,
    /// 便签
    Sticky,
    // ── 画板图形库（「图形」面板）──
    Triangle,
    RightTriangle,
    Pentagon,
    Octagon,
    Star,
    Cross,
    /// 单向箭头（块状）
    BlockArrow,
    /// 双向箭头（块状）
    DoubleArrow,
    /// 矩形对话气泡
    Bubble,
    // ── 更多图形（「更多图形」面板）──
    /// 燕尾箭头（> 形）
    Chevron,
    /// 五边形箭头（右侧尖）
    HomePlate,
    /// 椭圆对话气泡
    BubbleRound,
    LeftArrow,
    Cloud,
    /// 左侧文字 + 右侧大括号（≡{）
    BraceLeft,
    /// 左侧大括号 + 右侧文字（}≡）
    BraceRight,
    /// 弧形（四分之三圆环）
    BlockArc,
    /// 扇形（四分之三圆）
    Pie,
    Cube,
    // 流程图
    /// 直接存取存储器（横放的圆柱）
    HCylinder,
    /// 手动输入
    ManualInput,
    /// 延迟
    Delay,
    /// 跨页引用
    OffPage,
    /// 顺序数据（磁带）
    Tape,
    /// 卡片
    Card,
    /// 纸带
    PunchedTape,
    /// 显示
    Display,
    /// 内部存储
    InternalStorage,
    /// 循环界限
    LoopLimit,
    /// 或
    OrJunction,
    /// 汇总连接
    SumJunction,
    /// 对照
    Collate,
    /// 排序
    Sort,
    /// 合并
    Merge,
    /// 存储数据
    StoredData,
    /// 多文档
    MultiDoc,
    // 类图
    ClassBox,
    InterfaceBox,
    // 时序图（生命线：头部 + 竖直虚线）
    LifeActor,
    LifeBox,
    LifeDb,
    LifeQueue,
    LifeBoundary,
    LifeControl,
    LifeEntity,
    LifeCollections,
    /// 激活条
    Activation,
    /// 组合片段（带虚线分隔，alt / else）
    FrameAlt,
    // 数据流图
    /// 加工（Gane-Sarson，圆角矩形 + 标题线）
    DfdProcess,
    /// 数据存储（上下两条线）
    DataStore,
    /// 数据存储（左侧封口 + 编号栏）
    DataStoreId,
    /// 右侧开口的框
    OpenRect,
    // 实体关系图
    Entity,
    /// 主键列 + 字段列
    Entity2Key,
    Entity2,
    Entity3,
    // 组件图
    Component,
    /// 右上角带组件图标
    Component2,
    /// 提供接口（棒棒糖 -○）
    ProvidedIface,
    /// 组装连接（-(○)-）
    AssemblyIface,
    /// 需求接口（)-）
    RequiredIface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Arrow {
    #[default]
    None,
    Filled,
    Open,
    Hollow,
    Circle,
    Cross,
    DiamondFilled,
    DiamondHollow,
    /// ER 图基数：恰好一个 / 零或一个 / 一或多个 / 零或多个
    One,
    ZeroOrOne,
    OneOrMany,
    ZeroOrMany,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    Left,
    /// 垂直居中、右对齐
    Right,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Align {
    /// 由水平（0 左 1 中 2 右）与垂直（0 顶 1 中 2 底）对齐组合。
    pub fn from_hv(h: u8, v: u8) -> Align {
        match (v, h) {
            (0, 0) => Align::TopLeft,
            (0, 2) => Align::TopRight,
            (0, _) => Align::TopCenter,
            (2, 0) => Align::BottomLeft,
            (2, 2) => Align::BottomRight,
            (2, _) => Align::BottomCenter,
            (_, 0) => Align::Left,
            (_, 2) => Align::Right,
            _ => Align::Center,
        }
    }
    pub fn is_top(self) -> bool {
        matches!(self, Align::TopLeft | Align::TopCenter | Align::TopRight)
    }
    pub fn is_bottom(self) -> bool {
        matches!(self, Align::BottomLeft | Align::BottomCenter | Align::BottomRight)
    }
    /// 水平对齐：0 左、1 中、2 右。
    pub fn h(self) -> u8 {
        match self {
            Align::TopLeft | Align::Left | Align::BottomLeft => 0,
            Align::TopRight | Align::Right | Align::BottomRight => 2,
            _ => 1,
        }
    }
}

/// 文字修饰位：斜体 / 下划线 / 删除线
pub const DECO_ITALIC: u8 = 1;
pub const DECO_UNDERLINE: u8 = 2;
pub const DECO_STRIKE: u8 = 4;

/// 形状内的分栏（类图：类名 / 属性 / 方法）
#[derive(Clone, Debug, Default)]
pub struct Section {
    pub lines: Vec<String>,
    pub left: bool,
    pub bold: bool,
}

#[derive(Clone, Debug)]
pub struct ShapeItem {
    pub id: String,
    pub kind: ShapeKind,
    pub rect: Rect,
    pub fill: Option<Col>,
    pub stroke: Option<Col>,
    pub stroke_w: f32,
    pub dash: bool,
    /// 点状虚线（优先于 dash）
    pub dotted: bool,
    pub radius: f32,
    pub text: String,
    pub text_color: Col,
    pub font_size: f32,
    pub bold: bool,
    /// 斜体 / 下划线 / 删除线（DECO_* 位）
    pub deco: u8,
    pub align: Align,
    pub sections: Vec<Section>,
    /// 选中部分文字设置的样式分段（见 `rich`），为空表示整段同一样式
    pub spans: Vec<Span>,
    /// 多格图形（类图 / 实体等）标题以外各格的文字
    pub rows: Vec<String>,
    pub shadow: bool,
    pub opacity: f32,
    /// 手绘风格随机种子（保持每次绘制形状一致）
    pub seed: u64,
}

impl ShapeItem {
    pub fn new(kind: ShapeKind, rect: Rect) -> Self {
        ShapeItem {
            id: String::new(),
            kind,
            rect,
            fill: None,
            stroke: Some(Col::rgb(0x333333)),
            stroke_w: 1.5,
            dash: false,
            dotted: false,
            radius: 6.,
            text: String::new(),
            text_color: Col::rgb(0x1f2329),
            font_size: 14.,
            bold: false,
            deco: 0,
            align: Align::Center,
            sections: vec![],
            spans: vec![],
            rows: vec![],
            shadow: false,
            opacity: 1.,
            seed: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LineItem {
    pub id: String,
    pub points: Vec<Pt>,
    pub curved: bool,
    pub stroke: Col,
    pub width: f32,
    pub dash: bool,
    pub dotted: bool,
    pub start: Arrow,
    pub end: Arrow,
    pub label: String,
    pub start_label: String,
    pub end_label: String,
    pub label_color: Col,
    pub label_bg: Option<Col>,
    pub font_size: f32,
    pub seed: u64,
    /// 曲线两端的出线方向（各自朝外，单位向量）：设置后曲线从端点沿此方向伸出，箭头也按它定方向
    pub tangents: Option<(Pt, Pt)>,
}

impl LineItem {
    /// 画箭头时箭头尾部的参考点：(终点处, 起点处)。
    pub fn arrow_from(&self) -> (Pt, Pt) {
        let n = self.points.len();
        match self.tangents.filter(|_| self.curved) {
            Some((da, db)) => (self.points[n - 1].add(db.scale(10.)), self.points[0].add(da.scale(10.))),
            None => (self.points[n - 2], self.points[1]),
        }
    }
    pub fn new(points: Vec<Pt>) -> Self {
        LineItem {
            id: String::new(),
            points,
            curved: false,
            stroke: Col::rgb(0x333333),
            width: 1.5,
            dash: false,
            dotted: false,
            start: Arrow::None,
            end: Arrow::Filled,
            label: String::new(),
            start_label: String::new(),
            end_label: String::new(),
            label_color: Col::rgb(0x333333),
            label_bg: Some(Col::rgb(0xffffff)),
            font_size: 12.,
            seed: 1,
            tangents: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug)]
pub struct TextItem {
    pub pos: Pt,
    pub text: String,
    pub size: f32,
    pub color: Col,
    pub bold: bool,
    pub anchor: Anchor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    Move(Pt),
    Line(Pt),
    Cubic(Pt, Pt, Pt),
    Close,
}

#[derive(Clone, Debug)]
pub struct PathItem {
    pub cmds: Vec<PathCmd>,
    pub fill: Option<Col>,
    pub stroke: Option<Col>,
    pub width: f32,
    pub dash: bool,
}

#[derive(Clone, Debug)]
pub struct ImageItem {
    pub rect: Rect,
    pub path: String,
}

/// 线性图标（画板图标库，24×24 描边 SVG，按 color 着色）。
#[derive(Clone, Debug)]
pub struct IconItem {
    pub rect: Rect,
    pub name: String,
    pub color: Col,
}

#[derive(Clone, Debug)]
pub enum Item {
    Shape(ShapeItem),
    Line(LineItem),
    Text(TextItem),
    Path(PathItem),
    Image(ImageItem),
    Icon(IconItem),
}

impl Item {
    pub fn bounds(&self) -> Rect {
        match self {
            Item::Shape(s) => s.rect,
            Item::Line(l) => geom::points_bounds(&l.points).inflate(l.font_size.max(8.)),
            Item::Text(t) => {
                let w = geom::estimate_width(&t.text, t.size, t.bold);
                let x = match t.anchor {
                    Anchor::Start => t.pos.x,
                    Anchor::Middle => t.pos.x - w / 2.,
                    Anchor::End => t.pos.x - w,
                };
                rect(x, t.pos.y - t.size, w, t.size * 1.4)
            }
            Item::Path(p) => geom::path_bounds(&p.cmds).inflate(p.width),
            Item::Image(i) => i.rect,
            Item::Icon(i) => i.rect,
        }
    }
}

/// 一张完整的图。
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub items: Vec<Item>,
    pub width: f32,
    pub height: f32,
    pub bg: Option<Col>,
    pub rough: bool,
    pub font: Option<String>,
}

impl Scene {
    /// 重新计算尺寸，并把所有内容平移到 (pad, pad) 起点。
    pub fn fit(&mut self, pad: f32) {
        let Some(b) = self.items.iter().map(|i| i.bounds()).reduce(|a, b| a.union(&b)) else {
            self.width = 2. * pad;
            self.height = 2. * pad;
            return;
        };
        let (dx, dy) = (pad - b.x, pad - b.y);
        for it in &mut self.items {
            geom::translate(it, dx, dy);
        }
        self.width = b.w + 2. * pad;
        self.height = b.h + 2. * pad;
    }

    /// 命中测试：返回包含点的最上层形状 id。
    pub fn hit_shape(&self, p: Pt) -> Option<&ShapeItem> {
        self.items.iter().rev().find_map(|it| match it {
            Item::Shape(s) if !s.id.is_empty() && s.rect.contains(p) && s.kind != ShapeKind::Frame => Some(s),
            _ => None,
        })
    }
}

/// 文本测量：布局阶段需要知道文字宽度。界面里用 GPUI 精确测量，测试 / 导出用估算。
pub trait Measure {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32;
}

pub struct EstimateMeasure;

impl Measure for EstimateMeasure {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        geom::estimate_width(text, size, bold)
    }
}
