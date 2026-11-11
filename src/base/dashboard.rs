//! 仪表盘（R14，精简版）：和数据表平级。组件有指标卡、柱状图、条形图、折线图、饼图、透视表、文本；
//! 每个组件选数据源（表 + 视图）、维度、指标和汇总方式，可筛选、排序、取前 N 项；12 列网格里拖动和缩放。

use super::config::CondSet;
use super::editor::{BaseEditor, Drag, PickItem, Popup};
use super::formula::{fmt_num, FVal};
use super::model::*;
use super::widgets::*;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::collections::HashMap;
use std::rc::Rc;

pub(super) const WIDGET_KINDS: [(&str, &str, &str); 7] = [
    ("metric", "指标卡", "sigma"),
    ("bar", "柱状图", "chart-bar"),
    ("hbar", "条形图", "rows"),
    ("line", "折线图", "chart-line"),
    ("pie", "饼图", "chart-pie"),
    ("pivot", "透视表", "table"),
    ("text", "文本", "type"),
];
pub(super) const AGGS: [(&str, &str); 5] = [("count", "记录数"), ("sum", "求和"), ("avg", "平均值"), ("max", "最大值"), ("min", "最小值")];
const ROW_H: f32 = 84.;
const GAP: f32 = 12.;

/// 组件数据：维度标签 + 数值；透视表另有列维度和矩阵。
#[derive(Default)]
pub(super) struct WData {
    pub labels: Vec<String>,
    pub colors: Vec<Option<u8>>,
    pub values: Vec<f64>,
    pub total: f64,
    pub cols: Vec<String>,
    pub matrix: Vec<Vec<f64>>,
}

fn agg(vals: &[f64], how: &str, n: usize) -> f64 {
    match how {
        "sum" => vals.iter().sum(),
        "avg" if !vals.is_empty() => vals.iter().sum::<f64>() / vals.len() as f64,
        "max" => vals.iter().cloned().fold(f64::MIN, f64::max).max(if vals.is_empty() { 0. } else { f64::MIN }),
        "min" => vals.iter().cloned().fold(f64::MAX, f64::min).min(if vals.is_empty() { 0. } else { f64::MAX }),
        "avg" => 0.,
        _ => n as f64,
    }
}

fn short(n: f64) -> String {
    if n.abs() >= 1e8 {
        format!("{}亿", fmt_num((n / 1e6).round() / 100.))
    } else if n.abs() >= 1e4 {
        format!("{}万", fmt_num((n / 100.).round() / 100.))
    } else {
        fmt_num((n * 100.).round() / 100.)
    }
}

impl Base {
    /// 计算一个组件的数据。
    pub(super) fn widget_data(&self, w: &Widget) -> WData {
        let mut out = WData::default();
        let Some(t) = self.table(&w.table) else { return out };
        let tid = t.meta.id.clone();
        let base_rows: Vec<String> = match t.views.get(&w.view) {
            Some(v) => self.view_records(&tid, v, ""),
            None => t.order.clone(),
        };
        let rows: Vec<String> = base_rows.into_iter().filter(|r| self.record_matches(&tid, r, &w.filter)).collect();
        let how = if w.field.is_empty() { "count" } else { w.agg.as_str() };
        let num = |r: &String| -> Option<f64> {
            if w.field.is_empty() {
                return None;
            }
            match self.fval(&tid, r, &w.field) {
                FVal::List(l) => Some(l.iter().filter_map(|x| x.num()).sum()),
                v if v.is_null() => None,
                v => v.num(),
            }
        };
        let all: Vec<f64> = rows.iter().filter_map(num).collect();
        out.total = agg(&all, how, rows.len());
        if w.kind == "metric" || w.kind == "text" || w.dim.is_empty() {
            return out;
        }
        let mut order: Vec<(String, String, Option<u8>)> = vec![];
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for r in &rows {
            for (k, label, c) in self.group_keys(&tid, r, &w.dim, true) {
                if !groups.contains_key(&k) {
                    order.push((k.clone(), label, c));
                }
                groups.entry(k).or_default().push(r.clone());
            }
        }
        self.order_groups(&tid, &w.dim, &mut order, false);
        let mut items: Vec<(String, Option<u8>, f64, Vec<String>)> = order
            .into_iter()
            .map(|(k, label, c)| {
                let rs = groups.remove(&k).unwrap_or_default();
                let v: Vec<f64> = rs.iter().filter_map(num).collect();
                // 「未分组」固定灰色，不和第一个选项撞色
                let c = if k.is_empty() { Some(11) } else { c };
                (label, c, agg(&v, how, rs.len()), rs)
            })
            .collect();
        if w.desc || w.top > 0 {
            items.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            if !w.desc {
                items.reverse();
            }
        }
        if w.top > 0 && items.len() > w.top {
            if !w.desc {
                items.reverse();
                items.truncate(w.top);
                items.reverse();
            } else {
                items.truncate(w.top);
            }
        }
        if w.kind == "pivot" && !w.dim2.is_empty() {
            let mut cols: Vec<(String, String, Option<u8>)> = vec![];
            for r in &rows {
                for k in self.group_keys(&tid, r, &w.dim2, true) {
                    if !cols.iter().any(|c| c.0 == k.0) {
                        cols.push(k);
                    }
                }
            }
            self.order_groups(&tid, &w.dim2, &mut cols, false);
            for (_, _, _, rs) in &items {
                let row: Vec<f64> = cols
                    .iter()
                    .map(|(ck, _, _)| {
                        let sub: Vec<&String> = rs.iter().filter(|r| self.group_keys(&tid, r, &w.dim2, true).iter().any(|k| &k.0 == ck)).collect();
                        let v: Vec<f64> = sub.iter().filter_map(|r| num(r)).collect();
                        agg(&v, how, sub.len())
                    })
                    .collect();
                out.matrix.push(row);
            }
            out.cols = cols.into_iter().map(|c| c.1).collect();
        }
        for (label, c, v, _) in items {
            out.labels.push(label);
            out.colors.push(c);
            out.values.push(v);
        }
        out
    }
}

impl BaseEditor {
    pub(super) fn update_dash(&mut self, cx: &mut Context<Self>, merge: bool, f: impl FnOnce(&mut Dashboard)) {
        let Some(id) = self.dash.clone() else { return };
        let mut m = self.b().meta.clone();
        if let Some(d) = m.dashboards.iter_mut().find(|d| d.id == id) {
            f(d);
        }
        self.bm().set_meta(m);
        if merge {
            self.commit_merge(Some("dash"), cx)
        } else {
            self.commit(cx)
        }
    }

    pub(super) fn update_widget(&mut self, wid: &str, cx: &mut Context<Self>, merge: bool, f: impl FnOnce(&mut Widget)) {
        let wid = wid.to_string();
        self.update_dash(cx, merge, move |d| {
            if let Some(w) = d.widgets.iter_mut().find(|w| w.id == wid) {
                f(w);
            }
        });
    }

    pub(super) fn add_dashboard(&mut self, cx: &mut Context<Self>) {
        let mut m = self.b().meta.clone();
        let id = new_id("dsh");
        let n = m.dashboards.len() + 1;
        let tid = self.tid.clone();
        let mut d = Dashboard { id: id.clone(), name: format!("仪表盘 {n}"), widgets: vec![] };
        let group = self.fields().into_iter().find(|f| f.kind.has_options()).map(|f| f.id).unwrap_or_default();
        d.widgets.push(Widget {
            id: new_id("wgt"),
            kind: "metric".into(),
            title: "记录总数".into(),
            table: tid.clone(),
            agg: "count".into(),
            x: 0,
            y: 0,
            w: 3,
            h: 2,
            ..Default::default()
        });
        if !group.is_empty() {
            d.widgets.push(Widget {
                id: new_id("wgt"),
                kind: "bar".into(),
                title: "按分组统计".into(),
                table: tid.clone(),
                dim: group.clone(),
                agg: "count".into(),
                x: 3,
                y: 0,
                w: 5,
                h: 4,
                ..Default::default()
            });
            d.widgets.push(Widget {
                id: new_id("wgt"),
                kind: "pie".into(),
                title: "占比".into(),
                table: tid,
                dim: group,
                agg: "count".into(),
                x: 8,
                y: 0,
                w: 4,
                h: 4,
                ..Default::default()
            });
        }
        m.dashboards.push(d);
        self.bm().set_meta(m);
        self.commit(cx);
        self.dash = Some(id);
        self.close_popups();
        cx.notify();
    }

    fn add_widget(&mut self, kind: &str, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let dim = self.fields().into_iter().find(|f| f.kind.has_options() || f.kind == FieldType::Person).map(|f| f.id).unwrap_or_default();
        let label = WIDGET_KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("组件");
        let id = new_id("wgt");
        let kind = kind.to_string();
        let id2 = id.clone();
        self.update_dash(cx, false, move |d| {
            let y = d.widgets.iter().map(|w| w.y + w.h).max().unwrap_or(0);
            let (w, h) = match kind.as_str() {
                "metric" => (3, 2),
                "text" => (4, 2),
                "pivot" => (6, 4),
                _ => (6, 4),
            };
            d.widgets.push(Widget {
                id: id2,
                kind: kind.clone(),
                title: label.into(),
                table: tid,
                dim: if matches!(kind.as_str(), "metric" | "text") { String::new() } else { dim },
                agg: "count".into(),
                text: if kind == "text" { "在这里写说明".into() } else { String::new() },
                x: 0,
                y,
                w,
                h,
                ..Default::default()
            });
        });
        self.open_popup(Popup::Widget(id), point(px(520.), px(140.)), cx);
    }

    /// 仪表盘页：标题栏 + 12 列网格。
    pub(super) fn render_dashboard(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.dash.as_ref().and_then(|id| self.b().meta.dashboards.iter().find(|d| &d.id == id).cloned()) else {
            self.dash = None;
            return div().into_any_element();
        };
        let bounds = self.frame.borrow().get("dash").copied().unwrap_or_default();
        let width = f32::from(bounds.size.width).max(600.) - 32.;
        let col_w = (width + GAP) / 12.;
        // 拖动中的组件按鼠标位置实时预览
        let drag = match &self.drag {
            Some(Drag::Widget { id, resize, start, orig }) => {
                let dx = ((f32::from(self.mouse.x - start.x)) / col_w).round() as i32;
                let dy = ((f32::from(self.mouse.y - start.y)) / (ROW_H + GAP)).round() as i32;
                Some((
                    id.clone(),
                    if *resize {
                        (orig.0 as i32, orig.1 as i32, (orig.2 as i32 + dx).clamp(2, 12 - orig.0 as i32), (orig.3 as i32 + dy).max(2))
                    } else {
                        ((orig.0 as i32 + dx).clamp(0, 12 - orig.2 as i32), (orig.1 as i32 + dy).max(0), orig.2 as i32, orig.3 as i32)
                    },
                ))
            }
            _ => None,
        };
        let max_y = d.widgets.iter().map(|w| w.y as i32 + w.h as i32).max().unwrap_or(0) + 4;
        let mut grid = div().relative().w(px(width)).h(px(max_y as f32 * (ROW_H + GAP)));
        for w in &d.widgets {
            let (x, y, ww, hh) = match &drag {
                Some((id, g)) if id == &w.id => *g,
                _ => (w.x as i32, w.y as i32, w.w as i32, w.h as i32),
            };
            let el = self.render_widget(w, t, ww as f32 * col_w - GAP, hh as f32 * (ROW_H + GAP) - GAP, window, cx);
            grid = grid.child(div().absolute().left(px(x as f32 * col_w)).top(px(y as f32 * (ROW_H + GAP))).child(el));
        }
        let frame = self.frame.clone();
        let rename =
            self.cb_input(&format!("dash:name:{}", d.id), "仪表盘名称", &d.name, |this, text, cx| this.update_dash(cx, true, |d| d.name = text), window, cx);
        let did = d.id.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .h(px(48.))
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border)
                    .child(icon("dashboard", 16., rgb(0x7f3bf5).into()))
                    .child(div().w(px(220.)).child(input_box(t).h(px(30.)).child(rename)))
                    .child(div().flex_1())
                    .children(WIDGET_KINDS.iter().map(|(k, l, ic)| {
                        let k = k.to_string();
                        tool_btn(SharedString::from(format!("wadd-{k}")), ic, *l, false, t).on_click(cx.listener(move |this, _, _, cx| this.add_widget(&k, cx)))
                    }))
                    .child(tool_btn("dash-del", "trash", "", false, t).tooltip(tip("删除仪表盘")).on_click(cx.listener(move |this, _, _, cx| {
                        let mut m = this.b().meta.clone();
                        m.dashboards.retain(|x| x.id != did);
                        this.bm().set_meta(m);
                        this.dash = None;
                        this.commit(cx);
                        this.show_toast("已删除仪表盘 · ⌘Z 撤销", cx);
                    }))),
            )
            .child(
                div()
                    .id("dash-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(px(16.))
                    .bg(t.sidebar_bg)
                    .relative()
                    .child(
                        canvas(
                            move |b, _, _| {
                                frame.borrow_mut().insert("dash".into(), b);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .when(d.widgets.is_empty(), |el| el.child(div().p(px(40.)).text_color(t.text_muted).child("用上方按钮添加图表、指标卡或透视表")))
                    .child(grid),
            )
            .into_any_element()
    }

    fn render_widget(&mut self, w: &Widget, t: &Theme, width: f32, height: f32, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let data = self.b().widget_data(w);
        let (id1, id2, id3, id4) = (w.id.clone(), w.id.clone(), w.id.clone(), w.id.clone());
        let pos = (w.x, w.y, w.w, w.h);
        let body_h = height - 44.;
        let body: AnyElement = match w.kind.as_str() {
            "metric" => {
                let f = self.b().field(&w.table, &w.field).cloned();
                let text = match f {
                    Some(f) if w.agg != "count" => super::calc::format_fval(&f, &FVal::Num(data.total)),
                    _ => fmt_num(data.total),
                };
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .px(px(16.))
                    .child(div().text_size(px(34.)).font_weight(FontWeight::SEMIBOLD).child(text))
                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(AGGS.iter().find(|a| a.0 == w.agg).map(|a| a.1).unwrap_or("记录数")))
                    .into_any_element()
            }
            "text" => div().flex_1().px(px(16.)).text_size(px(14.)).child(w.text.clone()).into_any_element(),
            "pivot" => self.render_widget_part_2(w, t, &data),
            k if data.labels.is_empty() => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child(if w.dim.is_empty() && k != "metric" { "请在设置里选择维度字段" } else { "没有数据" })
                .into_any_element(),
            "hbar" => {
                let max = data.values.iter().cloned().fold(0., f64::max).max(1e-9);
                div()
                    .id(SharedString::from(format!("hb-{}", w.id)))
                    .flex_1()
                    .px(px(12.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(data.labels.iter().zip(&data.values).enumerate().map(|(i, (l, v))| {
                        let c = data.colors[i].unwrap_or((i % OPT_COLORS) as u8);
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().w(px(90.)).flex_none().text_size(px(12.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(l.clone()))
                            .child(div().flex_1().h(px(14.)).child(div().h_full().w(relative((*v / max) as f32)).rounded(px(3.)).bg(solid_color(c))))
                            .child(div().w(px(56.)).text_size(px(12.)).child(short(*v)))
                    }))
                    .into_any_element()
            }
            "bar" => {
                let max = data.values.iter().cloned().fold(0., f64::max).max(1e-9);
                let n = data.values.len().max(1) as f32;
                let bw = ((width - 40.) / n - 8.).clamp(6., 60.);
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .items_end()
                    .justify_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .pb(px(6.))
                    .children(data.labels.iter().zip(&data.values).enumerate().map(|(i, (l, v))| {
                        let c = data.colors[i].unwrap_or((i % OPT_COLORS) as u8);
                        let h = ((body_h - 46.).max(10.) * (*v / max) as f32).max(2.);
                        div()
                            .w(px(bw))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(2.))
                            .child(div().text_size(px(11.)).text_color(t.text_secondary).child(short(*v)))
                            .child(div().w_full().h(px(h)).rounded_t(px(3.)).bg(solid_color(c)))
                            .child(
                                div()
                                    .w(px(bw + 8.))
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .flex()
                                    .justify_center()
                                    .child(l.clone()),
                            )
                    }))
                    .into_any_element()
            }
            kind => {
                // 折线图 / 饼图：canvas 画路径
                let vals = data.values.clone();
                let colors: Vec<u8> = data.colors.iter().enumerate().map(|(i, c)| c.unwrap_or((i % OPT_COLORS) as u8)).collect();
                let line = kind == "line";
                let stroke: Hsla = t.accent;
                let chart = canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let total: f64 = vals.iter().sum::<f64>().max(1e-9);
                        if line {
                            let max = vals.iter().cloned().fold(0., f64::max).max(1e-9);
                            let n = vals.len().max(2) as f32 - 1.;
                            let pt = |i: usize, v: f64| {
                                point(
                                    b.left() + px(8. + f32::from(b.size.width - px(16.)) * i as f32 / n),
                                    b.bottom() - px(8.) - (b.size.height - px(16.)) * (v / max) as f32,
                                )
                            };
                            let mut pb = PathBuilder::stroke(px(2.));
                            for (i, v) in vals.iter().enumerate() {
                                if i == 0 {
                                    pb.move_to(pt(i, *v))
                                } else {
                                    pb.line_to(pt(i, *v))
                                }
                            }
                            if let Ok(p) = pb.build() {
                                window.paint_path(p, stroke);
                            }
                            for (i, v) in vals.iter().enumerate() {
                                let c = pt(i, *v);
                                window.paint_quad(fill(Bounds::new(c - point(px(3.), px(3.)), size(px(6.), px(6.))), stroke).corner_radii(px(3.)));
                            }
                        } else {
                            let r = (b.size.width.min(b.size.height) / 2. - px(6.)).max(px(10.));
                            let c = b.center();
                            let mut a0 = -std::f32::consts::FRAC_PI_2;
                            for (i, v) in vals.iter().enumerate() {
                                let sweep = (*v / total) as f32 * std::f32::consts::TAU;
                                if sweep <= 0. {
                                    continue;
                                }
                                let mut pb = PathBuilder::fill();
                                pb.move_to(c);
                                let steps = ((sweep / 0.05).ceil() as usize).max(2);
                                for s in 0..=steps {
                                    let a = a0 + sweep * s as f32 / steps as f32;
                                    pb.line_to(point(c.x + r * a.cos(), c.y + r * a.sin()));
                                }
                                pb.close();
                                if let Ok(p) = pb.build() {
                                    window.paint_path(p, solid_color(colors[i]));
                                }
                                a0 += sweep;
                            }
                        }
                    },
                )
                .size_full();
                let total: f64 = data.values.iter().sum::<f64>().max(1e-9);
                let legend = div().flex().flex_col().gap(px(4.)).w(px(130.)).flex_none().children(
                    data.labels.iter().zip(&data.values).enumerate().take(12).map(|(i, (l, v))| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .child(div().size(px(8.)).rounded(px(2.)).bg(solid_color(data.colors[i].unwrap_or((i % OPT_COLORS) as u8))))
                            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(l.clone()))
                            .child(div().text_color(t.text_muted).child(if line {
                                short(*v)
                            } else {
                                format!("{}%", fmt_num((v / total * 1000.).round() / 10.))
                            }))
                    }),
                );
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .px(px(12.))
                    .pb(px(8.))
                    .child(div().flex_1().h(px(body_h - 8.)).child(chart))
                    .child(legend)
                    .into_any_element()
            }
        };
        div()
            .id(SharedString::from(format!("w-{}", w.id)))
            .group("wdg")
            .relative()
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .child(self.render_widget_part(w, t, cx, id1.clone(), id2.clone(), id3.clone(), pos))
            .child(body)
            .child(
                div()
                    .id(SharedString::from(format!("wrs-{}", w.id)))
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .size(px(14.))
                    .cursor_nwse_resize()
                    .invisible()
                    .group_hover("wdg", |s| s.visible())
                    .child(icon("grip", 10., t.text_muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.drag = Some(Drag::Widget { id: id4.clone(), resize: true, start: ev.position, orig: pos });
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element()
    }
    /// 从 `render_widget` 拆出。
    fn render_widget_part_2(&self, w: &Widget, t: &Theme, data: &WData) -> AnyElement {
        let head = div()
            .flex()
            .flex_row()
            .border_b_1()
            .border_color(t.border)
            .child(div().w(px(120.)).flex_none().px(px(8.)).text_size(px(12.)).text_color(t.text_muted).child(""))
            .children(data.cols.iter().map(|c| div().w(px(90.)).flex_none().px(px(8.)).text_size(px(12.)).text_color(t.text_muted).child(c.clone())));
        let rows = data.labels.iter().enumerate().map(|(i, l)| {
            div()
                .flex()
                .flex_row()
                .h(px(28.))
                .items_center()
                .border_b_1()
                .border_color(t.border)
                .child(div().w(px(120.)).flex_none().px(px(8.)).text_size(px(12.5)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(l.clone()))
                .children(
                    data.matrix
                        .get(i)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|v| div().w(px(90.)).flex_none().px(px(8.)).text_size(px(12.5)).child(short(v))),
                )
                .child(div().w(px(90.)).flex_none().px(px(8.)).text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(short(data.values[i])))
        });
        div()
            .id(SharedString::from(format!("pv-{}", w.id)))
            .flex_1()
            .px(px(12.))
            .overflow_scroll()
            .child(head.child(div().w(px(90.)).px(px(8.)).text_size(px(12.)).text_color(t.text_muted).child("合计")))
            .children(rows)
            .into_any_element()
    }

    /// 从 `render_widget` 拆出。
    fn render_widget_part(&self, w: &Widget, t: &Theme, cx: &mut Context<Self>, id1: String, id2: String, id3: String, pos: (u8, u8, u8, u8)) -> AnyElement {
        div()
            .id(SharedString::from(format!("wh-{}", w.id)))
            .h(px(40.))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .px(px(14.))
            .gap(px(6.))
            .cursor_move()
            .child(
                div().flex_1().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).overflow_hidden().whitespace_nowrap().text_ellipsis().child(w.title.clone()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("wset-{}", w.id)))
                    .invisible()
                    .group_hover("wdg", |s| s.visible())
                    .cursor_pointer()
                    .child(icon("settings", 14., t.text_muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_popup(Popup::Widget(id1.clone()), ev.position, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("wdel-{}", w.id)))
                    .invisible()
                    .group_hover("wdg", |s| s.visible())
                    .cursor_pointer()
                    .child(icon("trash", 14., t.text_muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let id = id2.clone();
                            this.update_dash(cx, false, move |d| d.widgets.retain(|w| w.id != id));
                        }),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag::Widget { id: id3.clone(), resize: false, start: ev.position, orig: pos });
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    pub(super) fn dashboard_drop(&mut self, d: Drag, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Drag::Widget { id, resize, start, orig } = d else { return };
        let bounds = self.frame.borrow().get("dash").copied().unwrap_or_default();
        let col_w = (f32::from(bounds.size.width).max(600.) - 32. + GAP) / 12.;
        let dx = (f32::from(pos.x - start.x) / col_w).round() as i32;
        let dy = (f32::from(pos.y - start.y) / (ROW_H + GAP)).round() as i32;
        if dx == 0 && dy == 0 {
            return;
        }
        self.update_widget(&id, cx, false, move |w| {
            if resize {
                w.w = (orig.2 as i32 + dx).clamp(2, 12 - orig.0 as i32) as u8;
                w.h = (orig.3 as i32 + dy).max(2) as u8;
            } else {
                w.x = (orig.0 as i32 + dx).clamp(0, 12 - orig.2 as i32) as u8;
                w.y = (orig.1 as i32 + dy).max(0) as u8;
            }
        });
    }

    /// 组件设置面板。
    pub(super) fn widget_panel(&mut self, wid: String, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let w = {
            let b = self.b();
            self.dash.as_ref().and_then(|d| b.meta.dashboards.iter().find(|x| &x.id == d)).and_then(|d| d.widgets.iter().find(|x| x.id == wid).cloned())
        };
        let Some(w) = w else { return div().into_any_element() };
        let label = |s: &'static str| div().text_size(px(12.)).text_color(t.text_muted).child(s);
        let w1 = wid.clone();
        let title =
            self.cb_input(&format!("w:title:{wid}"), "标题", &w.title, move |this, text, cx| this.update_widget(&w1, cx, true, |w| w.title = text), window, cx);
        let mut body = div().flex().flex_col().gap(px(8.)).px(px(14.)).pb(px(12.)).child(label("标题")).child(input_box(t).child(title));
        let kind_label = WIDGET_KINDS.iter().find(|k| k.0 == w.kind).map(|k| k.1).unwrap_or("");
        let wk = wid.clone();
        body = body.child(label("类型")).child(select_box("w-kind", kind_label, None, t).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                let items = WIDGET_KINDS.iter().map(|(k, l, ic)| PickItem::new(*k, *l).icon(ic)).collect();
                let wid = wk.clone();
                this.open_pick_cb(items, vec![], false, ev.position, move |this, k, cx| this.update_widget(&wid, cx, false, |w| w.kind = k), cx);
            }),
        ));
        if w.kind == "text" {
            let w2 = wid.clone();
            let text = self.cb_input(
                &format!("w:text:{wid}"),
                "文本内容",
                &w.text,
                move |this, text, cx| this.update_widget(&w2, cx, true, |w| w.text = text),
                window,
                cx,
            );
            body = body.child(label("内容")).child(input_box(t).child(text));
        } else {
            let tname = self.b().table(&w.table).map(|t| t.meta.name.clone()).unwrap_or("选择数据表".into());
            let vname = self.b().view(&w.table, &w.view).map(|v| v.name.clone()).unwrap_or("全部记录".into());
            let (wt, wv) = (wid.clone(), wid.clone());
            let tb = w.table.clone();
            body = body.child(label("数据源")).child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .child(select_box("w-tb", tname, Some("v-grid"), t).flex_1().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let items = this.table_items();
                            let wid = wt.clone();
                            this.open_pick_cb(
                                items,
                                vec![],
                                false,
                                ev.position,
                                move |this, tid, cx| {
                                    this.update_widget(&wid, cx, false, |w| {
                                        w.table = tid;
                                        w.view.clear();
                                        w.dim.clear();
                                        w.dim2.clear();
                                        w.field.clear();
                                        w.filter.conds.clear();
                                    })
                                },
                                cx,
                            );
                        }),
                    ))
                    .child(select_box("w-vw", vname, None, t).flex_1().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let mut items = vec![PickItem::new("", "全部记录")];
                            items.extend(
                                this.b()
                                    .table(&tb)
                                    .map(|t| {
                                        t.meta
                                            .views
                                            .iter()
                                            .filter_map(|v| t.views.get(v))
                                            .filter(|v| v.kind != ViewKind::Form)
                                            .map(|v| PickItem::new(v.id.clone(), v.name.clone()).icon(v.kind.icon()))
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default(),
                            );
                            let wid = wv.clone();
                            this.open_pick_cb(items, vec![], false, ev.position, move |this, v, cx| this.update_widget(&wid, cx, false, |w| w.view = v), cx);
                        }),
                    )),
            );
            let fname = |f: &str| self.b().field(&w.table, f).map(|x| x.name.clone());
            if w.kind != "metric" {
                let (wd, wd2, tb, tb2) = (wid.clone(), wid.clone(), w.table.clone(), w.table.clone());
                body = body.child(label(if w.kind == "pivot" { "行维度 / 列维度" } else { "维度（按哪个字段分组）" })).child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(select_box("w-dim", fname(&w.dim).unwrap_or("选择字段".into()), None, t).flex_1().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                let items = this.field_items_of(&tb, |f| f.kind != FieldType::Attachment);
                                let wid = wd.clone();
                                this.open_pick_cb(items, vec![], false, ev.position, move |this, f, cx| this.update_widget(&wid, cx, false, |w| w.dim = f), cx);
                            }),
                        ))
                        .when(w.kind == "pivot", |d| {
                            d.child(select_box("w-dim2", fname(&w.dim2).unwrap_or("选择列维度".into()), None, t).flex_1().on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    let items = this.field_items_of(&tb2, |f| f.kind != FieldType::Attachment);
                                    let wid = wd2.clone();
                                    this.open_pick_cb(
                                        items,
                                        vec![],
                                        false,
                                        ev.position,
                                        move |this, f, cx| this.update_widget(&wid, cx, false, |w| w.dim2 = f),
                                        cx,
                                    );
                                }),
                            ))
                        }),
                );
            }
            let (wf, wa, tb) = (wid.clone(), wid.clone(), w.table.clone());
            let agg_label = AGGS.iter().find(|a| a.0 == w.agg).map(|a| a.1).unwrap_or("记录数");
            body = body.child(label("指标")).child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .child(select_box("w-agg", agg_label, None, t).w(px(110.)).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let items = AGGS.iter().map(|(k, l)| PickItem::new(*k, *l)).collect();
                            let wid = wa.clone();
                            this.open_pick_cb(
                                items,
                                vec![],
                                false,
                                ev.position,
                                move |this, a, cx| {
                                    this.update_widget(&wid, cx, false, move |w| {
                                        if a == "count" {
                                            w.field.clear();
                                        }
                                        w.agg = a;
                                    })
                                },
                                cx,
                            );
                        }),
                    ))
                    .when(w.agg != "count", |d| {
                        d.child(select_box("w-fld", fname(&w.field).unwrap_or("选择数字字段".into()), None, t).flex_1().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                let items = this.field_items_of(&tb, |f| f.kind.numeric() || matches!(f.kind, FieldType::Formula | FieldType::Lookup));
                                let wid = wf.clone();
                                this.open_pick_cb(
                                    items,
                                    vec![],
                                    false,
                                    ev.position,
                                    move |this, f, cx| this.update_widget(&wid, cx, false, |w| w.field = f),
                                    cx,
                                );
                            }),
                        ))
                    }),
            );
            if w.kind != "metric" {
                let (ws, wtop) = (wid.clone(), wid.clone());
                let top = self.cb_input(
                    &format!("w:top:{wid}"),
                    "前 N 项（0 不限）",
                    &w.top.to_string(),
                    move |this, text, cx| this.update_widget(&wtop, cx, true, |w| w.top = text.trim().parse().unwrap_or(0)),
                    window,
                    cx,
                );
                body = body.child(label("排序与数量")).child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(
                            select_box(
                                "w-sort",
                                if w.desc {
                                    "按数值从大到小"
                                } else if w.top > 0 {
                                    "按数值从小到大"
                                } else {
                                    "按维度顺序"
                                },
                                None,
                                t,
                            )
                            .flex_1()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    let wid = ws.clone();
                                    this.open_pick_cb(
                                        vec![PickItem::new("0", "按维度顺序 / 从小到大"), PickItem::new("1", "按数值从大到小")],
                                        vec![],
                                        false,
                                        ev.position,
                                        move |this, k, cx| this.update_widget(&wid, cx, false, |w| w.desc = k == "1"),
                                        cx,
                                    );
                                }),
                            ),
                        )
                        .child(input_box(t).w(px(130.)).child(top)),
                );
            }
            body = body.child(label("筛选（全部满足）"));
            let tb = w.table.clone();
            for (i, c) in w.filter.conds.iter().enumerate() {
                let wc = wid.clone();
                let set: CondSet = Rc::new(move |this, c, cx| {
                    this.update_widget(&wc, cx, true, move |w| {
                        if let Some(x) = w.filter.conds.get_mut(i) {
                            *x = c;
                        }
                    })
                });
                let ed = self.cond_editor(&format!("w{wid}c{i}"), &tb, c, set, t, window, cx);
                let wx = wid.clone();
                body = body.child(div().flex().flex_row().items_center().gap(px(6.)).child(ed).child(
                    div().id(("w-cdel", i)).cursor_pointer().child(icon("x", 12., t.text_muted)).on_click(cx.listener(move |this, _, _, cx| {
                        this.update_widget(&wx, cx, false, |w| {
                            if i < w.filter.conds.len() {
                                w.filter.conds.remove(i);
                            }
                        })
                    })),
                ));
            }
            let wa2 = wid.clone();
            body = body.child(div().id("w-cadd").text_size(px(12.5)).text_color(t.accent).cursor_pointer().child("+ 添加筛选条件").on_click(cx.listener(
                move |this, _, _, cx| {
                    let Some(f) = this.b().table(&tb).and_then(|t| t.primary().cloned()) else { return };
                    this.update_widget(&wa2, cx, false, |w| w.filter.conds.push(super::config::blank_cond(&f)));
                },
            )));
        }
        panel(t)
            .w(px(460.))
            .child(div().px(px(14.)).pt(px(10.)).pb(px(8.)).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("组件设置"))
            .child(div().id("w-body").max_h(px(560.)).overflow_y_scroll().child(body))
            .into_any_element()
    }
}
