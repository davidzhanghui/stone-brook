//! 表格视图：行列双向虚拟化（只渲染可见区域）、冻结列、分组、选区与键盘导航、原地编辑、
//! 复制粘贴（与 Excel 互通的 TSV）、拖动列宽 / 列顺序 / 行顺序、底部统计栏。

use super::cells::Mode;
use super::editor::{BaseEditor, Drag, Popup, Sel};
use super::model::*;
use super::widgets::*;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::Value;
use std::rc::Rc;

pub(super) const HEAD_H: f32 = 36.;
pub(super) const IDX_W: f32 = 64.;
const GROUP_H: f32 = 42.;
const ADD_H: f32 = 36.;
const STAT_H: f32 = 32.;
const PLUS_W: f32 = 96.;
/// 字段编组行的高度
const BAND_H: f32 = 26.;

#[derive(Clone, Debug)]
pub(super) enum GRow {
    Group {
        depth: usize,
        path: String,
        label: String,
        color: Option<u8>,
        count: usize,
        fid: String,
        collapsed: bool,
        rids: Vec<String>,
    },
    /// `tdepth` 记录层级深度，`kids` 是否有子记录，`matched` 是否命中筛选（补出的祖先为 false）
    Rec {
        rid: String,
        depth: usize,
        n: usize,
        tdepth: usize,
        kids: bool,
        matched: bool,
    },
    Add {
        depth: usize,
        preset: Vec<(String, Value)>,
    },
}

/// 上一帧的排版结果。
#[derive(Default, Clone)]
pub struct GridLayout {
    pub(super) rows: Vec<GRow>,
    pub(super) row_y: Vec<f32>,
    pub(super) cols: Vec<String>,
    pub(super) col_x: Vec<f32>,
    pub(super) frozen: usize,
    pub(super) recs: Vec<String>,
}

impl GridLayout {
    fn total_h(&self) -> f32 {
        self.row_y.last().copied().unwrap_or(0.)
    }
    fn total_w(&self) -> f32 {
        self.col_x.last().copied().unwrap_or(IDX_W) + PLUS_W
    }
    fn frozen_w(&self) -> f32 {
        self.col_x.get(self.frozen).copied().unwrap_or(IDX_W)
    }
    /// 第 i 列在容器里的 x（非冻结列减去横向滚动）。
    fn x(&self, i: usize, sx: f32) -> f32 {
        let x = self.col_x.get(i).copied().unwrap_or(0.);
        if i >= self.frozen {
            x - sx
        } else {
            x
        }
    }
    fn rec_index(&self, rid: &str) -> Option<usize> {
        self.rows.iter().position(|r| matches!(r, GRow::Rec { rid: x, .. } if x == rid))
    }
}

/// 分组键 → 新记录预填值。
fn preset_value(f: &Field, key: &str) -> Option<Value> {
    if key.is_empty() {
        return None;
    }
    use FieldType::*;
    Some(match f.kind {
        Select | Stage => Value::from(key),
        MultiSelect | Person | Link | DuplexLink => Value::from(key.split(',').map(String::from).collect::<Vec<_>>()),
        Checkbox => Value::Bool(key == "1"),
        Date => Value::from(super::formula::parse_date(key)?),
        Number | Currency | Rating | Progress => Value::from(key.replace([',', '%', '¥', '$'], "").parse::<f64>().ok()?),
        Text | Url | Phone | Email => Value::from(key),
        _ => return None,
    })
}

impl BaseEditor {
    pub(super) fn col_width(&self, v: &View, f: &Field, primary: bool) -> f32 {
        if let Some(Drag::ColResize { fid, start_x, orig }) = &self.drag {
            if fid == &f.id {
                return (orig + f32::from(self.mouse.x - *start_x)).max(60.);
            }
        }
        v.widths.get(&f.id).copied().unwrap_or(if primary { 240. } else { 180. })
    }

    fn build_rows(&mut self, v: &View) -> Vec<GRow> {
        let tree = self.tree_rows();
        let groups: Vec<SortRule> = v.groups.iter().filter(|g| self.field(&g.field).is_some()).cloned().collect();
        // 开启层级时只按顶级记录分组，子记录跟着父记录走
        let roots: Vec<String> = tree.iter().filter(|x| x.1 == 0).map(|x| x.0.clone()).collect();
        let mut out = vec![];
        let mut n = 0;
        self.group_into(&roots, &groups, 0, String::new(), vec![], v, &mut out, &mut n);
        if tree.iter().any(|x| x.1 > 0) || self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some()) {
            // 把每个顶级记录后面补上它的子树
            let mut sub: std::collections::HashMap<String, Vec<(String, usize, bool)>> = Default::default();
            let mut cur: Option<String> = None;
            for x in tree.iter() {
                if x.1 == 0 {
                    cur = Some(x.0.clone());
                } else if let Some(c) = &cur {
                    sub.entry(c.clone()).or_default().push(x.clone());
                }
            }
            let has_kids: std::collections::HashSet<String> = {
                let b = self.b();
                b.table(&self.tid).map(|t| t.order.iter().filter_map(|r| b.parent_of(&self.tid, r)).collect()).unwrap_or_default()
            };
            let matched: std::collections::HashMap<&String, bool> = tree.iter().map(|x| (&x.0, x.2)).collect();
            let mut expanded = vec![];
            for r in out {
                match r {
                    GRow::Rec { rid, depth, .. } => {
                        let kids = has_kids.contains(&rid);
                        let m = matched.get(&rid).copied().unwrap_or(true);
                        let children = sub.remove(&rid).unwrap_or_default();
                        expanded.push(GRow::Rec { rid, depth, n: 0, tdepth: 0, kids, matched: m });
                        for (c, d, m) in children {
                            let kids = has_kids.contains(&c);
                            expanded.push(GRow::Rec { rid: c, depth, n: 0, tdepth: d, kids, matched: m });
                        }
                    }
                    g => expanded.push(g),
                }
            }
            let mut k = 0;
            for r in expanded.iter_mut() {
                if let GRow::Rec { n, .. } = r {
                    k += 1;
                    *n = k;
                }
            }
            return expanded;
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn group_into(
        &self,
        rids: &[String],
        groups: &[SortRule],
        level: usize,
        path: String,
        preset: Vec<(String, Value)>,
        v: &View,
        out: &mut Vec<GRow>,
        n: &mut usize,
    ) {
        if level >= groups.len() {
            for r in rids {
                *n += 1;
                out.push(GRow::Rec { rid: r.clone(), depth: level, n: *n, tdepth: 0, kids: false, matched: true });
            }
            out.push(GRow::Add { depth: level, preset });
            return;
        }
        let rule = &groups[level];
        let b = self.b();
        let mut keys: Vec<(String, String, Option<u8>)> = vec![];
        let mut members: std::collections::HashMap<String, Vec<String>> = Default::default();
        for r in rids {
            for (k, label, c) in b.group_keys(&self.tid, r, &rule.field, false) {
                if !members.contains_key(&k) {
                    keys.push((k.clone(), label, c));
                }
                members.entry(k).or_default().push(r.clone());
            }
        }
        b.order_groups(&self.tid, &rule.field, &mut keys, rule.desc);
        let f = b.field(&self.tid, &rule.field).cloned();
        drop(b);
        for (k, label, color) in keys {
            let p = format!("{path}/{}:{k}", rule.field);
            let collapsed = v.collapsed.contains(&p);
            let list = members.remove(&k).unwrap_or_default();
            out.push(GRow::Group { depth: level, path: p.clone(), label, color, count: list.len(), fid: rule.field.clone(), collapsed, rids: list.clone() });
            if !collapsed {
                let mut pre = preset.clone();
                if let Some(val) = f.as_ref().and_then(|f| preset_value(f, &k)) {
                    pre.push((rule.field.clone(), val));
                }
                self.group_into(&list, groups, level + 1, p, pre, v, out, n);
            }
        }
    }

    fn layout_grid(&mut self, v: &View) {
        // 排版缓存：结果列表、分组 / 折叠 / 行高 / 列宽都没变时直接复用上一帧（10 万行时每帧重排太贵）
        let tree = self.tree_rows();
        let hier = self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some());
        let resizing = matches!(self.drag, Some(Drag::ColResize { .. }));
        let key = format!(
            "{:p}|{}|{}|{}|{}|{}|{}|{}|{}",
            Rc::as_ptr(&tree),
            self.tid,
            self.vid,
            if !v.groups.is_empty() || hier { self.b().ver } else { 0 },
            serde_json::to_string(&(&v.groups, &v.collapsed, &v.hidden, &v.order, v.row_h, v.frozen)).unwrap_or_default(),
            serde_json::to_string(&v.widths.iter().collect::<std::collections::BTreeMap<_, _>>()).unwrap_or_default(),
            self.b().table(&self.tid).map(|t| t.meta.fields.join(",")).unwrap_or_default(),
            self.b().table(&self.tid).map(|t| t.fields.values().map(|f| f.name.len()).sum::<usize>()).unwrap_or(0),
            resizing
        );
        if !resizing && key == self.gl_key && !self.gl.rows.is_empty() {
            return;
        }
        self.gl_key = key;
        let rows = self.build_rows(v);
        let rh = v.row_height();
        let mut y = 0.;
        let mut row_y = vec![0.];
        for r in &rows {
            y += match r {
                GRow::Group { .. } => GROUP_H,
                GRow::Rec { .. } => rh,
                GRow::Add { .. } => ADD_H,
            };
            row_y.push(y);
        }
        let vis = self.visible();
        let mut x = IDX_W;
        let mut col_x = vec![IDX_W];
        for (i, f) in vis.iter().enumerate() {
            x += self.col_width(v, f, i == 0);
            col_x.push(x);
        }
        let recs = rows.iter().filter_map(|r| if let GRow::Rec { rid, .. } = r { Some(rid.clone()) } else { None }).collect();
        self.gl = GridLayout { rows, row_y, cols: vis.iter().map(|f| f.id.clone()).collect(), col_x, frozen: (v.frozen.max(1) as usize).min(vis.len()), recs };
    }

    fn clamp_scroll(&mut self) {
        let vw = f32::from(self.grid_bounds.size.width);
        let vh = f32::from(self.grid_bounds.size.height) - HEAD_H - STAT_H;
        let max_x = (self.gl.total_w() - vw + 40.).max(0.);
        let max_y = (self.gl.total_h() - vh + 80.).max(0.);
        self.scroll.0 = self.scroll.0.clamp(0., max_x);
        self.scroll.1 = self.scroll.1.clamp(0., max_y);
    }

    /// 单元格在窗口中的矩形（弹层定位用）。
    pub(super) fn cell_rect(&self, rid: &str, fid: &str) -> Option<Bounds<Pixels>> {
        let ri = self.gl.rec_index(rid)?;
        let ci = self.gl.cols.iter().position(|c| c == fid)?;
        let x = self.gl.x(ci, self.scroll.0);
        let y = HEAD_H + self.gl.row_y[ri] - self.scroll.1;
        let w = self.gl.col_x[ci + 1] - self.gl.col_x[ci];
        let h = self.gl.row_y[ri + 1] - self.gl.row_y[ri];
        let o = self.grid_bounds.origin;
        Some(Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(w), px(h))))
    }

    pub(super) fn ensure_visible(&mut self) {
        let Some(s) = self.sel.clone() else { return };
        let (Some(ri), Some(ci)) = (self.gl.rec_index(&s.rid), self.gl.cols.iter().position(|c| c == &s.fid)) else { return };
        let vh = f32::from(self.grid_bounds.size.height) - HEAD_H - STAT_H;
        let (y0, y1) = (self.gl.row_y[ri], self.gl.row_y[ri + 1]);
        if y0 < self.scroll.1 {
            self.scroll.1 = y0;
        } else if y1 > self.scroll.1 + vh {
            self.scroll.1 = y1 - vh;
        }
        if ci >= self.gl.frozen {
            let vw = f32::from(self.grid_bounds.size.width);
            let fw = self.gl.frozen_w();
            let (x0, x1) = (self.gl.col_x[ci], self.gl.col_x[ci + 1]);
            if x0 - self.scroll.0 < fw {
                self.scroll.0 = x0 - fw;
            } else if x1 - self.scroll.0 > vw {
                self.scroll.0 = x1 - vw;
            }
        }
        self.clamp_scroll();
    }

    // ───────────── 选区与键盘 ─────────────

    pub(super) fn move_sel(&mut self, dr: i32, dc: i32, extend: bool, cx: &mut Context<Self>) {
        if self.view().kind != ViewKind::Grid || self.detail.is_some() {
            return;
        }
        self.commit_cell_edit(cx);
        let (recs, cols) = (&self.gl.recs, &self.gl.cols);
        if recs.is_empty() || cols.is_empty() {
            return;
        }
        let Some(s) = self.sel.clone() else {
            self.sel = Some(Sel { rid: recs[0].clone(), fid: cols[0].clone(), arid: recs[0].clone(), afid: cols[0].clone() });
            cx.notify();
            return;
        };
        let ri = recs.iter().position(|r| r == &s.rid).unwrap_or(0) as i32;
        let ci = cols.iter().position(|c| c == &s.fid).unwrap_or(0) as i32;
        let nr = (ri + dr).clamp(0, recs.len() as i32 - 1) as usize;
        let nc = (ci + dc).clamp(0, cols.len() as i32 - 1) as usize;
        let (rid, fid) = (recs[nr].clone(), cols[nc].clone());
        self.sel = Some(if extend { Sel { rid, fid, arid: s.arid, afid: s.afid } } else { Sel { rid: rid.clone(), fid: fid.clone(), arid: rid, afid: fid } });
        self.ensure_visible();
        cx.notify();
    }

    /// 选区覆盖的 (记录, 字段)。
    fn sel_range(&self) -> (Vec<String>, Vec<String>) {
        let Some(s) = &self.sel else { return (vec![], vec![]) };
        let (recs, cols) = (&self.gl.recs, &self.gl.cols);
        let (Some(r0), Some(r1)) = (recs.iter().position(|r| r == &s.arid), recs.iter().position(|r| r == &s.rid)) else {
            return (vec![s.rid.clone()], vec![s.fid.clone()]);
        };
        let (Some(c0), Some(c1)) = (cols.iter().position(|c| c == &s.afid), cols.iter().position(|c| c == &s.fid)) else {
            return (vec![s.rid.clone()], vec![s.fid.clone()]);
        };
        (recs[r0.min(r1)..=r0.max(r1)].to_vec(), cols[c0.min(c1)..=c0.max(c1)].to_vec())
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.view().kind != ViewKind::Grid || self.gl.recs.is_empty() {
            return;
        }
        let (recs, cols) = (&self.gl.recs, &self.gl.cols);
        self.sel = Some(Sel { arid: recs[0].clone(), afid: cols[0].clone(), rid: recs[recs.len() - 1].clone(), fid: cols[cols.len() - 1].clone() });
        cx.notify();
    }

    pub(super) fn start_cell_edit(&mut self, initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.view().kind != ViewKind::Grid {
            return;
        }
        let Some(s) = self.sel.clone() else { return };
        let Some(f) = self.field(&s.fid) else { return };
        use FieldType::*;
        if !matches!(f.kind, Text | Number | Url | Phone | Email | Currency | Progress | Rating) {
            let at = self.cell_rect(&s.rid, &s.fid).map(|b| point(b.left(), b.bottom() + px(2.))).unwrap_or(self.mouse);
            self.open_cell_editor(&s.rid, &f, at, false, window, cx);
            return;
        }
        let cur = {
            let b = self.b();
            let raw = b.record(&self.tid, &s.rid).and_then(|r| r.cells.get(&s.fid)).cloned();
            match (f.kind, raw) {
                (Progress, Some(v)) => v.as_f64().map(|n| super::formula::fmt_num(n * 100.)).unwrap_or_default(),
                (_, Some(Value::Number(n))) => super::formula::fmt_num(n.as_f64().unwrap_or(0.)),
                (_, Some(Value::String(s))) => s,
                _ => String::new(),
            }
        };
        self.inputs.remove("cell");
        let text = initial.unwrap_or(cur);
        let i = self.input("cell", "", &text, true, window, cx);
        // 光标放在末尾（直接打字进入编辑时接着输入，不能覆盖第一个字）
        i.update(cx, |i, cx| {
            i.set_text(&text, cx);
            i.collapse_to_end(cx);
        });
        self.editing = Some((s.rid, s.fid));
        cx.notify();
    }

    pub(super) fn commit_cell_edit(&mut self, cx: &mut Context<Self>) {
        let Some((rid, fid)) = self.editing.take() else { return };
        let text = self.input_text("cell", cx);
        self.inputs.remove("cell");
        let Some(mut f) = self.field(&fid) else { return };
        let v = match f.kind {
            FieldType::Text | FieldType::Url | FieldType::Phone | FieldType::Email => Value::from(text),
            FieldType::Progress => text.trim().trim_end_matches('%').parse::<f64>().ok().map(|n| Value::from(n / 100.)).unwrap_or(Value::Null),
            _ => super::io::text_to_value(&mut f, &text).unwrap_or(Value::Null),
        };
        // 选中同一列的多个格子后编辑：应用到所选（R11 批量修改）
        let (recs, cols) = self.sel_range();
        if recs.len() > 1 && cols.len() == 1 && cols[0] == fid && recs.contains(&rid) {
            let tid = self.tid.clone();
            for r in &recs {
                self.bm().set_cell(&tid, r, &fid, v.clone());
            }
            self.commit(cx);
            self.show_toast(&format!("已应用到所选的 {} 个单元格", recs.len()), cx);
            return;
        }
        self.set_cell(&rid, &fid, v, false, cx);
    }

    pub(super) fn type_to_edit(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() || self.popup.is_some() || self.detail.is_some() || self.view().kind != ViewKind::Grid {
            return;
        }
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.function {
            return;
        }
        let Some(ch) = &ev.keystroke.key_char else { return };
        if ch.is_empty() || ch.chars().all(|c| c.is_control()) || ch == " " {
            return;
        }
        let Some(s) = &self.sel else { return };
        let kind = self.field(&s.fid).map(|f| f.kind);
        if matches!(
            kind,
            Some(FieldType::Text | FieldType::Number | FieldType::Url | FieldType::Phone | FieldType::Email | FieldType::Currency | FieldType::Progress)
        ) {
            self.start_cell_edit(Some(ch.clone()), window, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        // 弹层 / 详情打开时按删除键不能误清表格选区
        if self.view().kind != ViewKind::Grid || self.editing.is_some() || self.popup.is_some() || self.detail.is_some() {
            return;
        }
        let (recs, cols) = self.sel_range();
        let tid = self.tid.clone();
        for r in &recs {
            for c in &cols {
                self.bm().set_cell(&tid, r, c, Value::Null);
            }
        }
        self.commit(cx);
    }

    pub(super) fn copy_selection(&self) {
        let (recs, cols) = self.sel_range();
        if recs.is_empty() {
            return;
        }
        let b = self.b();
        let mut tsv = String::new();
        let mut html = String::from("<table>");
        for r in &recs {
            let vals: Vec<String> = cols.iter().map(|c| b.text(&self.tid, r, c)).collect();
            html.push_str("<tr>");
            for v in &vals {
                html.push_str(&format!("<td>{}</td>", v.replace('&', "&amp;").replace('<', "&lt;")));
            }
            html.push_str("</tr>");
            tsv.push_str(&vals.iter().map(|v| v.replace(['\t', '\n'], " ")).collect::<Vec<_>>().join("\t"));
            tsv.push('\n');
        }
        html.push_str("</table>");
        crate::clipboard::write(&tsv, Some(&html), None);
    }

    /// 粘贴 TSV / HTML 表格：从光标处开始填，行不够自动新增记录。
    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        if self.view().kind != ViewKind::Grid || self.editing.is_some() || self.popup.is_some() || self.detail.is_some() {
            return;
        }
        let Some(s) = self.sel.clone() else { return };
        let clip = crate::clipboard::read();
        let rows: Vec<Vec<String>> = clip
            .html
            .as_deref()
            .and_then(|h| {
                let doc = scraper::Html::parse_document(h);
                let tr = scraper::Selector::parse("tr").ok()?;
                let td = scraper::Selector::parse("td, th").ok()?;
                let rows: Vec<Vec<String>> =
                    doc.select(&tr).map(|r| r.select(&td).map(|c| c.text().collect::<String>().trim().to_string()).collect()).collect();
                (!rows.is_empty()).then_some(rows)
            })
            .or_else(|| clip.text.map(|t| t.trim_end_matches('\n').lines().map(|l| l.split('\t').map(String::from).collect()).collect()))
            .unwrap_or_default();
        if rows.is_empty() {
            return;
        }
        let tid = self.tid.clone();
        let mut recs = self.gl.recs.clone();
        let cols = self.gl.cols.clone();
        let (Some(r0), Some(c0)) = (recs.iter().position(|r| r == &s.rid), cols.iter().position(|c| c == &s.fid)) else { return };
        let mut fields: std::collections::HashMap<String, Field> = Default::default();
        for (i, row) in rows.iter().enumerate() {
            let rid = match recs.get(r0 + i) {
                Some(r) => r.clone(),
                None => {
                    let Some(r) = self.bm().new_record(&tid, None) else { break };
                    recs.push(r.id.clone());
                    r.id
                }
            };
            for (j, text) in row.iter().enumerate() {
                let Some(fid) = cols.get(c0 + j) else { break };
                let Some(f0) = fields.get(fid).cloned().or_else(|| self.field(fid)) else { continue };
                let f = fields.entry(fid.clone()).or_insert(f0);
                if f.kind.computed() || f.kind == FieldType::Attachment {
                    continue;
                }
                let v = if f.kind.is_link() {
                    let b = self.b();
                    let ids: Vec<String> = text
                        .split([',', '，', '、'])
                        .filter_map(|n| b.table(&f.prop.table)?.order.iter().find(|r| b.primary_text(&f.prop.table, r) == n.trim()).cloned())
                        .collect();
                    Value::from(ids)
                } else {
                    super::io::text_to_value(f, text).unwrap_or(Value::Null)
                };
                self.bm().set_cell(&tid, &rid, fid, v);
            }
        }
        for f in fields.into_values() {
            if f.kind.has_options() {
                self.bm().put_field(&tid, f);
            }
        }
        self.commit(cx);
        self.show_toast(&format!("已粘贴 {} 行", rows.len()), cx);
    }

    // ───────────── 鼠标 ─────────────

    fn cell_down(&mut self, rid: &str, fid: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.commit_cell_edit(cx);
        let was = self.sel.as_ref().is_some_and(|s| s.rid == rid && s.fid == fid && s.arid == rid && s.afid == fid);
        if ev.modifiers.shift {
            if let Some(s) = self.sel.as_mut() {
                s.rid = rid.into();
                s.fid = fid.into();
                cx.notify();
                return;
            }
        }
        self.sel = Some(Sel { rid: rid.into(), fid: fid.into(), arid: rid.into(), afid: fid.into() });
        let f = self.field(fid);
        if let Some(f) = f {
            // 再次点击已选中的单元格 / 双击：打开编辑器（单选、日期等单击已选中格即弹出）
            let popup_kind = !matches!(
                f.kind,
                FieldType::Text
                    | FieldType::Number
                    | FieldType::Url
                    | FieldType::Phone
                    | FieldType::Email
                    | FieldType::Currency
                    | FieldType::Progress
                    | FieldType::Checkbox
                    | FieldType::Rating
            );
            if ev.click_count >= 2 || (was && popup_kind) {
                let at = self.cell_rect(rid, fid).map(|b| point(b.left(), b.bottom() + px(2.))).unwrap_or(ev.position);
                self.open_cell_editor(rid, &f, at, true, window, cx);
                return;
            }
        }
        self.drag = Some(Drag::Select);
        cx.notify();
    }

    pub(super) fn grid_drag_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        match &mut self.drag {
            Some(Drag::Col { start, moved, .. }) | Some(Drag::Row { start, moved, .. }) => {
                if !*moved && (pos - *start).magnitude() > 4. {
                    *moved = true;
                }
                cx.notify();
            }
            Some(Drag::ColResize { .. }) => cx.notify(),
            Some(Drag::Select) => {
                // 拖出可视区时自动滚动
                let b = self.grid_bounds;
                if pos.y > b.bottom() - px(STAT_H + 10.) {
                    self.scroll.1 += 18.;
                } else if pos.y < b.top() + px(HEAD_H + 6.) {
                    self.scroll.1 -= 18.;
                }
                self.clamp_scroll();
                cx.notify();
            }
            _ => {}
        }
    }

    /// 落点：列拖拽返回插入到第几列之前；行拖拽返回插入到第几条记录之前（按显示顺序）。
    fn drop_col_index(&self, pos: Point<Pixels>) -> usize {
        let x = f32::from(pos.x - self.grid_bounds.origin.x);
        let n = self.gl.cols.len();
        (1..n)
            .find(|&i| {
                let mid = (self.gl.x(i, self.scroll.0) + self.gl.x(i, self.scroll.0) + self.gl.col_x[i + 1] - self.gl.col_x[i]) / 2.;
                x < mid
            })
            .unwrap_or(n)
    }

    /// 开启层级时，把记录拖到另一条记录的中间 40% 区域 = 设为它的子记录。
    fn reparent_target(&self, pos: Point<Pixels>, dragging: &str) -> Option<String> {
        self.b().table(&self.tid)?.meta.parent_field.as_ref()?;
        let y = f32::from(pos.y - self.grid_bounds.origin.y) - HEAD_H + self.scroll.1;
        for (i, r) in self.gl.rows.iter().enumerate() {
            if let GRow::Rec { rid, .. } = r {
                let (y0, y1) = (self.gl.row_y[i], self.gl.row_y[i + 1]);
                let h = y1 - y0;
                if y >= y0 + h * 0.3 && y <= y1 - h * 0.3 {
                    return (rid != dragging).then(|| rid.clone());
                }
            }
        }
        None
    }

    fn drop_row_index(&self, pos: Point<Pixels>) -> Option<usize> {
        let y = f32::from(pos.y - self.grid_bounds.origin.y) - HEAD_H + self.scroll.1;
        let mut best = None;
        for (i, r) in self.gl.rows.iter().enumerate() {
            if matches!(r, GRow::Rec { .. }) {
                let mid = (self.gl.row_y[i] + self.gl.row_y[i + 1]) / 2.;
                if y < mid {
                    return Some(i);
                }
                best = Some(i + 1);
            }
        }
        best
    }

    pub(super) fn grid_drop(&mut self, d: Drag, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        match d {
            Drag::ColResize { fid, start_x, orig } => {
                let w = (orig + f32::from(pos.x - start_x)).max(60.);
                self.update_view(cx, |v| {
                    v.widths.insert(fid, w);
                });
            }
            Drag::Col { fid, moved: false, .. } => {
                let _ = window;
                self.open_popup(Popup::ColMenu(fid), pos, cx);
            }
            Drag::Col { fid, .. } => {
                let to = self.drop_col_index(pos);
                let tid = self.tid.clone();
                let v = self.view();
                let mut order = self.b().table(&tid).map(|t| t.view_fields(&v)).unwrap_or_default();
                let Some(from) = order.iter().position(|f| f == &fid) else { return };
                if from == 0 {
                    self.show_toast("主字段固定在第一列", cx);
                    return;
                }
                // 可见列下标 → 全部字段下标
                let target = self.gl.cols.get(to).cloned();
                order.remove(from);
                let at = target.and_then(|t| order.iter().position(|f| f == &t)).unwrap_or(order.len()).max(1);
                order.insert(at, fid);
                self.update_view(cx, |v| v.order = order);
            }
            Drag::Row { rid, moved: true, .. } if self.reparent_target(pos, &rid).is_some() => {
                let target = self.reparent_target(pos, &rid).unwrap_or_default();
                let tid = self.tid.clone();
                let r = self.bm().set_parent(&tid, &rid, Some(&target));
                match r {
                    Ok(()) => {
                        self.commit(cx);
                        let name = self.b().title(&tid, &target);
                        self.show_toast(&format!("已移到「{name}」下面"), cx);
                    }
                    Err(e) => self.show_toast(&e, cx),
                }
            }
            Drag::Row { rid, moved: true, .. } => {
                if !self.view().sorts.is_empty() {
                    self.show_toast("已设置排序，不能手动拖动记录", cx);
                    return;
                }
                let Some(to) = self.drop_row_index(pos) else { return };
                let rank_of = |this: &Self, i: usize| -> Option<f64> {
                    match this.gl.rows.get(i) {
                        Some(GRow::Rec { rid, .. }) => this.b().record(&this.tid, rid).map(|r| r.rank),
                        _ => None,
                    }
                };
                let prev = (0..to).rev().find(|&i| matches!(self.gl.rows.get(i), Some(GRow::Rec { rid: r, .. }) if r != &rid)).and_then(|i| rank_of(self, i));
                let next = (to..self.gl.rows.len())
                    .find(|&i| matches!(self.gl.rows.get(i), Some(GRow::Rec { rid: r, .. }) if r != &rid))
                    .and_then(|i| rank_of(self, i));
                let tid = self.tid.clone();
                let rec = self.b().record(&tid, &rid).cloned();
                if let Some(mut r) = rec {
                    r.rank = rank_between(prev, next);
                    self.bm().put_record(&tid, r);
                    self.commit(cx);
                }
            }
            _ => {}
        }
    }

    /// 双击列边线：按内容自适应列宽（取表头和前 200 行里最长的文本）。
    pub(super) fn autofit_col(&mut self, fid: &str, cx: &mut Context<Self>) {
        let rows = self.rows();
        let est = |s: &str| s.chars().map(|c| if c.is_ascii() { 7.6 } else { 13.5 }).sum::<f32>();
        let w = {
            let b = self.b();
            let name = b.field(&self.tid, fid).map(|f| f.name.clone()).unwrap_or_default();
            let mut w = est(&name) + 60.;
            for r in rows.iter().take(200) {
                w = w.max(est(&b.text(&self.tid, r, fid)) + 28.);
            }
            w.clamp(80., 600.)
        };
        let f = fid.to_string();
        self.update_view(cx, move |v| {
            v.widths.insert(f, w);
        });
    }

    /// 在 `parent` 下新建一条子记录：父记录折叠时先展开，新记录选中主字段。超出层级上限时撤回并提示。
    pub(super) fn add_child_record(&mut self, parent: &str, cx: &mut Context<Self>) {
        let tid = self.tid.clone();
        let Some(r) = self.bm().new_record(&tid, None) else { return };
        let res = self.bm().set_parent(&tid, &r.id, Some(parent));
        match res {
            Ok(()) => {
                let k = format!("t:{parent}");
                let mut v = self.view();
                v.collapsed.retain(|x| x != &k);
                self.bm().put_view(&tid, v);
                self.commit(cx);
                let fid = self.gl.cols.first().cloned().unwrap_or_default();
                self.sel = Some(super::editor::Sel { rid: r.id.clone(), fid: fid.clone(), arid: r.id, afid: fid });
            }
            Err(e) => {
                self.bm().del_record(&tid, &r.id);
                self.commit(cx);
                self.show_toast(&e, cx);
            }
        }
    }

    /// 开启层级时各记录的直接子记录数（只统计父记录仍存在的）。
    fn child_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut out = std::collections::HashMap::new();
        let b = self.b();
        let Some(t) = b.table(&self.tid) else { return out };
        if t.meta.parent_field.is_none() {
            return out;
        }
        for rid in t.records.keys() {
            if let Some(p) = b.parent_of(&self.tid, rid) {
                *out.entry(p).or_insert(0) += 1;
            }
        }
        out
    }

    fn header_cell(&mut self, f: &Field, i: usize, w: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (fid, fid2, fid3) = (f.id.clone(), f.id.clone(), f.id.clone());
        let v = self.view();
        let sorted = v.sorts.iter().find(|s| s.field == f.id).map(|s| s.desc);
        let filtered = v.filter.conds.iter().any(|c| c.field == f.id);
        div()
            .id(SharedString::from(format!("hd-{}", f.id)))
            .group("hd")
            .relative()
            .w(px(w))
            .h_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .border_r_1()
            .border_b_1()
            .border_color(t.border)
            .bg(t.table_header)
            .text_size(px(13.))
            .text_color(t.text_secondary)
            .hover(|s| s.bg(t.hover))
            .cursor_pointer()
            .child(icon(f.kind.icon(), 14., t.text_muted))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(f.name.clone()))
            .when(filtered, |d| d.child(icon("filter", 12., t.accent)))
            .when_some(sorted, |d, desc| d.child(icon(if desc { "arrow-down" } else { "arrow-up" }, 12., t.accent)))
            .when(!f.desc.is_empty(), |d| d.child(icon("info", 12., t.text_muted)))
            .child(div().invisible().group_hover("hd", |s| s.visible()).child(icon("chevron-down", 13., t.text_muted)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, w, cx| {
                    w.focus(&this.focus);
                    this.commit_cell_edit(cx);
                    this.drag = Some(Drag::Col { fid: fid.clone(), start: ev.position, moved: false });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::ColMenu(fid2.clone()), ev.position, cx)),
            )
            // 右边缘：拖动调整列宽
            .child(
                div()
                    .id(SharedString::from(format!("rs-{}", f.id)))
                    .absolute()
                    .right(px(-3.))
                    .top_0()
                    .bottom_0()
                    .w(px(7.))
                    .cursor_col_resize()
                    .hover(|s| s.bg(t.accent.opacity(0.5)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if ev.click_count >= 2 {
                                this.drag = None;
                                this.autofit_col(&fid3, cx);
                                return;
                            }
                            this.drag = Some(Drag::ColResize { fid: fid3.clone(), start_x: ev.position.x, orig: w });
                        }),
                    ),
            )
            .when(i == 0, |d| d.font_weight(FontWeight::MEDIUM))
            .into_any_element()
    }

    pub(super) fn render_grid(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.view();
        self.layout_grid(&v);
        self.clamp_scroll();
        let gl = self.gl.clone();
        let (sx, sy) = self.scroll;
        let vw = f32::from(self.grid_bounds.size.width).max(400.);
        let vh = f32::from(self.grid_bounds.size.height).max(300.);
        let fields: Vec<Field> = gl.cols.iter().filter_map(|c| self.field(c)).collect();
        let fw = gl.frozen_w();
        let rh = v.row_height();
        let tall = v.row_h > 0;
        // 可见行 / 列范围
        let r0 = gl.row_y.partition_point(|&y| y <= sy).saturating_sub(1);
        let r1 = gl.row_y.partition_point(|&y| y < sy + vh).min(gl.rows.len());
        let c_lo = (gl.frozen..fields.len()).find(|&i| gl.col_x[i + 1] - sx > fw).unwrap_or(fields.len());
        let c_hi = (gl.frozen..fields.len()).rfind(|&i| gl.col_x[i] - sx < vw).map(|i| i + 1).unwrap_or(gl.frozen);
        let (sel_recs, sel_cols) = self.sel_range();
        let cur = self.sel.clone();
        let editing = self.editing.clone();
        let colors = self.colors();
        let checked = self.checked.clone();
        let any_checked = !checked.is_empty();
        let comments = self.comment_counts.clone();
        let collapsed_tree = v.collapsed.clone();
        let hier = self.b().table(&self.tid).is_some_and(|t| t.meta.parent_field.is_some());
        let child_counts = self.child_counts();
        let entity = cx.entity();
        let mut layer = div().absolute().inset_0();
        for i in r0..r1 {
            let y = HEAD_H + gl.row_y[i] - sy;
            let h = gl.row_y[i + 1] - gl.row_y[i];
            match gl.rows[i].clone() {
                GRow::Group { depth, path, label, color, count, fid, collapsed, rids } => {
                    layer =
                        self.render_grid_group(t, cx, collapsed, color, count, depth, &fid, &fields, h, label.clone(), layer, &path, &rids, v.clone(), vw, y);
                }
                GRow::Add { depth, preset } => {
                    layer = self.render_grid_add(t, cx, depth, &gl, h, i, layer, preset.clone(), vw, y);
                }
                GRow::Rec { rid, depth, n, tdepth, kids, matched } => {
                    let selected_row = sel_recs.contains(&rid) || checked.contains(&rid);
                    let row_bg = if selected_row { t.accent_soft } else { t.bg };
                    let tree_open = !collapsed_tree.contains(&format!("t:{rid}"));
                    let mk_cell = |this: &mut Self, ci: usize, cx: &mut Context<Self>| -> AnyElement {
                        let f = &fields[ci];
                        let w = gl.col_x[ci + 1] - gl.col_x[ci];
                        let content = this.cell_content(&rid, f, Mode::Grid, t, cx);
                        let in_sel = selected_row && sel_cols.contains(&f.id);
                        let right = this.right_align(f);
                        let (r1, f1) = (rid.clone(), f.id.clone());
                        let (r2, f2) = (rid.clone(), f.id.clone());
                        let fill = colors.at(&rid, &f.id).map(|c| tag_color(c, t.dark).0.opacity(if t.dark { 0.55 } else { 0.85 }));
                        let r_tree = rid.clone();
                        let caret = (ci == 0 && (kids || tdepth > 0)).then(|| {
                            div()
                                .id(SharedString::from(format!("tc-{rid}")))
                                .size(px(16.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .mr(px(4.))
                                .rounded(px(3.))
                                .cursor_pointer()
                                .hover(|s| s.bg(t.hover))
                                .when(kids, |d| d.child(icon(if tree_open { "chevron-down" } else { "chevron-right" }, 12., t.text_secondary)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        let k = format!("t:{r_tree}");
                                        // 折叠状态是界面状态：直接写视图，不进撤销
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
                                    }),
                                )
                        });
                        // 主字段右侧：悬停显示「+ 添加子记录」，有子记录时常驻显示子记录数
                        let kid_n = child_counts.get(&rid).copied().unwrap_or(0);
                        let tree_tools = (ci == 0 && hier && matched).then(|| {
                            let r_add = rid.clone();
                            div()
                                .absolute()
                                .top_0()
                                .right(px(4.))
                                .h_full()
                                .flex()
                                .flex_row()
                                .when(tall, |d| d.items_start().pt(px(6.)))
                                .when(!tall, |d| d.items_center())
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(2.))
                                        .pl(px(4.))
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("addkid-{rid}")))
                                                .size(px(20.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .invisible()
                                                .group_hover("grow", |s| s.visible())
                                                .cursor_pointer()
                                                .hover(|s| s.bg(t.hover))
                                                .child(icon("plus", 13., t.text_secondary))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _, _, cx| {
                                                        cx.stop_propagation();
                                                        this.add_child_record(&r_add, cx);
                                                    }),
                                                ),
                                        )
                                        .when(kid_n > 0, |d| {
                                            d.child(div().w(px(1.)).h(px(12.)).bg(t.border).invisible().group_hover("grow", |s| s.visible())).child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap(px(2.))
                                                    .px(px(4.))
                                                    .text_size(px(12.))
                                                    .text_color(t.text_muted)
                                                    .child(icon("list-tree", 12., t.text_muted))
                                                    .child(kid_n.to_string()),
                                            )
                                        }),
                                )
                        });
                        div()
                            .w(px(w))
                            .h_full()
                            .flex_none()
                            .px(px(8.))
                            .when(ci == 0, |d| d.relative().pl(px(8. + depth as f32 * 20. + tdepth as f32 * 18.)))
                            // 给右侧的「+ / 子记录数」留位置，长文字在它前面截断
                            .when(tree_tools.is_some(), |d| d.pr(px(if kid_n > 0 { 64. } else { 30. })))
                            .when_some(fill, |d, c| d.bg(c))
                            .when(!matched, |d| d.opacity(0.55))
                            .children(caret)
                            .flex()
                            .when(tall, |d| d.items_start().pt(px(8.)))
                            .when(!tall, |d| d.items_center())
                            .when(right, |d| d.justify_end())
                            .overflow_hidden()
                            .border_r_1()
                            .border_b_1()
                            .border_color(t.border)
                            .text_size(px(13.))
                            .when(in_sel, |d| d.bg(t.accent.opacity(0.08)))
                            .child(content)
                            .children(tree_tools)
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, w, cx| this.cell_down(&r1, &f1, ev, w, cx)))
                            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                                if matches!(this.drag, Some(Drag::Select)) && ev.pressed_button == Some(MouseButton::Left) {
                                    if let Some(s) = this.sel.as_mut() {
                                        if s.rid != r2 || s.fid != f2 {
                                            s.rid = r2.clone();
                                            s.fid = f2.clone();
                                            cx.notify();
                                        }
                                    }
                                }
                            }))
                            .into_any_element()
                    };
                    let mut scrolled = div().absolute().top_0().h_full().left(px(gl.col_x.get(c_lo).copied().unwrap_or(fw) - sx)).flex().flex_row();
                    for ci in c_lo..c_hi {
                        scrolled = scrolled.child(mk_cell(self, ci, cx));
                    }
                    let (r3, r4, r5) = (rid.clone(), rid.clone(), rid.clone());
                    let idx = div()
                        .relative()
                        .w(px(IDX_W))
                        .h_full()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .px(px(4.))
                        .border_r_1()
                        .border_b_1()
                        .border_color(t.border)
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .child(
                            div()
                                .id(SharedString::from(format!("grip-{rid}")))
                                .size(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .invisible()
                                .group_hover("grow", |s| s.visible())
                                .cursor_grab()
                                .child(icon("grip-v", 12., t.text_muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.drag = Some(Drag::Row { rid: r3.clone(), start: ev.position, moved: false });
                                    }),
                                ),
                        )
                        .child({
                            let on = checked.contains(&rid);
                            let r6 = rid.clone();
                            div()
                                .id(SharedString::from(format!("ck-{rid}")))
                                .flex_1()
                                .flex()
                                .justify_center()
                                .cursor_pointer()
                                .child(if on || any_checked {
                                    check_box(on, t).into_any_element()
                                } else {
                                    div().group_hover("grow", |s| s.invisible()).child(n.to_string()).into_any_element()
                                })
                                .when(!on && !any_checked, |d| {
                                    d.child(div().absolute().invisible().group_hover("grow", |s| s.visible()).child(check_box(false, t)))
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !this.checked.remove(&r6) {
                                            this.checked.insert(r6.clone());
                                        }
                                        cx.notify();
                                    }),
                                )
                        })
                        .when_some(comments.get(&rid).copied(), |d, n| {
                            d.child(
                                div()
                                    .absolute()
                                    .right(px(2.))
                                    .top(px(2.))
                                    .px(px(3.))
                                    .rounded(px(6.))
                                    .bg(t.accent)
                                    .text_color(white())
                                    .text_size(px(9.))
                                    .child(n.to_string()),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("exp-{rid}")))
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .invisible()
                                .group_hover("grow", |s| s.visible())
                                .hover(|s| s.bg(t.hover))
                                .cursor_pointer()
                                .child(icon("expand-2", 12., t.accent))
                                .tooltip(tip("展开记录（空格）"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.detail = Some(r4.clone());
                                        cx.notify();
                                    }),
                                ),
                        );
                    let mut frozen =
                        div().absolute().left_0().top_0().h_full().flex().flex_row().bg(row_bg).group_hover("grow", |s| s.bg(t.sidebar_bg)).child(idx);
                    for ci in 0..gl.frozen {
                        frozen = frozen.child(mk_cell(self, ci, cx));
                    }
                    layer = layer.child(
                        div()
                            .id(SharedString::from(format!("row-{rid}")))
                            .group("grow")
                            .absolute()
                            .top(px(y))
                            .left_0()
                            .w(px(vw))
                            .h(px(h))
                            .bg(row_bg)
                            .hover(|s| s.bg(t.sidebar_bg))
                            .child(scrolled)
                            .child(frozen.when(sx > 0., |d| d.shadow_md()))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::RowMenu(r5.clone()), ev.position, cx)),
                            ),
                    );
                }
            }
        }
        let _ = rh;
        // 表头
        let mut head_scrolled = div().absolute().top_0().h_full().left(px(gl.col_x.get(c_lo).copied().unwrap_or(fw) - sx)).flex().flex_row();
        for ci in c_lo..c_hi {
            let w = gl.col_x[ci + 1] - gl.col_x[ci];
            head_scrolled = head_scrolled.child(self.header_cell(&fields[ci], ci, w, t, cx));
        }
        let plus_x = gl.col_x.last().copied().unwrap_or(IDX_W) - if fields.len() > gl.frozen { sx } else { 0. };
        let head_plus = div()
            .id("hd-plus")
            .absolute()
            .top_0()
            .left(px(plus_x))
            .w(px(PLUS_W))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_r_1()
            .border_color(t.border)
            .bg(t.table_header)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(icon("plus", 15., t.text_secondary))
            .tooltip(tip("新增字段"))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, w, cx| this.edit_field(None, None, ev.position, w, cx)));
        let mut head_frozen = div().absolute().left_0().top_0().h_full().flex().flex_row().bg(t.table_header).child(
            div()
                .w(px(IDX_W))
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_b_1()
                .border_color(t.border)
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child("#"),
        );
        for ci in 0..gl.frozen.min(fields.len()) {
            let w = gl.col_x[ci + 1] - gl.col_x[ci];
            head_frozen = head_frozen.child(self.header_cell(&fields[ci], ci, w, t, cx));
        }
        let header = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(vw))
            .h(px(HEAD_H))
            .bg(t.table_header)
            .child(head_scrolled)
            .child(head_plus)
            .child(head_frozen.when(sx > 0., |d| d.shadow_md()));
        // 选区 / 光标框
        let mut overlay = div().absolute().inset_0();
        if let Some(s) = &cur {
            let rect = |rid: &str, fid: &str| -> Option<(f32, f32, f32, f32)> {
                let ri = gl.rec_index(rid)?;
                let ci = gl.cols.iter().position(|c| c == fid)?;
                Some((gl.x(ci, sx), HEAD_H + gl.row_y[ri] - sy, gl.col_x[ci + 1] - gl.col_x[ci], gl.row_y[ri + 1] - gl.row_y[ri]))
            };
            if let Some((x, y, w, h)) = rect(&s.rid, &s.fid) {
                if y + h > HEAD_H && editing.is_none() {
                    overlay = overlay.child(div().absolute().left(px(x - 1.)).top(px(y - 1.)).w(px(w + 1.)).h(px(h + 1.)).border_2().border_color(t.accent));
                }
            }
        }
        // 原地编辑框
        let edit_overlay = editing.as_ref().and_then(|(rid, fid)| {
            let ri = gl.rec_index(rid)?;
            let ci = gl.cols.iter().position(|c| c == fid)?;
            let (x, y, w, h) = (gl.x(ci, sx), HEAD_H + gl.row_y[ri] - sy, gl.col_x[ci + 1] - gl.col_x[ci], (gl.row_y[ri + 1] - gl.row_y[ri]).max(34.));
            let input = self.inputs.get("cell")?.0.clone();
            Some(
                div()
                    .absolute()
                    .left(px(x - 1.))
                    .top(px(y - 1.))
                    .min_w(px(w + 2.))
                    .h(px(h + 2.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .bg(t.panel_bg)
                    .border_2()
                    .border_color(t.accent)
                    .shadow_md()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .occlude()
                    .child(input),
            )
        });
        // 拖拽落点指示
        let indicator = match &self.drag {
            Some(Drag::Col { moved: true, .. }) => {
                let i = self.drop_col_index(self.mouse);
                let x = if i < gl.cols.len() { gl.x(i, sx) } else { gl.col_x.last().copied().unwrap_or(0.) - sx };
                Some(div().absolute().top_0().bottom_0().left(px(x - 1.)).w(px(2.)).bg(t.accent))
            }
            Some(Drag::Row { moved: true, .. }) => self.drop_row_index(self.mouse).map(|i| {
                let y = HEAD_H + gl.row_y[i.min(gl.row_y.len() - 1)] - sy;
                div().absolute().left_0().right_0().top(px(y - 1.)).h(px(2.)).bg(t.accent)
            }),
            _ => None,
        };
        // 底部统计栏
        let rows = self.gl.recs.clone();
        let mut stat_scrolled = div().absolute().top_0().h_full().left(px(gl.col_x.get(c_lo).copied().unwrap_or(fw) - sx)).flex().flex_row();
        let stat_cell = |this: &Self, ci: usize, cx: &mut Context<Self>| {
            let f = &fields[ci];
            let w = gl.col_x[ci + 1] - gl.col_x[ci];
            let kind = v.stats.get(&f.id).cloned().unwrap_or_default();
            let text = if kind.is_empty() { String::new() } else { this.b().stat(&this.tid, &rows, &f.id, &kind) };
            let fid = f.id.clone();
            div()
                .id(SharedString::from(format!("st-{}", f.id)))
                .group("st")
                .w(px(w))
                .h_full()
                .flex_none()
                .px(px(8.))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .when(!text.is_empty(), |d| d.child(text))
                .when(kind.is_empty(), |d| {
                    d.child(
                        div()
                            .invisible()
                            .group_hover("st", |s| s.visible())
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .text_color(t.text_muted)
                            .child("统计")
                            .child(icon("chevron-down", 12., t.text_muted)),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| this.open_popup(Popup::Stat(fid.clone()), ev.position, cx)),
                )
        };
        for ci in c_lo..c_hi {
            stat_scrolled = stat_scrolled.child(stat_cell(self, ci, cx));
        }
        let mut stat_frozen = div().absolute().left_0().top_0().h_full().flex().flex_row().bg(t.bg).child(div().w(px(IDX_W)).h_full().flex_none());
        for ci in 0..gl.frozen.min(fields.len()) {
            stat_frozen = stat_frozen.child(stat_cell(self, ci, cx));
        }
        let stats =
            div().absolute().left_0().bottom_0().w(px(vw)).h(px(STAT_H)).bg(t.bg).border_t_1().border_color(t.border).child(stat_scrolled).child(stat_frozen);
        let empty = gl.recs.is_empty().then(|| {
            div()
                .absolute()
                .top(px(HEAD_H + gl.total_h() + 40.))
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .text_size(px(13.))
                .text_color(t.text_muted)
                .child(if self.search.is_empty() && v.filter.conds.is_empty() {
                    "还没有记录，点「新增记录」或直接粘贴 Excel 数据"
                } else {
                    "没有符合条件的记录"
                })
                .when(!v.filter.conds.is_empty() || !self.search.is_empty(), |d| {
                    d.child(ghost_btn("empty-clear", "清除筛选和搜索", t).on_click(cx.listener(|this, _, _, cx| {
                        this.search.clear();
                        this.search_open = false;
                        this.inputs.remove("search");
                        this.update_view(cx, |v| v.filter.conds.clear());
                    })))
                })
        });
        let _ = window;
        let band = self.group_band(&gl, &fields, sx, fw, vw, t);
        let grid = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(
                canvas(
                    move |b, _, cx| {
                        entity.update(cx, |this, cx| {
                            if this.grid_bounds != b {
                                this.grid_bounds = b;
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(layer)
            .children(empty)
            .child(overlay)
            .child(header)
            .children(indicator)
            .child(stats)
            .children(edit_overlay)
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let d = ev.delta.pixel_delta(px(30.));
                let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                // 按住 Shift 滚轮横向滚动
                let before = this.scroll;
                if ev.modifiers.shift && dx == 0. {
                    this.scroll.0 -= dy;
                } else {
                    this.scroll.0 -= dx;
                    this.scroll.1 -= dy;
                }
                this.clamp_scroll();
                // 文档内嵌时：表格自己能滚就不交给文档，滚到头了再让文档滚
                if this.scroll != before {
                    cx.stop_propagation();
                }
                cx.notify();
            }));
        match band {
            Some(b) => div().size_full().flex().flex_col().child(b).child(div().flex_1().min_h(px(0.)).child(grid)).into_any_element(),
            None => grid.into_any_element(),
        }
    }
    /// 从 `render_grid` 拆出。
    fn render_grid_group(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        collapsed: bool,
        color: Option<u8>,
        count: usize,
        depth: usize,
        fid: &str,
        fields: &[Field],
        h: f32,
        label: String,
        mut layer: Div,
        path: &String,
        rids: &[String],
        v: View,
        vw: f32,
        y: f32,
    ) -> Div {
        let fname = self.field(fid).map(|f| f.name).unwrap_or_default();
        let p = path.clone();
        // 分组小计：按底部统计栏的配置，算这一组的记录
        let subtotals: Vec<String> = fields
            .iter()
            .filter_map(|f| {
                let k = v.stats.get(&f.id).filter(|k| !k.is_empty() && k.as_str() != "count")?;
                let s = self.b().stat(&self.tid, rids, &f.id, k);
                (!s.is_empty()).then(|| format!("{}：{s}", f.name))
            })
            .collect();
        layer = layer.child(
            div()
                .id(SharedString::from(format!("g-{path}")))
                .absolute()
                .top(px(y))
                .left_0()
                .w(px(vw))
                .h(px(h))
                .pl(px(12. + depth as f32 * 20.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .bg(t.sidebar_bg)
                .border_b_1()
                .border_color(t.border)
                .cursor_pointer()
                .child(icon(if collapsed { "chevron-right" } else { "chevron-down" }, 14., t.text_secondary))
                .child(div().text_size(px(12.)).text_color(t.text_muted).child(fname))
                .child(match color {
                    Some(c) => tag(label, c, t).into_any_element(),
                    None => div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(label).into_any_element(),
                })
                .child(div().text_size(px(12.)).text_color(t.text_muted).child(count.to_string()))
                .when(!subtotals.is_empty(), |d| d.child(div().ml(px(16.)).text_size(px(12.)).text_color(t.text_secondary).child(subtotals.join("　"))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let p = p.clone();
                    this.update_view(cx, |v| {
                        if v.collapsed.contains(&p) {
                            v.collapsed.retain(|x| x != &p);
                        } else {
                            v.collapsed.push(p);
                        }
                    })
                })),
        );
        layer
    }

    /// 从 `render_grid` 拆出。
    fn render_grid_add(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        depth: usize,
        gl: &GridLayout,
        h: f32,
        i: usize,
        mut layer: Div,
        preset: Vec<(String, Value)>,
        vw: f32,
        y: f32,
    ) -> Div {
        layer = layer.child(
            div()
                .id(("add-row", i))
                .absolute()
                .top(px(y))
                .left_0()
                .w(px((gl.total_w() - PLUS_W).min(vw)))
                .h(px(h))
                .pl(px(16. + depth as f32 * 20.))
                .flex()
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(t.border)
                .text_size(px(13.))
                .text_color(t.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover).text_color(t.text_secondary))
                .child(icon("plus", 14., t.text_muted))
                .child("新增记录")
                .on_click(cx.listener(move |this, _, w, cx| {
                    if let Some(rid) = this.add_record(preset.clone(), None, cx) {
                        let fid = this.gl.cols.first().cloned().unwrap_or_default();
                        this.sel = Some(Sel { rid: rid.clone(), fid: fid.clone(), arid: rid, afid: fid });
                        w.focus(&this.focus);
                    }
                })),
        );
        layer
    }

    /// 字段编组：表头上方一行编组名（跨过组内的列）。
    fn group_band(&self, gl: &GridLayout, fields: &[Field], sx: f32, fw: f32, vw: f32, t: &Theme) -> Option<AnyElement> {
        let groups = self.b().table(&self.tid).map(|t| t.meta.groups.clone()).unwrap_or_default();
        if groups.iter().all(|g| g.fields.is_empty()) {
            return None;
        }
        let mut band = div().relative().w(px(vw)).h(px(BAND_H)).flex_none().bg(t.table_header).border_b_1().border_color(t.border).overflow_hidden();
        for g in &groups {
            let idx: Vec<usize> = fields.iter().enumerate().filter(|(_, f)| g.fields.contains(&f.id)).map(|(i, _)| i).collect();
            let (Some(&a), Some(&b)) = (idx.first(), idx.last()) else { continue };
            let x0 = gl.x(a, sx).max(if a >= gl.frozen { fw } else { 0. });
            let x1 = gl.x(b, sx) + gl.col_x[b + 1] - gl.col_x[b];
            if x1 <= x0 {
                continue;
            }
            band = band.child(
                div()
                    .absolute()
                    .left(px(x0))
                    .top_0()
                    .h(px(BAND_H))
                    .w(px(x1 - x0))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .border_r_1()
                    .border_color(t.border)
                    .bg(t.accent_soft)
                    .text_size(px(12.))
                    .text_color(t.accent)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(g.name.clone()),
            );
        }
        Some(band.into_any_element())
    }
}
