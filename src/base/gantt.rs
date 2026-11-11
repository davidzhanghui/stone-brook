//! 甘特视图：左侧任务列表 + 右侧时间轴；拖动进度条整体平移，拖两端调整开始 / 截止日期，红线标出今天。

use super::editor::{BaseEditor, Drag, Popup};
use super::formula::{self, FVal};
use super::model::*;
use super::widgets::*;
use crate::theme::Theme;
use crate::ui::icon;
use chrono::{Datelike, Local, TimeZone};
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::Value;

const DAY: i64 = 86_400_000;
const LIST_W: f32 = 260.;
const ROW_H: f32 = 40.;
const HEAD_H: f32 = 52.;

fn day_px(scale: u8) -> f32 {
    match scale {
        1 => 14.,
        2 => 5.,
        3 => 2.,
        4 => 0.8,
        _ => 36.,
    }
}

/// 日期是周几（0 周一 … 6 周日）。
fn wday(ms: i64) -> u8 {
    Local.timestamp_millis_opt(ms).single().map(|d| d.weekday().num_days_from_monday() as u8).unwrap_or(0)
}

/// 只算工作日时：把日期挪到最近的工作日（向后）。
fn snap_workday(v: &View, mut d: i64) -> i64 {
    for _ in 0..7 {
        if v.is_workday(wday(d)) {
            return d;
        }
        d += DAY;
    }
    d
}

/// [s, e] 之间的工作日数（含两端）。
pub(super) fn workdays_between(v: &View, s: i64, e: i64) -> i64 {
    let mut n = 0;
    let mut d = s;
    while d <= e && n < 100_000 {
        if v.is_workday(wday(d)) {
            n += 1;
        }
        d += DAY;
    }
    n
}

/// 从 s 开始数 n 个工作日后的日期（n ≥ 1，s 本身算第 1 天）。
fn add_workdays(v: &View, s: i64, n: i64) -> i64 {
    let mut d = s;
    let mut k = if v.is_workday(wday(s)) { 1 } else { 0 };
    while k < n.max(1) {
        d += DAY;
        if v.is_workday(wday(d)) {
            k += 1;
        }
    }
    d
}

/// 拖动后的 (开始, 结束)：只算工作日时开始吸附到工作日，并保持工作日数不变。
fn moved_span(v: &View, mode: u8, orig: (i64, i64), dd: i64) -> (i64, i64) {
    let (s, e) = match mode {
        1 => ((orig.0 + dd).min(orig.1), orig.1),
        2 => (orig.0, (orig.1 + dd).max(orig.0)),
        _ => (orig.0 + dd, orig.1 + dd),
    };
    if !v.workdays_only {
        return (s, e);
    }
    match mode {
        0 => {
            let n = workdays_between(v, orig.0, orig.1);
            let s = snap_workday(v, s);
            (s, add_workdays(v, s, n))
        }
        1 => (snap_workday(v, s).min(e), e),
        _ => (s, e),
    }
}

/// 甘特左侧的一行：分组标题或任务（带层级深度）。
enum GLine {
    Group(String, usize),
    Rec(String, usize),
}

impl BaseEditor {
    /// 记录的 (开始, 结束) 日期（结束缺失时视为当天）。
    fn span(&self, rid: &str, v: &View) -> Option<(i64, i64)> {
        let b = self.b();
        let s = match b.fval(&self.tid, rid, v.date_field.as_deref()?) {
            FVal::Date(d) => formula::day_start(d),
            _ => return None,
        };
        let e = v.end_field.as_deref().and_then(|f| match b.fval(&self.tid, rid, f) {
            FVal::Date(d) => Some(formula::day_start(d)),
            _ => None,
        });
        Some((s, e.unwrap_or(s).max(s)))
    }

    pub(super) fn render_gantt(&mut self, t: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.view();
        if v.date_field.as_ref().and_then(|f| self.field(f)).is_none() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .text_color(t.text_muted)
                .child("甘特视图需要开始日期和截止日期字段")
                .child(
                    primary_btn("g-set", "设置日期字段", t)
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::DateSet, ev.position, cx))),
                )
                .into_any_element();
        }
        let _ = self.colors();
        let tree = self.tree_rows();
        let rows: Vec<String> = tree.iter().map(|x| x.0.clone()).collect();
        let depth: std::collections::HashMap<String, usize> = tree.iter().map(|x| (x.0.clone(), x.1)).collect();
        // 分组：按视图的第一个分组字段（开启层级时只按顶级任务分组，子任务跟着走）
        let mut lines: Vec<GLine> = vec![];
        match v.groups.first().filter(|g| self.field(&g.field).is_some()) {
            Some(g) => {
                let mut keys: Vec<(String, String, Option<u8>)> = vec![];
                let mut members: std::collections::HashMap<String, Vec<String>> = Default::default();
                let mut cur_root = String::new();
                for r in &rows {
                    if depth.get(r).copied().unwrap_or(0) == 0 {
                        cur_root = r.clone();
                        let k = self.b().group_keys(&self.tid, r, &g.field, false).into_iter().next().unwrap_or((String::new(), "未分组".into(), None));
                        if !members.contains_key(&k.0) {
                            keys.push(k.clone());
                        }
                        members.entry(k.0).or_default().push(r.clone());
                    } else if let Some(list) = members.values_mut().find(|l| l.contains(&cur_root)) {
                        list.push(r.clone());
                    }
                }
                self.b().order_groups(&self.tid, &g.field, &mut keys, g.desc);
                for (k, label, _) in keys {
                    let list = members.remove(&k).unwrap_or_default();
                    lines.push(GLine::Group(label, list.len()));
                    lines.extend(list.into_iter().map(|r| {
                        let d = depth.get(&r).copied().unwrap_or(0);
                        GLine::Rec(r, d)
                    }));
                }
            }
            None => lines.extend(tree.iter().map(|x| GLine::Rec(x.0.clone(), x.1))),
        }
        let today = formula::today();
        let spans: Vec<Option<(i64, i64)>> = rows.iter().map(|r| self.span(r, &v)).collect();
        let span_of: std::collections::HashMap<&String, Option<(i64, i64)>> = rows.iter().zip(spans.iter().cloned()).collect();
        let lo = spans.iter().flatten().map(|s| s.0).min().unwrap_or(today).min(today) - 14 * DAY;
        let hi = spans.iter().flatten().map(|s| s.1).max().unwrap_or(today).max(today) + 60 * DAY;
        let dp = day_px(v.scale);
        let days = ((hi - lo) / DAY) as usize + 1;
        let total_w = days as f32 * dp;
        // 第一次打开时把今天滚到左侧附近
        if self.gantt_scroll == 0. {
            self.gantt_scroll = (((today - lo) / DAY) as f32 * dp - 120.).max(0.);
        }
        let width = self.frame.borrow().get("gantt").map(|b| f32::from(b.size.width) - LIST_W).unwrap_or(800.);
        self.gantt_scroll = self.gantt_scroll.clamp(0., (total_w - width).max(0.));
        let sx = self.gantt_scroll;
        let x_of = |ms: i64| ((ms - lo) / DAY) as f32 * dp - sx;
        // 拖动中的临时偏移
        let live = match &self.drag {
            Some(Drag::Bar { rid, mode, start_x, orig, .. }) => {
                let dd = (f32::from(self.mouse.x - *start_x) / dp).round() as i64 * DAY;
                let (s, e) = moved_span(&v, *mode, *orig, dd);
                Some((rid.clone(), s, e))
            }
            _ => None,
        };
        let prog_field = v.progress_field.clone().filter(|f| self.field(f).is_some());
        // 时间轴表头：月份 + 日 / 周
        let mut head_top = div().absolute().top_0().left_0().h(px(26.)).w_full();
        let mut head_bot = div().absolute().top(px(26.)).left_0().h(px(26.)).w_full();
        let mut grid_lines = div().absolute().inset_0();
        let first_visible = ((sx / dp) as i64).max(0);
        let last_visible = (((sx + width) / dp) as i64 + 1).min(days as i64);
        for i in first_visible..last_visible {
            let ms = lo + i * DAY;
            let Some(d) = Local.timestamp_millis_opt(ms).single() else { continue };
            let x = i as f32 * dp - sx;
            let top_label = match v.scale {
                3 if (d.day() == 1 && (d.month() - 1) % 3 == 0) || i == first_visible => Some(format!("{} 年 Q{}", d.year(), (d.month() - 1) / 3 + 1)),
                4 if (d.day() == 1 && d.month() == 1) || i == first_visible => Some(format!("{} 年", d.year())),
                3 | 4 => None,
                _ if d.day() == 1 || i == first_visible => Some(format!("{} 年 {} 月", d.year(), d.month())),
                _ => None,
            };
            if let Some(l) = top_label {
                head_top = head_top.child(
                    div().absolute().left(px(x.max(0.) + 6.)).top(px(5.)).text_size(px(12.)).font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(l),
                );
                grid_lines = grid_lines.child(div().absolute().left(px(x)).top_0().bottom_0().w(px(1.)).bg(t.border));
            }
            if v.scale >= 3 && d.day() == 1 {
                head_bot = head_bot.child(
                    div()
                        .absolute()
                        .left(px(x + 2.))
                        .h_full()
                        .flex()
                        .items_center()
                        .text_size(px(10.5))
                        .text_color(t.text_secondary)
                        .whitespace_nowrap()
                        .child(if v.scale == 4 && d.month() % 3 != 1 { String::new() } else { format!("{}月", d.month()) }),
                );
                grid_lines = grid_lines.child(div().absolute().left(px(x)).top_0().bottom_0().w(px(1.)).bg(t.border.opacity(0.4)));
            }
            let weekend = !v.is_workday(d.weekday().num_days_from_monday() as u8);
            match v.scale {
                0 => {
                    head_bot = head_bot.child(
                        div()
                            .absolute()
                            .left(px(x))
                            .w(px(dp))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.))
                            .text_color(if ms == today {
                                t.accent
                            } else if weekend {
                                t.text_muted
                            } else {
                                t.text_secondary
                            })
                            .child(d.day().to_string()),
                    );
                    if weekend {
                        grid_lines = grid_lines.child(div().absolute().left(px(x)).w(px(dp)).top_0().bottom_0().bg(t.sidebar_bg));
                    }
                }
                1 if d.weekday().num_days_from_monday() == 0 => {
                    head_bot = head_bot.child(
                        div()
                            .absolute()
                            .left(px(x + 3.))
                            .h_full()
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(t.text_secondary)
                            .whitespace_nowrap()
                            .child(format!("{}/{}", d.month(), d.day())),
                    );
                    grid_lines = grid_lines.child(div().absolute().left(px(x)).top_0().bottom_0().w(px(1.)).bg(t.border.opacity(0.6)));
                }
                _ => {}
            }
        }
        let today_x = x_of(today) + dp / 2.;
        let mut list = div().id("gantt-rows").flex_1().min_h(px(0.)).overflow_y_scroll().relative();
        let mut body = div().relative().w_full().h(px((lines.len() as f32 + 1.) * ROW_H));
        let df = v.date_field.clone().unwrap_or_default();
        let collapsed = v.collapsed.clone();
        let mut n_task = 0;
        for (i, line) in lines.iter().enumerate() {
            let y = i as f32 * ROW_H;
            let (rid, tdepth) = match line {
                GLine::Group(label, n) => {
                    body = body.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(y))
                            .h(px(ROW_H))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .bg(t.sidebar_bg)
                            .border_b_1()
                            .border_color(t.border)
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(label.clone())
                            .child(div().text_size(px(12.)).text_color(t.text_muted).font_weight(FontWeight::NORMAL).child(n.to_string())),
                    );
                    continue;
                }
                GLine::Rec(r, d) => (r, *d),
            };
            n_task += 1;
            let title = self.b().title(&self.tid, rid);
            let span = match &live {
                Some((r, s, e)) if r == rid => Some((*s, *e)),
                _ => span_of.get(rid).cloned().flatten(),
            };
            let r1 = rid.clone();
            let color = self.record_color(rid).map(solid_color).unwrap_or(t.accent);
            let mut lane =
                div().absolute().left(px(LIST_W)).right_0().top(px(y)).h(px(ROW_H)).border_b_1().border_color(t.border.opacity(0.5)).overflow_hidden();
            match span {
                Some((s, e)) if s == e => {
                    // 里程碑：开始等于结束（或没有截止日期）画成菱形
                    let cxm = x_of(s) + dp / 2.;
                    let r2 = rid.clone();
                    let col = color;
                    lane = lane
                        .child(
                            div()
                                .id(SharedString::from(format!("ms-{rid}")))
                                .absolute()
                                .left(px(cxm - 9.))
                                .top(px(ROW_H / 2. - 9.))
                                .size(px(18.))
                                .cursor_grab()
                                .child(
                                    canvas(
                                        |_, _, _| {},
                                        move |b, _, window, _| {
                                            let c = b.center();
                                            let mut pb = PathBuilder::fill();
                                            pb.move_to(point(c.x, b.top()));
                                            pb.line_to(point(b.right(), c.y));
                                            pb.line_to(point(c.x, b.bottom()));
                                            pb.line_to(point(b.left(), c.y));
                                            pb.close();
                                            if let Ok(p) = pb.build() {
                                                window.paint_path(p, col);
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, ev: &MouseDownEvent, _, _cx| {
                                        let v = this.view();
                                        if let Some(sp) = this.span(&r2, &v) {
                                            this.drag = Some(Drag::Bar { rid: r2.clone(), mode: 0, start_x: ev.position.x, orig: sp, moved: false });
                                        }
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(cxm + 14.))
                                .top(px(11.))
                                .text_size(px(12.))
                                .text_color(t.text_secondary)
                                .whitespace_nowrap()
                                .child(format!("{title} · {}", formula::fmt_date(s, 3))),
                        );
                }
                Some((s, e)) => {
                    lane = self.render_gantt_part(t, cx, color, dp, e, lane, &prog_field, rid, s, &title, v.clone(), sx, lo);
                }
                None => {
                    // 没有日期：点击时间轴直接排期
                    let df2 = df.clone();
                    let end = v.end_field.clone();
                    lane = lane.child(
                        div()
                            .id(SharedString::from(format!("unsched-{rid}")))
                            .absolute()
                            .inset_0()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_soft))
                            .child(div().absolute().left(px(12.)).top(px(11.)).text_size(px(12.)).text_color(t.text_muted).child("未排期 · 点击时间轴设置"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    let b = this.frame.borrow().get("gantt").copied();
                                    let Some(b) = b else { return };
                                    let x = f32::from(ev.position.x - b.left()) - LIST_W + this.gantt_scroll;
                                    let day = lo + (x / day_px(this.view().scale)) as i64 * DAY;
                                    let tid = this.tid.clone();
                                    this.bm().set_cell(&tid, &r1, &df2, Value::from(day));
                                    if let Some(e) = &end {
                                        this.bm().set_cell(&tid, &r1, e, Value::from(day + 2 * DAY));
                                    }
                                    this.commit(cx);
                                }),
                            ),
                    );
                }
            }
            let r5 = rid.clone();
            body = body.child(self.render_gantt_part_2(t, cx, &collapsed, n_task, r5.clone(), rid, tdepth, title.clone(), &tree, y)).child(lane);
        }
        body = body.child(self.render_gantt_g_add(t, cx, &lines));
        list = list
            .child(
                div().absolute().left(px(LIST_W)).top_0().right_0().h(px((lines.len() as f32 + 1.) * ROW_H).max(px(2000.))).overflow_hidden().child(grid_lines),
            )
            .child(body);
        let frame = self.frame.clone();
        let scales = [("日", 0u8), ("周", 1), ("月", 2), ("季", 3), ("年", 4)];
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .h(px(HEAD_H))
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().absolute().left(px(LIST_W)).right_0().top_0().bottom_0().overflow_hidden().child(head_top).child(head_bot))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(px(LIST_W))
                            .h_full()
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .border_r_1()
                            .border_color(t.border)
                            .bg(t.table_header)
                            .child(div().flex_1().text_size(px(12.)).text_color(t.text_muted).child("任务"))
                            .children(scales.iter().map(|(label, s)| {
                                let s = *s;
                                let on = v.scale == s;
                                div()
                                    .id(("scale", s as usize))
                                    .px(px(8.))
                                    .h(px(24.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                                    .when(!on, |d| d.text_color(t.text_secondary).hover(|s| s.bg(t.hover)))
                                    .child(*label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.gantt_scroll = 0.;
                                        this.update_view(cx, |v| v.scale = s);
                                    }))
                            })),
                    ),
            )
            .child(list)
            .when(today_x > 0. && today_x < width, |d| {
                d.child(div().absolute().top(px(HEAD_H - 6.)).bottom_0().left(px(LIST_W + today_x)).w(px(2.)).bg(rgb(0xf54a45)).opacity(0.8))
            })
            .child(
                canvas(
                    move |b, _, _| {
                        frame.borrow_mut().insert("gantt".into(), b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let d = ev.delta.pixel_delta(px(30.));
                let dx = if ev.modifiers.shift { f32::from(d.y) } else { f32::from(d.x) };
                if dx != 0. {
                    this.gantt_scroll = (this.gantt_scroll - dx).max(0.);
                    cx.notify();
                }
            }))
            .into_any_element()
    }
    /// 从 `render_gantt` 拆出。
    fn render_gantt_g_add(&self, t: &Theme, cx: &mut Context<Self>, lines: &[GLine]) -> AnyElement {
        div()
            .id("g-add")
            .absolute()
            .left_0()
            .top(px(lines.len() as f32 * ROW_H))
            .w(px(LIST_W))
            .h(px(ROW_H))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .text_color(t.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(icon("plus", 14., t.text_muted))
            .child("新增任务")
            .on_click(cx.listener(|this, _, _, cx| {
                let v = this.view();
                let mut preset = vec![];
                if let Some(f) = v.date_field.clone() {
                    preset.push((f, Value::from(formula::today())));
                }
                if let Some(f) = v.end_field.clone() {
                    preset.push((f, Value::from(formula::today() + 3 * DAY)));
                }
                if let Some(rid) = this.add_record(preset, None, cx) {
                    this.detail = Some(rid);
                }
            }))
            .into_any_element()
    }

    /// 从 `render_gantt` 拆出。
    fn render_gantt_part_2(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        collapsed: &[String],
        n_task: usize,
        r5: String,
        rid: &String,
        tdepth: usize,
        title: String,
        tree: &std::rc::Rc<Vec<(String, usize, bool)>>,
        y: f32,
    ) -> AnyElement {
        div()
            .id(SharedString::from(format!("gl-{rid}")))
            .absolute()
            .left_0()
            .top(px(y))
            .w(px(LIST_W))
            .h(px(ROW_H))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_r_1()
            .border_color(t.border)
            .bg(t.bg)
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(div().text_size(px(11.)).text_color(t.text_muted).w(px(22.)).child(n_task.to_string()))
            .child(div().w(px(tdepth as f32 * 14.)).flex_none())
            .when(tree.iter().any(|x| x.1 > 0) || !self.b().children_of(&self.tid, rid).is_empty(), |d| {
                let open = !collapsed.contains(&format!("t:{rid}"));
                let kids = !self.b().children_of(&self.tid, rid).is_empty();
                let r6 = rid.clone();
                d.child(
                    div()
                        .id(SharedString::from(format!("gtc-{rid}")))
                        .size(px(14.))
                        .flex_none()
                        .when(kids, |d| d.child(icon(if open { "chevron-down" } else { "chevron-right" }, 11., t.text_secondary)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let k = format!("t:{r6}");
                            let tid = this.tid.clone();
                            let mut v = this.view();
                            if v.collapsed.contains(&k) {
                                v.collapsed.retain(|x| x != &k);
                            } else {
                                v.collapsed.push(k);
                            }
                            this.bm().put_view(&tid, v);
                            this.bm().commit_silent();
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.detail = Some(r5.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    /// 从 `render_gantt` 拆出。
    fn render_gantt_part(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        color: Hsla,
        dp: f32,
        e: i64,
        mut lane: Div,
        prog_field: &Option<String>,
        rid: &String,
        s: i64,
        title: &String,
        v: View,
        sx: f32,
        lo: i64,
    ) -> Div {
        let x_of = |ms: i64| ((ms - lo) / DAY) as f32 * dp - sx;
        let (x0, x1) = (x_of(s), x_of(e) + dp);
        let days_n = if v.workdays_only { workdays_between(&v, s, e) } else { (e - s) / DAY + 1 };
        let progress = prog_field.as_ref().and_then(|f| self.b().fval(&self.tid, rid, f).num()).map(|p| if p > 1. { p / 100. } else { p }.clamp(0., 1.) as f32);
        let (r2, r3, r4) = (rid.clone(), rid.clone(), rid.clone());
        let handle = |id: String, mode: u8, rid: String, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(id))
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(8.))
                .when(mode == 1, |d| d.left_0())
                .when(mode == 2, |d| d.right_0())
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let v = this.view();
                        if let Some(sp) = this.span(&rid, &v) {
                            this.drag = Some(Drag::Bar { rid: rid.clone(), mode, start_x: ev.position.x, orig: sp, moved: false });
                        }
                    }),
                )
        };
        lane = lane.child(
            div()
                .id(SharedString::from(format!("bar-{rid}")))
                .absolute()
                .left(px(x0))
                .w(px((x1 - x0).max(dp)))
                .top(px(8.))
                .h(px(ROW_H - 16.))
                .rounded(px(5.))
                .bg(color)
                .shadow_sm()
                .flex()
                .items_center()
                .px(px(8.))
                .text_size(px(12.))
                .text_color(white())
                .overflow_hidden()
                .whitespace_nowrap()
                .cursor_grab()
                .relative()
                .when_some(progress, |d, p| d.child(div().absolute().left_0().top_0().bottom_0().w(relative(p)).bg(black().opacity(0.18))))
                .child(if x1 - x0 > 80. {
                    format!(
                        "{title} · {days_n} {}{}",
                        if v.workdays_only { "个工作日" } else { "天" },
                        progress.map(|p| format!(" · {}%", (p * 100.).round())).unwrap_or_default()
                    )
                } else {
                    String::new()
                })
                .child(handle(format!("bl-{rid}"), 1, r3, cx))
                .child(handle(format!("br-{rid}"), 2, r4, cx))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, _cx| {
                        let v = this.view();
                        if let Some(sp) = this.span(&r2, &v) {
                            this.drag = Some(Drag::Bar { rid: r2.clone(), mode: 0, start_x: ev.position.x, orig: sp, moved: false });
                        }
                    }),
                ),
        );
        if x1 - x0 <= 80. {
            lane = lane.child(
                div().absolute().left(px(x1 + 6.)).top(px(11.)).text_size(px(12.)).text_color(t.text_secondary).whitespace_nowrap().child(title.clone()),
            );
        }
        lane
    }

    pub(super) fn gantt_drag_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(Drag::Bar { start_x, moved, .. }) = &mut self.drag {
            if (pos.x - *start_x).abs() > px(3.) {
                *moved = true;
            }
        }
        cx.notify();
    }

    pub(super) fn gantt_drop(&mut self, d: Drag, cx: &mut Context<Self>) {
        let Drag::Bar { rid, mode, start_x, orig, moved } = d else { return };
        if !moved {
            self.detail = Some(rid);
            return;
        }
        let v = self.view();
        let dd = (f32::from(self.mouse.x - start_x) / day_px(v.scale)).round() as i64 * DAY;
        let (s, e) = moved_span(&v, mode, orig, dd);
        let tid = self.tid.clone();
        if let Some(f) = v.date_field.as_ref().filter(|f| self.field(f).is_some_and(|x| !x.kind.computed())) {
            self.bm().set_cell(&tid, &rid, f, Value::from(s));
        }
        if let Some(f) = v.end_field.as_ref().filter(|f| self.field(f).is_some_and(|x| !x.kind.computed())) {
            self.bm().set_cell(&tid, &rid, f, Value::from(e));
        }
        self.commit(cx);
    }
}
