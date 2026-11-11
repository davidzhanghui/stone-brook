//! 记录详情（展开记录）：以表单形式查看和编辑一条记录的全部字段，可上下翻看当前视图里的记录。

use super::cells::Mode;
use super::editor::BaseEditor;
use super::formula;
use super::model::*;
use super::widgets::*;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::Value;

impl BaseEditor {
    fn detail_nav(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(cur) = self.detail.clone() else { return };
        let rows = self.rows();
        let Some(i) = rows.iter().position(|r| r == &cur) else { return };
        let j = i as i32 + delta;
        if j >= 0 && (j as usize) < rows.len() {
            self.detail = Some(rows[j as usize].clone());
            self.close_popups();
            cx.notify();
        }
    }

    pub(super) fn render_detail(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rid = self.detail.clone()?;
        let rec = self.b().record(&self.tid, &rid).cloned();
        let Some(rec) = rec else {
            self.detail = None;
            return None;
        };
        if self.detail_loaded.as_ref() != Some(&rid) {
            self.inputs.retain(|k, _| !k.starts_with("d:"));
            self.detail_loaded = Some(rid.clone());
        }
        let v = self.view();
        let fields: Vec<Field> = {
            let b = self.b();
            let tb = b.table(&self.tid)?;
            tb.view_fields(&v).iter().filter_map(|f| tb.fields.get(f).cloned()).collect()
        };
        let rows = self.rows();
        let pos = rows.iter().position(|r| r == &rid);
        let title = self.b().title(&self.tid, &rid);
        let layout = self.b().table(&self.tid).map(|t| t.meta.detail_layout).unwrap_or(0);
        let fgroups = self.b().table(&self.tid).map(|t| t.meta.groups.clone()).unwrap_or_default();
        let mut items: Vec<(String, AnyElement)> = vec![];
        for (i, f) in fields.iter().enumerate() {
            let hidden = i > 0 && v.hidden.contains(&f.id);
            let raw = rec.cells.get(&f.id).cloned();
            use FieldType::*;
            let value: AnyElement = match f.kind {
                Text | Number | Url | Phone | Email | Currency | Progress => {
                    let init = match (f.kind, &raw) {
                        (Progress, Some(v)) => v.as_f64().map(|n| format!("{}%", formula::fmt_num(n * 100.))).unwrap_or_default(),
                        (_, Some(Value::Number(n))) => formula::fmt_num(n.as_f64().unwrap_or(0.)),
                        (_, Some(Value::String(s))) => s.clone(),
                        _ => String::new(),
                    };
                    let ph = match f.kind {
                        Number | Currency => "请输入数字",
                        Progress => "例如 60%",
                        Url => "https://",
                        _ => "请输入",
                    };
                    let inp = self.input(&format!("d:{}", f.id), ph, &init, false, window, cx);
                    let link = (f.kind == Url && !init.is_empty()).then(|| if init.contains("://") { init.clone() } else { format!("https://{init}") });
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(32.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(transparent_black())
                                .hover(|s| s.border_color(t.border_strong))
                                .text_size(px(14.))
                                .line_height(px(22.))
                                .child(inp),
                        )
                        .when_some(link, |d, url| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!("dl-{}", f.id)))
                                    .px(px(6.))
                                    .text_size(px(12.))
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .child("打开")
                                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                            )
                        })
                        .into_any_element()
                }
                Checkbox | Rating => {
                    div().min_h(px(32.)).px(px(8.)).flex().items_center().child(self.cell_content(&rid, f, Mode::Detail, t, cx)).into_any_element()
                }
                k if k.computed() => div()
                    .min_h(px(32.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .child(self.cell_content(&rid, f, Mode::Detail, t, cx))
                    .when(f.kind != FieldType::Button, |d| d.child(div().text_size(px(11.)).text_color(t.text_muted).child("自动计算")))
                    .into_any_element(),
                _ => {
                    let empty = is_empty_value(raw.as_ref());
                    let content = self.cell_content(&rid, f, Mode::Detail, t, cx);
                    let (r1, f1) = (rid.clone(), f.clone());
                    div()
                        .id(SharedString::from(format!("dv-{}", f.id)))
                        .min_h(px(32.))
                        .px(px(8.))
                        .py(px(4.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(14.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(if empty {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_color(t.placeholder)
                                .child(icon("plus", 13., t.placeholder))
                                .child(match f.kind {
                                    Attachment => "上传附件",
                                    Date => "选择日期",
                                    Person => "添加人员",
                                    Link | DuplexLink => "添加关联记录",
                                    _ => "请选择",
                                })
                                .into_any_element()
                        } else {
                            content
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, w, cx| {
                                if f1.kind == Attachment && is_empty_value(this.b().record(&this.tid, &r1).and_then(|r| r.cells.get(&f1.id))) {
                                    this.pick_files(r1.clone(), f1.id.clone(), cx);
                                    return;
                                }
                                this.open_cell_editor(&r1, &f1, ev.position, false, w, cx);
                            }),
                        )
                        .into_any_element()
                }
            };
            let fid = f.id.clone();
            let multi = layout == 1;
            items.push((
                f.id.clone(),
                div()
                    .flex()
                    .when(!multi, |d| d.flex_row().items_start().gap(px(12.)))
                    .when(multi, |d| d.flex_col().gap(px(2.)))
                    .py(px(2.))
                    .child(
                        div()
                            .id(SharedString::from(format!("dlab-{fid}")))
                            .when(!multi, |d| d.w(px(150.)).h(px(32.)))
                            .when(multi, |d| d.h(px(24.)))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .text_color(t.text_secondary)
                            .when(hidden, |d| d.opacity(0.6))
                            .child(icon(f.kind.icon(), 14., t.text_muted))
                            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(f.name.clone()))
                            .when(hidden, |d| d.child(icon("eye-off", 12., t.text_muted)))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, w, cx| this.edit_field(Some(fid.clone()), None, ev.position, w, cx)),
                            ),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(value))
                    .into_any_element(),
            ));
        }
        let body: AnyElement = match self.detail_tab {
            1 => self.detail_links(&rid, t, cx),
            2 => self.detail_history(&rid, t),
            3 => self.detail_comments(&rid, t, window, cx),
            _ => match layout {
                1 => div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_x(px(24.))
                    .gap_y(px(10.))
                    .children(items.into_iter().map(|(_, el)| div().w(px(310.)).child(el)))
                    .into_any_element(),
                2 if !fgroups.is_empty() => {
                    let mut out = div().flex().flex_col().gap(px(14.));
                    let mut rest: Vec<(String, AnyElement)> = vec![];
                    let mut by: Vec<Vec<AnyElement>> = fgroups.iter().map(|_| vec![]).collect();
                    for (fid, el) in items {
                        match fgroups.iter().position(|g| g.fields.contains(&fid)) {
                            Some(i) => by[i].push(el),
                            None => rest.push((fid, el)),
                        }
                    }
                    out = out.child(div().flex().flex_col().gap(px(4.)).children(rest.into_iter().map(|x| x.1)));
                    for (g, els) in fgroups.iter().zip(by) {
                        if els.is_empty() {
                            continue;
                        }
                        out = out.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .p(px(12.))
                                .rounded(px(8.))
                                .bg(t.sidebar_bg)
                                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).pb(px(4.)).child(g.name.clone()))
                                .children(els),
                        );
                    }
                    out.into_any_element()
                }
                _ => div().flex().flex_col().gap(px(4.)).children(items.into_iter().map(|x| x.1)).into_any_element(),
            },
        };
        let nlinks = fields.iter().filter(|f| f.kind.is_link()).map(|f| str_list(rec.cells.get(&f.id)).len()).sum::<usize>();
        let ncomments = self.comment_counts.get(&rid).copied().unwrap_or(0);
        let tab = self.detail_tab;
        let tabs = div()
            .flex()
            .flex_row()
            .gap(px(4.))
            .border_b_1()
            .border_color(t.border)
            .children([("字段".to_string(), 0u8), (format!("关联 {nlinks}"), 1), ("修改历史".into(), 2), (format!("评论 {ncomments}"), 3)].into_iter().map(
                |(l, k)| {
                    let on = tab == k;
                    div()
                        .id(("dtab", k as usize))
                        .px(px(10.))
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .text_size(px(13.))
                        .cursor_pointer()
                        .when(on, |d| d.text_color(t.accent).border_b_2().border_color(t.accent))
                        .when(!on, |d| d.text_color(t.text_secondary))
                        .child(l)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.detail_tab = k;
                            if k == 3 {
                                this.refresh_comment_counts();
                            }
                            cx.notify();
                        }))
                },
            ))
            .child(div().flex_1())
            .when(tab == 0, |d| {
                d.child(select_box("d-layout", ["单列", "多列", "按编组"][layout.min(2) as usize], Some("layout"), t).h(px(26.)).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        let items = ["单列", "多列", "按编组"].iter().enumerate().map(|(i, l)| super::editor::PickItem::new(i.to_string(), *l)).collect();
                        this.open_pick_cb(
                            items,
                            vec![],
                            false,
                            ev.position,
                            |this, k, cx| {
                                let tid = this.tid.clone();
                                let meta = this.b().table(&tid).map(|t| t.meta.clone());
                                if let Some(mut m) = meta {
                                    m.detail_layout = k.parse().unwrap_or(0);
                                    this.bm().put_table(m);
                                    this.commit(cx);
                                }
                            },
                            cx,
                        );
                    }),
                ))
            });
        let (r_del, r_dup) = (rid.clone(), rid.clone());
        let can_prev = pos.is_some_and(|p| p > 0);
        let can_next = pos.is_some_and(|p| p + 1 < rows.len());
        let modal = div()
            .w(px(if layout == 1 && tab == 0 { 820. } else { 720. }))
            .max_h(relative(0.86))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .occlude()
            .child(self.render_detail_d_dup(t, cx, can_next, can_prev, pos, r_del.clone(), r_dup.clone(), &rows))
            .child(
                div()
                    .id("d-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.detail_scroll)
                    .px(px(28.))
                    .py(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(tabs)
                    .child(body)
                    .child(
                        div()
                            .id("d-addf")
                            .mt(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .text_color(t.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.accent))
                            .child(icon("plus", 14., t.text_muted))
                            .child("新增字段")
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, w, cx| this.edit_field(None, None, ev.position, w, cx))),
                    )
                    .child(div().pt(px(12.)).border_t_1().border_color(t.border).text_size(px(12.)).text_color(t.text_muted).child(format!(
                        "创建于 {} · 最后更新于 {}",
                        formula::fmt_date(rec.created, 1),
                        formula::fmt_date(rec.updated, 1)
                    ))),
            );
        Some(
            div()
                .id("detail-mask")
                .absolute()
                .inset_0()
                .bg(t.overlay)
                .flex()
                .items_center()
                .justify_center()
                .child(modal)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, w, cx| {
                        if this.popup.is_none() {
                            this.detail = None;
                            w.focus(&this.focus);
                            cx.notify();
                        }
                    }),
                )
                .into_any_element(),
        )
    }
    /// 从 `render_detail` 拆出。
    fn render_detail_d_dup(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        can_next: bool,
        can_prev: bool,
        pos: Option<usize>,
        r_del: String,
        r_dup: String,
        rows: &std::rc::Rc<Vec<String>>,
    ) -> AnyElement {
        let nav_btn = |id: &'static str, ic: &'static str, ok: bool, d: i32, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .when(!ok, |s| s.opacity(0.35))
                .child(icon(ic, 15., t.text_secondary))
                .on_click(cx.listener(move |this, _, _, cx| this.detail_nav(d, cx)))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(16.))
            .h(px(52.))
            .flex_none()
            .border_b_1()
            .border_color(t.border)
            .child(nav_btn("d-up", "chevron-up", can_prev, -1, cx).tooltip(tip("上一条记录")))
            .child(nav_btn("d-down", "chevron-down", can_next, 1, cx).tooltip(tip("下一条记录")))
            .child(div().text_size(px(12.)).text_color(t.text_muted).child(pos.map(|p| format!("{} / {}", p + 1, rows.len())).unwrap_or_default()))
            .child(div().flex_1())
            .child(
                div()
                    .id("d-dup")
                    .px(px(8.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(6.))
                    .text_size(px(12.5))
                    .text_color(t.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon("copy", 13., t.text_secondary))
                    .child("创建副本")
                    .on_click(cx.listener(move |this, _, _, cx| this.duplicate_record(&r_dup, cx))),
            )
            .child(
                div()
                    .id("d-del")
                    .px(px(8.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(6.))
                    .text_size(px(12.5))
                    .text_color(t.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon("trash", 13., t.text_secondary))
                    .child("删除")
                    .on_click(cx.listener(move |this, _, _, cx| this.delete_records(std::slice::from_ref(&r_del), cx))),
            )
            .child(
                div()
                    .id("d-close")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon("x", 16., t.text_secondary))
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.detail = None;
                        this.close_popups();
                        w.focus(&this.focus);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// 关联标签页：每个关联字段一张小表（主字段 + 前几个字段），可移除、可添加。
    fn detail_links(&mut self, rid: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let links: Vec<Field> = self.fields().into_iter().filter(|f| f.kind.is_link()).collect();
        if links.is_empty() {
            return div().py(px(20.)).text_size(px(13.)).text_color(t.text_muted).child("这张数据表还没有关联字段").into_any_element();
        }
        let mut out = div().flex().flex_col().gap(px(16.));
        for f in links {
            let ids: Vec<String> = str_list(self.b().record(&self.tid, rid).and_then(|r| r.cells.get(&f.id)))
                .into_iter()
                .filter(|r| self.b().record(&f.prop.table, r).is_some())
                .collect();
            let cols: Vec<Field> = self
                .b()
                .table(&f.prop.table)
                .map(|t| t.field_list().into_iter().filter(|x| !x.kind.is_link() && x.kind != FieldType::Attachment).take(4).cloned().collect())
                .unwrap_or_default();
            let tname = self.b().table(&f.prop.table).map(|t| t.meta.name.clone()).unwrap_or_default();
            let (r1, f1) = (rid.to_string(), f.clone());
            let mut tbl = div().flex().flex_col().rounded(px(8.)).border_1().border_color(t.border).overflow_hidden();
            tbl = tbl.child(div().flex().flex_row().h(px(30.)).items_center().bg(t.table_header).children(cols.iter().map(|c| {
                div().w(px(150.)).px(px(8.)).text_size(px(12.)).text_color(t.text_muted).overflow_hidden().whitespace_nowrap().child(c.name.clone())
            })));
            for (i, r) in ids.iter().enumerate() {
                let b = self.b();
                let cells: Vec<String> = cols.iter().map(|c| b.text(&f.prop.table, r, &c.id)).collect();
                drop(b);
                let (r2, fid, other) = (rid.to_string(), f.id.clone(), r.clone());
                tbl = tbl.child(
                    div()
                        .id(SharedString::from(format!("dl-{}-{i}", f.id)))
                        .group("dlr")
                        .flex()
                        .flex_row()
                        .h(px(32.))
                        .items_center()
                        .border_t_1()
                        .border_color(t.border)
                        .hover(|s| s.bg(t.hover))
                        .children(
                            cells
                                .into_iter()
                                .map(|c| div().w(px(150.)).px(px(8.)).text_size(px(13.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(c)),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(SharedString::from(format!("dlx-{}-{i}", f.id)))
                                .mr(px(8.))
                                .invisible()
                                .group_hover("dlr", |s| s.visible())
                                .cursor_pointer()
                                .child(icon("x", 13., t.text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut cur = str_list(this.b().record(&this.tid, &r2).and_then(|r| r.cells.get(&fid)));
                                    cur.retain(|x| x != &other);
                                    this.set_cell(&r2, &fid, Value::from(cur), false, cx);
                                })),
                        ),
                );
            }
            out = out.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(icon(f.kind.icon(), 14., t.text_muted))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(f.name.clone()))
                            .child(div().text_size(px(12.)).text_color(t.text_muted).child(format!("关联「{tname}」· {} 条", ids.len())))
                            .child(div().flex_1())
                            .child(
                                ghost_btn(SharedString::from(format!("dla-{}", f.id)), "添加关联", t)
                                    .h(px(26.))
                                    .on_click(cx.listener(move |this, ev: &ClickEvent, w, cx| this.open_cell_editor(&r1, &f1, ev.position(), false, w, cx))),
                            ),
                    )
                    .child(tbl),
            );
        }
        out.into_any_element()
    }

    /// 修改历史标签页（本机记录，修改人固定为「我」）。
    fn detail_history(&mut self, rid: &str, t: &Theme) -> AnyElement {
        let hist = super::db::history(&self.obj_id, rid);
        if hist.is_empty() {
            return div()
                .py(px(20.))
                .text_size(px(13.))
                .text_color(t.text_muted)
                .child("还没有修改记录（只记录在本机修改的内容，保留 180 天）")
                .into_any_element();
        }
        let me = crate::settings::author();
        let me = if me.is_empty() { "我".to_string() } else { me };
        let b = self.b();
        let show = |f: Option<&Field>, v: &Option<Value>| -> String {
            let Some(v) = v else { return "（空）".into() };
            match f {
                Some(f) if f.kind.has_options() => {
                    str_list(Some(v)).iter().map(|id| f.opt(id).map(|o| o.name.clone()).unwrap_or_else(|| "已删除的选项".into())).collect::<Vec<_>>().join("、")
                }
                Some(f) if f.kind.is_link() => str_list(Some(v)).iter().map(|r| b.title(&f.prop.table, r)).collect::<Vec<_>>().join("、"),
                Some(f) if f.kind == FieldType::Date => {
                    v.as_i64().map(|d| formula::fmt_date(d, if d == formula::day_start(d) { 0 } else { 1 })).unwrap_or_default()
                }
                Some(f) if f.kind == FieldType::Checkbox => {
                    if v.as_bool().unwrap_or(false) {
                        "已勾选".into()
                    } else {
                        "未勾选".into()
                    }
                }
                Some(f) if f.kind == FieldType::Attachment => attachments(Some(v)).iter().map(|a| a.name.clone()).collect::<Vec<_>>().join("、"),
                Some(f) if f.kind == FieldType::Progress => v.as_f64().map(|n| format!("{}%", formula::fmt_num(n * 100.))).unwrap_or_default(),
                _ => match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => formula::fmt_num(n.as_f64().unwrap_or(0.)),
                    Value::Array(a) => a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join("、"),
                    v => v.to_string(),
                },
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(hist.into_iter().map(|(fid, a, bv, at)| {
                let f = b.field(&self.tid, &fid);
                let what = if fid.is_empty() {
                    if bv.is_some() {
                        "创建了这条记录".to_string()
                    } else {
                        "删除了这条记录".to_string()
                    }
                } else {
                    format!("修改了「{}」：{} → {}", f.map(|f| f.name.clone()).unwrap_or("已删除的字段".into()), show(f, &a), show(f, &bv))
                };
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(t.border)
                    .text_size(px(13.))
                    .child(div().w(px(130.)).flex_none().text_color(t.text_muted).child(formula::fmt_date(at, 1)))
                    .child(div().flex_none().text_color(t.text_secondary).child(me.clone()))
                    .child(div().flex_1().child(what))
            }))
            .into_any_element()
    }

    /// 评论标签页：复用文档评论表（文档 id 记为 `base:多维表格 id`，锚点为记录 id）。
    fn detail_comments(&mut self, rid: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let doc = format!("base:{}", self.obj_id);
        let list: Vec<crate::store::Comment> =
            crate::objects::store().map(|s| s.comments(&doc).into_iter().filter(|c| c.anchor == rid).collect()).unwrap_or_default();
        let inp = self.input("cb:keep:comment", "写评论，回车发送", "", false, window, cx);
        let (r1, d1) = (rid.to_string(), doc.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(list.into_iter().map(|c| {
                let (id1, id2, resolved) = (c.id.clone(), c.id.clone(), c.resolved);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(t.sidebar_bg)
                    .when(resolved, |d| d.opacity(0.55))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .items_center()
                            .child(avatar(if c.author.is_empty() { "我" } else { &c.author }, 20.))
                            .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(if c.author.is_empty() {
                                "我".to_string()
                            } else {
                                c.author.clone()
                            }))
                            .child(div().text_size(px(12.)).text_color(t.text_muted).child(formula::fmt_date(c.created, 1)))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id(SharedString::from(format!("cr-{id1}")))
                                    .text_size(px(12.))
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .child(if resolved { "重新打开" } else { "解决" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(s) = crate::objects::store() {
                                            s.set_comment_resolved(&id1, !resolved);
                                        }
                                        this.refresh_comment_counts();
                                        cx.notify();
                                    })),
                            )
                            .child(div().id(SharedString::from(format!("cd-{id2}"))).cursor_pointer().child(icon("trash", 12., t.text_muted)).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(s) = crate::objects::store() {
                                        s.delete_comment(&id2);
                                    }
                                    this.refresh_comment_counts();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(div().text_size(px(13.5)).child(c.body.clone()))
            }))
            .child(div().flex().flex_row().gap(px(8.)).child(input_box(t).flex_1().child(inp)).child(primary_btn("c-send", "发送", t).on_click(cx.listener(
                move |this, _, _, cx| {
                    let text = this.input_text("cb:keep:comment", cx);
                    if text.trim().is_empty() {
                        return;
                    }
                    if let Some(s) = crate::objects::store() {
                        s.add_comment(&d1, 0, &r1, text.trim(), &crate::settings::author());
                    }
                    if let Some((i, _)) = this.inputs.get("cb:keep:comment") {
                        i.update(cx, |i, cx| i.set_text("", cx));
                    }
                    this.refresh_comment_counts();
                    cx.notify();
                },
            ))))
            .into_any_element()
    }
}
