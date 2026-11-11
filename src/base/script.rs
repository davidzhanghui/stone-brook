//! 脚本驱动测试的入口（`STONEBROOK_SCRIPT`，仅 dev 构建）。

use super::editor::*;
use super::model::*;
use gpui::*;
use serde_json::Value;

impl BaseEditor {
    /// 测试脚本：add / set 行号 字段名 值 / view 种类 / filter 字段 op 值 / sort 字段 / group 字段 / detail 行号 / kanban 行号 列名 / dump
    #[cfg(feature = "dev")]
    pub fn test(&mut self, cmd: &str, arg: &str, window: &mut Window, cx: &mut Context<Self>) {
        let parts: Vec<&str> = arg.splitn(3, ' ').collect();
        if let Some((_, f)) = COMMANDS.iter().find(|(names, _)| names.contains(&cmd)) {
            f(self, cmd, arg, &parts, window, cx);
        }
        cx.notify();
    }
}

#[cfg(feature = "dev")]
fn fid_by_name(this: &BaseEditor, n: &str) -> Option<String> {
    this.fields().into_iter().find(|f| f.name == n).map(|f| f.id)
}

#[cfg(feature = "dev")]
type Cmd = fn(&mut BaseEditor, &str, &str, &[&str], &mut Window, &mut Context<BaseEditor>);

/// 测试命令名 → 处理函数。
#[cfg(feature = "dev")]
#[allow(unused_variables)]
static COMMANDS: &[(&[&str], Cmd)] = &[
    (&["add"], |this, cmd, arg, parts, window, cx| {
        let n: usize = arg.parse().unwrap_or(1);
        for _ in 0..n {
            this.add_record(vec![], None, cx);
        }
    }),
    (&["set"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let rows = this.rows();
        let Some(rid) = parts[0].parse::<usize>().ok().and_then(|i| rows.get(i).cloned()) else { return };
        let Some(fid) = fid_by_name(this, parts[1]) else { return };
        let Some(mut f) = this.field(&fid) else { return };
        let raw = parts.get(2).copied().unwrap_or("");
        let v = if f.kind == FieldType::Attachment {
            let assets = crate::objects::store().map(|s| s.assets_dir()).unwrap_or_default();
            let files: Vec<Attachment> = raw.split('|').filter_map(|p| import_file(&assets, std::path::Path::new(p))).collect();
            serde_json::to_value(files).unwrap_or(Value::Null)
        } else if f.kind.is_link() {
            let tid = f.prop.table.clone();
            let b = this.b();
            let ids: Vec<String> =
                raw.split(',').filter_map(|name| b.table(&tid)?.order.iter().find(|r| b.primary_text(&tid, r) == name.trim()).cloned()).collect();
            Value::from(ids)
        } else {
            super::io::text_to_value(&mut f, raw).unwrap_or(Value::Null)
        };
        let tid = this.tid.clone();
        this.bm().put_field(&tid, f);
        this.set_cell(&rid, &fid, v, false, cx);
    }),
    (&["field"], |this, cmd, arg, parts, window, cx| {
        // field 名称 类型序号 [公式 / 关联表名 / 查找 关联字段名.目标字段名.汇总]
        let kinds: Vec<FieldType> = TYPE_MENU.iter().flat_map(|g| g.1.iter().copied()).collect();
        let Some(kind) = parts.get(1).and_then(|k| k.parse::<usize>().ok()).and_then(|i| kinds.get(i).copied()) else { return };
        let mut f = Field::new(parts[0], kind);
        let extra = parts.get(2).copied().unwrap_or("");
        match kind {
            FieldType::Formula => f.prop.expr = extra.into(),
            FieldType::Link | FieldType::DuplexLink => {
                let tid = this.b().table_list().iter().find(|t| t.meta.name == extra).map(|t| t.meta.id.clone()).unwrap_or_default();
                f.prop.table = tid;
            }
            FieldType::Lookup => {
                let ps: Vec<&str> = extra.split('.').collect();
                if let Some(link) = ps.first().and_then(|n| fid_by_name(this, n)) {
                    let target_t = this.field(&link).map(|l| l.prop.table).unwrap_or_default();
                    f.prop.link = link;
                    f.prop.target =
                        this.b().table(&target_t).and_then(|t| t.field_by_name(ps.get(1).copied().unwrap_or("")).map(|x| x.id.clone())).unwrap_or_default();
                    f.prop.agg = ps.get(2).copied().unwrap_or("").into();
                }
            }
            FieldType::Currency => f.prop.fmt = 2,
            _ => {}
        }
        this.draft = Some(Draft { fid: None, kind, prop: f.prop.clone(), insert_at: None, err: None });
        let i = this.input("fd:name", "字段名称", parts[0], false, window, cx);
        i.update(cx, |i, cx| i.set_text(parts[0], cx));
        this.save_draft(window, cx);
    }),
    (&["opts"], |this, cmd, arg, parts, window, cx| {
        // opts 字段名 选项1,选项2,…（设置单选 / 多选选项）
        if let Some(mut f) = fid_by_name(this, parts[0]).and_then(|id| this.field(&id)) {
            for n in parts.get(1).copied().unwrap_or("").split(',') {
                f.ensure_opt(n.trim());
            }
            let tid = this.tid.clone();
            this.bm().put_field(&tid, f);
            this.commit(cx);
        }
    }),
    (&["view"], |this, cmd, arg, parts, window, cx| {
        let k = match arg {
            "kanban" => ViewKind::Kanban,
            "gallery" => ViewKind::Gallery,
            "calendar" => ViewKind::Calendar,
            "gantt" => ViewKind::Gantt,
            "form" => ViewKind::Form,
            _ => ViewKind::Grid,
        };
        let existing = this.b().table(&this.tid).and_then(|t| t.meta.views.iter().find(|v| t.views.get(*v).is_some_and(|x| x.kind == k)).cloned());
        match existing {
            Some(v) => this.switch_view(&v, cx),
            None => this.add_view(k, cx),
        }
    }),
    (&["table"], |this, cmd, arg, parts, window, cx| {
        let tid = this.b().table_list().iter().find(|t| t.meta.name == arg).map(|t| t.meta.id.clone());
        match tid {
            Some(t) => this.switch_table(&t, cx),
            None => {
                let tid = this.bm().add_table(arg, ViewKind::Grid, 3);
                this.commit(cx);
                this.switch_table(&tid, cx);
            }
        }
    }),
    (&["filter"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let Some(fid) = fid_by_name(this, parts[0]) else { return };
        let op = match parts[1] {
            "is" => Op::Is,
            "isnot" => Op::IsNot,
            "contains" => Op::Contains,
            "empty" => Op::Empty,
            "notempty" => Op::NotEmpty,
            "gt" => Op::Gt,
            "lt" => Op::Lt,
            _ => Op::Contains,
        };
        let f = this.field(&fid);
        let raw = parts.get(2).copied().unwrap_or("");
        let value = match f.as_ref().map(|f| f.kind) {
            Some(k) if k.has_options() => {
                Value::from(raw.split(',').filter_map(|n| f.as_ref().and_then(|f| f.opt_by_name(n.trim())).map(|o| o.id.clone())).collect::<Vec<_>>())
            }
            _ => Value::from(raw),
        };
        this.update_view(cx, |v| v.filter.conds.push(Cond { field: fid, op, value }));
    }),
    (&["clearfilter"], |this, cmd, arg, parts, window, cx| this.update_view(cx, |v| v.filter.conds.clear())),
    (&["sort"], |this, cmd, arg, parts, window, cx| {
        let (n, desc) = arg.split_once(' ').map(|(a, b)| (a, b == "desc")).unwrap_or((arg, false));
        if let Some(fid) = fid_by_name(this, n) {
            this.update_view(cx, |v| v.sorts = vec![SortRule { field: fid, desc }]);
        }
    }),
    (&["group"], |this, cmd, arg, parts, window, cx| {
        if let Some(fid) = fid_by_name(this, arg) {
            this.update_view(cx, |v| v.groups = vec![SortRule { field: fid, desc: false }]);
        } else {
            this.update_view(cx, |v| v.groups.clear());
        }
    }),
    (&["stat"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        if let Some(fid) = fid_by_name(this, parts[0]) {
            let k = parts[1].to_string();
            this.update_view(cx, |v| {
                v.stats.insert(fid, k);
            });
        }
    }),
    (&["hide"], |this, cmd, arg, parts, window, cx| {
        if let Some(fid) = fid_by_name(this, arg) {
            this.update_view(cx, |v| v.hidden.push(fid));
        }
    }),
    (&["rowh"], |this, cmd, arg, parts, window, cx| {
        let h = arg.parse().unwrap_or(1);
        this.update_view(cx, |v| v.row_h = h);
    }),
    (&["compact"], |this, cmd, arg, parts, window, cx| this.update_view(cx, |v| v.compact = arg != "off")),
    (&["detail"], |this, cmd, arg, parts, window, cx| {
        let rows = this.rows();
        this.detail = arg.parse::<usize>().ok().and_then(|i| rows.get(i).cloned());
    }),
    (&["close"], |this, cmd, arg, parts, window, cx| {
        this.detail = None;
        this.close_popups();
    }),
    (&["popup"], |this, cmd, arg, parts, window, cx| {
        let at = point(px(400.), px(160.));
        let fid0 = this.visible().get(1).map(|f| f.id.clone()).unwrap_or_default();
        let rid0 = this.rows().first().cloned().unwrap_or_default();
        let p = match parts[0] {
            "filter" => Popup::Filter,
            "sort" => Popup::Sort,
            "group" => Popup::Group,
            "fields" => Popup::Fields,
            "newview" => Popup::NewView,
            "more" => Popup::More,
            "col" => Popup::ColMenu(fid0),
            "options" => Popup::Options { rid: rid0, fid: parts.get(1).and_then(|n| fid_by_name(this, n)).unwrap_or(fid0) },
            "date" => {
                use chrono::Datelike;
                let n = chrono::Local::now();
                Popup::Date { rid: rid0, fid: parts.get(1).and_then(|n| fid_by_name(this, n)).unwrap_or(fid0), ym: (n.year(), n.month()) }
            }
            "links" => Popup::Links { rid: rid0, fid: parts.get(1).and_then(|n| fid_by_name(this, n)).unwrap_or(fid0) },
            "kset" => Popup::KanbanSet,
            "colors" => Popup::Colors,
            "batch" => Popup::BatchEdit,
            "groups" => Popup::FieldGroups,
            "auto" => Popup::Automations,
            "formset" => Popup::FormSet,
            "export" => Popup::Export,
            "dateset" => Popup::DateSet,
            "rowmenu" => Popup::RowMenu(rid0.clone()),
            "delfield" => Popup::DelField(parts.get(1).and_then(|n| fid_by_name(this, n)).unwrap_or(fid0)),
            "widget" => {
                let w = this.dash.as_ref().and_then(|d| {
                    this.b().meta.dashboards.iter().find(|x| &x.id == d).and_then(|d| d.widgets.get(1).or(d.widgets.first()).map(|w| w.id.clone()))
                });
                match w {
                    Some(w) => Popup::Widget(w),
                    None => return,
                }
            }
            "editfield" => {
                let fid = parts.get(1).and_then(|n| fid_by_name(this, n)).unwrap_or(fid0);
                this.popup = None;
                this.edit_field(Some(fid), None, at, window, cx);
                return;
            }
            "newfield" => {
                this.popup = None;
                this.edit_field(None, None, at, window, cx);
                return;
            }
            _ => return,
        };
        this.open_popup(p, at, cx);
    }),
    (&["kanban"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        // 把第 N 张卡片移到某列（按列名）
        let rows = this.rows();
        let Some(rid) = parts[0].parse::<usize>().ok().and_then(|i| rows.get(i).cloned()) else { return };
        let v = this.view();
        let Some(gf) = v.group_field.clone() else { return };
        let key = this.field(&gf).and_then(|f| f.opt_by_name(parts[1]).map(|o| o.id.clone())).unwrap_or_default();
        this.move_card(&rid, None, &key, None, cx);
    }),
    (&["sel"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let rows = this.rows();
        let vis = this.visible();
        if let (Some(r), Some(f)) =
            (parts[0].parse::<usize>().ok().and_then(|i| rows.get(i).cloned()), parts[1].parse::<usize>().ok().and_then(|i| vis.get(i).map(|f| f.id.clone())))
        {
            this.sel = Some(Sel { rid: r.clone(), fid: f.clone(), arid: r, afid: f });
        }
    }),
    (&["edit"], |this, cmd, arg, parts, window, cx| this.start_cell_edit(None, window, cx)),
    (&["type"], |this, cmd, arg, parts, window, cx| {
        if let Some((i, _)) = this.inputs.get("cell") {
            i.update(cx, |i, cx| i.set_text(arg, cx));
        }
    }),
    (&["search"], |this, cmd, arg, parts, window, cx| {
        this.search = arg.to_string();
        this.search_open = !arg.is_empty();
        if this.search_open {
            let i = this.input("search", "搜索记录", arg, false, window, cx);
            i.update(cx, |i, cx| i.set_text(arg, cx));
        }
    }),
    (&["undo"], |this, cmd, arg, parts, window, cx| this.undo(cx)),
    (&["redo"], |this, cmd, arg, parts, window, cx| this.redo(cx)),
    (&["import"], |this, cmd, arg, parts, window, cx| this.import_path(std::path::Path::new(arg), false, cx)),
    (&["append"], |this, cmd, arg, parts, window, cx| this.import_path(std::path::Path::new(arg), true, cx)),
    (&["lock"], |this, cmd, arg, parts, window, cx| {
        let tid = this.tid.clone();
        let mut v = this.view();
        v.locked = arg != "off";
        this.bm().put_view(&tid, v);
        this.commit(cx);
    }),
    // color 字段 运算符 值 [kind]：加一条填色规则
    (&["color"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let Some(fid) = fid_by_name(this, parts[0]) else { return };
        let rest: Vec<&str> = parts.get(2).copied().unwrap_or("").split(' ').collect();
        let op = match parts[1] {
            "gt" => Op::Gt,
            "lt" => Op::Lt,
            "is" => Op::Is,
            "notempty" => Op::NotEmpty,
            "col" => Op::Empty,
            _ => Op::Contains,
        };
        let kind: u8 = rest.get(1).and_then(|k| k.parse().ok()).unwrap_or(0);
        let color = (this.view().colors.len() % OPT_COLORS) as u8 + 1;
        let value = Value::from(rest.first().copied().unwrap_or(""));
        this.update_view(cx, |v| {
            v.colors.push(ColorRule {
                id: new_id("clr"),
                kind: if parts[1] == "col" { 2 } else { kind },
                field: fid.clone(),
                cond: (parts[1] != "col").then(|| Cond { field: fid, op, value }),
                color,
            })
        });
    }),
    (&["hier"], |this, cmd, arg, parts, window, cx| this.toggle_hierarchy(cx)),
    // child 子行号 父行号（按当前显示顺序）
    (&["child"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let rows = this.tree_rows();
        let get = |i: &str| i.parse::<usize>().ok().and_then(|i| rows.get(i).map(|x| x.0.clone()));
        if let (Some(c), Some(p)) = (get(parts[0]), get(parts[1])) {
            let tid = this.tid.clone();
            let r = this.bm().set_parent(&tid, &c, Some(&p));
            if let Err(e) = r {
                println!("set_parent: {e}");
            }
            this.commit(cx);
        }
    }),
    (&["check"], |this, cmd, arg, parts, window, cx| {
        let rows = this.rows();
        this.checked = arg.split_whitespace().filter_map(|i| i.parse::<usize>().ok().and_then(|i| rows.get(i).cloned())).collect();
    }),
    (&["dash"], |this, cmd, arg, parts, window, cx| this.add_dashboard(cx)),
    (&["nodash"], |this, cmd, arg, parts, window, cx| this.dash = None),
    (&["calmode"], |this, cmd, arg, parts, window, cx| {
        let k: u8 = arg.parse().unwrap_or(0);
        let tid = this.tid.clone();
        let mut v = this.view();
        v.cal_mode = k;
        this.bm().put_view(&tid, v);
        this.bm().commit_silent();
    }),
    (&["scale"], |this, cmd, arg, parts, window, cx| {
        let k: u8 = arg.parse().unwrap_or(0);
        this.gantt_scroll = 0.;
        this.update_view(cx, |v| v.scale = k);
    }),
    (&["workdays"], |this, cmd, arg, parts, window, cx| this.update_view(cx, |v| v.workdays_only = arg != "off")),
    (&["progress"], |this, cmd, arg, parts, window, cx| {
        if let Some(fid) = fid_by_name(this, arg) {
            this.update_view(cx, |v| v.progress_field = Some(fid));
        }
    }),
    (&["enddate"], |this, cmd, arg, parts, window, cx| {
        if let Some(fid) = fid_by_name(this, arg) {
            this.update_view(cx, |v| v.end_field = Some(fid));
        }
    }),
    (&["detailtab"], |this, cmd, arg, parts, window, cx| {
        this.detail_tab = arg.parse().unwrap_or(0);
        this.refresh_comment_counts();
    }),
    (&["comment"], |this, cmd, arg, parts, window, cx| {
        if let (Some(rid), Some(st)) = (this.detail.clone(), crate::objects::store()) {
            st.add_comment(&format!("base:{}", this.obj_id), 0, &rid, arg, "我");
            this.refresh_comment_counts();
        }
    }),
    (&["layout"], |this, cmd, arg, parts, window, cx| {
        let tid = this.tid.clone();
        let meta = this.b().table(&tid).map(|t| t.meta.clone());
        if let Some(mut m) = meta {
            m.detail_layout = arg.parse().unwrap_or(0);
            this.bm().put_table(m);
            this.commit(cx);
        }
    }),
    // fgroup 名称 字段1,字段2
    (&["fgroup"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let ids: Vec<String> = parts[1].split(',').filter_map(|n| fid_by_name(this, n.trim())).collect();
        let tid = this.tid.clone();
        let meta = this.b().table(&tid).map(|t| t.meta.clone());
        if let Some(mut m) = meta {
            m.groups.push(FieldGroup { id: new_id("grp"), name: parts[0].into(), fields: ids });
            this.bm().put_table(m);
            this.commit(cx);
        }
    }),
    // autoadd：示例自动化（勾选「完成」时评分设为 5 并发通知）
    (&["autoadd"], |this, cmd, arg, parts, window, cx| {
        let tid = this.tid.clone();
        let (Some(done), Some(rate)) = (fid_by_name(this, "完成"), fid_by_name(this, "评分")) else { return };
        let mut m = this.b().meta.clone();
        let id = new_id("auto");
        m.automations.push(Automation {
            id: id.clone(),
            name: "完成即满分".into(),
            on: true,
            trigger: "match".into(),
            table: tid,
            cond: Filter { any: false, conds: vec![Cond { field: done, op: Op::Checked, value: Value::Null }] },
            actions: vec![
                AutoAction { kind: "update".into(), sets: vec![FieldSet { field: rate, value: Value::from(5) }], ..Default::default() },
                AutoAction { kind: "notify".into(), text: "{{[文本]}} 已完成".into(), ..Default::default() },
            ],
            ..Default::default()
        });
        this.bm().set_meta(m);
        this.commit(cx);
        this.auto_sel = Some(id);
    }),
    (&["press"], |this, cmd, arg, parts, window, cx| {
        if !(parts.len() >= 2) {
            return;
        }

        let rows = this.rows();
        if let (Some(r), Some(f)) = (parts[0].parse::<usize>().ok().and_then(|i| rows.get(i).cloned()), fid_by_name(this, parts[1])) {
            this.press_button(&r, &f, cx);
        }
    }),
    (&["stress"], |this, cmd, arg, parts, window, cx| {
        let n: usize = arg.parse().unwrap_or(2000);
        let tid = super::stress::build(&mut this.bm(), n);
        this.commit(cx);
        this.switch_table(&tid, cx);
    }),
    (&["find"], |this, cmd, arg, parts, window, cx| this.find_step(1, cx)),
    // lookupf 名称 目标表名 目标字段名 条件字段名：打开「按条件查找」的字段编辑器
    (&["lookupf"], |this, cmd, arg, parts, window, cx| {
        let p: Vec<&str> = arg.split(' ').collect();
        let (Some(name), Some(tname)) = (p.first(), p.get(1)) else { return };
        let Some(tt) = this.b().table_list().iter().find(|t| t.meta.name == *tname).map(|t| t.meta.id.clone()) else { return };
        let target = p.get(2).and_then(|n| this.b().table(&tt).and_then(|t| t.field_by_name(n)).map(|f| f.id.clone())).unwrap_or_default();
        let cf = p.get(3).and_then(|n| this.b().table(&tt).and_then(|t| t.field_by_name(n)).map(|f| f.id.clone())).unwrap_or_default();
        let rhs = this.b().table(&this.tid).and_then(|t| t.primary().map(|f| f.id.clone())).unwrap_or_default();
        let mut prop = default_prop(FieldType::Lookup);
        prop.lookup_table = tt;
        prop.target = target;
        prop.agg = "sum".into();
        prop.conds = vec![LookupCond { field: cf, op: Op::Is, rhs, value: Value::Null }];
        this.close_popups();
        this.draft = Some(Draft { fid: None, kind: FieldType::Lookup, prop, insert_at: None, err: None });
        let i = this.input("fd:name", "字段名称", name, true, window, cx);
        i.update(cx, |i, cx| i.set_text(name, cx));
        this.popup = Some((Popup::FieldEdit, point(px(500.), px(120.))));
    }),
    (&["savefield"], |this, cmd, arg, parts, window, cx| this.save_draft(window, cx)),
    // fexpr 文本：在打开的公式编辑器里输入（看自动补全）
    (&["fexpr"], |this, cmd, arg, parts, window, cx| {
        if let Some((i, _)) = this.inputs.get("fd:expr").map(|x| (x.0.clone(), ())) {
            i.update(cx, |i, cx| i.set_text(arg, cx));
        }
        if let Some(d) = this.draft.as_mut() {
            d.prop.expr = arg.to_string();
        }
    }),
    (&["dump"], |this, cmd, arg, parts, window, cx| {
        let b = this.b();
        let rows = b.view_records(&this.tid, &this.view(), &this.search);
        let t = b.table(&this.tid).unwrap();
        let fields = t.visible_fields(&this.view());
        println!("== {} / {} ({} 条)", t.meta.name, this.view().name, rows.len());
        println!("{}", fields.iter().map(|f| t.fields[f].name.clone()).collect::<Vec<_>>().join(" | "));
        for r in rows.iter().take(30) {
            println!("{}", fields.iter().map(|f| b.text(&this.tid, r, f)).collect::<Vec<_>>().join(" | "));
        }
    }),
];
