//! 弹出面板：字段配置、筛选、排序、分组、行高、看板 / 日期设置、视图与数据表菜单、列菜单、
//! 行菜单、统计方式、字段编辑器（含公式编辑助手），以及通用的二级下拉选择（Pick）。

use super::calc::{self, CURRENCIES, NUM_FMTS, STATS};
use super::editor::{BaseEditor, Draft, Pick, PickItem, PickTarget, Popup};
use super::formula::{self, DATE_FMTS, FUNCS};
use super::model::*;
use super::widgets::*;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::Value;

fn kind_id(k: FieldType) -> String {
    format!("{k:?}")
}

fn kind_from(id: &str) -> Option<FieldType> {
    TYPE_MENU.iter().flat_map(|g| g.1.iter().copied()).find(|k| kind_id(*k) == id)
}

impl BaseEditor {
    fn open_pick(&mut self, target: PickTarget, items: Vec<PickItem>, selected: Vec<String>, multi: bool, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.pick = Some(Pick { target, items, selected, multi, at });
        cx.notify();
    }

    fn field_items(&self, filter: impl Fn(&Field) -> bool) -> Vec<PickItem> {
        self.fields().into_iter().filter(|f| filter(f)).map(|f| PickItem::new(f.id.clone(), f.name.clone()).icon(f.kind.icon())).collect()
    }

    fn apply_pick(&mut self, target: PickTarget, id: String, cx: &mut Context<Self>) {
        let multi = self.pick.as_ref().is_some_and(|p| p.multi);
        if multi {
            if let Some(p) = self.pick.as_mut() {
                if p.selected.contains(&id) {
                    p.selected.retain(|x| x != &id);
                } else {
                    p.selected.push(id.clone());
                }
            }
        } else {
            self.pick = None;
        }
        let selected = self.pick.as_ref().map(|p| p.selected.clone()).unwrap_or_default();
        if target == PickTarget::Cb {
            if let Some(cb) = self.pick_cb.clone() {
                if !multi {
                    self.pick_cb = None;
                }
                cb(self, if multi { selected.join("\n") } else { id }, cx);
            }
            cx.notify();
            return;
        }
        let draft_set = |this: &mut Self, f: &dyn Fn(&mut Draft)| {
            if let Some(d) = this.draft.as_mut() {
                f(d);
            }
        };
        match target {
            PickTarget::Conj => self.update_view(cx, |v| v.filter.any = id == "any"),
            PickTarget::SortField(i) => self.update_view(cx, |v| {
                if let Some(s) = v.sorts.get_mut(i) {
                    s.field = id;
                }
            }),
            PickTarget::GroupField(i) => self.update_view(cx, |v| {
                if let Some(s) = v.groups.get_mut(i) {
                    s.field = id;
                }
            }),
            PickTarget::FieldType => {
                if let Some(k) = kind_from(&id) {
                    let link_default = self.tid.clone();
                    draft_set(self, &|d| {
                        let keep = d.prop.options.clone();
                        d.kind = k;
                        d.prop = default_prop(k);
                        if k.has_options() && !keep.is_empty() {
                            d.prop.options = keep.clone();
                        }
                        if k.is_link() {
                            d.prop.table = link_default.clone();
                        }
                        d.err = None;
                    });
                    if k == FieldType::Formula {
                        self.inputs.remove("fd:expr");
                    }
                }
            }
            PickTarget::LinkTable => draft_set(self, &|d| d.prop.table = id.clone()),
            PickTarget::NumFmt => draft_set(self, &|d| d.prop.fmt = id.parse().unwrap_or(0)),
            PickTarget::DateFmt => draft_set(self, &|d| d.prop.fmt = id.parse().unwrap_or(0)),
            PickTarget::Currency => draft_set(self, &|d| d.prop.symbol = id.clone()),
            PickTarget::RatingSym => draft_set(self, &|d| d.prop.symbol = id.clone()),
            PickTarget::RatingMax => draft_set(self, &|d| d.prop.max = id.parse().unwrap_or(5.)),
            PickTarget::KanbanField => self.update_view(cx, |v| v.group_field = Some(id)),
            PickTarget::Cover => self.update_view(cx, |v| v.cover = (!id.is_empty()).then_some(id)),
            PickTarget::DateField => self.update_view(cx, |v| v.date_field = Some(id)),
            PickTarget::EndField => self.update_view(cx, |v| v.end_field = (!id.is_empty()).then_some(id)),
            PickTarget::Cb => {}
        }
        cx.notify();
    }

    pub(super) fn render_pick(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pick.as_ref()?;
        let at = p.at;
        let target = p.target.clone();
        let selected = p.selected.clone();
        let items = p.items.clone();
        let mut list = div().id("pick-list").max_h(px(360.)).overflow_y_scroll().flex().flex_col();
        let mut last_group = None;
        for it in items {
            if it.group != last_group {
                if let Some(g) = it.group {
                    list = list.child(section_label(g, t));
                }
                last_group = it.group;
            }
            let on = selected.contains(&it.id);
            let (tg, id) = (target.clone(), it.id.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("pk-{}", it.id)))
                    .mx(px(4.))
                    .px(px(8.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .when(on, |d| d.text_color(t.accent))
                    .when_some(it.icon, |d, i| d.child(icon(i, 15., if on { t.accent } else { t.text_secondary })))
                    .child(match it.color {
                        Some(c) => div().flex_1().child(tag(it.label.clone(), c, t)).into_any_element(),
                        None => div().flex_1().whitespace_nowrap().child(it.label.clone()).into_any_element(),
                    })
                    .when(on, |d| d.child(icon("check", 14., t.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(tg.clone(), id.clone(), cx))),
            );
        }
        Some(
            deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(panel(t).min_w(px(200.)).max_w(px(320.)).child(list).on_mouse_down_out(
                cx.listener(|this, _, _, cx| {
                    this.pick = None;
                    this.pick_cb = None;
                    cx.notify();
                }),
            )))
            .with_priority(8)
            .into_any_element(),
        )
    }

    pub(super) fn render_popup(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (p, at) = self.popup.clone()?;
        let inner: AnyElement = match &p {
            Popup::Options { .. } | Popup::Date { .. } | Popup::Links { .. } | Popup::People { .. } | Popup::Files { .. } => {
                self.render_cell_popup(&p, t, window, cx)?
            }
            Popup::Fields => self.fields_panel(t, cx),
            Popup::Filter => self.filter_panel(t, window, cx),
            Popup::Sort => self.sort_panel(t, cx),
            Popup::Group => self.group_panel(t, cx),
            Popup::RowHeight => self.rowh_panel(t, cx),
            Popup::KanbanSet => self.kanban_set_panel(t, cx),
            Popup::DateSet => self.date_set_panel(t, cx),
            Popup::More => self.more_menu(t, cx),
            Popup::NewView => self.new_view_menu(t, cx),
            Popup::ViewMenu(v) => self.view_menu(v.clone(), t, cx),
            Popup::TableMenu(tid) => self.table_menu(tid.clone(), t, cx),
            Popup::ColMenu(f) => self.col_menu(f.clone(), t, cx),
            Popup::RowMenu(r) => self.row_menu(r.clone(), t, cx),
            Popup::Stat(f) => self.stat_menu(f.clone(), t, cx),
            Popup::FieldEdit => self.field_editor(t, window, cx),
            Popup::KanbanCol(k) => self.render_kanban_col_menu(k.clone(), t, cx),
            Popup::Colors => self.colors_panel(t, window, cx),
            Popup::BatchEdit => self.batch_panel(t, window, cx),
            Popup::DelField(f) => self.del_field_panel(f.clone(), t, cx),
            Popup::FieldGroups => self.field_groups_panel(t, window, cx),
            Popup::FormSet => self.form_set_panel(t, window, cx),
            Popup::Import(p) => self.import_panel(p.clone(), t, cx),
            Popup::Export => self.export_menu(t, cx),
            Popup::Automations => self.automations_panel(t, window, cx),
            Popup::Widget(w) => self.widget_panel(w.clone(), t, window, cx),
        };
        Some(
            deferred(anchored().position(at + point(px(0.), px(4.))).snap_to_window_with_margin(px(8.)).child(div().child(inner).on_mouse_down_out(
                cx.listener(|this, _, _, cx| {
                    // 二级下拉打开时，点在它上面不应关闭一级面板
                    if this.pick.is_none() {
                        this.close_popups();
                        cx.notify();
                    }
                }),
            )))
            .with_priority(5)
            .into_any_element(),
        )
    }

    // ───────────── 字段配置 / 卡片配置 ─────────────

    fn fields_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = self.view();
        let all: Vec<Field> = {
            let b = self.b();
            let Some(tb) = b.table(&self.tid) else { return div().into_any_element() };
            tb.view_fields(&v).iter().filter_map(|f| tb.fields.get(f).cloned()).collect()
        };
        let card = matches!(v.kind, ViewKind::Kanban | ViewKind::Gallery);
        let cover_name = v.cover.as_ref().and_then(|c| self.field(c)).map(|f| f.name).unwrap_or("无封面".into());
        let n = all.len();
        let mut list = div().id("fp-list").max_h(px(380.)).overflow_y_scroll().flex().flex_col();
        for (i, f) in all.iter().enumerate() {
            let shown = i == 0 || !v.hidden.contains(&f.id);
            let (fid, fid2, fid3) = (f.id.clone(), f.id.clone(), f.id.clone());
            let order: Vec<String> = all.iter().map(|f| f.id.clone()).collect();
            let order2 = order.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("fp-{}", f.id)))
                    .group("fp")
                    .mx(px(4.))
                    .px(px(8.))
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .hover(|s| s.bg(t.hover))
                    .child(icon(f.kind.icon(), 15., t.text_secondary))
                    .child(
                        div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().when(!shown, |d| d.text_color(t.text_muted)).child(f.name.clone()),
                    )
                    .when(i == 0, |d| d.child(div().text_size(px(11.)).text_color(t.text_muted).child("主字段")))
                    .when(i > 1, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("fpu-{fid}")))
                                .invisible()
                                .group_hover("fp", |s| s.visible())
                                .cursor_pointer()
                                .child(icon("arrow-up", 13., t.text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut o = order.clone();
                                    o.swap(i, i - 1);
                                    this.update_view(cx, |v| v.order = o);
                                })),
                        )
                    })
                    .when(i > 0 && i + 1 < n, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("fpd-{fid2}")))
                                .invisible()
                                .group_hover("fp", |s| s.visible())
                                .cursor_pointer()
                                .child(icon("arrow-down", 13., t.text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut o = order2.clone();
                                    o.swap(i, i + 1);
                                    this.update_view(cx, |v| v.order = o);
                                })),
                        )
                    })
                    .when(i > 0, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("fpe-{fid3}")))
                                .cursor_pointer()
                                .child(icon(if shown { "eye" } else { "eye-off" }, 15., if shown { t.accent } else { t.text_muted }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let f = fid3.clone();
                                    this.update_view(cx, |v| {
                                        if v.hidden.contains(&f) {
                                            v.hidden.retain(|x| x != &f);
                                        } else {
                                            v.hidden.push(f);
                                        }
                                    })
                                })),
                        )
                    }),
            );
        }
        let all_ids: Vec<String> = all.iter().skip(1).map(|f| f.id.clone()).collect();
        panel(t)
            .w(px(300.))
            .when(card, |d| {
                d.child(section_label("封面", t))
                    .child(
                        div()
                            .px(px(12.))
                            .pb(px(6.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(select_box("fp-cover", cover_name, Some("image"), t).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    let mut items = vec![PickItem::new("", "无封面")];
                                    items.extend(this.field_items(|f| f.kind == FieldType::Attachment));
                                    let cur = this.view().cover.unwrap_or_default();
                                    this.open_pick(PickTarget::Cover, items, vec![cur], false, ev.position, cx);
                                }),
                            ))
                            .child(
                                div()
                                    .id("fp-fill")
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .cursor_pointer()
                                    .child(div().flex_1().text_size(px(13.)).child("封面裁切填满"))
                                    .child(switch(v.cover_fill, t))
                                    .on_click(cx.listener(|this, _, _, cx| this.update_view(cx, |v| v.cover_fill = !v.cover_fill))),
                            ),
                    )
                    .child(menu_sep(t))
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(12.))
                    .pt(px(6.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(div().flex_1().child(if card {
                        "卡片上显示的字段"
                    } else if v.kind == ViewKind::Form {
                        "表单收集的字段"
                    } else {
                        "字段（拖动顺序请用 ↑↓）"
                    }))
                    .child(
                        div()
                            .id("fp-all")
                            .px(px(4.))
                            .text_color(t.accent)
                            .cursor_pointer()
                            .child("全部显示")
                            .on_click(cx.listener(|this, _, _, cx| this.update_view(cx, |v| v.hidden.clear()))),
                    )
                    .child(div().id("fp-none").px(px(4.)).text_color(t.accent).cursor_pointer().child("全部隐藏").on_click(cx.listener(
                        move |this, _, _, cx| {
                            let ids = all_ids.clone();
                            this.update_view(cx, |v| v.hidden = ids)
                        },
                    ))),
            )
            .child(list)
            .child(menu_sep(t))
            .child(
                menu_item("fp-add", Some("plus"), "新增字段", t)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, w, cx| this.edit_field(None, None, ev.position, w, cx))),
            )
            .into_any_element()
    }

    // ───────────── 筛选 ─────────────

    fn filter_panel(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let v = self.view();
        let tid = self.tid.clone();
        let mut rows = div().flex().flex_col().gap(px(8.)).px(px(12.));
        for (i, c) in v.filter.conds.iter().enumerate() {
            if self.field(&c.field).is_none() {
                continue;
            }
            let set: super::config::CondSet = std::rc::Rc::new(move |this, c, cx| {
                this.update_view(cx, move |v| {
                    if let Some(x) = v.filter.conds.get_mut(i) {
                        *x = c;
                    }
                })
            });
            let editor = self.cond_editor(&format!("flt{i}"), &tid, c, set, t, window, cx);
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(36.)).text_size(px(12.)).text_color(t.text_muted).child(if i == 0 {
                        "当"
                    } else if v.filter.any {
                        "或"
                    } else {
                        "且"
                    }))
                    .child(editor)
                    .child(
                        div()
                            .id(("fdel", i))
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("trash", 14., t.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.inputs.retain(|k, _| !k.starts_with("cb:flt"));
                                this.update_view(cx, |v| {
                                    if i < v.filter.conds.len() {
                                        v.filter.conds.remove(i);
                                    }
                                })
                            })),
                    ),
            );
        }
        let n = v.filter.conds.len();
        panel(t)
            .min_w(px(460.))
            .pb(px(8.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .pt(px(8.))
                    .pb(px(8.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("设置筛选条件"))
                    .child(div().flex_1())
                    .when(n > 1, |d| {
                        d.child(div().text_size(px(12.)).text_color(t.text_muted).child("符合以下")).child(
                            select_box("f-conj", if v.filter.any { "任一条件" } else { "所有条件" }, None, t).h(px(26.)).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    this.open_pick(
                                        PickTarget::Conj,
                                        vec![PickItem::new("all", "所有条件"), PickItem::new("any", "任一条件")],
                                        vec![],
                                        false,
                                        ev.position,
                                        cx,
                                    );
                                }),
                            ),
                        )
                    }),
            )
            .when(n == 0, |d| d.child(div().px(px(12.)).pb(px(8.)).text_size(px(12.5)).text_color(t.text_muted).child("还没有筛选条件，筛选只影响当前视图")))
            .child(rows)
            .child(div().px(px(8.)).pt(px(6.)).child(menu_item("f-add", Some("plus"), "添加条件", t).text_color(t.accent).w(px(120.)).on_click(cx.listener(
                |this, _, _, cx| {
                    let f = this.visible().into_iter().next();
                    if let Some(f) = f {
                        this.update_view(cx, |v| v.filter.conds.push(super::config::blank_cond(&f)));
                    }
                },
            ))))
            .into_any_element()
    }

    // ───────────── 排序 / 分组 ─────────────

    fn rules_panel(&mut self, groups: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = self.view();
        let rules = if groups { v.groups.clone() } else { v.sorts.clone() };
        let mut rows = div().flex().flex_col().gap(px(8.)).px(px(12.));
        for (i, r) in rules.iter().enumerate() {
            let Some(f) = self.field(&r.field) else { continue };
            let (asc, desc) = if f.kind.numeric() || f.kind.is_date() {
                ("1 → 9", "9 → 1")
            } else if f.kind.has_options() {
                ("选项正序", "选项倒序")
            } else {
                ("A → Z", "Z → A")
            };
            let seg = |label: &'static str, on: bool, d: bool, cx: &mut Context<Self>| {
                div()
                    .id(SharedString::from(format!("{}-{i}-{d}", if groups { "gd" } else { "sd" })))
                    .px(px(10.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .text_size(px(12.5))
                    .cursor_pointer()
                    .when(on, |s| s.bg(t.accent_soft).text_color(t.accent))
                    .when(!on, |s| s.hover(|s| s.bg(t.hover)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_view(cx, |v| {
                            let list = if groups { &mut v.groups } else { &mut v.sorts };
                            if let Some(r) = list.get_mut(i) {
                                r.desc = d;
                            }
                        })
                    }))
            };
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(select_box(SharedString::from(format!("rf-{i}")), f.name.clone(), Some(f.kind.icon()), t).w(px(160.)).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let items = this.field_items(|f| !groups || !matches!(f.kind, FieldType::Attachment));
                            this.open_pick(if groups { PickTarget::GroupField(i) } else { PickTarget::SortField(i) }, items, vec![], false, ev.position, cx);
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(t.border_strong)
                            .overflow_hidden()
                            .child(seg(asc, !r.desc, false, cx))
                            .child(seg(desc, r.desc, true, cx)),
                    )
                    .child(
                        div()
                            .id(("rdel", i))
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("trash", 14., t.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_view(cx, |v| {
                                    let list = if groups { &mut v.groups } else { &mut v.sorts };
                                    if i < list.len() {
                                        list.remove(i);
                                    }
                                })
                            })),
                    ),
            );
        }
        let full = groups && rules.len() >= 3;
        let has_groups = groups && !rules.is_empty();
        panel(t)
            .min_w(px(360.))
            .pb(px(8.))
            .child(div().px(px(12.)).pt(px(8.)).pb(px(8.)).text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(if groups {
                "设置分组条件"
            } else {
                "设置排序条件"
            }))
            .when(rules.is_empty(), |d| {
                d.child(div().px(px(12.)).pb(px(8.)).text_size(px(12.5)).text_color(t.text_muted).child(if groups {
                    "按字段把记录分组显示，最多 3 级"
                } else {
                    "没有排序时按手动顺序显示，可拖动行调整"
                }))
            })
            .child(rows)
            .when(has_groups, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(12.))
                        .px(px(12.))
                        .pt(px(8.))
                        .text_size(px(12.5))
                        .text_color(t.accent)
                        .child(div().id("g-expand").cursor_pointer().child("全部展开").on_click(
                            cx.listener(|this, _, _, cx| this.update_view(cx, |v| v.collapsed.retain(|k| k.starts_with("t:") || k.starts_with("k:")))),
                        ))
                        .child(div().id("g-collapse").cursor_pointer().child("全部折叠").on_click(cx.listener(|this, _, _, cx| {
                            let paths: Vec<String> = this
                                .gl
                                .rows
                                .iter()
                                .filter_map(|r| match r {
                                    super::grid::GRow::Group { path, depth: 0, .. } => Some(path.clone()),
                                    _ => None,
                                })
                                .collect();
                            this.update_view(cx, move |v| {
                                for p in paths {
                                    if !v.collapsed.contains(&p) {
                                        v.collapsed.push(p);
                                    }
                                }
                            })
                        }))),
                )
            })
            .when(!full, |d| {
                d.child(div().px(px(8.)).pt(px(6.)).child(
                    menu_item("r-add", Some("plus"), if groups { "添加分组" } else { "添加排序" }, t).text_color(t.accent).w(px(120.)).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let used: Vec<String> = {
                                let v = this.view();
                                if groups {
                                    v.groups.iter().map(|g| g.field.clone()).collect()
                                } else {
                                    v.sorts.iter().map(|g| g.field.clone()).collect()
                                }
                            };
                            let f = this
                                .visible()
                                .into_iter()
                                .find(|f| !used.contains(&f.id) && (!groups || f.kind.has_options()))
                                .or_else(|| this.visible().into_iter().find(|f| !used.contains(&f.id)));
                            if let Some(f) = f {
                                this.update_view(cx, |v| {
                                    let r = SortRule { field: f.id.clone(), desc: false };
                                    if groups {
                                        v.groups.push(r)
                                    } else {
                                        v.sorts.push(r)
                                    }
                                });
                            }
                        },
                    )),
                ))
            })
            .into_any_element()
    }

    fn sort_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        self.rules_panel(false, t, cx)
    }

    fn group_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        self.rules_panel(true, t, cx)
    }

    fn rowh_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let cur = self.view().row_h;
        panel(t)
            .w(px(160.))
            .children(["低", "中等", "高", "超高"].iter().enumerate().map(|(i, l)| {
                let on = cur == i as u8;
                menu_item(("rh", i), None, *l, t).when(on, |d| d.text_color(t.accent).child(icon("check", 14., t.accent))).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.close_popups();
                        this.update_view(cx, |v| v.row_h = i as u8)
                    },
                ))
            }))
            .into_any_element()
    }

    fn kanban_set_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = self.view();
        let cur = v.group_field.as_ref().and_then(|f| self.field(f));
        panel(t)
            .w(px(280.))
            .pb(px(8.))
            .child(section_label("看板分组依据", t))
            .child(
                div().px(px(12.)).child(
                    select_box("ks-field", cur.as_ref().map(|f| f.name.clone()).unwrap_or("请选择字段".into()), cur.as_ref().map(|f| f.kind.icon()), t)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                let items = this.field_items(|f| f.kind.kanban_groupable());
                                let cur = this.view().group_field.unwrap_or_default();
                                this.open_pick(PickTarget::KanbanField, items, vec![cur], false, ev.position, cx);
                            }),
                        ),
                ),
            )
            .child(div().px(px(12.)).pt(px(6.)).text_size(px(12.)).text_color(t.text_muted).child("支持单选、多选、人员、复选框、流程、评分、单向 / 双向关联"))
            .child(div().pt(px(6.)).child(menu_item("ks-new", Some("plus"), "新建单选字段作为分组", t).on_click(cx.listener(|this, _, _, cx| {
                let mut f = Field::new("分组", FieldType::Select);
                for n in ["待处理", "进行中", "已完成"] {
                    f.ensure_opt(n);
                }
                let id = f.id.clone();
                this.insert_field(f, None);
                this.close_popups();
                this.update_view(cx, |v| v.group_field = Some(id));
            }))))
            .into_any_element()
    }

    fn date_set_panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let v = self.view();
        let gantt = v.kind == ViewKind::Gantt;
        let name = |f: &Option<String>| f.as_ref().and_then(|f| self.field(f)).map(|f| f.name);
        let (s_name, e_name) = (name(&v.date_field).unwrap_or("请选择".into()), name(&v.end_field).unwrap_or("无（单日）".into()));
        let c_name = name(&v.color_field).unwrap_or("默认（第一个单选字段）".into());
        let p_name = name(&v.progress_field).unwrap_or("不显示".into());
        let mut p = panel(t)
            .w(px(300.))
            .pb(px(8.))
            .child(section_label(if gantt { "开始日期" } else { "按哪个日期字段排进日历" }, t))
            .child(div().px(px(12.)).child(select_box("ds-start", s_name, Some("calendar"), t).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let items = this.field_items(|f| f.kind.is_date() || (f.kind == FieldType::Formula));
                    this.open_pick(PickTarget::DateField, items, vec![], false, ev.position, cx);
                }),
            )))
            .child(section_label(
                if gantt {
                    "截止日期（不设或与开始相同时显示为里程碑 ◆）"
                } else {
                    "结束日期（可选，跨天的记录画成横条）"
                },
                t,
            ))
            .child(div().px(px(12.)).child(select_box("ds-end", e_name, Some("calendar"), t).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let mut items = vec![PickItem::new("", "无（单日）")];
                    items.extend(this.field_items(|f| f.kind.is_date() || f.kind == FieldType::Formula));
                    this.open_pick(PickTarget::EndField, items, vec![], false, ev.position, cx);
                }),
            )))
            .child(section_label("颜色跟随（填色规则的整行颜色优先）", t))
            .child(div().px(px(12.)).child(select_box("ds-color", c_name, Some("palette"), t).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let mut items = vec![PickItem::new("", "默认（第一个单选字段）")];
                    items.extend(this.field_items(|f| matches!(f.kind, FieldType::Select | FieldType::Stage)));
                    this.open_pick_cb(
                        items,
                        vec![],
                        false,
                        ev.position,
                        |this, id, cx| this.update_view(cx, |v| v.color_field = (!id.is_empty()).then_some(id)),
                        cx,
                    );
                }),
            )));
        if gantt {
            let wd = v.workdays_only;
            let days: Vec<bool> = (0..7u8).map(|d| v.is_workday(d)).collect();
            p = p
                .child(section_label("进度（任务条按比例填充）", t))
                .child(div().px(px(12.)).child(select_box("ds-prog", p_name, Some("f-progress"), t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        let mut items = vec![PickItem::new("", "不显示")];
                        items.extend(this.field_items(|f| matches!(f.kind, FieldType::Progress | FieldType::Number | FieldType::Formula)));
                        this.open_pick_cb(
                            items,
                            vec![],
                            false,
                            ev.position,
                            |this, id, cx| this.update_view(cx, |v| v.progress_field = (!id.is_empty()).then_some(id)),
                            cx,
                        );
                    }),
                )))
                .child(
                    div()
                        .id("ds-wd")
                        .mx(px(12.))
                        .mt(px(10.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .cursor_pointer()
                        .child(div().flex_1().text_size(px(13.)).child("只算工作日（拖动按工作日吸附）"))
                        .child(switch(wd, t))
                        .on_click(cx.listener(move |this, _, _, cx| this.update_view(cx, |v| v.workdays_only = !wd))),
                )
                .child(div().px(px(12.)).pt(px(8.)).flex().flex_row().gap(px(4.)).children(["一", "二", "三", "四", "五", "六", "日"].iter().enumerate().map(
                    |(i, l)| {
                        let on = days[i];
                        div()
                            .id(("ds-day", i))
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .border_1()
                            .border_color(if on { t.accent } else { t.border_strong })
                            .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                            .child(*l)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_view(cx, |v| {
                                    let mut cur: Vec<u8> = (0..7u8).filter(|d| v.is_workday(*d)).collect();
                                    if cur.contains(&(i as u8)) {
                                        cur.retain(|x| *x != i as u8);
                                    } else {
                                        cur.push(i as u8);
                                    }
                                    cur.sort();
                                    v.workdays = if cur.is_empty() { vec![0] } else { cur };
                                })
                            }))
                    },
                )));
        }
        p.into_any_element()
    }

    fn more_menu(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.view().locked;
        let hier = self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some());
        let autosave = Self::autosave_views();
        panel(t)
            .w(px(240.))
            .child(
                menu_item("m-export", Some("download"), "导出（CSV / Excel）", t)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::Export, ev.position, cx))),
            )
            .child(menu_item("m-import", Some("upload-file"), "导入 Excel / CSV", t).on_click(cx.listener(|this, _, _, cx| this.import_file_dialog(cx))))
            .child(menu_sep(t))
            .child(
                menu_item("m-tree", Some("list-tree"), if hier { "关闭记录层级" } else { "开启记录层级（子记录）" }, t)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_hierarchy(cx))),
            )
            .child(
                menu_item("m-groups", Some("columns"), "字段编组", t)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::FieldGroups, ev.position, cx))),
            )
            .child(
                menu_item("m-auto", Some("zap"), "自动化", t)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::Automations, ev.position, cx))),
            )
            .child(menu_item("m-hist", Some("f-history"), "清空修改历史", t).on_click(cx.listener(|this, _, _, cx| {
                this.close_popups();
                super::db::clear_history(&this.obj_id);
                this.show_toast("已清空这个多维表格的修改历史", cx);
            })))
            .child(menu_item("m-autosave", Some(if autosave { "check" } else { "square" }), "视图配置改动立即保存", t).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(st) = crate::objects::store() {
                        st.set_kv("base_view_autosave", if autosave { "0" } else { "1" });
                    }
                    if autosave {
                        this.show_toast("已关闭：筛选、排序等改动先临时生效，点工具栏「保存」才写入视图", cx);
                    } else {
                        this.save_view_draft(true, cx);
                    }
                    cx.notify();
                },
            )))
            .child(menu_sep(t))
            .child(
                menu_item("m-lock", Some(if locked { "unlock" } else { "lock" }), if locked { "解锁视图" } else { "锁定视图（防止修改筛选、排序等配置）" }, t)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_popups();
                        let tid = this.tid.clone();
                        let mut v = this.view();
                        v.locked = !locked;
                        this.bm().put_view(&tid, v);
                        this.commit(cx);
                    })),
            )
            .into_any_element()
    }

    fn new_view_menu(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let descs = [
            "适合任何场景的数据录入与管理",
            "按状态 / 负责人分组，拖动卡片推进",
            "以图片为主的卡片墙",
            "按开始 / 截止日期管理项目排期",
            "按日期查看日程安排",
            "生成问卷，填写即新增记录",
        ];
        panel(t)
            .w(px(300.))
            .child(section_label("新建视图", t))
            .children(ViewKind::ALL.iter().zip(descs).map(|(k, d)| {
                let k = *k;
                div()
                    .id(SharedString::from(format!("nv-{}", k.short())))
                    .mx(px(4.))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(div().size(px(30.)).rounded(px(6.)).bg(Hsla::from(rgb(k.color())).opacity(0.12)).flex().items_center().justify_center().child(icon(
                        k.icon(),
                        16.,
                        rgb(k.color()).into(),
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.)).child(k.label()))
                            .child(div().text_size(px(11.5)).text_color(t.text_muted).child(d)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_popups();
                        this.add_view(k, cx);
                    }))
            }))
            .into_any_element()
    }

    fn view_menu(&mut self, vid: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(v) = self.b().view(&self.tid, &vid).cloned() else { return div().into_any_element() };
        let many = self.b().table(&self.tid).map(|t| t.meta.views.len() > 1).unwrap_or(false);
        let (v1, v2, v3, v4) = (v.clone(), v.clone(), v.clone(), vid.clone());
        panel(t)
            .w(px(200.))
            .child(
                menu_item("vm-rename", Some("pencil"), "重命名视图", t)
                    .on_click(cx.listener(move |this, _, w, cx| this.start_rename(&format!("view:{}", v1.id), &v1.name, w, cx))),
            )
            .child(menu_item("vm-copy", Some("copy"), "复制视图", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                let tid = this.tid.clone();
                let mut nv = v2.clone();
                nv.id = new_id("viw");
                nv.name = format!("{} 副本", v2.name);
                nv.locked = false;
                let Some(mut m) = this.b().table(&tid).map(|t| t.meta.clone()) else { return };
                let at = m.views.iter().position(|x| x == &v2.id).map(|i| i + 1).unwrap_or(m.views.len());
                m.views.insert(at, nv.id.clone());
                let id = nv.id.clone();
                this.bm().put_view(&tid, nv);
                this.bm().put_table(m);
                this.commit(cx);
                this.switch_view(&id, cx);
            })))
            .child(menu_item("vm-lock", Some(if v.locked { "unlock" } else { "lock" }), if v.locked { "解锁视图" } else { "锁定视图" }, t).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    let tid = this.tid.clone();
                    let mut v = v3.clone();
                    v.locked = !v.locked;
                    this.bm().put_view(&tid, v);
                    this.commit(cx);
                }),
            ))
            .when(many, |d| {
                d.child(menu_sep(t)).child(menu_item("vm-del", Some("trash"), "删除视图", t).text_color(t.fg_palette[1]).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.close_popups();
                        let tid = this.tid.clone();
                        let Some(mut m) = this.b().table(&tid).map(|t| t.meta.clone()) else { return };
                        m.views.retain(|x| x != &v4);
                        let next = m.views.first().cloned().unwrap_or_default();
                        m.active_view = next.clone();
                        this.bm().del_view(&tid, &v4);
                        this.bm().put_table(m);
                        this.commit(cx);
                        this.switch_view(&next, cx);
                        this.show_toast("已删除视图 · ⌘Z 撤销", cx);
                    },
                )))
            })
            .into_any_element()
    }

    fn table_menu(&mut self, tid: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = self.b().table(&tid).map(|t| t.meta.name.clone()).unwrap_or_default();
        let many = self.b().meta.tables.len() > 1;
        let (t1, t2, t3, n1) = (tid.clone(), tid.clone(), tid.clone(), name.clone());
        panel(t)
            .w(px(200.))
            .child(
                menu_item("tm-rename", Some("pencil"), "重命名", t)
                    .on_click(cx.listener(move |this, _, w, cx| this.start_rename(&format!("table:{t1}"), &n1, w, cx))),
            )
            .child(menu_item("tm-copy", Some("copy"), "复制数据表", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                let Some(src) = this.b().table(&t2).map(|t| {
                    (
                        t.meta.clone(),
                        t.fields.values().cloned().collect::<Vec<_>>(),
                        t.records.values().cloned().collect::<Vec<_>>(),
                        t.views.values().cloned().collect::<Vec<_>>(),
                    )
                }) else {
                    return;
                };
                let (mut m, fields, records, views) = src;
                // 字段 / 视图 id 在整个多维表格内唯一：复制时换新 id 并替换引用
                let mut map: std::collections::HashMap<String, String> = Default::default();
                for f in &fields {
                    map.insert(f.id.clone(), new_id("fld"));
                }
                let remap = |s: &str| map.get(s).cloned().unwrap_or_else(|| s.to_string());
                let ntid = new_id("tbl");
                m.id = ntid.clone();
                m.name = format!("{} 副本", m.name);
                m.fields = m.fields.iter().map(|f| remap(f)).collect();
                let mut vmap = vec![];
                for v in &views {
                    vmap.push((v.id.clone(), new_id("viw")));
                }
                m.views = m.views.iter().map(|v| vmap.iter().find(|x| &x.0 == v).map(|x| x.1.clone()).unwrap_or_default()).collect();
                m.active_view = m.views.first().cloned().unwrap_or_default();
                this.bm().tables.insert(ntid.clone(), Default::default());
                this.bm().put_table(m);
                for mut f in fields {
                    f.id = remap(&f.id);
                    f.prop.link = remap(&f.prop.link);
                    if f.kind == FieldType::DuplexLink {
                        // 复制出的双向关联退化为单向，避免和原表抢同一个回链字段
                        f.kind = FieldType::Link;
                        f.prop.back.clear();
                    }
                    this.bm().put_field(&ntid, f);
                }
                for mut r in records {
                    r.id = new_id("rec");
                    r.cells = r.cells.into_iter().map(|(k, v)| (remap(&k), v)).collect();
                    this.bm().put_record(&ntid, r);
                }
                for mut v in views {
                    v.id = vmap.iter().find(|x| x.0 == v.id).map(|x| x.1.clone()).unwrap_or_default();
                    v.order = v.order.iter().map(|f| remap(f)).collect();
                    v.hidden = v.hidden.iter().map(|f| remap(f)).collect();
                    v.filter.conds.iter_mut().for_each(|c| c.field = remap(&c.field));
                    v.sorts.iter_mut().for_each(|c| c.field = remap(&c.field));
                    v.groups.iter_mut().for_each(|c| c.field = remap(&c.field));
                    v.widths = v.widths.into_iter().map(|(k, w)| (remap(&k), w)).collect();
                    v.stats = v.stats.into_iter().map(|(k, w)| (remap(&k), w)).collect();
                    for f in [&mut v.group_field, &mut v.cover, &mut v.date_field, &mut v.end_field] {
                        if let Some(x) = f.as_mut() {
                            *x = remap(x);
                        }
                    }
                    this.bm().put_view(&ntid, v);
                }
                let mut bm = this.b().meta.clone();
                let at = bm.tables.iter().position(|x| x == &t2).map(|i| i + 1).unwrap_or(bm.tables.len());
                bm.tables.insert(at, ntid.clone());
                this.bm().set_meta(bm);
                this.commit(cx);
                this.switch_table(&ntid, cx);
            })))
            .when(many, |d| {
                d.child(menu_sep(t)).child(menu_item("tm-del", Some("trash"), "删除数据表", t).text_color(t.fg_palette[1]).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.close_popups();
                        this.bm().del_table(&t3);
                        let mut m = this.b().meta.clone();
                        m.tables.retain(|x| x != &t3);
                        if m.active == t3 {
                            m.active = m.tables.first().cloned().unwrap_or_default();
                        }
                        let next = m.active.clone();
                        this.bm().set_meta(m);
                        this.commit(cx);
                        this.switch_table(&next, cx);
                        this.show_toast("已删除数据表 · ⌘Z 撤销", cx);
                    },
                )))
            })
            .into_any_element()
    }

    // ───────────── 列 / 行 / 统计 ─────────────

    fn col_menu(&mut self, fid: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(f) = self.field(&fid) else { return div().into_any_element() };
        let is_primary = self.b().table(&self.tid).and_then(|t| t.meta.fields.first().cloned()) == Some(fid.clone());
        let tix = self.b().table(&self.tid).and_then(|t| t.meta.fields.iter().position(|x| x == &fid)).unwrap_or(0);
        let vis_ix = self.gl.cols.iter().position(|c| c == &fid).unwrap_or(0);
        let grid = self.view().kind == ViewKind::Grid;
        let (f1, f2, f3, f4, f5, f6, f7, f8) = (fid.clone(), fid.clone(), fid.clone(), fid.clone(), fid.clone(), fid.clone(), fid.clone(), fid.clone());
        let at = self.popup.as_ref().map(|p| p.1).unwrap_or_default();
        let kind = f.kind;
        panel(t)
            .w(px(220.))
            .child(
                menu_item("cm-edit", Some("pencil"), "编辑字段", t)
                    .on_click(cx.listener(move |this, _, w, cx| this.edit_field(Some(f1.clone()), None, at, w, cx))),
            )
            .child(menu_sep(t))
            .when(!is_primary, |d| {
                d.child(
                    menu_item("cm-left", Some("col-left"), "向左插入字段", t)
                        .on_click(cx.listener(move |this, _, w, cx| this.edit_field(None, Some(tix.saturating_sub(1)), at, w, cx))),
                )
            })
            .child(
                menu_item("cm-right", Some("col-right"), "向右插入字段", t)
                    .on_click(cx.listener(move |this, _, w, cx| this.edit_field(None, Some(tix), at, w, cx))),
            )
            .child(menu_sep(t))
            .child(menu_item("cm-asc", Some("arrow-up"), "升序排列", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                let f = f2.clone();
                this.update_view(cx, |v| v.sorts = vec![SortRule { field: f, desc: false }]);
            })))
            .child(menu_item("cm-desc", Some("arrow-down"), "降序排列", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                let f = f3.clone();
                this.update_view(cx, |v| v.sorts = vec![SortRule { field: f, desc: true }]);
            })))
            .when(grid && kind != FieldType::Attachment, |d| {
                d.child(menu_item("cm-group", Some("group-by"), "按此字段分组", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    let f = f4.clone();
                    this.update_view(cx, |v| {
                        if !v.groups.iter().any(|g| g.field == f) && v.groups.len() < 3 {
                            v.groups.push(SortRule { field: f, desc: false });
                        }
                    });
                })))
            })
            .child(menu_item("cm-filter", Some("filter"), "按此字段筛选", t).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let f = f5.clone();
                    this.update_view(cx, |v| {
                        v.filter.conds.push(Cond {
                            field: f,
                            op: ops_for(kind)[0],
                            value: if kind == FieldType::Checkbox { Value::Bool(true) } else { Value::Null },
                        })
                    });
                    this.open_popup(Popup::Filter, ev.position, cx);
                }),
            ))
            .child(menu_sep(t))
            .when(grid, |d| {
                d.child(menu_item("cm-freeze", Some("columns"), "冻结至此列", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    this.update_view(cx, |v| v.frozen = (vis_ix + 1).clamp(1, 6) as u8);
                })))
            })
            .when(grid, |d| {
                let fid = fid.clone();
                d.child(menu_item("cm-color", Some("palette"), "整列填色", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    let f = fid.clone();
                    let color = (this.view().colors.len() % OPT_COLORS) as u8;
                    this.update_view(cx, |v| {
                        if v.colors.len() < MAX_COLOR_RULES {
                            v.colors.push(ColorRule { id: new_id("clr"), kind: 2, field: f, cond: None, color });
                        }
                    });
                })))
            })
            .when(!is_primary, |d| {
                d.child(menu_item("cm-hide", Some("eye-off"), "隐藏字段", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    let f = f6.clone();
                    this.update_view(cx, |v| v.hidden.push(f));
                })))
            })
            .child(menu_item("cm-copy", Some("copy"), "复制字段", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                let Some(src) = this.field(&f7) else { return };
                let mut nf = src.clone();
                nf.id = new_id("fld");
                nf.name = format!("{} 副本", src.name);
                if nf.kind == FieldType::DuplexLink {
                    nf.kind = FieldType::Link;
                    nf.prop.back.clear();
                }
                let tid = this.tid.clone();
                let recs: Vec<Record> = this.b().table(&tid).map(|t| t.records.values().cloned().collect()).unwrap_or_default();
                this.insert_field(nf.clone(), Some(tix));
                for mut r in recs {
                    if let Some(v) = r.cells.get(&src.id).cloned() {
                        r.cells.insert(nf.id.clone(), v);
                        this.bm().put_record(&tid, r);
                    }
                }
                this.commit(cx);
            })))
            .when(!is_primary, |d| {
                d.child(menu_sep(t)).child(menu_item("cm-del", Some("trash"), "删除字段", t).text_color(t.fg_palette[1]).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.open_popup(Popup::DelField(f8.clone()), ev.position, cx);
                    }),
                ))
            })
            .into_any_element()
    }

    pub(super) fn delete_field(&mut self, fid: &str, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let Some(f) = self.field(fid) else { return };
        let Some(mut m) = self.b().table(&tid).map(|t| t.meta.clone()) else { return };
        if m.fields.first().map(|x| x.as_str()) == Some(fid) {
            self.show_toast("主字段不能删除", cx);
            return;
        }
        m.fields.retain(|x| x != fid);
        self.bm().put_table(m);
        self.bm().del_field(&tid, fid);
        // 双向关联：对面的回链字段退化为单向关联
        if f.kind == FieldType::DuplexLink && !f.prop.back.is_empty() {
            let tmp = self.b().field(&f.prop.table, &f.prop.back).cloned();
            if let Some(mut back) = tmp {
                back.kind = FieldType::Link;
                back.prop.back.clear();
                self.bm().put_field(&f.prop.table, back);
            }
        }
        self.commit(cx);
        self.sel = None;
        self.show_toast(&format!("已删除字段「{}」· ⌘Z 撤销", f.name), cx);
    }

    fn row_menu(&mut self, rid: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let sel_rows: Vec<String> = match &self.sel {
            Some(s) if s.rid != s.arid => {
                let recs = &self.gl.recs;
                let (a, b) = (recs.iter().position(|r| r == &s.arid), recs.iter().position(|r| r == &s.rid));
                match (a, b) {
                    (Some(a), Some(b)) if (a.min(b)..=a.max(b)).any(|i| recs[i] == rid) => recs[a.min(b)..=a.max(b)].to_vec(),
                    _ => vec![rid.clone()],
                }
            }
            _ => vec![rid.clone()],
        };
        let n = sel_rows.len();
        let (r1, r2, r3, r4) = (rid.clone(), rid.clone(), rid.clone(), rid.clone());
        let sorted = !self.view().sorts.is_empty();
        let hier = self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some());
        let parent = self.b().parent_of(&self.tid, &rid);
        let grand = parent.as_ref().and_then(|p| self.b().parent_of(&self.tid, p));
        let (r5, r6, r7) = (rid.clone(), rid.clone(), rid.clone());
        panel(t)
            .w(px(200.))
            .child(menu_item("rm-open", Some("expand-2"), "展开记录", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                this.detail = Some(r1.clone());
                cx.notify();
            })))
            .when(!sorted, |d| {
                d.child(menu_item("rm-above", Some("row-above"), "向上插入记录", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    let tid = this.tid.clone();
                    let prev = this.b().table(&tid).and_then(|t| {
                        let i = t.order.iter().position(|x| x == &r2)?;
                        if i == 0 {
                            None
                        } else {
                            t.order.get(i - 1).cloned()
                        }
                    });
                    match prev {
                        Some(p) => {
                            this.add_record(vec![], Some(&p), cx);
                        }
                        None => {
                            let first = this.b().record(&tid, &r2).map(|r| r.rank);
                            let r = this.bm().new_record(&tid, Some(rank_between(None, first)));
                            if r.is_some() {
                                this.commit(cx);
                            }
                        }
                    }
                })))
                .child(menu_item("rm-below", Some("row-below"), "向下插入记录", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    this.add_record(vec![], Some(&r3), cx);
                })))
            })
            .when(hier, |d| {
                d.child(menu_item("rm-child", Some("list-tree"), "添加子记录", t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_popups();
                    this.add_child_record(&r5, cx);
                })))
                .when(parent.is_some(), |d| {
                    let g = grand.clone();
                    d.child(menu_item("rm-up", Some("arrow-up"), "升一级", t).on_click(cx.listener(move |this, _, _, cx| {
                        this.close_popups();
                        let tid = this.tid.clone();
                        let r = this.bm().set_parent(&tid, &r6, g.as_deref());
                        if let Err(e) = r {
                            this.show_toast(&e, cx);
                        }
                        this.commit(cx);
                    })))
                    .child(menu_item("rm-top", Some("arrow-up-to-line"), "设为顶级记录", t).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.close_popups();
                            let tid = this.tid.clone();
                            this.bm().set_parent(&tid, &r7, None).ok();
                            this.commit(cx);
                        },
                    )))
                })
            })
            .child(menu_item("rm-copy", Some("copy"), "创建记录副本", t).on_click(cx.listener(move |this, _, _, cx| {
                this.close_popups();
                this.duplicate_record(&r4, cx);
            })))
            .child(menu_sep(t))
            .child(
                menu_item("rm-del", Some("trash"), if n > 1 { format!("删除 {n} 条记录") } else { "删除记录".into() }, t).text_color(t.fg_palette[1]).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.close_popups();
                        this.delete_records(&sel_rows, cx);
                    }),
                ),
            )
            .into_any_element()
    }

    fn stat_menu(&mut self, fid: String, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(f) = self.field(&fid) else { return div().into_any_element() };
        let cur = self.view().stats.get(&fid).cloned().unwrap_or_default();
        let numeric = f.kind.numeric() || f.kind == FieldType::Formula || f.kind == FieldType::Lookup;
        panel(t)
            .w(px(180.))
            .children(
                STATS
                    .iter()
                    .filter(|(k, _)| match *k {
                        "sum" | "avg" | "max" | "min" => numeric,
                        "checked" => f.kind == FieldType::Checkbox,
                        "filled" | "empty" | "filled_pct" | "unique" => f.kind != FieldType::Checkbox,
                        _ => true,
                    })
                    .map(|(k, l)| {
                        let (k, fid) = (k.to_string(), fid.clone());
                        let on = k == cur;
                        menu_item(SharedString::from(format!("stat-{k}")), None, *l, t)
                            .when(on, |d| d.text_color(t.accent).child(icon("check", 14., t.accent)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_popups();
                                let (k, f) = (k.clone(), fid.clone());
                                this.update_view(cx, |v| {
                                    if k.is_empty() {
                                        v.stats.remove(&f);
                                    } else {
                                        v.stats.insert(f, k);
                                    }
                                })
                            }))
                    }),
            )
            .into_any_element()
    }

    // ───────────── 字段编辑器 ─────────────

    /// 打开字段编辑器：`fid` 为空表示新增（`insert_at` 为插在第几个字段之后）。
    pub(super) fn edit_field(&mut self, fid: Option<String>, insert_at: Option<usize>, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popups();
        let (name, kind, prop) = match fid.as_ref().and_then(|f| self.field(f)) {
            Some(f) => (f.name.clone(), f.kind, f.prop.clone()),
            None => {
                let n = self.fields().len() + 1;
                (format!("字段 {n}"), FieldType::Text, Prop::default())
            }
        };
        self.draft = Some(Draft { fid, kind, prop, insert_at, err: None });
        let i = self.input("fd:name", "字段名称", &name, true, window, cx);
        i.update(cx, |i, cx| i.set_text(&name, cx));
        self.popup = Some((Popup::FieldEdit, at));
        cx.notify();
    }

    fn field_editor(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.draft.as_ref() else { return div().into_any_element() };
        let (kind, prop, editing, err) = (d.kind, d.prop.clone(), d.fid.clone(), d.err.clone());
        let name_input = self.input("fd:name", "字段名称", "", true, window, cx);
        let is_primary = editing.as_ref().is_some_and(|f| self.b().table(&self.tid).and_then(|t| t.meta.fields.first().cloned()).as_ref() == Some(f));
        let mut body = div().flex().flex_col().gap(px(10.)).px(px(14.)).pb(px(12.));
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_size(px(12.)).text_color(t.text_muted).child("字段名称"))
                .child(input_box(t).child(name_input)),
        );
        body = body.child(div().flex().flex_col().gap(px(6.)).child(div().text_size(px(12.)).text_color(t.text_muted).child("字段类型")).child(
            select_box("fe-type", kind.label(), Some(kind.icon()), t).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let items: Vec<PickItem> = TYPE_MENU
                        .iter()
                        .flat_map(|(g, ks)| {
                            ks.iter()
                                .filter(|k| {
                                    !is_primary
                                        || !matches!(
                                            k,
                                            FieldType::Attachment | FieldType::Checkbox | FieldType::Link | FieldType::DuplexLink | FieldType::Lookup
                                        )
                                })
                                .map(move |k| PickItem { id: kind_id(*k), label: k.label().into(), icon: Some(k.icon()), color: None, group: Some(*g) })
                        })
                        .collect();
                    this.open_pick(PickTarget::FieldType, items, vec![kind_id(kind)], false, ev.position, cx);
                }),
            ),
        ));
        let label = |s: &'static str| div().text_size(px(12.)).text_color(t.text_muted).child(s);
        match kind {
            FieldType::Number => {
                body = body.child(label("数字格式")).child(select_box("fe-nf", NUM_FMTS[prop.fmt as usize % NUM_FMTS.len()], None, t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        let items = NUM_FMTS
                            .iter()
                            .enumerate()
                            .map(|(i, l)| PickItem::new(i.to_string(), format!("{l}　{}", calc::fmt_number(1234.5678, i as u8))))
                            .collect();
                        this.open_pick(PickTarget::NumFmt, items, vec![], false, ev.position, cx);
                    }),
                ));
            }
            FieldType::Currency => {
                body = body
                    .child(label("货币"))
                    .child(select_box("fe-cur", prop.symbol.clone(), None, t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.open_pick(PickTarget::Currency, CURRENCIES.iter().map(|c| PickItem::new(*c, *c)).collect(), vec![], false, ev.position, cx);
                        }),
                    ))
                    .child(label("小数位数"))
                    .child(select_box("fe-dec", format!("{} 位", prop.fmt), None, t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.open_pick(
                                PickTarget::NumFmt,
                                (0..5).map(|i| PickItem::new(i.to_string(), format!("{i} 位"))).collect(),
                                vec![],
                                false,
                                ev.position,
                                cx,
                            );
                        }),
                    ));
            }
            FieldType::Progress => {
                body = body.child(label("小数位数")).child(select_box("fe-pdec", format!("{} 位", prop.fmt), None, t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.open_pick(
                            PickTarget::NumFmt,
                            (0..3).map(|i| PickItem::new(i.to_string(), format!("{i} 位"))).collect(),
                            vec![],
                            false,
                            ev.position,
                            cx,
                        );
                    }),
                ));
            }
            FieldType::Rating => {
                let sym = RATING_SYMS.iter().find(|s| s.0 == prop.symbol).map(|s| s.1).unwrap_or("星星");
                body = body
                    .child(label("图标"))
                    .child(select_box("fe-rs", sym, Some(rating_icon(&prop.symbol)), t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.open_pick(
                                PickTarget::RatingSym,
                                RATING_SYMS.iter().map(|(k, l)| PickItem::new(*k, *l).icon(rating_icon(k))).collect(),
                                vec![],
                                false,
                                ev.position,
                                cx,
                            );
                        }),
                    ))
                    .child(label("最高分"))
                    .child(select_box("fe-rm", format!("{} 分", prop.max as i32), None, t).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.open_pick(
                                PickTarget::RatingMax,
                                (1..=10).map(|i| PickItem::new(i.to_string(), format!("{i} 分"))).collect(),
                                vec![],
                                false,
                                ev.position,
                                cx,
                            );
                        }),
                    ));
            }
            FieldType::Date | FieldType::CreatedTime | FieldType::ModifiedTime => {
                body = body.child(label("日期格式")).child(select_box("fe-df", DATE_FMTS[prop.fmt as usize % DATE_FMTS.len()], None, t).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        let now = crate::store::now_ms();
                        let items = DATE_FMTS.iter().enumerate().map(|(i, _)| PickItem::new(i.to_string(), formula::fmt_date(now, i as u8))).collect();
                        this.open_pick(PickTarget::DateFmt, items, vec![], false, ev.position, cx);
                    }),
                ));
                if kind == FieldType::Date {
                    body = body.child(
                        div()
                            .id("fe-auto")
                            .flex()
                            .flex_row()
                            .items_center()
                            .cursor_pointer()
                            .child(div().flex_1().text_size(px(13.)).child("新记录自动填写创建日期"))
                            .child(switch(prop.auto_fill, t))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(d) = this.draft.as_mut() {
                                    d.prop.auto_fill = !d.prop.auto_fill;
                                }
                                cx.notify();
                            })),
                    );
                }
            }
            FieldType::Select | FieldType::MultiSelect | FieldType::Stage => {
                body = self.field_editor_select(t, window, cx, body, kind, &prop);
            }
            FieldType::Link | FieldType::DuplexLink => {
                body = self.field_editor_link(t, cx, body, editing.clone(), kind, &prop);
            }
            FieldType::Lookup => {
                body = body.child(self.lookup_editor(&prop, t, window, cx));
            }
            FieldType::Formula => {
                body = body.child(self.formula_helper(&prop, t, window, cx));
                body = body.child(self.out_format_editor(&prop, t, cx));
            }
            FieldType::AutoNumber => {
                let inp = self.input("fd:prefix", "例如 NO-", &prop.prefix, false, window, cx);
                let preview = calc::auto_number(&Field { id: String::new(), name: String::new(), kind, prop: prop.clone(), desc: String::new() }, 1);
                body = body
                    .child(label("前缀"))
                    .child(input_box(t).child(inp))
                    .child(label("编号位数"))
                    .child(div().flex().flex_row().gap(px(6.)).children([0u8, 3, 4, 5, 6].into_iter().map(|n| {
                        let on = prop.digits == n;
                        div()
                            .id(("fe-dig", n as usize))
                            .px(px(10.))
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(if on { t.accent } else { t.border_strong })
                            .when(on, |d| d.text_color(t.accent))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .child(if n == 0 { "不补零".to_string() } else { format!("{n} 位") })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.draft.as_mut() {
                                    d.prop.digits = n;
                                }
                                cx.notify();
                            }))
                    })))
                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(format!("示例：{preview}")));
            }
            _ => {}
        }
        for el in self.field_extra(kind, &prop, t, window, cx) {
            body = body.child(el);
        }
        if let Some(e) = err {
            body = body.child(div().text_size(px(12.5)).text_color(t.fg_palette[1]).child(e));
        }
        let w = if matches!(kind, FieldType::Formula | FieldType::Lookup | FieldType::Button) { 500. } else { 340. };
        panel(t)
            .w(px(w))
            .child(div().px(px(14.)).pt(px(10.)).pb(px(8.)).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(if editing.is_some() {
                "编辑字段"
            } else {
                "新增字段"
            }))
            .child(div().id("fe-body").max_h(px(600.)).overflow_y_scroll().child(body))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .px(px(14.))
                    .pt(px(6.))
                    .pb(px(10.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(div().flex_1())
                    .child(ghost_btn("fe-cancel", "取消", t).on_click(cx.listener(|this, _, w, cx| {
                        this.close_popups();
                        w.focus(&this.focus);
                        cx.notify();
                    })))
                    .child(primary_btn("fe-ok", "确定", t).on_click(cx.listener(|this, _, w, cx| this.save_draft(w, cx)))),
            )
            .into_any_element()
    }
    /// 从 `field_editor` 拆出。
    fn field_editor_link(&self, t: &Theme, cx: &mut Context<Self>, mut body: Div, editing: Option<String>, kind: FieldType, prop: &Prop) -> Div {
        let label = |s: &'static str| div().text_size(px(12.)).text_color(t.text_muted).child(s);
        let tname = self.b().table(&prop.table).map(|t| t.meta.name.clone()).unwrap_or("请选择数据表".into());
        let locked = editing.is_some() && kind == FieldType::DuplexLink;
        body = body
            .child(label("关联的数据表"))
            .child(select_box("fe-lt", tname, Some("v-grid"), t).when(!locked, |d| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        let items = this.b().table_list().iter().map(|t| PickItem::new(t.meta.id.clone(), t.meta.name.clone()).icon("v-grid")).collect();
                        this.open_pick(PickTarget::LinkTable, items, vec![], false, ev.position, cx);
                    }),
                )
            }))
            .child(
                div()
                    .id("fe-single")
                    .flex()
                    .flex_row()
                    .items_center()
                    .cursor_pointer()
                    .child(div().flex_1().text_size(px(13.)).child("只允许关联一条记录"))
                    .child(switch(prop.single, t))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(d) = this.draft.as_mut() {
                            d.prop.single = !d.prop.single;
                        }
                        cx.notify();
                    })),
            )
            .when(kind == FieldType::DuplexLink, |d| {
                d.child(div().text_size(px(12.)).text_color(t.text_muted).child("双向关联会在对方数据表自动新增一个回链字段，两边互相同步"))
            });
        body
    }

    /// 从 `field_editor` 拆出。
    fn field_editor_select(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>, mut body: Div, kind: FieldType, prop: &Prop) -> Div {
        let label = |s: &'static str| div().text_size(px(12.)).text_color(t.text_muted).child(s);
        let mut list = div().id("fe-opts").max_h(px(220.)).overflow_y_scroll().flex().flex_col().gap(px(6.));
        for o in &prop.options {
            let inp = self.input(&format!("fd:opt:{}", o.id), "选项名称", &o.name, false, window, cx);
            let (oid, oid2) = (o.id.clone(), o.id.clone());
            let (bg, fg) = tag_color(o.color, t.dark);
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .id(SharedString::from(format!("oc-{oid}")))
                            .size(px(20.))
                            .flex_none()
                            .rounded(px(5.))
                            .bg(bg)
                            .border_1()
                            .border_color(fg.opacity(0.4))
                            .cursor_pointer()
                            .tooltip(tip("点击换颜色"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(o) = this.draft.as_mut().and_then(|d| d.prop.options.iter_mut().find(|o| o.id == oid)) {
                                    o.color = (o.color + 1) % OPT_COLORS as u8;
                                }
                                cx.notify();
                            })),
                    )
                    .child(input_box(t).flex_1().h(px(28.)).child(inp))
                    .child(
                        div()
                            .id(SharedString::from(format!("od-{oid2}")))
                            .size(px(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("x", 13., t.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.draft.as_mut() {
                                    d.prop.options.retain(|o| o.id != oid2);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        body = body.child(label(if kind == FieldType::Stage { "流程节点（按顺序）" } else { "选项" })).child(list).child(
            div()
                .id("fe-optadd")
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(13.))
                .text_color(t.accent)
                .cursor_pointer()
                .child(icon("plus", 14., t.accent))
                .child("添加选项")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.draft.as_mut() {
                        let n = d.prop.options.len();
                        d.prop.options.push(SelOpt { id: new_id("opt"), name: format!("选项 {}", n + 1), color: (n % OPT_COLORS) as u8 });
                    }
                    cx.notify();
                })),
        );
        body
    }

    /// 公式编辑助手：表达式输入、实时预览（用第一条记录试算）、字段与函数快捷插入。
    fn formula_helper(&mut self, prop: &Prop, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let inp = self.input("fd:expr", "例如 [数量] * [单价]", &prop.expr, false, window, cx);
        let expr = prop.expr.clone();
        let preview = {
            let b = self.b();
            let t2 = b.table(&self.tid);
            match formula::parse(&expr) {
                Err(e) if !expr.trim().is_empty() => Err(e),
                Err(_) => Ok(String::new()),
                Ok(e) => {
                    let mut missing = vec![];
                    let mut refs = vec![];
                    formula::refs(&e, &mut refs);
                    for r in &refs {
                        if t2.and_then(|t| t.field_by_name(r)).is_none() {
                            missing.push(r.clone());
                        }
                    }
                    if !missing.is_empty() {
                        Err(format!("字段不存在：{}", missing.join("、")))
                    } else if let Some(rid) = t2.and_then(|t| t.order.first().cloned()) {
                        let tid = self.tid.clone();
                        let v = formula::eval(&e, &mut |name| match b.table(&tid).and_then(|t| t.field_by_name(name)) {
                            Some(f) => b.fval(&tid, &rid, &f.id),
                            None => formula::FVal::Null,
                        });
                        Ok(format!("第一条记录的结果：{}", if v.is_null() { "（空）".to_string() } else { v.text() }))
                    } else {
                        Ok(String::new())
                    }
                }
            }
        };
        let fields: Vec<Field> = self.fields().into_iter().filter(|f| Some(&f.id) != self.draft.as_ref().and_then(|d| d.fid.as_ref())).collect();
        let insert = |s: String| {
            move |this: &mut BaseEditor, _: &ClickEvent, _: &mut Window, cx: &mut Context<BaseEditor>| {
                if let Some((i, _)) = this.inputs.get("fd:expr").map(|x| (x.0.clone(), ())) {
                    let cur = i.read(cx).text();
                    let next = format!("{cur}{s}");
                    i.update(cx, |i, cx| i.set_text(&next, cx));
                    if let Some(d) = this.draft.as_mut() {
                        d.prop.expr = next;
                    }
                    cx.notify();
                }
            }
        };
        // 自动补全：末尾是未闭合的 `[xx` 时补字段，是字母开头的半个名字时补函数（附参数说明）
        let suggest: Vec<(String, String, String)> = {
            let chars: Vec<char> = expr.chars().collect();
            let open = chars.iter().rposition(|c| *c == '[');
            let closed = chars.iter().rposition(|c| *c == ']');
            if let Some(o) = open.filter(|o| closed.is_none_or(|c| c < *o)) {
                let part: String = chars[o + 1..].iter().collect();
                let head: String = chars[..o].iter().collect();
                fields
                    .iter()
                    .filter(|f| f.name.contains(part.trim()))
                    .take(8)
                    .map(|f| (format!("{head}[{}]", f.name), f.name.clone(), f.kind.label().to_string()))
                    .collect()
            } else {
                let st = chars.iter().rposition(|c| !(c.is_alphanumeric() || *c == '_')).map(|i| i + 1).unwrap_or(0);
                let part: String = chars[st..].iter().collect::<String>().to_uppercase();
                let head: String = chars[..st].iter().collect();
                if part.is_empty() || part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    vec![]
                } else {
                    FUNCS
                        .iter()
                        .filter(|f| f.0.starts_with(&part) && f.0 != part)
                        .take(8)
                        .map(|f| (format!("{head}{}(", f.0), f.0.to_string(), f.2.to_string()))
                        .collect()
                }
            }
        };
        let mut cats: Vec<&str> = vec![];
        for f in FUNCS {
            if !cats.contains(&f.1) {
                cats.push(f.1);
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(12.)).text_color(t.text_muted).child("公式（用 [字段名] 引用字段）"))
            .child(input_box(t).h(px(34.)).font_family("Menlo").child(inp))
            .child(match &preview {
                Ok(s) => div().text_size(px(12.)).text_color(t.text_secondary).child(s.clone()),
                Err(e) => div().text_size(px(12.)).text_color(t.fg_palette[1]).child(e.clone()),
            })
            .when(!suggest.is_empty(), |d| {
                d.child(div().flex().flex_col().rounded(px(6.)).border_1().border_color(t.border).py(px(2.)).children(suggest.into_iter().enumerate().map(
                    |(i, (full, name, hint))| {
                        div()
                            .id(("fx-sg", i))
                            .px(px(8.))
                            .h(px(26.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(div().font_family("Menlo").text_size(px(12.)).child(name))
                            .child(
                                div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.5)).text_color(t.text_muted).child(hint),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some((i, _)) = this.inputs.get("fd:expr").map(|x| (x.0.clone(), ())) {
                                    i.update(cx, |i, cx| {
                                        i.set_text(&full, cx);
                                        i.collapse_to_end(cx);
                                    });
                                }
                                if let Some(d) = this.draft.as_mut() {
                                    d.prop.expr = full.clone();
                                }
                                cx.notify();
                            }))
                    },
                )))
            })
            .child(div().text_size(px(12.)).text_color(t.text_muted).child("插入字段"))
            .child(div().flex().flex_row().flex_wrap().gap(px(4.)).children(fields.iter().map(|f| {
                div()
                    .id(SharedString::from(format!("fx-f-{}", f.id)))
                    .px(px(6.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .rounded(px(4.))
                    .bg(t.hover)
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.accent_soft))
                    .child(icon(f.kind.icon(), 11., t.text_secondary))
                    .child(f.name.clone())
                    .on_click(cx.listener(insert(format!("[{}]", f.name))))
            })))
            .child(div().text_size(px(12.)).text_color(t.text_muted).child("插入函数"))
            .child(div().id("fx-funcs").max_h(px(150.)).overflow_y_scroll().flex().flex_col().gap(px(6.)).children(cats.into_iter().map(|c| {
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(div().w(px(32.)).text_size(px(11.)).text_color(t.text_muted).pt(px(3.)).child(c))
                    .children(FUNCS.iter().filter(|f| f.1 == c).map(|(name, _, desc)| {
                        div()
                            .id(SharedString::from(format!("fx-{name}")))
                            .px(px(6.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .text_size(px(11.5))
                            .font_family("Menlo")
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_soft))
                            .child(*name)
                            .tooltip(tip(desc))
                            .on_click(cx.listener(insert(format!("{name}("))))
                    }))
            })))
            .into_any_element()
    }

    /// 给已有的关联字段在对面表建回链字段，并按当前关联值回填（不提交）。
    fn make_duplex(&mut self, fid: &str) {
        let tid = self.tid.clone();
        let Some(mut f) = self.field(fid) else { return };
        let target = f.prop.table.clone();
        let Some(mut m) = self.b().table(&target).map(|t| t.meta.clone()) else { return };
        let tname = self.b().table(&tid).map(|t| t.meta.name.clone()).unwrap_or_default();
        let mut back = Field::new(&tname, FieldType::DuplexLink);
        if self.b().table(&target).is_some_and(|t| t.field_by_name(&tname).is_some()) {
            back.name = format!("{tname}（{}）", f.name);
        }
        back.prop.table = tid.clone();
        back.prop.back = f.id.clone();
        f.prop.back = back.id.clone();
        m.fields.push(back.id.clone());
        let pairs: Vec<(String, Vec<String>)> = self
            .b()
            .table(&tid)
            .map(|t| t.records.values().map(|r| (r.id.clone(), str_list(r.cells.get(fid)))).filter(|x| !x.1.is_empty()).collect())
            .unwrap_or_default();
        self.bm().put_field(&target, back.clone());
        self.bm().put_table(m);
        self.bm().put_field(&tid, f);
        let mut backs: std::collections::HashMap<String, Vec<String>> = Default::default();
        for (rid, list) in pairs {
            for o in list {
                backs.entry(o).or_default().push(rid.clone());
            }
        }
        for (o, list) in backs {
            let rec = self.b().record(&target, &o).cloned();
            if let Some(mut r) = rec {
                r.cells.insert(back.id.clone(), Value::from(list));
                self.bm().put_record(&target, r);
            }
        }
    }

    /// 「确定」：校验并写入字段；修改类型时按规则转换已有数据。
    pub(super) fn save_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((kind0, fid0)) = self.draft.as_ref().map(|d| (d.kind, d.fid.clone())) else { return };
        let tid = self.tid.clone();
        let mut name = self.inputs.get("fd:name").map(|x| x.0.read(cx).text()).unwrap_or_default().trim().to_string();
        if name.is_empty() {
            name = kind0.label().to_string();
        }
        let dup = self.b().table(&tid).map(|t| t.fields.values().any(|f| f.name == name && Some(&f.id) != fid0.as_ref())).unwrap_or(false);
        let Some(d) = self.draft.as_mut() else { return };
        if dup {
            d.err = Some(format!("已有名为「{name}」的字段"));
            cx.notify();
            return;
        }
        let mut prop = d.prop.clone();
        prop.options.retain(|o| !o.name.trim().is_empty());
        let kind = d.kind;
        match kind {
            FieldType::Formula => {
                if let Err(e) = formula::parse(&prop.expr) {
                    d.err = Some(format!("公式有误：{e}"));
                    cx.notify();
                    return;
                }
            }
            FieldType::Link | FieldType::DuplexLink if prop.table.is_empty() => {
                d.err = Some("请选择要关联的数据表".into());
                cx.notify();
                return;
            }
            FieldType::Lookup if (prop.lookup_table.is_empty() && prop.link.is_empty()) || prop.target.is_empty() => {
                d.err = Some(if prop.lookup_table.is_empty() { "请选择关联字段和要引用的字段" } else { "请选择要引用的字段" }.into());
                cx.notify();
                return;
            }
            _ => {}
        }
        // 约定：单表公式 + 查找引用合计不超过 100 个，避免误用导致卡顿
        let computed = self
            .b()
            .table(&tid)
            .map(|t| t.fields.values().filter(|f| matches!(f.kind, FieldType::Formula | FieldType::Lookup) && Some(&f.id) != fid0.as_ref()).count())
            .unwrap_or(0);
        let Some(d) = self.draft.as_mut() else { return };
        if matches!(kind, FieldType::Formula | FieldType::Lookup) && computed >= 100 {
            d.err = Some("一张数据表最多 100 个公式和查找引用字段".into());
            cx.notify();
            return;
        }
        let (fid, insert_at) = (d.fid.clone(), d.insert_at);
        match fid {
            None => {
                let mut f = Field::new(&name, kind);
                f.prop = prop;
                if kind == FieldType::DuplexLink {
                    let tname = self.b().table(&tid).map(|t| t.meta.name.clone()).unwrap_or_default();
                    let target = f.prop.table.clone();
                    let mut back = Field::new(&tname, FieldType::DuplexLink);
                    if self.b().table(&target).is_some_and(|t| t.field_by_name(&tname).is_some()) {
                        back.name = format!("{tname}（{name}）");
                    }
                    back.prop.table = tid.clone();
                    back.prop.back = f.id.clone();
                    f.prop.back = back.id.clone();
                    let tmp = self.b().table(&target).map(|t| t.meta.clone());
                    if let Some(mut m) = tmp {
                        m.fields.push(back.id.clone());
                        self.bm().put_field(&target, back);
                        self.bm().put_table(m);
                    }
                }
                let new_id = f.id.clone();
                let after = insert_at.and_then(|i| self.b().table(&tid).and_then(|t| t.meta.fields.get(i).cloned()));
                self.insert_field(f, insert_at);
                // 视图里有自定义顺序时，把新字段放在参照字段之后
                let mut v = self.view();
                if !v.order.is_empty() {
                    let mut order = self.b().table(&tid).map(|t| t.view_fields(&v)).unwrap_or_default();
                    order.retain(|x| x != &new_id);
                    let at = after.and_then(|a| order.iter().position(|x| x == &a)).map(|i| i + 1).unwrap_or(order.len());
                    order.insert(at, new_id.clone());
                    v.order = order;
                    self.bm().put_view(&tid, v);
                }
            }
            Some(fid) => {
                let Some(old) = self.field(&fid) else { return };
                let mut nf = old.clone();
                nf.name = name;
                nf.kind = kind;
                nf.prop = prop;
                if old.kind == FieldType::DuplexLink && kind == FieldType::DuplexLink {
                    nf.prop.back = old.prop.back.clone();
                    nf.prop.table = old.prop.table.clone();
                }
                if old.kind != kind || (old.kind.is_link() && old.prop.table != nf.prop.table) {
                    // 先用旧字段算出转换后的值，再写入新字段
                    let rids: Vec<String> = self.b().table(&tid).map(|t| t.order.clone()).unwrap_or_default();
                    let mut conv = vec![];
                    {
                        let b = self.b();
                        for r in &rids {
                            conv.push((
                                r.clone(),
                                if kind.computed() {
                                    b.record(&tid, r).and_then(|x| x.cells.get(&fid)).cloned()
                                } else {
                                    calc::convert_value(&b, &tid, r, &old, &mut nf)
                                },
                            ));
                        }
                    }
                    if old.kind == FieldType::DuplexLink && kind != FieldType::DuplexLink {
                        let tmp = self.b().field(&old.prop.table, &old.prop.back).cloned();
                        if let Some(mut back) = tmp {
                            back.kind = FieldType::Link;
                            back.prop.back.clear();
                            self.bm().put_field(&old.prop.table, back);
                        }
                        nf.prop.back.clear();
                    }
                    self.bm().rename_field(&tid, nf.clone());
                    for (r, v) in conv {
                        let tmp = self.b().record(&tid, &r).cloned();
                        if let Some(mut rec) = tmp {
                            match v {
                                Some(v) => rec.cells.insert(fid.clone(), v),
                                None => rec.cells.remove(&fid),
                            };
                            self.bm().put_record(&tid, rec);
                        }
                    }
                } else {
                    // 删掉的选项从单元格里清除
                    if kind.has_options() {
                        let removed: Vec<String> = old.prop.options.iter().filter(|o| nf.opt(&o.id).is_none()).map(|o| o.id.clone()).collect();
                        if !removed.is_empty() {
                            let recs: Vec<Record> = self
                                .b()
                                .table(&tid)
                                .map(|t| t.records.values().filter(|r| str_list(r.cells.get(&fid)).iter().any(|x| removed.contains(x))).cloned().collect())
                                .unwrap_or_default();
                            for mut r in recs {
                                let left: Vec<String> = str_list(r.cells.get(&fid)).into_iter().filter(|x| !removed.contains(x)).collect();
                                if left.is_empty() {
                                    r.cells.remove(&fid);
                                } else if kind == FieldType::MultiSelect {
                                    r.cells.insert(fid.clone(), Value::from(left));
                                } else {
                                    r.cells.insert(fid.clone(), Value::from(left[0].clone()));
                                }
                                self.bm().put_record(&tid, r);
                            }
                        }
                    }
                    self.bm().rename_field(&tid, nf);
                }
                // 已有字段改成双向关联：在对面表补一个回链字段，并把现有关联同步过去
                if kind == FieldType::DuplexLink && old.kind != FieldType::DuplexLink {
                    self.make_duplex(&fid);
                }
            }
        }
        self.commit(cx);
        self.close_popups();
        window.focus(&self.focus);
        cx.notify();
    }
}
