//! 单元格对象的交互（「插入」菜单）：复选框、下拉列表、日期选项、日期提醒、图片、迷你图、
//! 链接、附件、备注、评论。数据见 `ext.rs`，绘制见 `paint.rs::paint_obj`。

use super::ext::{Cmt, FileRef, Opt, Remind, Rule, OPT_COLORS};
use super::*;
use crate::base::widgets::{check_box, input_box, primary_btn};
use crate::text_area::{TextArea, TextAreaEvent, TextStyle};
use crate::theme::Theme;
use crate::ui::icon;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use gpui::prelude::FluentBuilder;

/// 贴在单元格上的弹出层。
pub(crate) enum CellPop {
    /// 下拉列表选择器（可搜索）
    Pick { cell: Cell, q: Entity<LineInput>, sel: usize, _s: Subscription },
    /// 日期选择（日期选项 / 日期提醒）
    Date(Box<DatePop>),
    /// 链接卡片：网址 + 编辑 / 取消链接
    LinkCard { cell: Cell },
    /// 插入 / 编辑链接
    LinkEdit { cell: Cell, text: Entity<LineInput>, url: Entity<LineInput>, _s: Vec<Subscription> },
    /// 备注（黄色便签）
    Note { cell: Cell, area: Entity<TextArea>, _s: Subscription },
    /// 评论
    Comment { cell: Cell, input: Entity<LineInput>, _s: Subscription },
    /// 查看大图
    Image { path: String },
    /// 附件全屏预览（顶栏「× 退出 | 图标 文件名 / 大小 | 下载」，下方铺满预览内容）
    FileView { cell: Cell, idx: usize },
    /// 附件重命名
    FileRename { cell: Cell, idx: usize, input: Entity<LineInput>, _s: Subscription },
}

pub(crate) struct DatePop {
    pub cell: Cell,
    pub ym: (i32, u32),
    pub day: Option<NaiveDate>,
    pub time: Option<(u32, u32)>,
    pub remind: bool,
    pub before: i32,
    /// 0 无 1 时间列表 2 提醒时间列表 3 年月列表
    pub open: u8,
    pub note: Entity<LineInput>,
    _s: Subscription,
}

/// 提醒时间选项：(提前分钟, 文字)
const BEFORE: [(i32, &str); 8] = [
    (0, "事件发生时"),
    (5, "提前 5 分钟"),
    (15, "提前 15 分钟"),
    (30, "提前 30 分钟"),
    (60, "提前 1 个小时"),
    (120, "提前 2 个小时"),
    (1440, "提前 1 天"),
    (2880, "提前 2 天"),
];

/// Excel 序列日期 ↔ 本地时间
pub(crate) fn to_serial(dt: NaiveDateTime) -> f64 {
    let base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap().and_hms_opt(0, 0, 0).unwrap();
    (dt - base).num_seconds() as f64 / 86400.
}

pub(crate) fn from_serial(v: f64) -> Option<NaiveDateTime> {
    let base = NaiveDate::from_ymd_opt(1899, 12, 30)?.and_hms_opt(0, 0, 0)?;
    base.checked_add_signed(chrono::Duration::seconds((v * 86400.).round() as i64))
}

impl SheetEditor {
    pub(crate) fn cell_num(&self, r: i32, c: i32) -> Option<f64> {
        match self.model.borrow().get_model().get_cell_value_by_index(self.sheet, r, c) {
            Ok(ironcalc::base::cell::CellValue::Number(v)) => Some(v),
            _ => None,
        }
    }

    fn sel_rect(&self) -> (i32, i32, i32, i32) {
        let a = self.sel_used();
        (a.row, a.column, a.row + a.height - 1, a.column + a.width - 1)
    }

    /// 插入菜单 / 单元格对象相关命令；不认识的命令返回 false。
    pub(crate) fn run_obj(&mut self, head: &str, arg: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (r0, c0, r1, c1) = self.sel_rect();
        match head {
            "ins-check" => {
                // 复选框：区域内的值统一为 TRUE / FALSE（原来是真值的保持勾选）
                self.batch(|this| {
                    this.edit_sx(|x| {
                        x.clear_dv(r0, c0, r1, c1);
                        x.dv.push(ext::Dv { r0, c0, r1, c1, rule: Rule::Check, reject: false, hint: String::new() });
                    });
                    let mut m = this.model.borrow_mut();
                    for r in r0..=r1.min(r0 + 5000) {
                        for c in c0..=c1 {
                            let on = matches!(m.get_model().get_cell_value_by_index(this.sheet, r, c), Ok(ironcalc::base::cell::CellValue::Boolean(true)));
                            m.set_user_input(this.sheet, r, c, if on { "TRUE" } else { "FALSE" }).ok();
                        }
                    }
                });
                self.changed(cx);
            }
            "toggle-check" => {
                let (r, c) = arg.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((self.cur.r, self.cur.c));
                self.toggle_check(r, c, cx);
            }
            "ins-list" => self.open_list_dlg(window, cx),
            "ins-date" | "ins-remind" => {
                let remind = head == "ins-remind";
                self.edit_sx(|x| {
                    x.clear_dv(r0, c0, r1, c1);
                    x.dv.push(ext::Dv { r0, c0, r1, c1, rule: Rule::Date { time: remind }, reject: false, hint: String::new() });
                });
                self.changed(cx);
                // 插入后立即弹出日期选择（空格子自动填今天）
                self.open_date(self.cur, remind, window, cx);
            }
            "date-pick" => self.open_date(self.cur, false, window, cx),
            "ins-image" => self.pick_files(true, cx),
            "ins-file" => self.pick_files(false, cx),
            "ins-link" => self.open_link_edit(self.cur, window, cx),
            "link-card" => {
                self.cpop = Some(CellPop::LinkCard { cell: self.cur });
                cx.notify();
            }
            "unlink" => {
                let c = self.cur;
                self.batch(|this| {
                    let mut m = this.model.borrow_mut();
                    m.delete_cell_link(this.sheet, c.r, c.c).ok();
                    // 去掉链接样式（下划线 + 蓝字）
                    if let Ok(mut st) = m.get_model().get_style_for_cell(this.sheet, c.r, c.c) {
                        st.font.u = false;
                        st.font.color = ironcalc::base::types::Color::None;
                        drop(m);
                        this.paste_styles(&area(this.sheet, c.r, c.c, c.r, c.c), &[vec![st]]);
                    }
                });
                self.cpop = None;
                self.changed(cx);
            }
            "ins-note" => self.open_note(self.cur, window, cx),
            "del-note" => {
                let c = self.cur;
                self.update_sx(|x| x.cell_mut(c.r, c.c).note.clear(), cx);
            }
            "comment" => self.open_comment(self.cur, window, cx),
            "ins-spark" => self.open_spark_dlg(window, cx),
            "edit-spark" => {
                let c = self.cur;
                if self.sx().cell(c.r, c.c).is_some_and(|x| x.spark.is_some()) {
                    self.open_spark_panel(c, cx);
                }
            }
            "del-spark" => {
                let c = Cell {
                    r: arg.split(',').next().and_then(|v| v.parse().ok()).unwrap_or(self.cur.r),
                    c: arg.split(',').nth(1).and_then(|v| v.parse().ok()).unwrap_or(self.cur.c),
                };
                self.update_sx(|x| x.cell_mut(c.r, c.c).spark = None, cx);
                self.side = None;
            }
            "del-image" => {
                let c = self.cur;
                self.update_sx(|x| x.cell_mut(c.r, c.c).image.clear(), cx);
            }
            "del-files" => {
                let c = self.cur;
                self.update_sx(|x| x.cell_mut(c.r, c.c).files.clear(), cx);
            }
            "del-dv" => {
                self.update_sx(|x| x.clear_dv(r0, c0, r1, c1), cx);
            }
            "open-file" => {
                let (c, i) = (self.cur, arg.parse::<usize>().unwrap_or(0));
                if self.sx().cell(c.r, c.c).is_some_and(|x| i < x.files.len()) {
                    self.open_file_view(c, i, window, cx);
                }
            }
            "view-image" => {
                if let Some(x) = self.sx().cell(self.cur.r, self.cur.c).filter(|x| !x.image.is_empty()) {
                    self.cpop = Some(CellPop::Image { path: x.image.clone() });
                    cx.notify();
                }
            }
            _ => return false,
        }
        true
    }

    /// 点击单元格对象；处理了返回 true。
    pub(crate) fn obj_click(&mut self, cell: Cell, at: Point<Pixels>, clicks: usize, cmd: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let sx = self.sx();
        // 日期按钮在活动格右侧（画在相邻格上）
        if let Some(Rule::Date { .. }) = sx.dv_at(self.cur.r, self.cur.c).map(|d| &d.rule) {
            let r = self.cell_rect(self.cur);
            if at.x >= r.right() + px(2.) && at.x <= r.right() + px(22.) && at.y >= r.top() && at.y <= r.bottom() {
                let c = self.cur;
                self.open_date(c, false, window, cx);
                window.prevent_default();
                return true;
            }
        }
        let r = self.cell_rect(cell);
        let rule = sx.dv_at(cell.r, cell.c).map(|d| d.rule.clone());
        let x = sx.cell(cell.r, cell.c).cloned();
        let select = |this: &mut Self, cx: &mut Context<Self>| {
            this.hide_marquee();
            this.select(cell, cell, SelKind::Cells, cx);
        };
        match rule {
            Some(Rule::Check) => {
                let z = self.zoom;
                let h = self.model.borrow().get_cell_style(self.sheet, cell.r, cell.c).ok().and_then(|s| s.alignment.map(|a| a.horizontal));
                use ironcalc::base::types::HorizontalAlignment as H;
                let cxp = match h {
                    Some(H::Left) => r.left() + px(5. * z + 6. * z),
                    Some(H::Right) => r.right() - px(5. * z + 6. * z),
                    _ => r.left() + r.size.width / 2.,
                };
                if (at.x - cxp).abs() <= px(10. * z) {
                    select(self, cx);
                    self.toggle_check(cell.r, cell.c, cx);
                    return true;
                }
            }
            Some(Rule::List { .. }) => {
                let was = self.cur == cell && self.kind == SelKind::Cells;
                if at.x >= r.right() - px(18. * self.zoom) || was || clicks >= 2 {
                    select(self, cx);
                    self.open_pick(cell, window, cx);
                    // 根节点 track_focus 会在冒泡阶段把焦点抢回表格，阻止它（搜索框要拿到键盘）
                    window.prevent_default();
                    return true;
                }
            }
            Some(Rule::Date { .. }) if clicks >= 2 => {
                select(self, cx);
                self.open_date(cell, false, window, cx);
                window.prevent_default();
                return true;
            }
            _ => {}
        }
        if let Some(x) = x {
            // 迷你图：单击只选中，双击打开配置面板（右键「迷你图 ▸ 编辑迷你图」也可以）
            if x.spark.is_some() && clicks >= 2 {
                select(self, cx);
                self.open_spark_panel(cell, cx);
                return true;
            }
            if !x.image.is_empty() && clicks >= 2 {
                select(self, cx);
                self.cpop = Some(CellPop::Image { path: x.image.clone() });
                return true;
            }
            if !x.files.is_empty() && clicks == 1 {
                // 点在文件名上打开
                let lh = 12. * self.zoom * 1.35;
                let top = r.top() + (r.size.height - px(lh * x.files.len() as f32)) / 2.;
                let i = ((at.y - top) / px(lh)).floor() as i64;
                if i >= 0 && (i as usize) < x.files.len() && at.x > r.left() + px(18. * self.zoom) {
                    select(self, cx);
                    self.open_file_view(cell, i as usize, window, cx);
                    return true;
                }
            }
        }
        if let Some(url) = self.cell_link(cell.r, cell.c) {
            if cmd {
                cx.open_url(&url);
                return true;
            }
            if clicks == 1 {
                select(self, cx);
                self.cpop = Some(CellPop::LinkCard { cell });
                return true;
            }
        }
        false
    }

    pub(crate) fn open_spark_panel(&mut self, cell: Cell, cx: &mut Context<Self>) {
        self.side = Some(dialogs::Side::Spark(cell));
        self.spark_open = 0;
        self.spark_adv = false;
        cx.notify();
    }

    pub(crate) fn toggle_check(&mut self, r: i32, c: i32, cx: &mut Context<Self>) {
        let on = matches!(self.model.borrow().get_model().get_cell_value_by_index(self.sheet, r, c), Ok(ironcalc::base::cell::CellValue::Boolean(true)));
        self.model.borrow_mut().set_user_input(self.sheet, r, c, if on { "FALSE" } else { "TRUE" }).ok();
        self.changed(cx);
    }

    /// 选区里是复选框的格子一起切换（空格键）。
    pub(crate) fn toggle_checks_in_sel(&mut self, cx: &mut Context<Self>) -> bool {
        let sx = self.sx();
        let (r0, c0, r1, c1) = self.sel_rect();
        let cells: Vec<(i32, i32)> = (r0..=r1.min(r0 + 2000))
            .flat_map(|r| (c0..=c1).map(move |c| (r, c)))
            .filter(|&(r, c)| matches!(sx.dv_at(r, c).map(|d| &d.rule), Some(Rule::Check)))
            .collect();
        if cells.is_empty() {
            return false;
        }
        let on = matches!(
            self.model.borrow().get_model().get_cell_value_by_index(self.sheet, cells[0].0, cells[0].1),
            Ok(ironcalc::base::cell::CellValue::Boolean(true))
        );
        self.batch(|this| {
            let mut m = this.model.borrow_mut();
            for (r, c) in cells {
                m.set_user_input(this.sheet, r, c, if on { "FALSE" } else { "TRUE" }).ok();
            }
        });
        self.changed(cx);
        true
    }

    /// 弹出层 / 对话框里的 Tab：在其中的输入框之间循环切换焦点，不移动单元格。处理了返回 true。
    pub(crate) fn pop_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        use super::data::DataDlg;
        let fh = |i: &Entity<LineInput>| i.focus_handle(cx);
        let inputs: Vec<FocusHandle> = match (&self.cpop, &self.dlg) {
            (Some(CellPop::LinkEdit { text, url, .. }), _) => vec![fh(text), fh(url)],
            (Some(CellPop::Date(d)), _) => {
                if d.remind {
                    vec![fh(&d.note)]
                } else {
                    vec![]
                }
            }
            (Some(CellPop::Comment { input, .. }), _) => vec![fh(input)],
            (Some(CellPop::Pick { q, .. }), _) => vec![fh(q)],
            (Some(CellPop::FileRename { input, .. }), _) => vec![fh(input)],
            (Some(CellPop::Note { .. } | CellPop::Image { .. } | CellPop::FileView { .. }), _) => vec![],
            (_, Some(dialogs::Dlg::List(d))) => d.opts.iter().map(|o| fh(&o.0)).chain(d.more.then(|| fh(&d.src))).collect(),
            (_, Some(dialogs::Dlg::Spark(d))) => vec![fh(&d.src), fh(&d.dst)],
            (_, Some(dialogs::Dlg::Data(d))) => match d.as_ref() {
                DataDlg::Cf { range, a, b, .. } => vec![fh(range), fh(a), fh(b)],
                DataDlg::Dv { range, a, b, hint, .. } => vec![fh(range), fh(a), fh(b), fh(hint)],
                DataDlg::Split { custom, .. } => vec![fh(custom)],
                DataDlg::XRef { range, .. } => vec![fh(range)],
                _ => vec![],
            },
            _ => return false,
        };
        if !inputs.is_empty() {
            let n = inputs.len();
            let next = match inputs.iter().position(|f| f.is_focused(window)) {
                Some(i) if back => (i + n - 1) % n,
                Some(i) => (i + 1) % n,
                None if back => n - 1,
                None => 0,
            };
            window.focus(&inputs[next]);
            cx.notify();
        }
        true
    }

    // ───────────── 文件 / 图片 ─────────────

    fn pick_files(&mut self, image: bool, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: !image,
            prompt: Some(if image { "插入图片".into() } else { "上传附件".into() }),
            ..Default::default()
        });
        let cell = self.cur;
        cx.spawn(async move |this: WeakEntity<SheetEditor>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            this.update(cx, |this, cx| this.add_files(cell, paths, image, cx)).ok();
        })
        .detach();
    }

    /// 文件复制进资源目录后挂到单元格上（测试脚本也走这里）。
    pub(crate) fn add_files(&mut self, cell: Cell, paths: Vec<std::path::PathBuf>, image: bool, cx: &mut Context<Self>) {
        let dir = crate::objects::store().map(|s| s.assets_dir()).unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&dir).ok();
        let mut refs = vec![];
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if image && !crate::editor::is_image_name(&name) {
                self.toast = Some("请选择图片文件".into());
                continue;
            }
            let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            let dst = dir.join(format!("{}.{ext}", crate::store::new_uuid()));
            if crate::assets::import_copy(&p, &dst).is_ok() {
                let size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                refs.push(FileRef { name, path: dst.to_string_lossy().to_string(), size });
            }
        }
        if refs.is_empty() {
            cx.notify();
            return;
        }
        let row_h = self.model.borrow().get_row_height(self.sheet, cell.r).unwrap_or(25.);
        let n = refs.len() as f64;
        self.batch(|this| {
            this.edit_sx(|x| {
                let cx_ = x.cell_mut(cell.r, cell.c);
                if image {
                    cx_.image = refs[0].path.clone();
                } else {
                    cx_.files.extend(refs);
                }
            });
            // 图片 / 多个附件：撑高行
            let need = if image { 44. } else { 8. + 18. * n };
            if row_h < need {
                this.model.borrow_mut().set_rows_height(this.sheet, cell.r, cell.r, need).ok();
            }
        });
        self.changed(cx);
    }

    pub(crate) fn cell_file(&self, cell: Cell, idx: usize) -> Option<FileRef> {
        self.sx().cell(cell.r, cell.c)?.files.get(idx).cloned()
    }

    /// 附件全屏预览。视频打开即播放（同文档里的「放大预览」）。
    pub(crate) fn open_file_view(&mut self, cell: Cell, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_file = None;
        if let Some(f) = self.cell_file(cell, idx) {
            crate::editor::file_view_opening(self, &f.path, &f.name);
        }
        self.cpop = Some(CellPop::FileView { cell, idx });
        window.focus(&self.focus);
        cx.notify();
    }

    /// 全屏预览关闭后：停掉播放器、收起叠加的原生预览视图。
    pub(crate) fn release_file_view(&mut self) {
        crate::editor::release_file_view(self);
    }

    /// 下载（另存为）附件。
    pub(crate) fn download_file(&mut self, cell: Cell, idx: usize, cx: &mut Context<Self>) {
        let Some(f) = self.cell_file(cell, idx) else { return };
        crate::editor::save_file_as(&f.path, &f.name, cx, |this, dst, _| {
            this.toast = Some(dst.map(|d| format!("已下载到 {}", d.display())).unwrap_or_else(|| "下载失败".into()).into());
        });
    }

    pub(crate) fn delete_file(&mut self, cell: Cell, idx: usize, cx: &mut Context<Self>) {
        self.hover_file = None;
        if matches!(self.cpop, Some(CellPop::FileView { .. } | CellPop::FileRename { .. })) {
            self.cpop = None;
        }
        self.update_sx(
            |x| {
                let fs = &mut x.cell_mut(cell.r, cell.c).files;
                if idx < fs.len() {
                    fs.remove(idx);
                }
            },
            cx,
        );
    }

    /// 重命名附件：在悬浮工具栏的位置换成输入框，回车确认。
    pub(crate) fn open_file_rename(&mut self, cell: Cell, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = self.cell_file(cell, idx) else { return };
        let input = cx.new(|cx| LineInput::new("输入文件名", cx));
        input.update(cx, |i, cx| i.set_text(&f.name, cx));
        let s = cx.subscribe_in(&input, window, |this, inp, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => {
                let name = inp.read(cx).text().trim().to_string();
                if let Some(CellPop::FileRename { cell, idx, .. }) = this.cpop.take() {
                    if !name.is_empty() && this.cell_file(cell, idx).is_some_and(|f| f.name != name) {
                        this.update_sx(
                            |x| {
                                if let Some(f) = x.cell_mut(cell.r, cell.c).files.get_mut(idx) {
                                    f.name = name;
                                }
                            },
                            cx,
                        );
                    }
                }
                window.focus(&this.focus);
                cx.notify();
            }
            InputEvent::Cancel => {
                this.cpop = None;
                window.focus(&this.focus);
                cx.notify();
            }
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.hover_file = None;
        self.cpop = Some(CellPop::FileRename { cell, idx, input, _s: s });
        cx.notify();
    }

    /// 悬停附件时的浮动工具栏：文件名 + 重命名 / 全屏预览 / 下载 / 删除。
    pub(crate) fn file_toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (cell, idx) = self.hover_file?;
        if self.cpop.is_some() || self.dlg.is_some() || self.pop.is_some() || self.editing.is_some() {
            return None;
        }
        let f = self.cell_file(cell, idx)?;
        let r = self.cell_rect(cell);
        let lh = 12. * self.zoom * 1.35;
        let n = self.sx().cell(cell.r, cell.c).map(|x| x.files.len()).unwrap_or(1);
        // 紧贴这一行文件名下方（与格子重叠一点，鼠标移过去不经过别的格子）
        let line_bottom = r.top() + (r.size.height - px(lh * n as f32)) / 2. + px(lh * (idx + 1) as f32);
        let btn = |id: &'static str, ic: &'static str, tip: &'static str| Self::ib(id, ic, tip, false, t).size(px(24.));
        Some(
            div()
                .id("file-tb")
                .absolute()
                .left(r.left() - self.bounds.origin.x + px(8.))
                .top(line_bottom - self.bounds.origin.y)
                .occlude()
                .h(px(32.))
                .pl(px(10.))
                .pr(px(4.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .rounded(px(6.))
                .shadow_md()
                .text_size(px(12.5))
                .text_color(t.text)
                .child(div().max_w(px(220.)).mr(px(8.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(f.name.clone()))
                .child(btn("ftb-rn", "pencil", "重命名").on_click(cx.listener(move |this, _, w, cx| this.open_file_rename(cell, idx, w, cx))))
                .child(btn("ftb-full", "maximize", "全屏预览").on_click(cx.listener(move |this, _, w, cx| this.open_file_view(cell, idx, w, cx))))
                .child(btn("ftb-dl", "download", "下载").on_click(cx.listener(move |this, _, _, cx| this.download_file(cell, idx, cx))))
                .child(btn("ftb-del", "trash", "删除").on_click(cx.listener(move |this, _, _, cx| this.delete_file(cell, idx, cx))))
                .on_hover(cx.listener(|this, on: &bool, _, cx| {
                    if !*on {
                        this.hover_file = None;
                        cx.notify();
                    }
                }))
                .into_any_element(),
        )
    }

    /// 附件全屏预览浮层：盖住整个窗口（见 `editor::file_view_el`）。
    fn file_view_el(&mut self, cell: Cell, idx: usize, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = self.cell_file(cell, idx)?;
        Some(crate::editor::file_view_el(
            self,
            &f.name,
            &f.path,
            f.size,
            t,
            window,
            cx,
            |this, w, cx| {
                this.cpop = None;
                w.focus(&this.focus);
                cx.notify();
            },
            move |this, cx| this.download_file(cell, idx, cx),
        ))
    }

    // ───────────── 下拉列表选择器 ─────────────

    pub(crate) fn open_pick(&mut self, cell: Cell, window: &mut Window, cx: &mut Context<Self>) {
        let q = cx.new(|cx| LineInput::new("查找选项", cx));
        let s = cx.subscribe_in(&q, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                if let Some(CellPop::Pick { sel, .. }) = &mut this.cpop {
                    *sel = 0;
                }
                cx.notify();
            }
            InputEvent::Up | InputEvent::Down => {
                let n = this.pick_options(cx).len().max(1);
                if let Some(CellPop::Pick { sel, .. }) = &mut this.cpop {
                    *sel = if matches!(ev, InputEvent::Up) { (*sel + n - 1) % n } else { (*sel + 1) % n };
                }
                cx.notify();
            }
            InputEvent::Submit { .. } => {
                let opts = this.pick_options(cx);
                if let Some(CellPop::Pick { sel, .. }) = &this.cpop {
                    if let Some(o) = opts.get(*sel).cloned() {
                        this.pick_value(&o.text, window, cx);
                    }
                }
            }
            InputEvent::Cancel => {
                this.cpop = None;
                window.focus(&this.focus);
                cx.notify();
            }
        });
        window.focus(&q.focus_handle(cx));
        self.cpop = Some(CellPop::Pick { cell, q, sel: 0, _s: s });
        cx.notify();
    }

    /// 下拉规则的选项（来自区域时实时读取）。
    pub(crate) fn list_opts(&self, r: i32, c: i32) -> Option<(Vec<Opt>, bool, bool)> {
        let sx = self.sx();
        let Some(Rule::List { opts, multi, colored, src }) = sx.dv_at(r, c).map(|d| d.rule.clone()) else { return None };
        if src.is_empty() {
            return Some((opts, multi, colored));
        }
        let mut out: Vec<Opt> = vec![];
        if let Some((r0, c0, r1, c1)) = funcs::parse_range(&src) {
            let m = self.model.borrow();
            for r in r0..=r1.min(r0 + 500) {
                for c in c0..=c1 {
                    let v = m.get_formatted_cell_value(self.sheet, r, c).unwrap_or_default();
                    if !v.is_empty() && !out.iter().any(|o| o.text == v) {
                        out.push(Opt { color: (out.len() % OPT_COLORS.len()) as u8, text: v, hex: String::new() });
                    }
                }
            }
        }
        Some((out, multi, colored))
    }

    fn pick_options(&self, cx: &App) -> Vec<Opt> {
        let Some(CellPop::Pick { cell, q, .. }) = &self.cpop else { return vec![] };
        let q = q.read(cx).text().to_lowercase();
        self.list_opts(cell.r, cell.c).map(|o| o.0).unwrap_or_default().into_iter().filter(|o| q.is_empty() || o.text.to_lowercase().contains(&q)).collect()
    }

    fn pick_value(&mut self, v: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(CellPop::Pick { cell, .. }) = &self.cpop else { return };
        let cell = *cell;
        let multi = self.list_opts(cell.r, cell.c).is_some_and(|o| o.1);
        let cur = self.model.borrow().get_formatted_cell_value(self.sheet, cell.r, cell.c).unwrap_or_default();
        let new = if multi {
            let mut vals = ext::split_multi(&cur);
            if let Some(i) = vals.iter().position(|x| x == v) {
                vals.remove(i);
            } else {
                vals.push(v.to_string());
            }
            vals.join(",")
        } else if cur == v {
            String::new()
        } else {
            v.to_string()
        };
        self.set_value(cell, &new, cx);
        self.sync_formula(cx);
        if !multi {
            self.cpop = None;
            window.focus(&self.focus);
        }
        cx.notify();
    }

    // ───────────── 日期选择 ─────────────

    pub(crate) fn open_date(&mut self, cell: Cell, remind: bool, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.cell_num(cell.r, cell.c).and_then(from_serial);
        let now = Local::now().naive_local();
        let (saved, time_rule) = {
            let sx = self.sx();
            let time_rule = matches!(sx.dv_at(cell.r, cell.c).map(|d| &d.rule), Some(Rule::Date { time: true }));
            (sx.cell(cell.r, cell.c).and_then(|x| x.remind.clone()), time_rule)
        };
        // 「日期提醒」格（规则带时间）第一次打开：默认开启提醒
        let remind = remind || saved.is_some() || (time_rule && existing.is_none());
        let has_time = remind || time_rule || existing.is_some_and(|d| d.hour() + d.minute() > 0);
        let base = existing.unwrap_or(now);
        let note = cx.new(|cx| LineInput::new("输入提醒内容", cx));
        if let Some(r) = &saved {
            note.update(cx, |n, cx| {
                n.set_text(&r.note, cx);
                n.collapse_to_end(cx);
            });
        }
        // 备注框里回车：保存并关闭；Esc：关闭
        let s = cx.subscribe_in(&note, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => this.date_close(true, window, cx),
            InputEvent::Cancel => this.date_close(false, window, cx),
            _ => {}
        });
        let mut dp = DatePop {
            cell,
            ym: (base.year(), base.month()),
            day: existing.map(|d| d.date()),
            time: has_time.then(|| (base.hour(), base.minute())),
            remind,
            before: saved.map(|r| r.before).unwrap_or(0),
            open: 0,
            note,
            _s: s,
        };
        if existing.is_none() {
            // 第一次弹出：自动选中今天并填进单元格
            dp.day = Some(now.date());
            self.date_apply(&dp, cx);
        }
        self.cpop = Some(CellPop::Date(Box::new(dp)));
        cx.notify();
    }

    /// 关闭日期弹层；`save` 时把提醒备注等写回（回车 /「保存提醒」）。
    pub(crate) fn date_close(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(CellPop::Date(dp)) = self.cpop.take() {
            if save && dp.remind {
                self.date_apply(&dp, cx);
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    /// 把日期弹层的选择写回单元格（值 + 数字格式 + 提醒）。
    fn date_apply(&mut self, dp: &DatePop, cx: &mut Context<Self>) {
        let Some(day) = dp.day else { return };
        let (h, mi) = dp.time.unwrap_or((0, 0));
        let dt = day.and_hms_opt(h, mi, 0).unwrap_or_default();
        let fmt = if dp.time.is_some() { "yyyy/m/d h:mm" } else { "yyyy/m/d" };
        let cell = dp.cell;
        let remind = dp.remind.then(|| Remind { before: dp.before, note: dp.note.read(cx).text(), fired: 0 });
        self.batch(|this| {
            let mut m = this.model.borrow_mut();
            // 加半秒：避免浮点误差让 h:mm 显示成前一分钟
            m.set_user_input(this.sheet, cell.r, cell.c, &format!("{}", to_serial(dt) + 0.5 / 86400.)).ok();
            m.update_range_style(&area(this.sheet, cell.r, cell.c, cell.r, cell.c), "num_fmt", fmt).ok();
            drop(m);
            this.edit_sx(|x| {
                let cur = x.cell(cell.r, cell.c).and_then(|c| c.remind.clone());
                // 时间或提醒设置变了才重置「已提醒」
                x.cell_mut(cell.r, cell.c).remind = remind.map(|mut r| {
                    if let Some(c) = cur.filter(|c| c.before == r.before) {
                        r.fired = c.fired;
                    }
                    r
                });
            });
        });
        self.changed(cx);
        self.sync_formula(cx);
    }

    fn date_update(&mut self, f: impl FnOnce(&mut DatePop), apply: bool, cx: &mut Context<Self>) {
        let Some(CellPop::Date(mut dp)) = self.cpop.take() else { return };
        f(&mut dp);
        if apply {
            self.date_apply(&dp, cx);
        }
        self.cpop = Some(CellPop::Date(dp));
        cx.notify();
    }

    // ───────────── 链接 / 备注 / 评论 ─────────────

    pub(crate) fn open_link_edit(&mut self, cell: Cell, window: &mut Window, cx: &mut Context<Self>) {
        let (label, target) = {
            let m = self.model.borrow();
            let label = m.get_formatted_cell_value(self.sheet, cell.r, cell.c).unwrap_or_default();
            let target = match m.get_cell_link(self.sheet, cell.r, cell.c) {
                Ok(Some(ironcalc::base::types::Link::External { target, .. })) => target,
                _ => String::new(),
            };
            (label, target)
        };
        let text = cx.new(|cx| LineInput::new("输入文本", cx));
        let url = cx.new(|cx| LineInput::new("输入链接", cx));
        text.update(cx, |i, cx| i.set_text(&label, cx));
        url.update(cx, |i, cx| i.set_text(&target, cx));
        let mk = |inp: &Entity<LineInput>, window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(inp, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Submit { .. } => this.link_submit(window, cx),
                InputEvent::Cancel => {
                    this.cpop = None;
                    window.focus(&this.focus);
                    cx.notify();
                }
                InputEvent::Changed => cx.notify(),
                _ => {}
            })
        };
        let s = vec![mk(&text, window, cx), mk(&url, window, cx)];
        window.focus(&if label.is_empty() { text.focus_handle(cx) } else { url.focus_handle(cx) });
        self.cpop = Some(CellPop::LinkEdit { cell, text, url, _s: s });
        cx.notify();
    }

    fn link_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(CellPop::LinkEdit { cell, text, url, .. }) = &self.cpop else { return };
        let (cell, label, mut target) = (*cell, text.read(cx).text(), url.read(cx).text().trim().to_string());
        if target.is_empty() {
            return;
        }
        if !target.contains("://") && !target.starts_with("mailto:") {
            target = format!("https://{target}");
        }
        let label = if label.trim().is_empty() { target.clone() } else { label };
        self.model.borrow_mut().set_cell_link(self.sheet, cell.r, cell.c, ironcalc::base::types::Link::External { target, tooltip: None }, Some(&label)).ok();
        self.cpop = None;
        window.focus(&self.focus);
        self.changed(cx);
        self.sync_formula(cx);
    }

    pub(crate) fn cell_link(&self, r: i32, c: i32) -> Option<String> {
        match self.model.borrow().get_cell_link(self.sheet, r, c) {
            Ok(Some(ironcalc::base::types::Link::External { target, .. })) => Some(target),
            Ok(Some(ironcalc::base::types::Link::Internal { location, .. })) => Some(location),
            _ => None,
        }
    }

    pub(crate) fn open_note(&mut self, cell: Cell, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.sx().cell(cell.r, cell.c).map(|x| x.note.clone()).unwrap_or_default();
        let style = TextStyle { font_size: 13., line_height: 19., color: rgb(0x1f2329).into(), bold: false, align: 0, v_align: 0, deco: 0 };
        let area = cx.new(|cx| {
            let mut a = TextArea::new(&text, "输入备注", style, cx);
            a.set_scrollable();
            a
        });
        let s = cx.subscribe_in(&area, window, |this, _, ev: &TextAreaEvent, window, cx| {
            if matches!(ev, TextAreaEvent::Commit) {
                this.close_note(window, cx);
            }
        });
        window.focus(&area.focus_handle(cx));
        self.cpop = Some(CellPop::Note { cell, area, _s: s });
        cx.notify();
    }

    pub(crate) fn close_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(CellPop::Note { cell, area, .. }) = self.cpop.take() {
            let text = area.read(cx).text().trim_end().to_string();
            if self.sx().cell(cell.r, cell.c).map(|x| x.note.clone()).unwrap_or_default() != text {
                self.update_sx(|x| x.cell_mut(cell.r, cell.c).note = text, cx);
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    pub(crate) fn open_comment(&mut self, cell: Cell, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| LineInput::new("添加评论，回车发送", cx));
        let s = cx.subscribe_in(&input, window, |this, inp, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => {
                let text = inp.read(cx).text();
                if text.trim().is_empty() {
                    return;
                }
                if let Some(CellPop::Comment { cell, .. }) = &this.cpop {
                    let c = *cell;
                    this.update_sx(
                        |x| x.cell_mut(c.r, c.c).comments.push(Cmt { author: crate::settings::author(), text, ts: Local::now().timestamp_millis() }),
                        cx,
                    );
                }
                inp.update(cx, |i, cx| i.set_text("", cx));
            }
            InputEvent::Cancel => {
                this.cpop = None;
                window.focus(&this.focus);
                cx.notify();
            }
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.cpop = Some(CellPop::Comment { cell, input, _s: s });
        cx.notify();
    }

    // ───────────── 日期提醒 ─────────────

    /// 到点的日期提醒：发系统通知并记下已提醒（每 30 秒检查一次）。
    pub(crate) fn check_reminders(&mut self, cx: &mut Context<Self>) {
        let now = Local::now();
        let sheets = self.model.borrow().get_worksheets_properties();
        let e = (*self.ext()).clone();
        let mut fired = vec![];
        for (si, sp) in sheets.iter().enumerate() {
            let Some(sx) = e.sheets.get(&sp.sheet_id) else { continue };
            for x in &sx.cells {
                let Some(rm) = &x.remind else { continue };
                let Some(v) = (match self.model.borrow().get_model().get_cell_value_by_index(si as u32, x.r, x.c) {
                    Ok(ironcalc::base::cell::CellValue::Number(v)) => Some(v),
                    _ => None,
                }) else {
                    continue;
                };
                let Some(at) = from_serial(v).and_then(|d| Local.from_local_datetime(&d).single()) else { continue };
                let when = at - chrono::Duration::minutes(rm.before as i64);
                if when <= now && rm.fired < when.timestamp_millis() && now - when < chrono::Duration::hours(24) {
                    fired.push((sp.sheet_id, x.r, x.c, when.timestamp_millis(), at, rm.note.clone()));
                }
            }
        }
        if fired.is_empty() {
            return;
        }
        for (_, r, c, _, at, note) in &fired {
            let body = format!("{}{}{}", at.format("%Y/%m/%d %H:%M"), if note.is_empty() { "" } else { " · " }, note);
            let title = format!("表格提醒 {}{}", col_name(*c), r);
            crate::sysui::notify(&title, &body);
            self.toast = Some(format!("⏰ {title}：{body}").into());
        }
        // 只记「已提醒」，不进撤销历史
        let mut e2 = e;
        for (sid, r, c, ms, ..) in fired {
            if let Some(x) = e2.sheets.get_mut(&sid).and_then(|s| s.cells.iter_mut().find(|x| x.r == r && x.c == c)) {
                if let Some(rm) = &mut x.remind {
                    rm.fired = ms;
                }
            }
        }
        if let Ok(json) = serde_json::to_string(&e2) {
            self.model.borrow_mut().load_ext(json);
            crate::objects::touch(&self.obj_id);
        }
        cx.notify();
    }

    // ───────────── 弹出层界面 ─────────────

    pub(crate) fn cell_pop_el(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pop = self.cpop.as_ref()?;
        let cell = match pop {
            CellPop::Pick { cell, .. }
            | CellPop::LinkCard { cell }
            | CellPop::LinkEdit { cell, .. }
            | CellPop::Note { cell, .. }
            | CellPop::Comment { cell, .. }
            | CellPop::FileRename { cell, .. } => *cell,
            CellPop::FileView { cell, idx } => {
                let (cell, idx) = (*cell, *idx);
                return self.file_view_el(cell, idx, t, window, cx);
            }
            CellPop::Date(d) => d.cell,
            CellPop::Image { path } => {
                let path = path.clone();
                return Some(
                    deferred(
                        div()
                            .id("img-mask")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .bg(hsla(0., 0., 0., 0.75))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(img(std::path::PathBuf::from(path)).max_w(relative(0.85)).max_h(relative(0.85)).object_fit(ObjectFit::Contain))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cpop = None;
                                cx.notify();
                            })),
                    )
                    .with_priority(8)
                    .into_any_element(),
                );
            }
        };
        let r = self.cell_rect(cell);
        let below = point(r.left(), r.bottom() + px(4.));
        let body: AnyElement = match self.cpop.as_ref()? {
            CellPop::Pick { q, sel, .. } => {
                let (q, sel) = (q.clone(), *sel);
                let opts = self.pick_options(cx);
                let (_, multi, colored) = self.list_opts(cell.r, cell.c).unwrap_or_default();
                let cur = ext::split_multi(&self.model.borrow().get_formatted_cell_value(self.sheet, cell.r, cell.c).unwrap_or_default());
                Self::panel(t)
                    .w(r.size.width.max(px(180.)))
                    .child(div().px(px(10.)).h(px(30.)).flex().items_center().text_size(px(13.)).child(q))
                    .child(Self::sep(t).my(px(0.)))
                    .when(opts.is_empty(), |d| d.child(div().px(px(12.)).py(px(8.)).text_color(t.text_muted).child("没有选项")))
                    .children(opts.into_iter().enumerate().map(|(i, o)| {
                        let (bg, fg) = if colored { o.colors() } else { (0xdee0e3, 0x1f2329) };
                        let on = cur.contains(&o.text);
                        let text = o.text.clone();
                        div()
                            .id(("pick", i))
                            .mx(px(4.))
                            .px(px(6.))
                            .h(px(30.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(i == sel, |d| d.bg(t.hover))
                            .hover(|s| s.bg(t.hover))
                            .when(multi, |d| d.child(check_box(on, t)))
                            .child(
                                div()
                                    .px(px(8.))
                                    .h(px(20.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(10.))
                                    .bg(rgb(bg))
                                    .text_color(rgb(fg))
                                    .text_size(px(12.))
                                    .child(o.text.clone()),
                            )
                            .child(div().flex_1())
                            .when(on && !multi, |d| d.child(icon("check", 14., t.accent)))
                            .on_click(cx.listener(move |this, _, w, cx| this.pick_value(&text, w, cx)))
                    }))
                    .into_any_element()
            }
            CellPop::Date(dp) => self.date_panel(dp.as_ref(), t, cx),
            CellPop::LinkCard { .. } => {
                let url = self.cell_link(cell.r, cell.c).unwrap_or_default();
                let u2 = url.clone();
                Self::panel(t)
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .px(px(10.))
                    .h(px(36.))
                    .child(
                        div()
                            .id("lk-url")
                            .max_w(px(320.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(t.link)
                            .cursor_pointer()
                            .child(url)
                            .on_click(move |_, _, cx| cx.open_url(&u2)),
                    )
                    .child(div().w(px(12.)))
                    .child(Self::ib("lk-edit", "pencil", "编辑链接", false, t).on_click(cx.listener(move |this, _, w, cx| this.open_link_edit(cell, w, cx))))
                    .child(Self::ib("lk-un", "sh-unlink", "取消链接", false, t).on_click(Self::cmd("unlink", cx)))
                    .into_any_element()
            }
            CellPop::LinkEdit { text, url, .. } => {
                let ok = !url.read(cx).text().trim().is_empty();
                let row = |label: &'static str, inp: Entity<LineInput>| {
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .child(div().w(px(30.)).text_color(t.text_secondary).child(label))
                        .child(input_box(t).w(px(220.)).child(inp))
                };
                Self::panel(t)
                    .p(px(12.))
                    .gap(px(10.))
                    .child(row("文本", text.clone()))
                    .child(
                        div().flex().flex_row().items_end().gap(px(10.)).child(row("链接", url.clone())).child(
                            primary_btn("lk-ok", "确定", t).when(!ok, |d| d.opacity(0.45)).on_click(cx.listener(|this, _, w, cx| this.link_submit(w, cx))),
                        ),
                    )
                    .into_any_element()
            }
            CellPop::Note { area, .. } => div()
                .id("note")
                .occlude()
                .w(px(200.))
                .h(px(120.))
                .p(px(8.))
                .bg(rgb(0xfffbd1))
                .border_1()
                .border_color(rgb(0xf0e6a0))
                .shadow_md()
                .text_color(rgb(0x1f2329))
                .child(area.clone())
                .on_mouse_down_out(cx.listener(|this, _, w, cx| this.close_note(w, cx)))
                .into_any_element(),
            CellPop::Comment { input, .. } => {
                let list = self.sx().cell(cell.r, cell.c).map(|x| x.comments.clone()).unwrap_or_default();
                Self::panel(t)
                    .w(px(300.))
                    .p(px(10.))
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(format!("评论 · {}{}", col_name(cell.c), cell.r)))
                            .when(!list.is_empty(), |d| {
                                d.child(
                                    div()
                                        .id("cm-clear")
                                        .text_size(px(12.))
                                        .text_color(t.text_muted)
                                        .cursor_pointer()
                                        .child("全部解决")
                                        .on_click(cx.listener(move |this, _, _, cx| this.update_sx(|x| x.cell_mut(cell.r, cell.c).comments.clear(), cx))),
                                )
                            }),
                    )
                    .children(list.into_iter().enumerate().map(|(i, c)| {
                        let when = Local.timestamp_millis_opt(c.ts).single().map(|d| d.format("%m-%d %H:%M").to_string()).unwrap_or_default();
                        div()
                            .id(("cm", i))
                            .group("cm")
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(6.))
                                    .items_center()
                                    .child(crate::base::widgets::avatar(&c.author, 18.))
                                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child(c.author.clone()))
                                    .child(div().text_size(px(11.)).text_color(t.text_muted).child(when))
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .id(("cm-del", i))
                                            .invisible()
                                            .group_hover("cm", |s| s.visible())
                                            .cursor_pointer()
                                            .child(icon("trash", 12., t.text_muted))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.update_sx(
                                                    |x| {
                                                        let cs = &mut x.cell_mut(cell.r, cell.c).comments;
                                                        if i < cs.len() {
                                                            cs.remove(i);
                                                        }
                                                    },
                                                    cx,
                                                )
                                            })),
                                    ),
                            )
                            .child(div().pl(px(24.)).child(c.text.clone()))
                    }))
                    .child(input_box(t).child(input.clone()))
                    .into_any_element()
            }
            CellPop::FileRename { input, .. } => Self::panel(t)
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(8.))
                .child(input_box(t).w(px(240.)).child(input.clone()))
                .child(primary_btn("frn-ok", "确定", t).h(px(28.)).on_click(cx.listener(|this, _, w, cx| {
                    if let Some(CellPop::FileRename { input, .. }) = &this.cpop {
                        let input = input.clone();
                        input.update(cx, |_, cx| cx.emit(InputEvent::Submit { shift: false }));
                    }
                    let _ = w;
                })))
                .into_any_element(),
            CellPop::Image { .. } | CellPop::FileView { .. } => return None,
        };
        let at = match self.cpop.as_ref()? {
            CellPop::Note { .. } | CellPop::Comment { .. } => point(r.right() + px(4.), r.top()),
            _ => below,
        };
        Some(
            deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(div().id("cpop").child(body).on_mouse_down_out(cx.listener(
                |this, _, _, cx| {
                    // 备注自己处理关闭（要保存）
                    if !matches!(this.cpop, Some(CellPop::Note { .. })) {
                        this.cpop = None;
                        cx.notify();
                    }
                },
            ))))
            .with_priority(6)
            .into_any_element(),
        )
    }

    fn date_panel(&self, dp: &DatePop, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (y, mo) = dp.ym;
        let first = NaiveDate::from_ymd_opt(y, mo, 1).unwrap_or_default();
        // 周一开头
        let lead = first.weekday().num_days_from_monday() as i64;
        let start = first - chrono::Duration::days(lead);
        let today = Local::now().date_naive();
        let nav = |id: &'static str, ic: &'static str, d: i32, cx: &mut Context<Self>| {
            Self::ib(id, ic, "", false, t).size(px(24.)).on_click(cx.listener(move |this, _, _, cx| {
                this.date_update(
                    |p| {
                        let m = p.ym.0 * 12 + p.ym.1 as i32 - 1 + d;
                        p.ym = (m.div_euclid(12), m.rem_euclid(12) as u32 + 1);
                    },
                    false,
                    cx,
                )
            }))
        };
        let mut grid = div().flex().flex_col().gap(px(2.));
        grid = grid.child(
            div().flex().flex_row().children(
                ["一", "二", "三", "四", "五", "六", "日"]
                    .map(|w| div().w(px(32.)).h(px(26.)).flex().items_center().justify_center().text_size(px(12.)).text_color(t.text_muted).child(w)),
            ),
        );
        for wk in 0..6 {
            let mut row = div().flex().flex_row();
            for d in 0..7 {
                let day = start + chrono::Duration::days(wk * 7 + d);
                let other = day.month() != mo;
                let on = dp.day == Some(day);
                let is_today = day == today;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("d-{day}")))
                        .w(px(32.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(26.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(13.))
                                .when(other, |d| d.text_color(t.text_muted))
                                .when(is_today && !on, |d| d.text_color(t.accent).border_1().border_color(t.accent))
                                .when(on, |d| d.bg(t.accent).text_color(white()))
                                .hover(|s| s.bg(t.hover))
                                .child(day.day().to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.date_update(
                                |p| {
                                    p.day = Some(day);
                                    p.ym = (day.year(), day.month());
                                },
                                true,
                                cx,
                            )
                        })),
                );
            }
            grid = grid.child(row);
        }
        let time_label = dp.time.map(|(h, m)| format!("{h:02}:{m:02}")).unwrap_or_else(|| "00:00".into());
        let before_label = BEFORE.iter().find(|b| b.0 == dp.before).map(|b| b.1).unwrap_or("事件发生时");
        let (has_time, remind, open) = (dp.time.is_some(), dp.remind, dp.open);
        let sel_box = |id: &'static str, label: String, which: u8, cx: &mut Context<Self>| {
            crate::base::widgets::select_box(id, label, None, t)
                .w(px(150.))
                .when(open == which, |d| d.border_color(t.accent))
                .on_click(cx.listener(move |this, _, _, cx| this.date_update(|p| p.open = if p.open == which { 0 } else { which }, false, cx)))
        };
        // 下拉选项浮在选择框上方（不在原地撑开面板）。面板已在 deferred 里不能再嵌套 deferred：
        // 列表挂在所在行上、向上展开，只会盖住先画的日历 / 上一行，自然在最上层
        let list = |items: Vec<(String, Box<dyn Fn(&mut DatePop)>)>, sel: Option<usize>, cx: &mut Context<Self>| {
            div()
                .id("dl")
                .absolute()
                .left(px(78.))
                .bottom(px(34.))
                .w(px(170.))
                .max_h(px(196.))
                .overflow_y_scroll()
                .occlude()
                .p(px(4.))
                .bg(t.bg)
                .border_1()
                .border_color(t.border)
                .rounded(px(8.))
                .shadow_lg()
                .children(items.into_iter().enumerate().map(|(i, (label, f))| {
                    let f = std::rc::Rc::new(f);
                    let on = sel == Some(i);
                    div()
                        .id(("dli", i))
                        .px(px(10.))
                        .h(px(30.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(on, |d| d.bg(t.hover).text_color(t.accent))
                        .hover(|s| s.bg(t.hover))
                        .child(div().flex_1().child(label))
                        .when(on, |d| d.child(icon("check", 14., t.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let f = f.clone();
                            this.date_update(
                                move |p| {
                                    f(p);
                                    p.open = 0;
                                },
                                true,
                                cx,
                            )
                        }))
                }))
        };
        let time_row = div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .id("d-time")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .w(px(70.))
                    .cursor_pointer()
                    .child(check_box(has_time, t))
                    .child("时间")
                    .on_click(cx.listener(|this, _, _, cx| this.date_update(|p| p.time = if p.time.is_some() { None } else { Some((0, 0)) }, true, cx))),
            )
            .when(has_time, |d| d.child(sel_box("d-tsel", time_label, 1, cx)))
            .when(open == 1, |d| {
                let items: Vec<(String, Box<dyn Fn(&mut DatePop)>)> = (0..96)
                    .map(|i| {
                        let (h, m) = (i / 4, (i % 4) * 15);
                        (format!("{h:02}:{m:02}"), Box::new(move |p: &mut DatePop| p.time = Some((h, m))) as Box<dyn Fn(&mut DatePop)>)
                    })
                    .collect();
                let sel = dp.time.and_then(|(h, m)| (m % 15 == 0).then_some((h * 4 + m / 15) as usize));
                d.child(list(items, sel, cx))
            });
        let remind_row = div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(
                div().id("d-rm").flex().flex_row().items_center().gap(px(6.)).w(px(70.)).cursor_pointer().child(check_box(remind, t)).child("提醒").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.date_update(
                            |p| {
                                p.remind = !p.remind;
                                p.open = 0;
                                if p.remind && p.time.is_none() {
                                    p.time = Some((9, 0));
                                }
                                if p.day.is_none() {
                                    p.day = Some(Local::now().date_naive());
                                }
                            },
                            true,
                            cx,
                        )
                    }),
                ),
            )
            .when(remind, |d| d.child(sel_box("d-bsel", before_label.to_string(), 2, cx)))
            .when(open == 2, |d| {
                let items: Vec<(String, Box<dyn Fn(&mut DatePop)>)> = BEFORE
                    .iter()
                    .map(|(m, l)| {
                        let m = *m;
                        (l.to_string(), Box::new(move |p: &mut DatePop| p.before = m) as Box<dyn Fn(&mut DatePop)>)
                    })
                    .collect();
                let sel = BEFORE.iter().position(|b| b.0 == dp.before);
                d.child(list(items, sel, cx))
            });
        let mut p = Self::panel(t)
            .w(px(256.))
            .px(px(12.))
            .py(px(10.))
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(div().font_weight(FontWeight::MEDIUM).child(format!("{y}年 {mo}月")))
                    .child(div().flex_1())
                    .child(nav("d-prev", "chevron-left", -1, cx))
                    .child(nav("d-next", "chevron-right", 1, cx)),
            )
            .child(grid)
            .child(Self::sep(t).mx(px(0.)))
            .child(time_row)
            .child(Self::sep(t).mx(px(0.)))
            .child(remind_row);
        if remind {
            p = p
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(36.)).text_color(t.text_secondary).child("人员"))
                        .child(crate::base::widgets::avatar(&crate::settings::author(), 20.))
                        .child(crate::settings::author()),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(36.)).text_color(t.text_secondary).child("备注"))
                        .child(input_box(t).flex_1().child(dp.note.clone())),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .child(primary_btn("d-save", "保存提醒", t).h(px(26.)).on_click(cx.listener(|this, _, w, cx| this.date_close(true, w, cx)))),
                );
        }
        p.into_any_element()
    }
}

impl crate::editor::ImgHost for SheetEditor {
    fn img_views(&mut self) -> &mut HashMap<u64, crate::editor::ImgView> {
        &mut self.img_views
    }
}

impl crate::editor::FileViewHost for SheetEditor {
    fn fv_media(&mut self) -> &mut Option<u64> {
        &mut self.fv_media
    }
    fn image_sizes(&mut self) -> &mut HashMap<String, Option<(u32, u32)>> {
        &mut self.image_sizes
    }
}
