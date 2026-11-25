//! 代码绘图（画板「更多 → Mermaid 绘图 / PlantUML 绘图」）：
//! - 弹窗左侧实时预览（左上角选图表模板，顶部切换「画板样式 / 经典样式」，右下角缩放，可拖动平移），
//!   右侧带行号与语法着色的代码编辑区，底部「如何使用 … 绘图」与「取消 / 插入」；
//! - 「插入」后生成可编辑的画板图形，放在一个无底色、无边框的「代码分区」里（分区记着源码、语言与样式）；
//! - 选中代码分区：编辑代码、添加标题（左上角橙色标签）、填充、边框；改完「插入并替换」或「创建副本」；
//! - 单击图中元素先选中整张图，再单击才选中其中的元素。
//!
//! 画板样式：流程图 / 活动图 / 用例图 / 组件图生成带吸附连线的原生图形（配色：蓝色步骤、橙色判断、灰色起止），
//! 思维导图生成画板思维导图，其余图按渲染结果拆成图形、连线、路径与文字；
//! 经典样式：按 Mermaid / PlantUML 官方默认配色渲染后拆成画板元素。

use super::model::*;
use super::ui::panel;
use super::view::*;
use crate::canvas::paint::{paint_scene, View};
use crate::canvas::{pt, rect, Align, Anchor, Col, EstimateMeasure, Item, PathCmd, Pt, Rect, Scene, ShapeKind};
use crate::diagram::plantuml::{self, PumlKind};
use crate::text_area::{TextArea, TextAreaEvent, TextStyle};
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 代码绘图的语言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeLang {
    Puml,
    Mermaid,
}

impl CodeLang {
    pub fn from_u8(v: u8) -> CodeLang {
        if v == 1 {
            CodeLang::Mermaid
        } else {
            CodeLang::Puml
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            CodeLang::Puml => 0,
            CodeLang::Mermaid => 1,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            CodeLang::Puml => "PlantUML",
            CodeLang::Mermaid => "Mermaid",
        }
    }
    fn highlight(self) -> &'static str {
        match self {
            CodeLang::Puml => "plantuml",
            CodeLang::Mermaid => "mermaid",
        }
    }
    fn help_url(self) -> &'static str {
        match self {
            CodeLang::Puml => "https://plantuml.com/zh/",
            CodeLang::Mermaid => "https://mermaid.js.org/intro/syntax-reference.html",
        }
    }
    /// 图表模板（顺序的下拉菜单）。
    pub fn samples(self) -> &'static [(&'static str, &'static str)] {
        match self {
            CodeLang::Puml => &PUML_SAMPLES,
            CodeLang::Mermaid => &MERMAID_SAMPLES,
        }
    }
    fn classic_theme(self) -> &'static str {
        match self {
            CodeLang::Puml => "puml-classic",
            CodeLang::Mermaid => "mermaid-classic",
        }
    }
}

pub const PUML_SAMPLES: [(&str, &str); 8] = [
    (
        "活动图",
        "@startuml\ntitle 下单流程\nstart\n:浏览商品;\nrepeat\n  :加入购物车;\nrepeat while (继续购物？) is (是) not (否)\n:提交订单;\nwhile (支付成功？) is (否)\n  :重新支付;\nendwhile (是)\nfork\n  :通知仓库发货;\nfork again\n  :发送订单短信;\nend fork\nstop\n@enduml",
    ),
    (
        "时序图",
        "@startuml\nautonumber\nactor 用户 as User\nparticipant \"API网关\" as Gateway\nparticipant \"订单服务\" as Order\nparticipant \"库存服务\" as Stock\nparticipant \"支付服务\" as Pay\n\nUser -> Gateway: 提交订单请求\nactivate Gateway\nGateway -> Order: 创建订单\nactivate Order\n\nOrder -> Stock: 检查并锁定库存\nactivate Stock\nalt 库存充足\n    Stock --> Order: 锁定成功\n    Order --> Gateway: 订单创建成功，待支付\n    deactivate Stock\n\n    Gateway --> User: 返回支付页面\n    loop 最多重试3次\n        User -> Pay: 发起支付\n        Pay --> User: 支付结果\n    end\nelse 库存不足\n    Stock --> Order: 锁定失败\n    Order --> Gateway: 订单创建失败\n    Gateway --> User: 提示库存不足\nend\ndeactivate Order\ndeactivate Gateway\n@enduml",
    ),
    (
        "流程图",
        "@startuml\ntitle 请假审批\nstart\n:提交请假申请;\nif (请假天数 > 3？) then (是)\n  :部门经理审批;\n  :HR 备案;\nelse (否)\n  :直属领导审批;\nendif\n:通知申请人;\nstop\n@enduml",
    ),
    (
        "类图",
        "@startuml\ntitle 图形类\ninterface Shape {\n  +area() : double\n}\nabstract class Base {\n  #name : String\n}\nclass Circle {\n  -r : double\n  +area() : double\n}\nclass Rect {\n  -w : double\n  -h : double\n}\nShape <|.. Base\nBase <|-- Circle\nBase <|-- Rect\n@enduml",
    ),
    ("思维导图", "@startmindmap\n* 产品发布\n** 准备\n*** 需求冻结\n*** 测试回归\n** 发布\n*** 灰度\n*** 全量\n-- 宣传\n--- 官网\n--- 社群\n-- 复盘\n@endmindmap"),
    (
        "ER 图",
        "@startuml\ntitle 订单模型\nentity 用户 {\n  * id : int\n  --\n  name : varchar\n}\nentity 订单 {\n  * id : int\n  --\n  user_id : int <<FK>>\n  amount : decimal\n}\n用户 ||--o{ 订单 : 下单\n@enduml",
    ),
    (
        "组件图",
        "@startuml\ntitle 系统组件\npackage \"前端\" {\n  [Web 应用] as Web\n  [移动端] as App\n}\npackage \"后端\" {\n  [API 网关] as Gateway\n  [订单服务] as Order\n  [用户服务] as User\n}\ndatabase \"MySQL\" as DB\nWeb --> Gateway\nApp --> Gateway\nGateway --> Order\nGateway --> User\nOrder --> DB\nUser --> DB\n@enduml",
    ),
    ("用例图", "@startuml\ntitle 网上商城\nleft to right direction\nactor 顾客\nactor 管理员\nrectangle 商城 {\n  usecase \"浏览商品\" as UC1\n  usecase \"下单\" as UC2\n  usecase \"管理商品\" as UC3\n}\n顾客 --> UC1\n顾客 --> UC2\n管理员 --> UC3\n@enduml"),
];

pub const MERMAID_SAMPLES: [(&str, &str); 9] = [
    ("流程图", "flowchart TD\n    A(入学) --> B(基础课程学习)\n    B --> C{课程考核}\n    C -->|通过| D(专业课程学习)\n    C -->|不通过| E(重修基础课程)\n    D --> F(实践项目)\n    F --> G(毕业)"),
    ("时序图", "sequenceDiagram\n    participant Alice\n    participant Bob\n    Bob->>Alice: Hi Alice\n    Alice->>Bob: Hi Bob"),
    ("思维导图", "mindmap\n  root((产品规划))\n    用户\n      新用户\n      老用户\n    功能\n      核心功能\n      增值服务\n    运营\n      活动\n      渠道"),
    ("饼图", "pie title 宠物占比\n    \"狗\" : 386\n    \"猫\" : 85\n    \"兔子\" : 15"),
    ("时间线图", "timeline\n    title 社交媒体发展史\n    2002 : LinkedIn\n    2004 : Facebook : Google\n    2005 : YouTube\n    2006 : Twitter"),
    ("类图", "classDiagram\n    Animal <|-- Duck\n    Animal <|-- Fish\n    Animal : +int age\n    Animal : +String gender\n    Animal: +isMammal()\n    class Duck{\n      +String beakColor\n      +swim()\n    }\n    class Fish{\n      -int sizeInFeet\n      -canEat()\n    }"),
    ("ER 图", "erDiagram\n    CUSTOMER ||--o{ ORDER : places\n    ORDER ||--|{ LINE-ITEM : contains\n    CUSTOMER }|..|{ DELIVERY-ADDRESS : uses"),
    ("状态图", "stateDiagram-v2\n    [*] --> 待支付\n    待支付 --> 已支付: 支付\n    待支付 --> 已取消: 超时\n    已支付 --> 已发货: 发货\n    已发货 --> 已完成: 确认收货\n    已完成 --> [*]\n    已取消 --> [*]"),
    ("甘特图", "gantt\n    title 项目计划\n    dateFormat YYYY-MM-DD\n    section 设计\n    需求评审 :a1, 2024-01-01, 5d\n    方案设计 :a2, after a1, 7d\n    section 开发\n    开发实现 :b1, after a2, 14d\n    联调测试 :b2, after b1, 7d"),
];

/// 预览缩放档位。
const ZOOM_STOPS: [f32; 17] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1., 1.25, 1.5, 1.75, 2., 2.5, 3., 4.];

pub struct CodeDlg {
    pub area: Entity<TextArea>,
    /// 正在编辑的代码分区（None 为新建）
    pub target: Option<u64>,
    pub lang: CodeLang,
    pub tpl_open: bool,
    /// 当前选中的模板（下拉按钮上显示的名字）
    pub tpl: usize,
    /// 经典样式（否则为画板样式）
    pub classic: bool,
    /// 预览缩放：None 为自适应
    zoom: Option<f32>,
    /// 预览平移（屏幕像素）与拖动起点
    pan: (f32, f32),
    pan_drag: Option<(Point<Pixels>, (f32, f32))>,
    /// 预览缓存：源码与样式不变就不重新解析布局
    preview: RefCell<Option<(String, bool, Result<Arc<Scene>, String>)>>,
    _sub: Subscription,
}

impl CodeDlg {
    fn reset_view(&mut self) {
        self.zoom = None;
        self.pan = (0., 0.);
    }
}

/// 校验源码确实是该语言，给出友好的提示。
fn check_lang(src: &str, lang: CodeLang) -> Result<(), String> {
    let t = src.trim_start();
    match lang {
        CodeLang::Puml => {
            if plantuml::detect(src).is_none() {
                return Err(match t.strip_prefix("@start").and_then(|r| r.split_whitespace().next()) {
                    Some(kind) if kind != "uml" => {
                        format!("暂不支持 @start{kind}，目前支持 @startuml（时序 / 活动 / 用例 / 类 / 状态 / ER / 组件）、@startmindmap、@startwbs")
                    }
                    _ => "PlantUML 代码需要以 @startuml 或 @startmindmap 开头".into(),
                });
            }
        }
        CodeLang::Mermaid => {
            if t.is_empty() {
                return Err("输入 Mermaid 代码，例如：flowchart TD / sequenceDiagram / mindmap".into());
            }
            if t.starts_with("@start") {
                return Err("这是 PlantUML 代码，请使用「PlantUML 绘图」".into());
            }
        }
    }
    Ok(())
}

/// 按官方配色或画板配色渲染。
fn render_with(src: &str, theme: &str) -> Result<Scene, String> {
    crate::diagram::render(src, false, Some(theme), &mut EstimateMeasure, &BTreeMap::new()).map_err(|e| {
        if e.line > 0 {
            format!("第 {} 行：{}", e.line, e.msg)
        } else {
            e.msg
        }
    })
}

/// 路径（含控制点）的包围盒。
fn cmds_bounds(cmds: &[PathCmd]) -> Option<Rect> {
    let pts: Vec<Pt> = cmds
        .iter()
        .flat_map(|c| match c {
            PathCmd::Move(p) | PathCmd::Line(p) => vec![*p],
            PathCmd::Cubic(a, b, p) => vec![*a, *b, *p],
            PathCmd::Close => vec![],
        })
        .collect();
    (!pts.is_empty()).then(|| crate::canvas::geom::points_bounds(&pts))
}

/// 把渲染出的场景拆成画板元素：路径、图形、连线（端点落在图形边上时吸附到图形）、文字。
pub fn explode_scene(scene: &Scene, dark: bool) -> BoardDoc {
    let mut d = BoardDoc::new(false);
    let ink = Col::rgb(0x1f2329);
    let mut boxes: Vec<(Rect, u64, ShapeKind)> = vec![];
    // 路径（饼图扇形、导图分支）垫在最下面；再放图形，连线才能吸附上去（场景里连线往往排在图形前面）
    let is_path = |i: &&Item| matches!(i, Item::Path(_));
    let is_shape = |i: &&Item| matches!(i, Item::Shape(_));
    let ordered =
        scene.items.iter().filter(is_path).chain(scene.items.iter().filter(is_shape)).chain(scene.items.iter().filter(|i| !is_path(i) && !is_shape(i)));
    for item in ordered {
        match item {
            Item::Path(p) => {
                let Some(b) = cmds_bounds(&p.cmds) else { continue };
                if b.w < 0.5 && b.h < 0.5 {
                    continue;
                }
                let r = rect(b.x, b.y, b.w.max(1.), b.h.max(1.));
                let mut el = d.new_el(ElKind::Path { segs: PathSeg::normalize(&p.cmds, r) }, r, dark);
                el.fill = p.fill;
                match p.stroke {
                    Some(c) => el.stroke = c,
                    None => el.no_stroke = true,
                }
                el.stroke_w = p.width;
                el.dash = p.dash;
                d.els.push(el);
            }
            Item::Shape(s) => {
                if !s.sections.is_empty() {
                    // 类图的分栏：拆成上下相接的几个矩形，组合在一起
                    let g = d.alloc_id();
                    let lh = s.font_size * 1.45;
                    let natural: Vec<f32> = s.sections.iter().map(|sec| sec.lines.len().max(1) as f32 * lh + 8.).collect();
                    let total: f32 = natural.iter().sum();
                    let mut y = s.rect.y;
                    for (i, sec) in s.sections.iter().enumerate() {
                        let h = if i + 1 == s.sections.len() { s.rect.bottom() - y } else { natural[i] * s.rect.h / total.max(1.) };
                        let mut el = d.new_el(ElKind::Shape(ShapeKind::Rect), rect(s.rect.x, y, s.rect.w, h), dark);
                        el.text = sec.lines.join("\n");
                        el.bold = sec.bold;
                        el.text_align = if sec.left { 1 } else { 2 };
                        el.font_size = s.font_size;
                        el.fill = s.fill;
                        el.stroke = s.stroke.unwrap_or(ink);
                        el.stroke_w = s.stroke_w;
                        el.text_color = (s.text_color != ink).then_some(s.text_color);
                        el.group = Some(g);
                        boxes.push((el.rect(), el.id, ShapeKind::Rect));
                        d.els.push(el);
                        y += h;
                    }
                    continue;
                }
                let mut el = d.new_el(ElKind::Shape(s.kind), s.rect, dark);
                el.text = s.text.clone();
                el.fill = s.fill;
                match s.stroke {
                    Some(c) => el.stroke = c,
                    None => el.no_stroke = true,
                }
                el.stroke_w = s.stroke_w;
                el.dash = s.dash;
                el.dotted = s.dotted;
                el.font_size = s.font_size;
                el.bold = s.bold;
                el.text_color = (s.text_color != ink).then_some(s.text_color);
                el.text_align = match s.align {
                    Align::Left | Align::TopLeft => 1,
                    Align::Right | Align::TopRight => 3,
                    _ => 0,
                };
                if s.kind != ShapeKind::Frame {
                    boxes.push((el.rect(), el.id, s.kind));
                }
                d.els.push(el);
            }
            Item::Line(l) if l.points.len() >= 2 => {
                let bind = |p: Pt| -> End {
                    for (r, id, kind) in boxes.iter().rev() {
                        if !r.inflate(3.).contains(p) {
                            continue;
                        }
                        // 落在外框边上，或落在真实轮廓上（菱形、圆形等的边界点在外框里面）
                        let edge = (p.x - r.x).abs().min((p.x - r.right()).abs()).min((p.y - r.y).abs()).min((p.y - r.bottom()).abs());
                        if edge < 3.5 || crate::canvas::geom::boundary_point(*kind, *r, p).dist(p) < 3.5 {
                            let (ax, ay) = (((p.x - r.x) / r.w.max(1.)).clamp(0., 1.), ((p.y - r.y) / r.h.max(1.)).clamp(0., 1.));
                            // (0.5, 0.5) 表示「自动」，挪开一点点
                            let ax = if (ax - 0.5).abs() < 0.01 && (ay - 0.5).abs() < 0.01 { 0.511 } else { ax };
                            return End::Bound { el: *id, ax, ay };
                        }
                    }
                    End::Free { x: p.x, y: p.y }
                };
                let (a, b) = (bind(l.points[0]), bind(*l.points.last().unwrap()));
                let free = matches!(a, End::Free { .. }) && matches!(b, End::Free { .. });
                let ortho = l.points.windows(2).all(|w| (w[0].x - w[1].x).abs() < 0.5 || (w[0].y - w[1].y).abs() < 0.5);
                let style = |el: &mut El| {
                    el.stroke = l.stroke;
                    el.stroke_w = l.width;
                    el.dash = l.dash;
                    el.dotted = l.dotted;
                    el.font_size = l.font_size + 2.;
                };
                if free && l.points.len() > 2 {
                    // 两端都没吸附的折线（如自调用消息）：拆成几段直线，保持原样
                    let n = l.points.len() - 1;
                    for (i, w) in l.points.windows(2).enumerate() {
                        let mut el = d.new_el(
                            ElKind::Connector {
                                a: End::Free { x: w[0].x, y: w[0].y },
                                b: End::Free { x: w[1].x, y: w[1].y },
                                route: Route::Straight,
                                start: if i == 0 { l.start } else { crate::canvas::Arrow::None },
                                end: if i + 1 == n { l.end } else { crate::canvas::Arrow::None },
                            },
                            Rect::default(),
                            dark,
                        );
                        style(&mut el);
                        if i == n / 2 {
                            el.text = l.label.clone();
                        }
                        d.els.push(el);
                    }
                    continue;
                }
                let route = if l.curved && (l.points.len() > 2 || l.tangents.is_some()) {
                    Route::Curve
                } else if l.corner_radius > 0. {
                    Route::RoundElbow
                } else if l.points.len() == 2 {
                    Route::Straight
                } else if ortho {
                    Route::Elbow
                } else {
                    Route::Straight
                };
                let mut el = d.new_el(ElKind::Connector { a, b, route, start: l.start, end: l.end }, Rect::default(), dark);
                style(&mut el);
                el.text = l.label.clone();
                let id = el.id;
                d.els.push(el);
                // 保留布局算出的中间拐点，长边绕开中间的节点
                d.set_via(id, &l.points[1..l.points.len() - 1]);
            }
            Item::Text(t) if !t.text.trim().is_empty() => {
                let lines = t.text.lines().count().max(1) as f32;
                let lh = t.size * 1.38;
                let w = crate::canvas::geom::estimate_width(&t.text, t.size, t.bold);
                let left = match t.anchor {
                    Anchor::Start => t.pos.x,
                    Anchor::Middle => t.pos.x - w / 2.,
                    Anchor::End => t.pos.x - w,
                };
                let top = t.pos.y - t.size * 0.35 - lh * lines / 2.;
                // 估算宽度偏窄（粗体、数字），多留一些免得被折行
                let mut el = d.new_el(ElKind::Text, rect(left - 12., top - 6.4, w * 1.1 + 28., lines * lh + 12.), dark);
                el.text = t.text.clone();
                el.font_size = t.size;
                el.bold = t.bold;
                el.text_color = (t.color != ink).then_some(t.color);
                d.els.push(el);
            }
            _ => {}
        }
    }
    d
}

/// 「画板样式」的流程图配色：步骤浅蓝、判断浅橙、起止灰色，连线灰色 90° 圆角折线。
/// 源码里用 style / classDef 指定过颜色的节点保持原样。
fn board_flow_style(d: &mut BoardDoc) {
    let default_fill = Some(Col::rgb(0xeef3ff));
    for el in d.els.iter_mut() {
        match el.kind {
            ElKind::Shape(k) => {
                el.font_size = 13.;
                el.stroke_w = 1.5;
                if el.fill != default_fill {
                    continue;
                }
                let (fill, stroke) = match k {
                    ShapeKind::Diamond | ShapeKind::Hexagon => (0xfeead2, 0xff8800),
                    ShapeKind::Stadium | ShapeKind::Circle | ShapeKind::Ellipse | ShapeKind::DoubleCircle => (0xeff0f1, 0xbbbfc4),
                    _ => (0xe1eaff, 0x5b8cff),
                };
                el.fill = Some(Col::rgb(fill));
                el.stroke = Col::rgb(stroke);
            }
            ElKind::Connector { ref mut route, ref a, ref b, .. } => {
                let self_loop = matches!((a, b), (End::Bound { el: x, .. }, End::Bound { el: y, .. }) if x == y);
                if !self_loop {
                    *route = Route::RoundElbow;
                }
                el.stroke = Col::rgb(0x8f959e);
                el.stroke_w = 1.5;
                el.font_size = 12.;
            }
            _ => {}
        }
    }
}

/// Mermaid 思维导图 → (层级, 文字) 大纲：按缩进分层，去掉节点形状括号与图标 / 样式行。
fn mermaid_mind_outline(src: &str) -> Option<Vec<(usize, String)>> {
    let (lines, _) = crate::diagram::preprocess(src);
    let mut stack: Vec<usize> = vec![];
    let mut out = vec![];
    for (_, l) in lines.iter().skip(1) {
        let t = l.trim();
        if t.starts_with("::icon") || t.starts_with(":::") {
            continue;
        }
        let ind = l.len() - l.trim_start().len();
        while stack.last().is_some_and(|&i| i >= ind) {
            stack.pop();
        }
        stack.push(ind);
        let t = t.split(":::").next().unwrap_or(t).trim();
        // `id((文字))` / `id[文字]` / `id{{文字}}` / `id))文字((` → 文字
        let text = match t.find(['(', '[', '{', ')']) {
            Some(i) if i > 0 || t.len() > 2 => {
                let inner = t[i..].trim_matches(|c| "()[]{}".contains(c));
                if inner.is_empty() {
                    t
                } else {
                    inner
                }
            }
            _ => t,
        };
        let text = crate::diagram::clean_text(text);
        if !text.is_empty() {
            out.push((stack.len(), text));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// 由代码生成画板元素（坐标从原点附近开始）。
pub fn code_to_board(src: &str, lang: CodeLang, classic: bool, dark: bool) -> Result<BoardDoc, String> {
    check_lang(src, lang)?;
    if classic {
        return Ok(explode_scene(&render_with(src, lang.classic_theme())?, dark));
    }
    let mind = |lines: Vec<(usize, String)>, both: bool| {
        let mut d = BoardDoc::new(false);
        d.mind_from_outline(&lines, pt(0., 0.), if both { 1 } else { 0 }, dark);
        d
    };
    let flow = |m: &str| -> Option<BoardDoc> {
        let mut d = super::from_mermaid(m, dark)?;
        d.flow = false;
        board_flow_style(&mut d);
        Some(d)
    };
    match lang {
        CodeLang::Puml => match plantuml::detect(src) {
            Some(PumlKind::Mindmap) => {
                let lines = plantuml::mind_outline(src).ok_or("思维导图里没有节点（以 * 开头的行）")?;
                Ok(mind(lines, plantuml::mind_both_sides(src)))
            }
            Some(PumlKind::Activity | PumlKind::Usecase | PumlKind::Component) => flow(&plantuml::maybe_convert(src)).ok_or_else(|| "无法解析这张图".into()),
            _ => Ok(explode_scene(&render_with(src, "board")?, dark)),
        },
        CodeLang::Mermaid => match crate::diagram::detect(src) {
            Some(crate::diagram::Kind::Flowchart) => {
                // 先完整渲染一遍以拿到带行号的报错
                render_with(src, "board")?;
                flow(src).ok_or_else(|| "无法解析流程图".into())
            }
            Some(crate::diagram::Kind::Mindmap) => Ok(mind(mermaid_mind_outline(src).ok_or("思维导图里没有节点")?, true)),
            _ => Ok(explode_scene(&render_with(src, "board")?, dark)),
        },
    }
}

/// 将 Mermaid 源码直接构建成带「代码分区」元数据的画板文档（画板样式），
/// 插入文档后既可直接预览与拖动元素，也可在画板里二次编辑 Mermaid 源码与切换样式。
pub fn mermaid_board(src: &str, dark: bool) -> Result<BoardDoc, String> {
    let mut tmp = code_to_board(src, CodeLang::Mermaid, false, dark)?;
    if let Some(b) = tmp.bounds() {
        let mut sec = tmp.new_el(ElKind::Section, b.inflate(12.), dark);
        sec.text.clear();
        sec.fill = None;
        sec.no_stroke = true;
        sec.code = Some(src.to_string());
        sec.code_lang = CodeLang::Mermaid.to_u8();
        sec.code_classic = false;
        tmp.els.insert(0, sec);
    }
    Ok(tmp)
}

/// PlantUML 画板样式（测试用）。
#[cfg(test)]
pub fn puml_to_board(src: &str, dark: bool) -> Result<BoardDoc, String> {
    code_to_board(src, CodeLang::Puml, false, dark)
}

/// 源码对应的模板序号（编辑已有代码图时，下拉按钮显示它的图表类型）。
fn tpl_for(src: &str, lang: CodeLang) -> usize {
    let label = match lang {
        CodeLang::Puml => plantuml::detect(src).map(|k| k.label()),
        CodeLang::Mermaid => crate::diagram::detect(src).map(|k| if k == crate::diagram::Kind::Timeline { "时间线图" } else { k.label() }),
    };
    label.and_then(|l| lang.samples().iter().position(|(n, _)| *n == l)).unwrap_or(0)
}

impl BoardEditor {
    /// 打开代码绘图弹窗；target 为要编辑的代码分区（语言跟随分区）。
    pub(super) fn open_code(&mut self, target: Option<u64>, lang: CodeLang, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        self.close_overlays();
        let el = target.and_then(|id| self.doc.get(id)).filter(|e| e.code.is_some()).cloned();
        let lang = el.as_ref().map(|e| CodeLang::from_u8(e.code_lang)).unwrap_or(lang);
        let classic = el.as_ref().is_some_and(|e| e.code_classic);
        let text = el.as_ref().and_then(|e| e.code.clone()).unwrap_or_else(|| lang.samples()[0].1.to_string());
        let tpl = tpl_for(&text, lang);
        let t = crate::theme::theme(cx);
        let style = TextStyle { font_size: 13., line_height: 21., color: t.text, bold: false, align: 0, v_align: 0, deco: 0 };
        let area = cx.new(|cx| {
            let mut a = TextArea::new(&text, &format!("输入 {} 代码", lang.name()), style, cx);
            a.set_scrollable();
            a.set_code(lang.highlight());
            a
        });
        area.update(cx, |a, _| a.move_to_end());
        let sub = cx.subscribe_in(&area, window, |this, area, ev: &TextAreaEvent, window, cx| match ev {
            TextAreaEvent::Changed => {
                if let Some(d) = this.code_dlg.as_mut() {
                    d.reset_view();
                }
                cx.notify()
            }
            TextAreaEvent::Commit => this.close_code(window, cx),
            TextAreaEvent::Tab => area.update(cx, |a, cx| a.insert("    ", cx)),
        });
        window.focus(&area.focus_handle(cx));
        window.prevent_default();
        self.code_dlg = Some(CodeDlg {
            area,
            target: el.map(|e| e.id),
            lang,
            tpl_open: false,
            tpl,
            classic,
            zoom: None,
            pan: (0., 0.),
            pan_drag: None,
            preview: RefCell::new(None),
            _sub: sub,
        });
        cx.notify();
    }

    pub(super) fn close_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.code_dlg = None;
        window.focus(&self.focus);
        cx.notify();
    }

    /// 插入代码图：mode 0 新插入、1 插入并替换、2 创建副本。
    pub(super) fn insert_code(&mut self, mode: u8, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dlg) = self.code_dlg.as_ref() else { return };
        let (lang, classic) = (dlg.lang, dlg.classic);
        let src = dlg.area.read(cx).text().to_string();
        let target = dlg.target.and_then(|id| self.doc.get(id)).cloned();
        let dark = Self::dark(cx);
        let Ok(mut tmp) = code_to_board(&src, lang, classic, dark) else { return };
        let Some(b) = tmp.bounds() else { return };
        // 代码分区：贴着内容留一点边距，默认无底色、无边框、无标题；替换 / 副本沿用原分区的标题与样式
        let mut sec = tmp.new_el(ElKind::Section, b.inflate(12.), dark);
        match target.as_ref().filter(|_| mode != 0) {
            Some(old) => {
                sec.text = old.text.clone();
                sec.fill = old.fill;
                sec.stroke = old.stroke;
                sec.no_stroke = old.no_stroke;
                sec.opacity = old.opacity;
            }
            None => {
                sec.text.clear();
                sec.fill = None;
                sec.no_stroke = true;
            }
        }
        sec.code = Some(src);
        sec.code_lang = lang.to_u8();
        sec.code_classic = classic;
        let sr = sec.rect();
        tmp.els.insert(0, sec);
        // 放置位置：替换时放回原处，副本放在右边，新插入放在视口中心
        let (dx, dy) = match (mode, target.as_ref().map(|e| e.rect())) {
            (1, Some(r)) => (r.x - sr.x, r.y - sr.y),
            (2, Some(r)) => (r.right() + 80. - sr.x, r.y - sr.y),
            _ => {
                let c = self.to_world(self.canvas_bounds.center());
                (c.x - sr.center().x, c.y - sr.center().y)
            }
        };
        self.snapshot();
        if let (1, Some(old)) = (mode, target.as_ref()) {
            let id = old.id;
            let mut ids: std::collections::HashSet<u64> = self.doc.section_children(id).into_iter().collect();
            ids.insert(id);
            for k in ids.clone() {
                if self.doc.get(k).is_some_and(|e| e.is_mind()) {
                    ids.extend(self.doc.mind_subtree(k));
                }
            }
            self.doc.els.retain(|e| !ids.contains(&e.id));
            self.remove_dangling();
        }
        self.insert_copies_at(tmp.els, dx, dy);
        // 选中代码分区本身
        if let Some(sid) = self.doc.els.iter().find(|e| self.sel.contains(&e.id) && e.code.is_some()).map(|e| e.id) {
            self.sel = [sid].into();
        }
        self.code_dlg = None;
        self.tool = Tool::Select;
        window.focus(&self.focus);
        self.commit(cx);
        let vw = f32::from(self.canvas_bounds.size.width) / self.zoom;
        let vh = f32::from(self.canvas_bounds.size.height) / self.zoom;
        if sr.w > vw || sr.h > vh || mode == 2 {
            let r = self.selection_bounds();
            self.zoom_to(r);
        }
    }

    /// 标题按钮：没有标题时进入输入，有标题时隐藏（清空）标题。
    pub(super) fn code_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.single().filter(|e| e.code.is_some()).cloned() else { return };
        if e.text.is_empty() {
            self.start_edit(e.id, window, cx);
        } else {
            self.snapshot();
            if let Some(i) = self.doc.index(e.id) {
                self.doc.els[i].text.clear();
            }
            self.commit(cx);
        }
    }

    /// 代码分区内的元素所属的代码分区。
    pub(super) fn code_owner(&self, id: u64) -> Option<u64> {
        self.doc.els.iter().filter(|e| e.code.is_some() && e.id != id).find(|e| self.doc.section_children(e.id).contains(&id)).map(|e| e.id)
    }

    fn code_zoom_step(&mut self, dir: f32, fit: f32, cx: &mut Context<Self>) {
        if let Some(d) = self.code_dlg.as_mut() {
            let cur = d.zoom.unwrap_or(fit);
            let next =
                if dir > 0. { ZOOM_STOPS.iter().copied().find(|z| *z > cur + 0.001) } else { ZOOM_STOPS.iter().rev().copied().find(|z| *z < cur - 0.001) };
            if let Some(z) = next {
                // 以预览中心为基准缩放：平移量跟着等比变化
                let k = z / cur;
                d.pan = (d.pan.0 * k, d.pan.1 * k);
                d.zoom = Some(z);
            }
        }
        cx.notify();
    }

    pub(super) fn render_code_dlg(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dlg = self.code_dlg.as_ref()?;
        let src = dlg.area.read(cx).text().to_string();
        let (lang, classic) = (dlg.lang, dlg.classic);
        let dark = t.dark;
        // 预览与插入用同一套转换：所见即所得
        let preview = {
            let mut cache = dlg.preview.borrow_mut();
            if cache.as_ref().is_none_or(|c| c.0 != src || c.1 != classic) {
                let r = code_to_board(&src, lang, classic, dark).map(|d| Arc::new(d.to_scene(dark)));
                *cache = Some((src.clone(), classic, r));
            }
            cache.as_ref().unwrap().2.clone()
        };
        // 弹窗尺寸随窗口收缩
        let vp = window.viewport_size();
        let (dw, dh) = ((f32::from(vp.width) - 80.).clamp(720., 1080.), (f32::from(vp.height) - 80.).clamp(420., 620.));
        let editor_w = (dw * 0.4).min(440.);
        let (pw, ph) = (dw - editor_w, dh - 60.);
        let bounds_of = |s: &Scene| s.items.iter().map(|it| it.bounds()).reduce(|a, b| a.union(&b)).unwrap_or_default();
        let fit = match &preview {
            Ok(s) => {
                let b = bounds_of(s);
                ((pw - 64.) / b.w.max(1.)).min((ph - 120.) / b.h.max(1.)).min(1.)
            }
            Err(_) => 1.,
        };
        let scale = dlg.zoom.unwrap_or(fit);
        let pan = dlg.pan;
        let ok = preview.is_ok();
        let btn = |id: &'static str, label: &'static str, primary: bool, enabled: bool| {
            div()
                .id(id)
                .h(px(32.))
                .px(px(18.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(14.))
                .when(primary, |d| d.bg(t.accent).text_color(gpui::white()))
                .when(primary && enabled, |d| d.cursor_pointer().hover(|s| s.opacity(0.9)))
                .when(primary && !enabled, |d| d.opacity(0.45))
                .when(!primary, |d| d.cursor_pointer().border_1().border_color(t.border_strong).hover(|s| s.bg(t.hover)))
                .child(label)
        };
        let preview_el: AnyElement = match preview {
            Ok(scene) => {
                let b = bounds_of(&scene);
                canvas(
                    |bb, _, _| bb,
                    move |bb, _, window, cx| {
                        let (w, h) = (f32::from(bb.size.width), f32::from(bb.size.height));
                        // 预览区上方留出模板 / 样式按钮的位置
                        let (cxp, cyp) = (w / 2. + pan.0, (h + 40.) / 2. + pan.1);
                        let origin = point(bb.origin.x + px(cxp - b.center().x * scale), bb.origin.y + px(cyp - b.center().y * scale));
                        window.with_content_mask(Some(ContentMask { bounds: bb }), |window| paint_scene(&scene, View { origin, scale }, bb, window, cx));
                    },
                )
                .size_full()
                .into_any_element()
            }
            Err(e) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .p(px(32.))
                .child(icon("info", 28., t.text_muted))
                .child(div().text_size(px(13.)).text_color(t.text_muted).text_center().child(e))
                .into_any_element(),
        };
        // 左上角：图表类型下拉（「流程图 ∨」）
        let samples = lang.samples();
        let tpl_btn = div()
            .id("code-tpl-btn")
            .h(px(30.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(15.))
            .when(dlg.tpl_open, |d| d.bg(t.hover))
            .hover(|s| s.bg(t.hover))
            .child(icon("shapes", 15., t.text))
            .child(samples[dlg.tpl.min(samples.len() - 1)].0)
            .child(icon("chevron-down", 13., t.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = this.code_dlg.as_mut() {
                    d.tpl_open = !d.tpl_open;
                }
                cx.notify();
            }));
        let tpl_menu = dlg.tpl_open.then(|| {
            div().absolute().top(px(52.)).left(px(16.)).child(
                panel(t).py(px(4.)).w(px(124.)).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).children(samples.iter().enumerate().map(
                    |(i, (name, code))| {
                        let code = *code;
                        let on = i == dlg.tpl;
                        div()
                            .id(("code-tpl", i))
                            .h(px(32.))
                            .px(px(10.))
                            .mx(px(4.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(14.))
                            .when(on, |d| d.bg(t.hover))
                            .hover(|s| s.bg(t.hover))
                            .child(*name)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(d) = this.code_dlg.as_mut() {
                                    d.tpl_open = false;
                                    d.tpl = i;
                                    d.reset_view();
                                    let area = d.area.clone();
                                    area.update(cx, |a, cx| a.set_text(code, cx));
                                    window.focus(&area.focus_handle(cx));
                                }
                                cx.notify();
                            }))
                    },
                )),
            )
        });
        // 顶部居中：画板样式 / 经典样式
        let seg = |id: &'static str, label: &'static str, on: bool, value: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(14.))
                .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                .when(!on, |d| d.hover(|s| s.bg(t.hover)))
                .child(label)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(d) = this.code_dlg.as_mut() {
                        if d.classic != value {
                            d.classic = value;
                            d.reset_view();
                        }
                    }
                    cx.notify();
                }))
        };
        let style_switch = div()
            .p(px(3.))
            .flex()
            .gap(px(2.))
            .rounded(px(7.))
            .border_1()
            .border_color(t.border_strong)
            .bg(t.panel_bg)
            .child(seg("code-style-board", "画板样式", !classic, false))
            .child(seg("code-style-classic", "经典样式", classic, true));
        // 右下角：缩放
        let zoom_btn = |id: &'static str, ic: &'static str, dir: f32| {
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 17., t.text))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.code_zoom_step(dir, fit, cx)))
        };
        let zoom_bar = panel(t)
            .h(px(42.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(zoom_btn("code-zoom-out", "minus", -1.))
            .child(div().w(px(52.)).text_center().text_size(px(15.)).child(format!("{:.0}%", scale * 100.)))
            .child(zoom_btn("code-zoom-in", "plus", 1.));
        let preview_pane = div()
            .id("code-preview")
            .relative()
            .w(px(pw))
            .h_full()
            .overflow_hidden()
            .when(ok, |d| d.cursor(if dlg.pan_drag.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    if let Some(d) = this.code_dlg.as_mut() {
                        d.tpl_open = false;
                        d.pan_drag = Some((ev.position, d.pan));
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some(d) = this.code_dlg.as_mut() {
                    if let Some((start, orig)) = d.pan_drag {
                        d.pan = (orig.0 + f32::from(ev.position.x - start.x), orig.1 + f32::from(ev.position.y - start.y));
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.code_dlg.as_mut() {
                        d.pan_drag = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.code_dlg.as_mut() {
                        d.pan_drag = None;
                    }
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                let delta = ev.delta.pixel_delta(px(20.));
                if ev.modifiers.platform || ev.modifiers.control {
                    // ⌘ / Ctrl + 滚轮缩放
                    if let Some(d) = this.code_dlg.as_mut() {
                        let cur = d.zoom.unwrap_or(fit);
                        let z = (cur * (1. + f32::from(delta.y) * 0.004)).clamp(0.1, 4.);
                        let k = z / cur;
                        d.pan = (d.pan.0 * k, d.pan.1 * k);
                        d.zoom = Some(z);
                    }
                } else if let Some(d) = this.code_dlg.as_mut() {
                    d.pan = (d.pan.0 + f32::from(delta.x), d.pan.1 + f32::from(delta.y));
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(preview_el)
            .child(div().absolute().top(px(12.)).left(px(16.)).child(tpl_btn))
            .child(div().absolute().top(px(12.)).left_0().right_0().flex().justify_center().child(style_switch))
            .when(ok, |d| d.child(div().absolute().right(px(28.)).bottom(px(20.)).child(zoom_bar)))
            .children(tpl_menu);
        let editing = dlg.target.is_some();
        let help = format!("如何使用 {} 绘图", lang.name());
        let help_url = lang.help_url();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0., 0., 0., 0.25))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_code(window, cx)))
                .child(
                    panel(t)
                        .id("code-dlg")
                        .relative()
                        .w(px(dw))
                        .h(px(dh))
                        .rounded(px(12.))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div().flex_1().flex().overflow_hidden().child(preview_pane).child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .pt(px(12.))
                                    .pb(px(8.))
                                    .pr(px(6.))
                                    .border_l_1()
                                    .border_color(t.border)
                                    .bg(t.panel_bg)
                                    .child(dlg.area.clone()),
                            ),
                        )
                        .child(
                            div()
                                .h(px(60.))
                                .px(px(20.))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .border_t_1()
                                .border_color(t.border)
                                .child(
                                    div()
                                        .id("code-help")
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .cursor_pointer()
                                        .text_size(px(14.))
                                        .text_color(t.accent)
                                        .hover(|s| s.opacity(0.8))
                                        .child(icon("info", 15., t.accent))
                                        .child(help)
                                        .on_click(move |_, _, cx| cx.open_url(help_url)),
                                )
                                .child(div().flex_1())
                                .child(btn("code-cancel", "取消", false, true).on_click(cx.listener(|this, _, window, cx| this.close_code(window, cx))))
                                .when(editing, |d| {
                                    d.child(
                                        btn("code-copy", "创建副本", false, true)
                                            .when(ok, |b| b.on_click(cx.listener(|this, _, window, cx| this.insert_code(2, window, cx)))),
                                    )
                                    .child(
                                        btn("code-replace", "插入并替换", true, ok)
                                            .when(ok, |b| b.on_click(cx.listener(|this, _, window, cx| this.insert_code(1, window, cx)))),
                                    )
                                })
                                .when(!editing, |d| {
                                    d.child(
                                        btn("code-insert", "插入", true, ok)
                                            .when(ok, |b| b.on_click(cx.listener(|this, _, window, cx| this.insert_code(0, window, cx)))),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // 别用 `super::*`：会把 gpui 的 `test` 宏引进作用域
    use super::super::model::{ElKind, End};
    use super::{code_to_board, mermaid_mind_outline, puml_to_board, CodeLang};

    #[test]
    fn every_sample_converts_to_board() {
        for lang in [CodeLang::Puml, CodeLang::Mermaid] {
            for (name, src) in lang.samples() {
                for classic in [false, true] {
                    let d = code_to_board(src, lang, classic, false).unwrap_or_else(|e| panic!("{name}({classic}): {e}"));
                    assert!(d.els.len() >= 3, "{name}({classic}): {}", d.els.len());
                }
            }
        }
        // 思维导图（画板样式）直接生成画板节点
        let m = puml_to_board(super::PUML_SAMPLES[4].1, false).unwrap();
        assert!(m.els.iter().all(|e| e.is_mind()));
        let m = code_to_board(super::MERMAID_SAMPLES[2].1, CodeLang::Mermaid, false, false).unwrap();
        assert!(m.els.iter().all(|e| e.is_mind()) && m.els.len() == 10);
        // 类图的继承线吸附到图形上
        let c = puml_to_board(super::PUML_SAMPLES[3].1, false).unwrap();
        assert!(c.els.iter().any(|e| matches!(&e.kind, ElKind::Connector { a: End::Bound { .. }, .. })));
        // 饼图扇形变成路径元素
        let p = code_to_board(super::MERMAID_SAMPLES[3].1, CodeLang::Mermaid, false, false).unwrap();
        assert_eq!(p.els.iter().filter(|e| matches!(e.kind, ElKind::Path { .. })).count(), 3);
    }

    #[test]
    fn wrong_language_is_reported() {
        assert!(code_to_board("flowchart TD\nA-->B", CodeLang::Puml, false, false).is_err());
        assert!(code_to_board("@startuml\nA -> B\n@enduml", CodeLang::Mermaid, false, false).is_err());
        assert!(code_to_board("hello", CodeLang::Mermaid, false, false).is_err());
    }

    #[test]
    fn mermaid_mind_levels() {
        let o = mermaid_mind_outline("mindmap\n  root((中心))\n    A[甲]\n      A1\n    B{{乙}}").unwrap();
        assert_eq!(o, vec![(1, "中心".into()), (2, "甲".into()), (3, "A1".into()), (2, "乙".into())]);
    }

    /// task/puml-corpus.json：各类图的常见写法，除了明确不支持的类型都必须能渲染（新增写法踩坑后往里加样例）。
    #[test]
    fn corpus() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/task/puml-corpus.json");
        // 语料是本地文件，不在仓库里；没有时跳过
        let Ok(raw) = std::fs::read_to_string(path) else { return eprintln!("跳过：缺少 {path}") };
        let m: std::collections::BTreeMap<String, String> = serde_json::from_str(&raw).unwrap();
        let unsupported = ["empty", "gantt", "json", "salt"];
        let bad: Vec<String> = m
            .iter()
            .filter(|(k, _)| !unsupported.contains(&k.as_str()))
            .filter_map(|(k, src)| puml_to_board(src, false).err().map(|e| format!("{k}: {e}")))
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
    }
}
