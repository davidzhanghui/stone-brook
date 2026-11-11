//! 草图命令：撤销、删除、剪贴板、成组、层级、对齐、属性修改、文字编辑、导入导出、图片。

use super::geom::{self, v, R, V};
use super::model::Element;
use super::model::*;
use super::view::*;
use crate::canvas::math;
use crate::text_area::{TextArea, TextAreaEvent, TextStyle};
use gpui::prelude::FluentBuilder;
use gpui::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

impl SketchEditor {
    // ───────────── 撤销 ─────────────

    pub(super) fn do_undo(&mut self, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(d) = self.undo.pop_back() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.sel.retain(|id| self.doc.get(id).is_some());
            self.multi = None;
            self.commit(cx);
        }
    }

    pub(super) fn do_redo(&mut self, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        if let Some(d) = self.redo.pop() {
            self.undo.push_back(std::mem::replace(&mut self.doc, d));
            self.sel.retain(|id| self.doc.get(id).is_some());
            self.commit(cx);
        }
    }

    // ───────────── 选择 / 删除 ─────────────

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        self.sel = self.doc.live().filter(|e| !e.locked && e.container_id().is_none_or(|c| self.doc.get(c).is_none())).map(|e| e.id.clone()).collect();
        self.tool = Tool::Select;
        cx.notify();
    }

    pub(super) fn delete_sel(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        // 线编辑模式：删选中的节点（至少留两个）
        if let (Some(id), Some(i)) = (self.line_edit.clone(), self.line_pt) {
            if self.doc.get(&id).is_some_and(|e| e.pts().len() > 2 && i < e.pts().len()) {
                self.snapshot();
                if let Some(e) = self.doc.get_mut(&id) {
                    let mut pts = geom::abs_points(e);
                    pts.remove(i);
                    e.x = pts[0].x;
                    e.y = pts[0].y;
                    e.points = Some(pts.iter().map(|q| [q.x - pts[0].x, q.y - pts[0].y]).collect());
                    e.normalize_points();
                    e.bump();
                }
                self.line_pt = None;
                geom::update_bound_arrows(&mut self.doc, &[]);
                geom::layout_bound_text(&mut self.doc, &id);
                self.commit(cx);
            }
            return;
        }
        self.snapshot();
        let ids: Vec<String> = self.sel.iter().filter(|id| self.doc.get(id).is_some_and(|e| !e.locked)).cloned().collect();
        self.delete_ids(&ids);
        self.sel.clear();
        self.commit(cx);
    }

    /// 删除元素，并清理引用：绑定文字一起删；指向它的箭头解绑；框架删掉后子元素脱离。
    pub(super) fn delete_ids(&mut self, ids: &[String]) {
        let mut all: HashSet<String> = ids.iter().cloned().collect();
        for e in self.doc.live() {
            if e.container_id().is_some_and(|c| all.contains(c)) {
                all.insert(e.id.clone());
            }
        }
        self.doc.elements.retain(|e| !all.contains(&e.id));
        for e in self.doc.elements.iter_mut() {
            let mut changed = false;
            for s in [true, false] {
                if e.binding(s).is_some_and(|b| all.contains(&b.element_id)) {
                    e.set_binding(s, None);
                    changed = true;
                }
            }
            for id in &all {
                let n = e.bound_elements().len();
                e.remove_bound(id);
                changed |= n != e.bound_elements().len();
            }
            if e.frame_id().is_some_and(|f| all.contains(f)) {
                e.set("frameId", Value::Null);
                changed = true;
            }
            if e.container_id().is_some_and(|c| all.contains(c)) {
                e.set("containerId", Value::Null);
                changed = true;
            }
            if changed {
                e.bump();
            }
        }
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mermaid.is_some() || self.prompt.is_some() || self.eyedropper.is_some() {
            self.mermaid = None;
            self.prompt = None;
            self.eyedropper = None;
            window.focus(&self.focus);
        } else if self.line_edit.is_some() || self.crop_edit.is_some() {
            self.line_edit = None;
            self.line_pt = None;
            self.crop_edit = None;
        } else if self.editing.is_some() {
            self.finish_edit(cx);
            window.focus(&self.focus);
        } else if let Some(id) = self.multi.clone() {
            self.finish_linear(&id, cx);
        } else if self.menu.is_some() || self.popover.is_some() {
            self.menu = None;
            self.popover = None;
        } else if self.tool != Tool::Select {
            self.tool = Tool::Select;
        } else if !self.sel.is_empty() {
            self.sel.clear();
        } else {
            self.close(cx);
            return;
        }
        cx.notify();
    }

    pub(super) fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.multi.clone() {
            self.finish_linear(&id, cx);
            return;
        }
        let Some(e) = self.single().cloned() else { return };
        if e.locked {
            return;
        }
        if e.is_text() {
            self.start_text_edit(&e.id, window, cx);
        } else if e.is_container() || e.is_linear() {
            self.edit_bound_text(&e.id, window, cx);
        } else if e.is_frame() {
            self.start_frame_name_edit(&e.id, window, cx);
        }
    }

    pub(super) fn nudge(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        let ids = self.move_set();
        for id in &ids {
            if let Some(e) = self.doc.get_mut(id).filter(|e| !e.locked) {
                e.x += dx;
                e.y += dy;
                e.bump();
            }
        }
        geom::update_bound_arrows(&mut self.doc, &ids);
        self.commit(cx);
    }

    // ───────────── 复制粘贴 ─────────────

    /// 复制一组元素：换新 id，并把内部引用（容器、绑定、分组、框架）一起改掉。
    pub(super) fn clone_elements(src: &[Element], dx: f64, dy: f64) -> Vec<Element> {
        let map: HashMap<String, String> = src.iter().map(|e| (e.id.clone(), new_id())).collect();
        let mut gmap: HashMap<String, String> = HashMap::new();
        src.iter()
            .map(|e| {
                let mut c = e.clone();
                c.id = map[&e.id].clone();
                c.x += dx;
                c.y += dy;
                c.seed = rand_i32() as i64;
                c.version = 1;
                c.bump();
                c.locked = false;
                c.group_ids = e.group_ids.iter().map(|g| gmap.entry(g.clone()).or_insert_with(new_id).clone()).collect();
                let remap = |v: Option<&str>| v.and_then(|s| map.get(s).cloned());
                match remap(e.container_id()) {
                    Some(n) => c.set("containerId", json!(n)),
                    None if e.container_id().is_some() => c.set("containerId", Value::Null),
                    None => {}
                }
                if e.frame_id().is_some() {
                    c.set("frameId", remap(e.frame_id()).map(Value::String).unwrap_or(Value::Null));
                }
                for s in [true, false] {
                    if let Some(b) = e.binding(s) {
                        match map.get(&b.element_id) {
                            Some(n) => {
                                let k = if s { "startBinding" } else { "endBinding" };
                                let mut v = e.extra.get(k).cloned().unwrap_or(Value::Null);
                                v["elementId"] = json!(n);
                                c.set(k, v);
                            }
                            None => c.set_binding(s, None),
                        }
                    }
                }
                let bound: Vec<Value> = e.bound_elements().into_iter().filter_map(|(id, t)| map.get(&id).map(|n| json!({ "id": n, "type": t }))).collect();
                if e.get("boundElements").is_some() {
                    c.set("boundElements", if bound.is_empty() { Value::Null } else { Value::Array(bound) });
                }
                c.extra.remove("index");
                c
            })
            .collect()
    }

    pub(super) fn duplicate_offset(&mut self, dx: f64, dy: f64) {
        let ids = self.sel_with_bound();
        let src: Vec<Element> = self.doc.live().filter(|e| ids.contains(&e.id)).cloned().collect();
        let copies = Self::clone_elements(&src, dx, dy);
        let top: HashSet<String> = copies.iter().filter(|c| c.container_id().is_none_or(|p| !copies.iter().any(|x| x.id == p))).map(|c| c.id.clone()).collect();
        self.doc.elements.extend(copies);
        self.sel = top;
    }

    pub(super) fn duplicate(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        self.duplicate_offset(10., 10.);
        self.commit(cx);
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        let ids = self.sel_with_bound();
        let json = super::io::to_clipboard(&self.doc, &ids);
        crate::clipboard::write(&json, None, None);
        self.show_toast(format!("已复制 {} 个元素（可粘贴到 excalidraw.com）", self.sel.len()), cx);
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        self.copy(cx);
        self.delete_sel(cx);
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let c = crate::clipboard::read();
        let at = self.mouse_world;
        if let Some(text) = c.text.as_deref() {
            if let Some(mut d) = super::io::from_clipboard(text) {
                super::io::externalize_files(&mut d, &assets_dir());
                self.insert_doc(d, at, cx);
                return;
            }
        }
        if let Some((bytes, ext)) = c.image {
            if let Some(path) = crate::convert::save_image_bytes(&bytes, ext, &assets_dir()) {
                self.insert_image(path, Some(at), cx);
                return;
            }
        }
        if let Some(text) = c.text.filter(|t| !t.trim().is_empty()) {
            self.snapshot();
            let mut e = Element::new("text", at.x, at.y);
            self.apply_cur(&mut e);
            e.text = Some(text.clone());
            e.set("originalText", json!(text));
            geom::fit_text(&mut e);
            self.sel = [e.id.clone()].into_iter().collect();
            self.doc.elements.push(e);
            self.commit(cx);
        }
    }

    /// 把另一份草图（剪贴板 / 文件）的元素插到 at 附近。
    pub(super) fn insert_doc(&mut self, d: SketchDoc, at: V, cx: &mut Context<Self>) {
        let src: Vec<Element> = d.live().cloned().collect();
        if src.is_empty() {
            return;
        }
        self.snapshot();
        let r = geom::bounds_of(src.iter()).unwrap_or_default();
        let c = r.center();
        let copies = Self::clone_elements(&src, at.x - c.x, at.y - c.y);
        for (k, f) in d.files {
            self.doc.files.entry(k).or_insert(f);
        }
        self.sel = copies.iter().filter(|e| e.container_id().is_none()).map(|e| e.id.clone()).collect();
        self.doc.elements.extend(copies);
        self.tool = Tool::Select;
        self.commit(cx);
    }

    // ───────────── 分组 / 锁定 / 层级 ─────────────

    pub(super) fn group(&mut self, cx: &mut Context<Self>) {
        if self.sel.len() < 2 {
            return;
        }
        self.snapshot();
        let g = new_id();
        let ids = self.sel_with_bound();
        for e in self.doc.elements.iter_mut().filter(|e| ids.contains(&e.id)) {
            e.group_ids.push(g.clone());
            e.bump();
        }
        self.commit(cx);
    }

    pub(super) fn ungroup(&mut self, cx: &mut Context<Self>) {
        let ids = self.sel_with_bound();
        let groups: HashSet<String> = self.doc.live().filter(|e| ids.contains(&e.id)).filter_map(|e| e.group_ids.last().cloned()).collect();
        if groups.is_empty() {
            return;
        }
        self.snapshot();
        for e in self.doc.elements.iter_mut() {
            if e.group_ids.last().is_some_and(|g| groups.contains(g)) {
                e.group_ids.pop();
                e.bump();
            }
        }
        self.commit(cx);
    }

    pub(super) fn toggle_lock(&mut self, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        let lock = !self.selected().iter().all(|e| e.locked);
        let ids = self.sel_with_bound();
        for e in self.doc.elements.iter_mut().filter(|e| ids.contains(&e.id)) {
            e.locked = lock;
            e.bump();
        }
        if lock {
            self.sel.clear();
        }
        self.commit(cx);
    }

    pub(super) fn unlock_all(&mut self, cx: &mut Context<Self>) {
        self.snapshot();
        for e in self.doc.elements.iter_mut().filter(|e| e.locked) {
            e.locked = false;
            e.bump();
        }
        self.commit(cx);
    }

    /// 层级：1 上移一层、-1 下移一层、2 置顶、-2 置底（绑定文字紧跟容器）
    pub(super) fn z_order(&mut self, dir: i32, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        let ids: HashSet<String> = self.sel_with_bound().into_iter().collect();
        let order: Vec<String> = self.doc.elements.iter().map(|e| e.id.clone()).collect();
        let selected = |i: usize| ids.contains(&order[i]);
        let els = std::mem::take(&mut self.doc.elements);
        let (picked, mut rest): (Vec<Element>, Vec<Element>) = els.into_iter().partition(|e| ids.contains(&e.id));
        let n = order.len();
        let pos = match dir {
            2 => Some(rest.len()),
            -2 => Some(0),
            1 => {
                // 上移一层：越过选区上方最近的一个未选中元素
                let hi = (0..n).rev().find(|i| selected(*i)).unwrap_or(0);
                (hi + 1..n).find(|i| !selected(*i)).and_then(|j| rest.iter().position(|e| e.id == order[j])).map(|p| p + 1)
            }
            _ => {
                let lo = (0..n).find(|i| selected(*i)).unwrap_or(0);
                (0..lo).rev().find(|i| !selected(*i)).and_then(|j| rest.iter().position(|e| e.id == order[j]))
            }
        };
        let pos = pos.unwrap_or(if dir > 0 { rest.len() } else { 0 });
        let tail = rest.split_off(pos.min(rest.len()));
        rest.extend(picked);
        rest.extend(tail);
        self.doc.elements = rest;
        self.commit(cx);
    }

    // ───────────── 对齐 / 分布 ─────────────

    /// 0 左、1 水平居中、2 右、3 上、4 垂直居中、5 下
    pub(super) fn align(&mut self, k: u8, cx: &mut Context<Self>) {
        let units = self.units();
        if units.len() < 2 {
            return;
        }
        self.snapshot();
        let all = units.iter().map(|(_, r)| *r).reduce(|a, b| a.union(&b)).unwrap();
        for (ids, r) in &units {
            let (dx, dy) = math::align_delta(math::Align::from_index(k), (all.x0, all.y0, all.x1, all.y1), (r.x0, r.y0, r.x1, r.y1));
            self.shift_ids(ids, dx, dy);
        }
        let moved: Vec<String> = units.into_iter().flat_map(|(ids, _)| ids).collect();
        geom::update_bound_arrows(&mut self.doc, &moved);
        self.commit(cx);
    }

    /// 水平（true）/ 垂直分布：间距相等
    pub(super) fn distribute(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        let mut units = self.units();
        if units.len() < 3 {
            return;
        }
        self.snapshot();
        if horizontal {
            units.sort_by(|a, b| a.1.center().x.total_cmp(&b.1.center().x));
        } else {
            units.sort_by(|a, b| a.1.center().y.total_cmp(&b.1.center().y));
        }
        let spans: Vec<(f64, f64)> = units.iter().map(|(_, r)| if horizontal { (r.x0, r.w()) } else { (r.y0, r.h()) }).collect();
        for ((ids, _), (at, (from, _))) in units.iter().zip(math::distribute(&spans).into_iter().zip(spans)) {
            let d = at - from;
            if horizontal {
                self.shift_ids(ids, d, 0.);
            } else {
                self.shift_ids(ids, 0., d);
            }
        }
        let moved: Vec<String> = units.into_iter().flat_map(|(ids, _)| ids).collect();
        geom::update_bound_arrows(&mut self.doc, &moved);
        self.commit(cx);
    }

    /// 对齐单位：同一分组算一个整体
    fn units(&self) -> Vec<(Vec<String>, R)> {
        let mut out: Vec<(Vec<String>, R)> = vec![];
        let mut seen: HashSet<String> = HashSet::new();
        for e in self.selected() {
            if seen.contains(&e.id) {
                continue;
            }
            let members: Vec<&Element> = match e.group_ids.last() {
                Some(g) => self.doc.live().filter(|x| x.group_ids.contains(g) && self.sel.contains(&x.id)).collect(),
                None => vec![e],
            };
            let mut ids: Vec<String> = members.iter().map(|m| m.id.clone()).collect();
            for m in &members {
                seen.insert(m.id.clone());
                if let Some(t) = m.bound_text_id().filter(|_| !m.is_linear()) {
                    ids.push(t);
                }
            }
            if let Some(r) = geom::bounds_of(members) {
                out.push((ids, r));
            }
        }
        out
    }

    fn shift_ids(&mut self, ids: &[String], dx: f64, dy: f64) {
        for id in ids {
            if let Some(e) = self.doc.get_mut(id) {
                e.x += dx;
                e.y += dy;
                e.bump();
            }
        }
    }

    // ───────────── 属性 ─────────────

    /// 新元素套用当前样式
    pub(super) fn apply_cur(&self, e: &mut Element) {
        let c = &self.cur;
        if e.is_sticky() {
            e.stroke_color = c.sticky_ink.clone();
            e.background_color = c.sticky_bg.clone();
            e.fill_style = "solid".into();
            e.roughness = c.roughness;
            e.opacity = c.opacity;
            e.set("roundness", if c.round { json!({ "type": 3 }) } else { Value::Null });
            return;
        }
        e.stroke_color = c.stroke.clone();
        if !e.is_text() && !e.is_linear() || (e.is_linear() && c.bg != TRANSPARENT) {
            e.background_color = c.bg.clone();
        }
        e.fill_style = c.fill.clone();
        e.stroke_width = c.stroke_width;
        e.stroke_style = c.stroke_style.clone();
        e.roughness = c.roughness;
        e.opacity = c.opacity;
        if e.is_text() {
            e.background_color = TRANSPARENT.into();
            e.font_size = Some(c.font_size);
            e.set("fontFamily", json!(c.font_family));
            e.set("textAlign", json!(c.text_align));
        }
        match e.kind.as_str() {
            "rectangle" => e.set("roundness", if c.round { json!({ "type": 3 }) } else { Value::Null }),
            "diamond" => e.set("roundness", if c.round { json!({ "type": 2 }) } else { Value::Null }),
            "arrow" | "line" => {
                e.set("roundness", if c.round { json!({ "type": 2 }) } else { Value::Null });
                if e.kind == "arrow" {
                    e.set("startArrowhead", c.start_arrow.clone().map(Value::String).unwrap_or(Value::Null));
                    e.set("endArrowhead", c.end_arrow.clone().map(Value::String).unwrap_or(Value::Null));
                    match c.arrow_type {
                        "sharp" => e.set("roundness", Value::Null),
                        "elbow" => {
                            e.set("roundness", Value::Null);
                            e.set("elbowed", json!(true));
                            e.set("fixedSegments", Value::Null);
                        }
                        _ => e.set("roundness", json!({ "type": 2 })),
                    }
                }
            }
            _ => {}
        }
    }

    /// 改样式：作用于选中元素（文字类属性也作用于它们的绑定文字），同时记为当前样式。
    pub(super) fn set_prop(&mut self, f: impl Fn(&mut ItemStyle), g: impl Fn(&mut Element), cx: &mut Context<Self>) {
        f(&mut self.cur);
        if self.sel.is_empty() {
            cx.notify();
            return;
        }
        self.snapshot();
        let ids = self.sel_with_bound();
        let mut relayout = vec![];
        for e in self.doc.elements.iter_mut().filter(|e| ids.contains(&e.id) && !e.locked) {
            g(e);
            if e.is_text() {
                if let Some(c) = e.container_id() {
                    relayout.push(c.to_string());
                } else {
                    geom::fit_text(e);
                }
            }
            e.bump();
        }
        for c in relayout {
            geom::layout_bound_text(&mut self.doc, &c);
        }
        self.commit(cx);
    }

    // ───────────── 文字编辑 ─────────────

    pub(super) fn new_text_at(&mut self, w: V, window: &mut Window, cx: &mut Context<Self>) {
        self.snapshot();
        let p = self.snap(w);
        let mut e = Element::new("text", p.x, p.y);
        self.apply_cur(&mut e);
        let lh = e.font_size_or() * e.line_height();
        e.y -= lh / 2.;
        geom::fit_text(&mut e);
        let id = e.id.clone();
        self.doc.elements.push(e);
        self.update_frame_membership(std::slice::from_ref(&id));
        self.start_text_edit(&id, window, cx);
    }

    /// 编辑容器 / 箭头的绑定文字（没有就新建）
    pub(super) fn edit_bound_text(&mut self, container: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.doc.get(container).cloned() else { return };
        if let Some(t) = c.bound_text_id().filter(|t| self.doc.get(t).is_some()) {
            self.start_text_edit(&t, window, cx);
            return;
        }
        self.snapshot();
        let mut t = Element::new("text", c.x, c.y);
        self.apply_cur(&mut t);
        t.set("textAlign", json!("center"));
        t.set("verticalAlign", json!("middle"));
        t.set("containerId", json!(container));
        t.set("frameId", c.extra.get("frameId").cloned().unwrap_or(Value::Null));
        if c.is_linear() {
            t.font_size = Some((c.stroke_width * 4. + 8.).clamp(14., 20.).min(self.cur.font_size));
        }
        if c.is_sticky() {
            t.stroke_color = c.stroke_color.clone();
            t.set("baseFontSize", json!(self.cur.font_size));
        }
        t.group_ids = c.group_ids.clone();
        let tid = t.id.clone();
        // 文字紧跟在容器之后（层级）
        let at = self.doc.index(container).map(|i| i + 1).unwrap_or(self.doc.elements.len());
        self.doc.elements.insert(at, t);
        if let Some(ce) = self.doc.get_mut(container) {
            ce.add_bound(&tid, "text");
        }
        geom::layout_bound_text(&mut self.doc, container);
        self.start_text_edit(&tid, window, cx);
    }

    pub(super) fn start_text_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        let Some(e) = self.doc.get(id).cloned() else { return };
        let text = e.original_text();
        let style = self.text_style(&e, cx);
        let area = cx.new(|cx| TextArea::new(&text, "", style, cx));
        area.update(cx, |a, _| a.move_to_end());
        let tid = id.to_string();
        let sub = cx.subscribe_in(&area, window, move |this, area, ev: &TextAreaEvent, window, cx| match ev {
            TextAreaEvent::Changed => {
                let s = area.read(cx).text().to_string();
                this.live_text(&tid, s);
                cx.notify();
            }
            TextAreaEvent::Commit => {
                this.finish_edit(cx);
                window.focus(&this.focus);
            }
            TextAreaEvent::Tab => {}
        });
        window.focus(&area.focus_handle(cx));
        self.sel = self.select_target(id).into_iter().collect();
        self.editing = Some(Editing { id: id.to_string(), frame_name: false, area, _sub: sub });
        cx.notify();
    }

    pub(super) fn start_frame_name_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_edit(cx);
        let Some(e) = self.doc.get(id).cloned() else { return };
        self.snapshot();
        let style = TextStyle { font_size: 14. * self.zoom, line_height: 18. * self.zoom, color: theme_text(cx), bold: false, align: 0, v_align: 0, deco: 0 };
        let area = cx.new(|cx| TextArea::new(e.name().unwrap_or("Frame"), "框架名称", style, cx));
        area.update(cx, |a, _| a.select_all_text());
        let fid = id.to_string();
        let sub = cx.subscribe_in(&area, window, move |this, area, ev: &TextAreaEvent, window, cx| match ev {
            TextAreaEvent::Changed => {
                let s = area.read(cx).text().replace('\n', " ");
                if let Some(f) = this.doc.get_mut(&fid) {
                    f.set("name", json!(s));
                    f.bump();
                }
                cx.notify();
            }
            TextAreaEvent::Commit | TextAreaEvent::Tab => {
                this.finish_edit(cx);
                window.focus(&this.focus);
            }
        });
        window.focus(&area.focus_handle(cx));
        self.editing = Some(Editing { id: id.to_string(), frame_name: true, area, _sub: sub });
        cx.notify();
    }

    fn text_style(&self, e: &Element, cx: &App) -> TextStyle {
        let fs = e.font_size_or() as f32 * self.zoom;
        let dark = theme_dark(cx);
        let col = super::render::color(&e.stroke_color, 100., dark).map(|c| c.hsla()).unwrap_or(theme_text(cx));
        TextStyle {
            font_size: fs,
            line_height: fs * e.line_height() as f32,
            color: col,
            bold: false,
            align: match e.text_align() {
                "center" => 1,
                "right" => 2,
                _ => 0,
            },
            v_align: 0,
            deco: 0,
        }
    }

    fn live_text(&mut self, id: &str, s: String) {
        let Some(e) = self.doc.get_mut(id) else { return };
        e.set("originalText", json!(s));
        e.text = Some(s);
        e.bump();
        match e.container_id().map(|c| c.to_string()) {
            Some(c) if self.doc.get(&c).is_some() => {
                geom::layout_bound_text(&mut self.doc, &c);
            }
            _ => {
                if let Some(e) = self.doc.get_mut(id) {
                    geom::fit_text(e);
                }
            }
        }
    }

    pub(super) fn finish_edit(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.editing.take() else { return };
        // 以输入框里的最终文字为准（变更事件可能还没送达）
        let final_text = ed.area.read(cx).text().to_string();
        if !ed.frame_name && self.doc.get(&ed.id).is_some_and(|e| e.original_text() != final_text) {
            self.live_text(&ed.id, final_text);
        }
        if ed.frame_name {
            self.commit(cx);
            return;
        }
        let empty = self.doc.get(&ed.id).is_none_or(|e| e.original_text().trim().is_empty());
        if empty {
            let container = self.doc.get(&ed.id).and_then(|e| e.container_id().map(|s| s.to_string()));
            if let Some(i) = self.doc.index(&ed.id) {
                self.doc.elements.remove(i);
            }
            if let Some(c) = container.and_then(|c| self.doc.get_mut(&c).map(|_| c)) {
                if let Some(ce) = self.doc.get_mut(&c) {
                    ce.remove_bound(&ed.id);
                }
                geom::layout_bound_text(&mut self.doc, &c);
                self.sel = [c].into_iter().collect();
            } else {
                self.sel.remove(&ed.id);
            }
        }
        self.commit(cx);
    }

    pub(super) fn render_edit_overlay(&mut self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ed = self.editing.as_ref()?;
        let e = self.doc.get(&ed.id)?.clone();
        let o = self.canvas_bounds.origin;
        let a = ed.area.clone();
        if ed.frame_name {
            let p = self.to_screen(v(e.x, e.y - 22.));
            let style = TextStyle { font_size: 14. * self.zoom, line_height: 18. * self.zoom, color: t.text, bold: false, align: 0, v_align: 0, deco: 0 };
            a.update(cx, |a, _| a.set_style(style));
            return Some(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .absolute()
                            .left(p.x - o.x)
                            .top(p.y - o.y)
                            .w(px((e.width as f32 * self.zoom).max(120.)))
                            .h(px(20. * self.zoom))
                            .bg(t.bg)
                            .border_1()
                            .border_color(t.accent)
                            .child(a),
                    )
                    .into_any_element(),
            );
        }
        let style = self.text_style(&e, cx);
        let fam = super::render::font_family(e.font_family()).0;
        a.update(cx, |a, _| {
            a.set_style(style);
            a.set_font(fam);
        });
        let fs = e.font_size_or();
        // 编辑框：容器文字用容器可写区域，独立文字按内容 + 余量
        let (x, y, w, h) = match e.container_id().and_then(|c| self.doc.get(c)) {
            Some(c) if !c.is_linear() => {
                let mw = geom::container_text_max_w(c);
                let cc = geom::center(c);
                (cc.x - mw / 2., e.y, mw, e.height.max(fs * e.line_height()))
            }
            Some(_) => {
                let w = e.width.max(fs * 3.) + fs;
                (e.x + e.width / 2. - w / 2., e.y, w, e.height)
            }
            None => (e.x, e.y, e.width + fs * 2., e.height.max(fs * e.line_height())),
        };
        // 旋转的文字：编辑框放在旋转后的中心（水平显示）
        let c = geom::center(&e);
        let (x, y) = if e.angle != 0. { (c.x - w / 2., c.y - h / 2.) } else { (x, y) };
        let p = self.to_screen(v(x, y));
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .left(p.x - o.x)
                        .top(p.y - o.y)
                        .w(px(w as f32 * self.zoom + 4.))
                        .h(px(h as f32 * self.zoom + 4.))
                        .when(e.container_id().is_none(), |d| d.border_1().border_color(t.accent.opacity(0.5)))
                        .child(a),
                )
                .into_any_element(),
        )
    }

    // ───────────── 图片 ─────────────

    pub(super) fn pick_image(&mut self, at: Option<V>, _window: &mut Window, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions {
                files: true, directories: false, multiple: false, prompt: Some("插入图片".into()), ..Default::default()
            });
        let at = at.unwrap_or_else(|| self.to_world(self.canvas_bounds.center()));
        cx.spawn(async move |this: WeakEntity<SketchEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.first() {
                    let p = p.clone();
                    this.update(cx, |this, cx| this.drop_paths_at(vec![p], Some(at), cx)).ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn drop_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let at = self.mouse_world;
        self.drop_paths_at(paths, Some(at), cx);
    }

    fn drop_paths_at(&mut self, paths: Vec<std::path::PathBuf>, at: Option<V>, cx: &mut Context<Self>) {
        for p in paths {
            let name = p.to_string_lossy().to_lowercase();
            if name.ends_with(".excalidraw") || name.ends_with(".excalidraw.md") || name.ends_with(".excalidraw.json") {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    match super::io::parse(&s) {
                        Ok(mut d) => {
                            super::io::externalize_files(&mut d, &assets_dir());
                            self.insert_doc(d, at.unwrap_or(self.mouse_world), cx);
                        }
                        Err(e) => self.show_toast(format!("无法读取：{e}"), cx),
                    }
                }
                continue;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                if let Some(path) = crate::convert::save_image_bytes(&bytes, &ext, &assets_dir()) {
                    self.insert_image(path, at, cx);
                }
            }
        }
    }

    pub(super) fn insert_image(&mut self, path: String, at: Option<V>, cx: &mut Context<Self>) {
        let (iw, ih) = image::image_dimensions(&path).map(|(w, h)| (w as f64, h as f64)).unwrap_or((240., 160.));
        let s = (400. / iw.max(ih)).min(1.);
        let (w, h) = (iw * s, ih * s);
        let at = at.unwrap_or_else(|| self.to_world(self.canvas_bounds.center()));
        self.snapshot();
        let fid = new_id();
        self.doc.files.insert(
            fid.clone(),
            FileData {
                id: fid.clone(),
                mime_type: super::io::mime_of(&path).into(),
                data_url: String::new(),
                local_path: Some(path),
                extra: Default::default(),
            },
        );
        let mut e = Element::new("image", at.x - w / 2., at.y - h / 2.);
        e.width = w;
        e.height = h;
        e.set("fileId", json!(fid));
        e.extra.insert("created".into(), json!(now_ms()));
        let id = e.id.clone();
        self.doc.files.get_mut(&fid).unwrap().extra.insert("created".into(), json!(now_ms()));
        self.doc.elements.push(e);
        self.update_frame_membership(std::slice::from_ref(&id));
        self.sel = [id].into_iter().collect();
        self.tool = Tool::Select;
        self.commit(cx);
    }

    // ───────────── 导入导出 ─────────────

    pub(super) fn open_file(&mut self, cx: &mut Context<Self>) {
        self.popover = None;
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("打开".into()), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<SketchEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.first() {
                    let text = std::fs::read_to_string(p);
                    this.update(cx, |this, cx| match text.map_err(anyhow::Error::from).and_then(|t| super::io::parse(&t)) {
                        Ok(d) => this.replace_doc(d, cx),
                        Err(e) => this.show_toast(format!("无法打开：{e}"), cx),
                    })
                    .ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// 用打开的文件替换当前内容（可撤销）
    pub fn replace_doc(&mut self, mut d: SketchDoc, cx: &mut Context<Self>) {
        super::io::externalize_files(&mut d, &assets_dir());
        self.snapshot();
        self.doc = d;
        self.sel.clear();
        self.multi = None;
        self.grid = self.doc.app_state.get("gridModeEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let r = geom::bounds_of(self.doc.live());
        self.zoom_to(r, 1.);
        self.commit(cx);
        let n = self.doc.live().count();
        self.show_toast(format!("已打开，{n} 个元素"), cx);
    }

    pub(super) fn save_as(&mut self, cx: &mut Context<Self>) {
        self.popover = None;
        self.finish_edit(cx);
        let json = super::io::export_json(&self.doc);
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some("草图.excalidraw"));
        cx.spawn(async move |_: WeakEntity<SketchEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(path))) = rx.await {
                if std::fs::write(&path, json).is_ok() {
                    cx.update(|cx| cx.reveal_path(&path)).ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// 导出 PNG（scale 1/2/3）或 SVG；有选区时只导出选区
    pub(super) fn export_image(&mut self, png: Option<f32>, cx: &mut Context<Self>) {
        self.popover = None;
        let only: Vec<String> = if self.sel.is_empty() { vec![] } else { self.sel_with_bound() };
        let doc = self.doc.clone();
        let opts = super::svg::SvgOpts::default();
        let name = format!("草图.{}", if png.is_some() { "png" } else { "svg" });
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |_: WeakEntity<SketchEditor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = match png {
                    Some(s) => super::svg::to_png(&doc, &only, opts, s).is_some_and(|b| std::fs::write(&path, b).is_ok()),
                    None => std::fs::write(&path, super::svg::to_svg(&doc, &only, opts)).is_ok(),
                };
                if ok {
                    cx.update(|cx| cx.reveal_path(&path)).ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn copy_png(&mut self, cx: &mut Context<Self>) {
        let only: Vec<String> = if self.sel.is_empty() { vec![] } else { self.sel_with_bound() };
        if self.doc.is_empty() {
            return;
        }
        match super::svg::to_png(&self.doc, &only, super::svg::SvgOpts::default(), 2.) {
            Some(png) => {
                crate::clipboard::write_png(&png, "");
                self.show_toast("已复制为 PNG", cx);
            }
            None => self.show_toast("导出失败", cx),
        }
    }

    pub(super) fn clear_canvas(&mut self, cx: &mut Context<Self>) {
        self.popover = None;
        if self.doc.is_empty() {
            return;
        }
        self.snapshot();
        self.doc.elements.clear();
        self.sel.clear();
        self.commit(cx);
    }

    pub(super) fn set_bg(&mut self, c: &str, cx: &mut Context<Self>) {
        self.snapshot();
        self.doc.app_state.insert("viewBackgroundColor".into(), json!(c));
        self.commit(cx);
    }
}

pub(super) fn assets_dir() -> std::path::PathBuf {
    crate::store::data_dir().join("assets")
}

fn theme_dark(cx: &App) -> bool {
    crate::theme::theme(cx).dark
}

fn theme_text(cx: &App) -> Hsla {
    crate::theme::theme(cx).text
}
