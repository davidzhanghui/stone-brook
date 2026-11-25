//! 画板（P8-5）与流程图编辑器（P8-3）。
//!
//! - `model.rs`：元素数据模型（图形 / 文本 / 便签 / 图片 / 画笔 / 连线 / 分区）
//! - `view.rs` + `interact.rs` + `ops.rs` + `ui.rs` + `selbar.rs`：全屏编辑器（交互与界面）
//! - 本文件：与文本绘图互相转换（Mermaid ⇄ 流程图）、自动布局

mod code;
pub use code::mermaid_board;
pub mod drawio;
mod find;
pub mod iconlib;
pub mod icons;
mod interact;
mod library;
mod mind;
pub mod model;
mod ops;
mod selbar;
mod seq;
mod table;
mod templates;
mod ui;
pub mod view;
pub mod wire;

use crate::canvas::{rect, Arrow, EstimateMeasure, Item, ShapeKind};
use crate::diagram::sugiyama::{self, Dir, LEdge, LNode, Spacing};
use model::*;
use std::collections::{BTreeMap, HashMap};

/// Mermaid 流程图 → 可编辑流程图。
pub fn from_mermaid(src: &str, dark: bool) -> Option<BoardDoc> {
    let (lines, _) = crate::diagram::preprocess(src);
    let first = lines.first()?.1.trim().to_lowercase();
    if !(first.starts_with("flowchart") || first.starts_with("graph")) {
        return None;
    }
    let g = crate::diagram::flowchart::parse(&lines[0].1, &lines[1..]).ok()?;
    let theme = crate::diagram::Theme::default_light();
    let scene = g.to_scene(&theme, &mut EstimateMeasure, &BTreeMap::new());
    // 各条边的走线：端口在节点边上，中间为绕开其他节点的拐点
    let routes: HashMap<String, &Vec<crate::canvas::Pt>> =
        scene.items.iter().filter_map(|i| if let Item::Line(l) = i { Some((l.id.clone(), &l.points)) } else { None }).collect();
    let mut b = BoardDoc::new(true);
    let mut map: HashMap<String, u64> = HashMap::new();
    // 子图（带标题的大框）放在最底层
    for item in &scene.items {
        if let Item::Shape(s) = item {
            if s.id.is_empty() && !s.text.is_empty() {
                let mut el = b.new_el(ElKind::Shape(ShapeKind::RoundRect), s.rect, dark);
                el.fill = None;
                el.dash = true;
                el.stroke = crate::canvas::Col::rgb(0xc9cdd4);
                el.text = s.text.clone();
                el.font_size = 13.;
                b.els.push(el);
            }
        }
    }
    for item in &scene.items {
        if let Item::Shape(s) = item {
            if !s.id.is_empty() {
                let mut el = b.new_el(ElKind::Shape(s.kind), s.rect, dark);
                el.text = s.text.clone();
                el.font_size = 14.;
                if let Some(f) = g.index.get(&s.id).and_then(|&i| g.nodes[i].style.fill) {
                    el.fill = Some(f);
                }
                map.insert(s.id.clone(), el.id);
                b.els.push(el);
            }
        }
    }
    for (k, e) in g.edges.iter().enumerate() {
        if e.invisible {
            continue;
        }
        let (Some(&a), Some(&c)) = (map.get(&g.nodes[e.from].id), map.get(&g.nodes[e.to].id)) else { continue };
        let pts = routes.get(&format!("edge:{k}")).filter(|p| p.len() >= 2 && e.from != e.to);
        // 端口 → 元素内的归一化坐标；(0.5, 0.5) 表示「自动」，没有走线时用它
        let anchor = |id: u64, p: Option<crate::canvas::Pt>| -> End {
            match (p, b.get(id)) {
                (Some(p), Some(t)) => {
                    let (ax, ay) = (((p.x - t.x) / t.w.max(1.)).clamp(0., 1.), ((p.y - t.y) / t.h.max(1.)).clamp(0., 1.));
                    let ax = if (ax - 0.5).abs() < 0.01 && (ay - 0.5).abs() < 0.01 { 0.511 } else { ax };
                    End::Bound { el: id, ax, ay }
                }
                _ => End::Bound { el: id, ax: 0.5, ay: 0.5 },
            }
        };
        let mut el = b.new_el(
            ElKind::Connector {
                a: anchor(a, pts.map(|p| p[0])),
                b: anchor(c, pts.map(|p| *p.last().unwrap())),
                route: if e.from == e.to { Route::Curve } else { Route::RoundElbow },
                start: e.start,
                end: if e.end == Arrow::None && e.start == Arrow::None { Arrow::None } else { e.end },
            },
            rect(0., 0., 0., 0.),
            dark,
        );
        el.text = e.label.clone();
        el.dash = e.dash;
        el.font_size = 14.;
        if e.thick {
            el.stroke_w = 3.;
        }
        let id = el.id;
        b.els.push(el);
        if let Some(p) = pts {
            b.set_via(id, &p[1..p.len() - 1]);
        }
    }
    Some(b)
}

/// 流程图 → Mermaid 源码。
pub fn to_mermaid(b: &BoardDoc) -> String {
    let mut out = String::from("flowchart TD\n");
    let mut names: HashMap<u64, String> = HashMap::new();
    for (k, el) in b.els.iter().filter(|e| matches!(e.kind, ElKind::Shape(_) | ElKind::Sticky | ElKind::Text)).enumerate() {
        let name = format!("n{}", k + 1);
        let text = el.text.replace('"', "'").replace('\n', "<br>");
        let text = if text.is_empty() { " ".into() } else { text };
        let (o, c) = match el.shape_kind() {
            ShapeKind::RoundRect => ("(", ")"),
            ShapeKind::Stadium => ("([", "])"),
            ShapeKind::Diamond => ("{", "}"),
            ShapeKind::Circle | ShapeKind::Ellipse => ("((", "))"),
            ShapeKind::Cylinder => ("[(", ")]"),
            ShapeKind::Hexagon => ("{{", "}}"),
            ShapeKind::Parallelogram => ("[/", "/]"),
            ShapeKind::Subroutine => ("[[", "]]"),
            _ => ("[", "]"),
        };
        out.push_str(&format!("    {name}{o}\"{text}\"{c}\n"));
        names.insert(el.id, name);
    }
    for el in &b.els {
        if let ElKind::Connector { a: End::Bound { el: x, .. }, b: End::Bound { el: y, .. }, end, .. } = &el.kind {
            if let (Some(nx), Some(ny)) = (names.get(x), names.get(y)) {
                let op = match (el.dash, *end == Arrow::None) {
                    (true, false) => "-.->",
                    (true, true) => "-.-",
                    (false, true) => "---",
                    _ => "-->",
                };
                if el.text.is_empty() {
                    out.push_str(&format!("    {nx} {op} {ny}\n"));
                } else {
                    out.push_str(&format!("    {nx} {op}|{}| {ny}\n", el.text.replace('|', "/")));
                }
            }
        }
    }
    out
}

/// 一键自动布局：用 Sugiyama 重新排列形状（锁定的元素保持不动），连线自动跟随。
pub fn auto_layout(b: &mut BoardDoc) {
    let shapes: Vec<usize> =
        b.els.iter().enumerate().filter(|(_, e)| matches!(e.kind, ElKind::Shape(_) | ElKind::Sticky) && !e.locked).map(|(i, _)| i).collect();
    if shapes.is_empty() {
        return;
    }
    let pos_of: HashMap<u64, usize> = shapes.iter().enumerate().map(|(k, &i)| (b.els[i].id, k)).collect();
    let nodes: Vec<LNode> = shapes.iter().map(|&i| LNode { w: b.els[i].w, h: b.els[i].h, cluster: None }).collect();
    let mut edges = vec![];
    for el in &b.els {
        if let ElKind::Connector { a: End::Bound { el: x, .. }, b: End::Bound { el: y, .. }, .. } = &el.kind {
            if let (Some(&fx), Some(&fy)) = (pos_of.get(x), pos_of.get(y)) {
                edges.push(LEdge { from: fx, to: fy, minlen: 1, label_w: 0. });
            }
        }
    }
    let origin = shapes.iter().map(|&i| b.els[i].rect()).reduce(|a, c| a.union(&c)).unwrap();
    let lay = sugiyama::layout(&nodes, &edges, Dir::TB, &Spacing { node: 48., rank: 64. });
    for (k, &i) in shapes.iter().enumerate() {
        let c = lay.pos[k];
        b.els[i].x = origin.x + c.x - b.els[i].w / 2.;
        b.els[i].y = origin.y + c.y - b.els[i].h / 2.;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mermaid_roundtrip() {
        let b = super::from_mermaid("flowchart TD\n A[开始] --> B{判断}\n B -->|是| C([结束])", false).unwrap();
        assert_eq!(b.els.iter().filter(|e| e.is_connector()).count(), 2);
        let src = super::to_mermaid(&b);
        assert!(src.contains("开始") && src.contains("|是|"));
        let back = super::from_mermaid(&src, false).unwrap();
        assert_eq!(back.els.len(), b.els.len());
    }
}
