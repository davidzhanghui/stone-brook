//! 文本绘图（P8-2）：从零实现的 Mermaid 风格渲染器。
//!
//! 源码 → 解析（每种图一个手写解析器）→ 图中间表示 → 布局（Sugiyama / 时序 / 专用）→ 场景（canvas::Scene）
//! 场景既可以用 GPUI 直接绘制，也可以导出 SVG / PNG。
//!
//! 支持：flowchart / graph、sequenceDiagram、classDiagram、stateDiagram(-v2)、erDiagram、
//! gantt、pie、mindmap、timeline；以及 PlantUML 时序图（自动转换）。

pub mod class;
pub mod er;
pub mod flowchart;
pub mod gantt;
pub mod mindmap;
pub mod pie;
pub mod plantuml;
pub mod sequence;
pub mod state;
pub mod sugiyama;
pub mod timeline;

use crate::canvas::{Col, EstimateMeasure, Measure, Scene};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct DiagramError {
    /// 出错的行号（从 1 开始），0 表示整体错误
    pub line: usize,
    pub msg: String,
}

impl DiagramError {
    pub fn new(line: usize, msg: impl Into<String>) -> Self {
        DiagramError { line, msg: msg.into() }
    }
}

pub type DResult<T> = Result<T, DiagramError>;

/// 图表主题。
#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub node_fill: Col,
    pub node_stroke: Col,
    pub text: Col,
    pub line: Col,
    pub label_bg: Col,
    pub cluster_fill: Col,
    pub cluster_stroke: Col,
    pub note_fill: Col,
    pub note_stroke: Col,
    pub muted: Col,
    pub palette: [Col; 8],
    pub rough: bool,
    pub font_size: f32,
    /// 时序图激活条的填充（None 时同节点填充）
    pub bar_fill: Option<Col>,
}

impl Theme {
    pub fn default_light() -> Theme {
        Theme {
            dark: false,
            node_fill: Col::rgb(0xeef3ff),
            node_stroke: Col::rgb(0x3b6cf6),
            text: Col::rgb(0x1f2329),
            line: Col::rgb(0x5c6370),
            label_bg: Col::rgb(0xffffff),
            cluster_fill: Col::rgb(0xf7f8fa),
            cluster_stroke: Col::rgb(0xc9cdd4),
            note_fill: Col::rgb(0xfff8db),
            note_stroke: Col::rgb(0xe6c34a),
            muted: Col::rgb(0x8f959e),
            palette: [
                Col::rgb(0x3b6cf6),
                Col::rgb(0x2ea121),
                Col::rgb(0xf5a339),
                Col::rgb(0xe5484d),
                Col::rgb(0x8b5cf6),
                Col::rgb(0x14b8a6),
                Col::rgb(0xec4899),
                Col::rgb(0x64748b),
            ],
            rough: false,
            font_size: 14.,
            bar_fill: None,
        }
    }

    pub fn dark() -> Theme {
        Theme {
            dark: true,
            node_fill: Col::rgb(0x22304d),
            node_stroke: Col::rgb(0x5b8cff),
            text: Col::rgb(0xe3e4e6),
            line: Col::rgb(0xa6a9ae),
            label_bg: Col::rgb(0x1b1c1f),
            cluster_fill: Col::rgb(0x232428),
            cluster_stroke: Col::rgb(0x45484f),
            note_fill: Col::rgb(0x3a3420),
            note_stroke: Col::rgb(0xa88b2a),
            muted: Col::rgb(0x7c8087),
            palette: [
                Col::rgb(0x5b8cff),
                Col::rgb(0x63c45a),
                Col::rgb(0xf5a339),
                Col::rgb(0xf76964),
                Col::rgb(0xb18bf5),
                Col::rgb(0x2dd4bf),
                Col::rgb(0xf472b6),
                Col::rgb(0x94a3b8),
            ],
            ..Theme::default_light()
        }
    }

    pub fn neutral() -> Theme {
        Theme {
            node_fill: Col::rgb(0xf5f5f5),
            node_stroke: Col::rgb(0x666666),
            line: Col::rgb(0x666666),
            cluster_fill: Col::rgb(0xfafafa),
            cluster_stroke: Col::rgb(0xaaaaaa),
            ..Theme::default_light()
        }
    }

    pub fn forest() -> Theme {
        Theme {
            node_fill: Col::rgb(0xcde59b),
            node_stroke: Col::rgb(0x3f6b1c),
            line: Col::rgb(0x2b3a24),
            cluster_fill: Col::rgb(0xf1f8e4),
            cluster_stroke: Col::rgb(0x9cc46a),
            ..Theme::default_light()
        }
    }

    pub fn yellow() -> Theme {
        Theme {
            node_fill: Col::rgb(0xfdeeb5),
            node_stroke: Col::rgb(0xb98a00),
            line: Col::rgb(0x5c4a12),
            cluster_fill: Col::rgb(0xfffaeb),
            cluster_stroke: Col::rgb(0xe6c86e),
            ..Theme::default_light()
        }
    }

    pub fn purple() -> Theme {
        Theme {
            node_fill: Col::rgb(0xece4ff),
            node_stroke: Col::rgb(0x7a4fe0),
            line: Col::rgb(0x4b3f6b),
            cluster_fill: Col::rgb(0xf7f3ff),
            cluster_stroke: Col::rgb(0xc4b2f3),
            ..Theme::default_light()
        }
    }

    /// 画板样式（「画板样式」）：浅蓝图形、灰色连线、实心蓝色激活条。
    pub fn board() -> Theme {
        Theme {
            node_fill: Col::rgb(0xe1eaff),
            node_stroke: Col::rgb(0x5b8cff),
            line: Col::rgb(0x8f959e),
            cluster_fill: Col::rgb(0xffffff),
            cluster_stroke: Col::rgb(0xbbbfc4),
            muted: Col::rgb(0x8f959e),
            bar_fill: Some(Col::rgb(0x3370ff)),
            ..Theme::default_light()
        }
    }

    /// Mermaid 官方默认配色（「Mermaid 绘图 · 经典样式」）。
    pub fn mermaid_classic() -> Theme {
        Theme {
            node_fill: Col::rgb(0xececff),
            node_stroke: Col::rgb(0x9370db),
            text: Col::rgb(0x333333),
            line: Col::rgb(0x333333),
            label_bg: Col::rgb(0xe8e8e8),
            cluster_fill: Col::rgb(0xffffde),
            cluster_stroke: Col::rgb(0xaaaa33),
            note_fill: Col::rgb(0xfff5ad),
            note_stroke: Col::rgb(0xaaaa33),
            ..Theme::default_light()
        }
    }

    /// PlantUML 官方默认配色（「PlantUML 绘图 · 经典样式」）。
    pub fn puml_classic() -> Theme {
        Theme {
            node_fill: Col::rgb(0xe2e2f0),
            node_stroke: Col::rgb(0x181818),
            text: Col::rgb(0x000000),
            line: Col::rgb(0x181818),
            label_bg: Col::rgb(0xffffff),
            cluster_fill: Col::rgb(0xffffff),
            cluster_stroke: Col::rgb(0x181818),
            note_fill: Col::rgb(0xfeffdd),
            note_stroke: Col::rgb(0x181818),
            ..Theme::default_light()
        }
    }

    pub fn hand(dark: bool) -> Theme {
        let base = if dark { Theme::dark() } else { Theme::default_light() };
        Theme { rough: true, node_fill: if dark { Col::rgb(0x2b3550) } else { Col::rgb(0xdfe8ff) }, ..base }
    }

    pub fn by_name(name: &str, dark: bool) -> Theme {
        match name {
            "dark" => Theme::dark(),
            "neutral" => Theme::neutral(),
            "forest" | "green" => Theme::forest(),
            "yellow" => Theme::yellow(),
            "purple" => Theme::purple(),
            "hand" | "handdrawn" | "sketch" => Theme::hand(dark),
            "board" => Theme::board(),
            "mermaid-classic" => Theme::mermaid_classic(),
            "puml-classic" => Theme::puml_classic(),
            _ => {
                if dark {
                    Theme::dark()
                } else {
                    Theme::default_light()
                }
            }
        }
    }
}

/// 配色菜单：(init 指令里的主题名, 显示名)。顺序默认 / 灰色 / 绿色 / 黄色，后面是扩展。
pub const THEMES: [(&str, &str); 7] =
    [("default", "默认"), ("neutral", "灰色"), ("forest", "绿色"), ("yellow", "黄色"), ("purple", "紫色"), ("dark", "暗色"), ("hand", "手绘")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Flowchart,
    Sequence,
    Class,
    State,
    Er,
    Gantt,
    Pie,
    Mindmap,
    Timeline,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Flowchart => "流程图",
            Kind::Sequence => "时序图",
            Kind::Class => "类图",
            Kind::State => "状态图",
            Kind::Er => "ER 图",
            Kind::Gantt => "甘特图",
            Kind::Pie => "饼图",
            Kind::Mindmap => "思维导图",
            Kind::Timeline => "时间线",
        }
    }
    /// 节点可以拖动微调的图
    pub fn draggable(self) -> bool {
        matches!(self, Kind::Flowchart | Kind::Class | Kind::State | Kind::Er)
    }
}

/// 预处理：去掉 `%%` 注释与 init 指令，返回 (有效行（保留行号）, 指定的主题)。
pub fn preprocess(src: &str) -> (Vec<(usize, String)>, Option<String>) {
    let mut theme = None;
    let mut lines = vec![];
    for (i, raw) in src.lines().enumerate() {
        let t = raw.trim_end();
        let trimmed = t.trim_start();
        if trimmed.starts_with("%%{") {
            // %%{init: {"theme": "dark"}}%%
            if let Some(pos) = trimmed.find("theme") {
                let rest = &trimmed[pos + 5..];
                let name: String = rest
                    .trim_start_matches(|c: char| c == '"' || c == '\'' || c == ':' || c.is_whitespace())
                    .chars()
                    .take_while(|c| c.is_alphanumeric())
                    .collect();
                if !name.is_empty() {
                    theme = Some(name);
                }
            }
            continue;
        }
        if trimmed.starts_with("%%") {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        lines.push((i + 1, t.to_string()));
    }
    (lines, theme)
}

pub fn detect(src: &str) -> Option<Kind> {
    let src = plantuml::maybe_convert(src);
    let (lines, _) = preprocess(&src);
    let first = lines.first()?.1.trim().to_lowercase();
    let word = first.split_whitespace().next().unwrap_or("");
    Some(match word {
        "flowchart" | "graph" => Kind::Flowchart,
        "sequencediagram" => Kind::Sequence,
        "classdiagram" | "classdiagram-v2" => Kind::Class,
        "statediagram" | "statediagram-v2" => Kind::State,
        "erdiagram" => Kind::Er,
        "gantt" => Kind::Gantt,
        "pie" => Kind::Pie,
        "mindmap" => Kind::Mindmap,
        "timeline" => Kind::Timeline,
        _ => return None,
    })
}

/// 渲染入口。theme_override 为界面上选择的主题（源码中的 init 指令优先级更高）。
pub fn render(src: &str, dark: bool, theme_override: Option<&str>, measure: &mut dyn Measure, offsets: &BTreeMap<String, (f32, f32)>) -> DResult<Scene> {
    let src = plantuml::maybe_convert(src);
    let (lines, init_theme) = preprocess(&src);
    if lines.is_empty() {
        return Err(DiagramError::new(0, "输入图表源码，例如：flowchart LR / sequenceDiagram / classDiagram"));
    }
    let name = init_theme.as_deref().or(theme_override).unwrap_or("default");
    let theme = Theme::by_name(name, dark);
    let kind = detect(&src).ok_or_else(|| DiagramError::new(lines[0].0, format!("无法识别的图表类型「{}」", lines[0].1.trim())))?;
    let body = &lines[1..];
    let header = &lines[0].1;
    let mut scene = match kind {
        Kind::Flowchart => flowchart::render(header, body, &theme, measure, offsets)?,
        Kind::Sequence => sequence::render(body, &theme, measure)?,
        Kind::Class => class::render(body, &theme, measure, offsets)?,
        Kind::State => state::render(body, &theme, measure, offsets)?,
        Kind::Er => er::render(body, &theme, measure, offsets)?,
        Kind::Gantt => gantt::render(body, &theme, measure)?,
        Kind::Pie => pie::render(header, body, &theme, measure)?,
        Kind::Mindmap => mindmap::render(body, &theme, measure)?,
        Kind::Timeline => timeline::render(body, &theme, measure)?,
    };
    scene.rough = theme.rough;
    if scene.width <= 0. {
        scene.fit(16.);
    }
    Ok(scene)
}

/// 导出用：渲染为 SVG（文本宽度用估算），出错时返回错误说明。
pub fn source_to_svg(src: &str, offsets: &BTreeMap<String, (f32, f32)>) -> String {
    match render(src, false, None, &mut EstimateMeasure, offsets) {
        Ok(mut scene) => {
            scene.bg = Some(Col::rgb(0xffffff));
            crate::canvas::svg::to_svg(&scene)
        }
        Err(e) => format!("<pre>{}</pre>", e.msg),
    }
}

// ───────────── 解析辅助 ─────────────

/// 去掉引号、把 <br> 换成换行、去掉 Markdown 字符串的反引号。
pub fn clean_text(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let s = s.strip_prefix('`').and_then(|x| x.strip_suffix('`')).unwrap_or(s);
    s.replace("<br/>", "\n").replace("<br>", "\n").replace("<br />", "\n").replace("\\n", "\n").replace("&quot;", "\"").replace("#quot;", "\"")
}

/// 多行文字的尺寸（宽, 高）。
pub fn text_size(m: &mut dyn Measure, text: &str, size: f32, bold: bool) -> (f32, f32) {
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lines.iter().map(|l| m.width(l, size, bold)).fold(0., f32::max);
    (w, lines.len() as f32 * size * 1.38)
}

/// 解析 `fill:#f9f,stroke:#333,stroke-width:4px,color:#fff,stroke-dasharray: 5 5`
#[derive(Clone, Debug, Default)]
pub struct Style {
    pub fill: Option<Col>,
    pub stroke: Option<Col>,
    pub color: Option<Col>,
    pub stroke_width: Option<f32>,
    pub dash: bool,
    pub bold: bool,
}

impl Style {
    pub fn parse(s: &str) -> Style {
        let mut st = Style::default();
        for part in s.split(',') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let v = v.trim().trim_end_matches(';');
            match k.trim() {
                "fill" => st.fill = Col::parse(v),
                "stroke" => st.stroke = Col::parse(v),
                "color" => st.color = Col::parse(v),
                "stroke-width" => st.stroke_width = v.trim_end_matches("px").trim().parse().ok(),
                "stroke-dasharray" => st.dash = true,
                "font-weight" => st.bold = v.contains("bold") || v.parse::<u32>().is_ok_and(|w| w >= 600),
                _ => {}
            }
        }
        st
    }
    pub fn merge(&mut self, o: &Style) {
        if o.fill.is_some() {
            self.fill = o.fill;
        }
        if o.stroke.is_some() {
            self.stroke = o.stroke;
        }
        if o.color.is_some() {
            self.color = o.color;
        }
        if o.stroke_width.is_some() {
            self.stroke_width = o.stroke_width;
        }
        self.dash |= o.dash;
        self.bold |= o.bold;
    }
}

/// 节点被拖动微调后，与之相连的边改为直连（重新贴边）。
pub fn apply_offsets(scene: &mut Scene, offsets: &BTreeMap<String, (f32, f32)>, edge_ends: &[(String, String)]) {
    use crate::canvas::{geom::boundary_point, Item};
    if offsets.is_empty() {
        return;
    }
    let mut rects: std::collections::HashMap<String, (crate::canvas::ShapeKind, crate::canvas::Rect)> = Default::default();
    for it in scene.items.iter_mut() {
        if let Item::Shape(s) = it {
            if let Some((dx, dy)) = offsets.get(&s.id) {
                s.rect.x += dx;
                s.rect.y += dy;
            }
            if !s.id.is_empty() {
                rects.insert(s.id.clone(), (s.kind, s.rect));
            }
        }
    }
    let mut ei = 0;
    for it in scene.items.iter_mut() {
        if let Item::Line(l) = it {
            if l.id.starts_with("edge:") {
                if let Some((a, b)) = edge_ends.get(ei) {
                    if offsets.contains_key(a) || offsets.contains_key(b) {
                        if let (Some(&(ka, ra)), Some(&(kb, rb))) = (rects.get(a), rects.get(b)) {
                            if a != b {
                                let p0 = boundary_point(ka, ra, rb.center());
                                let p1 = boundary_point(kb, rb, ra.center());
                                l.points = vec![p0, p1];
                                l.tangents = None;
                                l.corner_radius = 0.;
                                l.label_pos = None;
                            }
                        }
                    }
                }
                ei += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Scene {
        render(src, false, None, &mut EstimateMeasure, &BTreeMap::new()).unwrap_or_else(|e| panic!("{src}\n→ 第 {} 行：{}", e.line, e.msg))
    }

    #[test]
    fn all_kinds_render() {
        for src in [
            "flowchart LR\n  A[开始] --> B{判断}\n  B -->|是| C([结束])\n  B -- 否 --> D[(数据库)]",
            "sequenceDiagram\n  Alice->>John: 你好\n  John-->>Alice: 好的",
            "classDiagram\n  Animal <|-- Duck\n  class Animal {\n    +int age\n    +eat()\n  }",
            "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> [*]",
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places",
            "gantt\n  title 计划\n  dateFormat YYYY-MM-DD\n  section A\n  任务1 :a1, 2024-01-01, 3d\n  任务2 :after a1, 2d",
            "pie title 宠物\n  \"狗\" : 386\n  \"猫\" : 85",
            "mindmap\n  root((中心))\n    A\n      A1\n    B",
            "timeline\n  title 历史\n  2002 : LinkedIn\n  2004 : Facebook : Google",
        ] {
            let s = ok(src);
            assert!(!s.items.is_empty(), "{src}");
            assert!(s.width > 10. && s.height > 10.);
        }
    }

    #[test]
    fn unknown_type_reports_line() {
        let e = render("hello world", false, None, &mut EstimateMeasure, &BTreeMap::new()).unwrap_err();
        assert_eq!(e.line, 1);
    }

    #[test]
    fn init_directive_selects_theme() {
        let (_, t) = preprocess("%%{init: {'theme':'forest'}}%%\nflowchart TD\nA-->B");
        assert_eq!(t.as_deref(), Some("forest"));
    }
}
