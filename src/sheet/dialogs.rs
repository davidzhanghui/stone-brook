//! 对话框与侧边面板：下拉列表设置、创建迷你图（含选择数据范围）、迷你图配置面板。
//! 其余数据类对话框（数据验证、排序、分列、清洗…）见 `data.rs`。

use super::ext::{Opt, Rule, Spark, OPT_COLORS, SPARK_THEMES};
use super::*;
use crate::base::widgets::{check_box, ghost_btn, input_box, primary_btn};
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder;

pub(crate) enum Dlg {
    List(Box<ListDlg>),
    Spark(Box<SparkDlg>),
    Data(Box<super::data::DataDlg>),
}

pub(crate) struct ListDlg {
    pub area: (i32, i32, i32, i32),
    pub multi: bool,
    pub colored: bool,
    /// (输入框, 颜色序号, 自选底色)
    pub opts: Vec<(Entity<LineInput>, u8, String)>,
    /// 正在给第几个选项选颜色
    pub color_pick: Option<usize>,
    /// 拖拽排序：起点
    pub drag: Option<usize>,
    /// 「更多选项设置」：选项来自区域、拒绝非法输入
    pub more: bool,
    pub src: Entity<LineInput>,
    pub reject: bool,
    _subs: Vec<Subscription>,
}

pub(crate) struct SparkDlg {
    pub src: Entity<LineInput>,
    pub dst: Entity<LineInput>,
    /// 正在从表格里选范围：0 数据范围 1 放置位置
    pub picking: Option<u8>,
    /// 编辑已有迷你图的「数据范围与位置」（保留样式）
    pub edit: Option<Cell>,
    _subs: Vec<Subscription>,
}

/// 迷你图面板里的下拉：点在外面收起（「更多颜色」取色器打开时除外）。
fn spark_drop_close(cx: &mut Context<SheetEditor>) -> Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)> {
    Box::new(cx.listener(|this, _: &MouseDownEvent, _, cx| {
        if this.picker.is_none() {
            this.spark_open = 0;
            cx.notify();
        }
    }))
}

/// 右侧面板。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Side {
    Spark(Cell),
    Cf,
    Funcs,
}

impl SheetEditor {
    fn opt_input(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> (Entity<LineInput>, Subscription) {
        let i = cx.new(|cx| LineInput::new("请输入选项", cx));
        i.update(cx, |i, cx| i.set_text(text, cx));
        let s = cx.subscribe_in(&i, window, |this, inp, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit { .. } => {
                // 回车：跳到下一项（最后一项时新增）
                let Some(Dlg::List(d)) = &this.dlg else { return };
                let at = d.opts.iter().position(|o| &o.0 == inp).unwrap_or(0);
                if at + 1 >= d.opts.len() {
                    this.list_add_opt(window, cx);
                } else {
                    let next = d.opts[at + 1].0.clone();
                    window.focus(&next.focus_handle(cx));
                }
                cx.notify();
            }
            InputEvent::Cancel => this.close_dlg(window, cx),
            InputEvent::Changed => cx.notify(),
            _ => {}
        });
        (i, s)
    }

    pub(crate) fn open_list_dlg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let a = self.sel_used();
        let area = (a.row, a.column, a.row + a.height - 1, a.column + a.width - 1);
        let (opts, multi, colored, src, reject) = match self.sx().dv_at(area.0, area.1).cloned() {
            Some(ext::Dv { rule: Rule::List { opts, multi, colored, src }, reject, .. }) => (opts, multi, colored, src, reject),
            _ => (vec![], false, false, String::new(), false),
        };
        let mut list = vec![];
        let mut subs = vec![];
        let n = opts.len().max(3);
        for i in 0..n {
            let text = opts.get(i).map(|o| o.text.clone()).unwrap_or_default();
            let color = opts.get(i).map(|o| o.color).unwrap_or((i % OPT_COLORS.len()) as u8);
            let hex = opts.get(i).map(|o| o.hex.clone()).unwrap_or_default();
            let (inp, s) = self.opt_input(&text, window, cx);
            list.push((inp, color, hex));
            subs.push(s);
        }
        let srci = cx.new(|cx| LineInput::new("例如 A1:A10", cx));
        srci.update(cx, |i, cx| i.set_text(&src, cx));
        window.focus(&list[0].0.focus_handle(cx));
        self.pop = None;
        self.dlg = Some(Dlg::List(Box::new(ListDlg {
            area,
            multi,
            colored,
            opts: list,
            color_pick: None,
            drag: None,
            more: !src.is_empty() || reject,
            src: srci,
            reject,
            _subs: subs,
        })));
        cx.notify();
    }

    fn list_add_opt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = match &self.dlg {
            Some(Dlg::List(d)) => d.opts.len(),
            _ => return,
        };
        let (inp, s) = self.opt_input("", window, cx);
        window.focus(&inp.focus_handle(cx));
        if let Some(Dlg::List(d)) = &mut self.dlg {
            d.opts.push((inp, (n % OPT_COLORS.len()) as u8, String::new()));
            d._subs.push(s);
        }
        cx.notify();
    }

    fn list_confirm(&mut self, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dlg::List(d)) = self.dlg.take() else { return };
        let (r0, c0, r1, c1) = d.area;
        let opts: Vec<Opt> = d
            .opts
            .iter()
            .map(|(i, c, hex)| Opt { text: i.read(cx).text().trim().to_string(), color: *c, hex: hex.clone() })
            .filter(|o| !o.text.is_empty())
            .fold(vec![], |mut v: Vec<Opt>, o| {
                if !v.iter().any(|x| x.text == o.text) {
                    v.push(o);
                }
                v
            });
        let src = if d.more { d.src.read(cx).text().trim().to_string() } else { String::new() };
        let rule = Rule::List { opts, multi: d.multi, colored: d.colored, src };
        let reject = d.more && d.reject;
        self.update_sx(
            |x| {
                x.clear_dv(r0, c0, r1, c1);
                if !remove {
                    x.dv.push(ext::Dv { r0, c0, r1, c1, rule, reject, hint: String::new() });
                }
            },
            cx,
        );
        window.focus(&self.focus);
    }

    pub(crate) fn close_dlg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dlg = None;
        self.dlg_subs.clear();
        window.focus(&self.focus);
        cx.notify();
    }

    pub(crate) fn open_spark_dlg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_spark_dlg_for(None, window, cx)
    }

    /// 创建迷你图；`edit` 为已有迷你图所在格时改它的数据范围与位置。
    pub(crate) fn open_spark_dlg_for(&mut self, edit: Option<Cell>, window: &mut Window, cx: &mut Context<Self>) {
        let a = self.sel_area();
        let multi = a.width > 1 || a.height > 1;
        let src = cx.new(|cx| LineInput::new("选择数据范围", cx));
        let dst = cx.new(|cx| LineInput::new("选择放置位置", cx));
        // 选中了一片数据：作为数据范围，放到下方（列）或右侧（行）；否则放在当前格
        let (sv, dv) = if let Some(c) = edit {
            (self.sx().cell(c.r, c.c).and_then(|x| x.spark.as_ref().map(|s| s.src.clone())).unwrap_or_default(), addr(c.r, c.c))
        } else if multi {
            let r = range_text(&a);
            let at = if a.height >= a.width { addr(a.row + a.height, a.column) } else { addr(a.row, a.column + a.width) };
            (r, at)
        } else {
            (String::new(), addr(self.cur.r, self.cur.c))
        };
        src.update(cx, |i, cx| i.set_text(&sv, cx));
        dst.update(cx, |i, cx| {
            i.set_text(&format!("${}${}", col_name(funcs::parse_addr(&dv).map(|x| x.1).unwrap_or(1)), funcs::parse_addr(&dv).map(|x| x.0).unwrap_or(1)), cx)
        });
        let mk = |i: &Entity<LineInput>, window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(i, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Submit { .. } => this.spark_confirm(window, cx),
                InputEvent::Cancel => this.close_dlg(window, cx),
                InputEvent::Changed => cx.notify(),
                _ => {}
            })
        };
        let subs = vec![mk(&src, window, cx), mk(&dst, window, cx)];
        window.focus(&src.focus_handle(cx));
        self.pop = None;
        self.dlg = Some(Dlg::Spark(Box::new(SparkDlg { src, dst, picking: None, edit, _subs: subs })));
        cx.notify();
    }

    fn spark_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dlg::Spark(d)) = &self.dlg else { return };
        let src = d.src.read(cx).text().replace('$', "");
        let dst = d.dst.read(cx).text().replace('$', "");
        let (Some(_), Some((r0, c0, r1, c1))) = (funcs::parse_range(src.trim()), funcs::parse_range(dst.trim())) else {
            self.toast = Some("请填写正确的数据范围和放置位置".into());
            cx.notify();
            return;
        };
        let src = src.trim().to_uppercase();
        let edit = d.edit;
        // 放置位置是一片区域时：按数据范围的行（或列）逐个放
        let (sr0, sc0, sr1, sc1) = funcs::parse_range(&src).unwrap();
        let group = crate::store::new_uuid();
        self.update_sx(
            |x| {
                if let Some(old) = edit {
                    // 编辑：保留样式，只换数据范围 / 位置
                    if let Some(mut sp) = x.cell_mut(old.r, old.c).spark.take() {
                        sp.src = src.clone();
                        x.cell_mut(r0, c0).spark = Some(sp);
                    }
                    return;
                }
                let cells: Vec<(i32, i32)> = (r0..=r1).flat_map(|r| (c0..=c1).map(move |c| (r, c))).collect();
                let n = cells.len() as i32;
                for (k, (r, c)) in cells.into_iter().enumerate() {
                    let k = k as i32;
                    let s = if n == 1 {
                        src.clone()
                    } else if r1 > r0 {
                        // 竖着放：每个对应数据范围的一行
                        range_text(&area(0, sr0 + k, sc0, sr0 + k, sc1))
                    } else {
                        range_text(&area(0, sr0, sc0 + k, sr1, sc0 + k))
                    };
                    x.cell_mut(r, c).spark = Some(Spark::new(s, &group));
                }
            },
            cx,
        );
        self.dlg = None;
        self.anchor = Cell { r: r0, c: c0 };
        self.cur = self.anchor;
        self.kind = SelKind::Cells;
        self.side = Some(Side::Spark(self.cur));
        window.focus(&self.focus);
        self.sync_formula(cx);
        cx.notify();
    }

    /// 迷你图「选择数据范围」：对话框收起，用户在表格里拖选，选区写回输入框。
    pub(crate) fn spark_pick_sync(&mut self, cx: &mut Context<Self>) {
        let a = self.sel_area();
        let text = range_text(&a);
        if let Some(Dlg::Spark(d)) = &self.dlg {
            match d.picking {
                Some(0) => d.src.update(cx, |i, cx| i.set_text(&text, cx)),
                Some(_) => d.dst.update(cx, |i, cx| i.set_text(&format!("${}${}", col_name(a.column), a.row), cx)),
                None => {}
            }
        }
    }

    pub(crate) fn edit_spark(&mut self, cell: Cell, f: impl FnOnce(&mut Spark), cx: &mut Context<Self>) {
        self.update_sx(
            |x| {
                if let Some(s) = &mut x.cell_mut(cell.r, cell.c).spark {
                    f(s);
                }
            },
            cx,
        );
    }

    #[cfg(feature = "dev")]
    pub(crate) fn test_dlg_ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.dlg {
            Some(Dlg::List(_)) => self.list_confirm(false, window, cx),
            Some(Dlg::Spark(_)) => self.spark_confirm(window, cx),
            Some(Dlg::Data(_)) => self.data_confirm(window, cx),
            None => {}
        }
    }

    // ───────────── 界面 ─────────────

    fn dlg_frame(title: &'static str, w: f32, t: &Theme, cx: &mut Context<Self>) -> Div {
        Self::panel(t).w(px(w)).px(px(20.)).py(px(16.)).gap(px(12.)).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::MEDIUM).child(title))
                .child(Self::ib("dlg-x", "x", "关闭", false, t).on_click(cx.listener(|this, _, w, cx| this.close_dlg(w, cx)))),
        )
    }

    pub(crate) fn dlg_el(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let body: AnyElement = match self.dlg.as_ref()? {
            Dlg::List(d) => self.list_dlg_body(d, t, cx),
            Dlg::Spark(d) => {
                if let Some(which) = d.picking {
                    // 选范围模式：不遮挡表格，顶部一个小条
                    let cur = range_text(&self.sel_area());
                    let bar = Self::panel(t)
                        .px(px(16.))
                        .py(px(12.))
                        .gap(px(10.))
                        .w(px(300.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(if which == 0 { "选择数据范围" } else { "选择放置位置" }))
                        .child(input_box(t).child(cur))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .justify_end()
                                .gap(px(8.))
                                .child(ghost_btn("pk-c", "取消", t).on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(Dlg::Spark(d)) = &mut this.dlg {
                                        d.picking = None;
                                    }
                                    cx.notify();
                                })))
                                .child(primary_btn("pk-ok", "确定", t).on_click(cx.listener(|this, _, _, cx| {
                                    this.spark_pick_sync(cx);
                                    if let Some(Dlg::Spark(d)) = &mut this.dlg {
                                        d.picking = None;
                                    }
                                    cx.notify();
                                }))),
                        );
                    return Some(
                        deferred(div().absolute().top(px(120.)).left_0().right_0().flex().justify_center().child(bar)).with_priority(7).into_any_element(),
                    );
                }
                let ok = !d.src.read(cx).text().trim().is_empty() && !d.dst.read(cx).text().trim().is_empty();
                let field = |label: &'static str, ic: &'static str, inp: Entity<LineInput>, which: u8, cx: &mut Context<Self>| {
                    div().flex().flex_col().gap(px(6.)).child(div().text_size(px(13.)).text_color(t.text_secondary).child(label)).child(
                        input_box(t).gap(px(6.)).child(icon(ic, 14., t.accent)).child(div().flex_1().child(inp)).child(
                            Self::ib(SharedString::from(format!("pk-{which}")), "sh-grid", "选择数据范围", false, t).size(px(22.)).on_click(cx.listener(
                                move |this, _, w, cx| {
                                    if let Some(Dlg::Spark(d)) = &mut this.dlg {
                                        d.picking = Some(which);
                                    }
                                    w.focus(&this.focus);
                                    cx.notify();
                                },
                            )),
                        ),
                    )
                };
                let (src, dst) = (d.src.clone(), d.dst.clone());
                Self::dlg_frame(if d.edit.is_some() { "编辑迷你图" } else { "创建迷你图" }, 360., t, cx)
                    .child(field("选择数据范围", "sh-b-all", src, 0, cx))
                    .child(field("选择放置位置", "sh-down", dst, 1, cx))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.))
                            .child(ghost_btn("sp-c", "取消", t).on_click(cx.listener(|this, _, w, cx| this.close_dlg(w, cx))))
                            .child(
                                primary_btn("sp-ok", "确认", t)
                                    .when(!ok, |d| d.opacity(0.45))
                                    .on_click(cx.listener(|this, _, w, cx| this.spark_confirm(w, cx))),
                            ),
                    )
                    .into_any_element()
            }
            Dlg::Data(_) => self.data_dlg_body(t, window, cx)?,
        };
        Some(
            deferred(
                div()
                    .id("dlg-mask")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(t.overlay)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().id("dlg").on_click(|_, _, cx| cx.stop_propagation()).child(body)),
            )
            .with_priority(7)
            .into_any_element(),
        )
    }

    fn list_dlg_body(&self, d: &ListDlg, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let radio = |id: &'static str, label: &'static str, on: bool, multi: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .child(
                    div()
                        .size(px(16.))
                        .rounded_full()
                        .border_1()
                        .border_color(if on { t.accent } else { t.border_strong })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(on, |d| d.child(div().size(px(8.)).rounded_full().bg(t.accent))),
                )
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(Dlg::List(d)) = &mut this.dlg {
                        d.multi = multi;
                    }
                    cx.notify();
                }))
        };
        let colored = d.colored;
        let rows = d
            .opts
            .iter()
            .enumerate()
            .map(|(i, (inp, color, hex))| {
                let (bg, fg) = ext::opt_colors(*color, hex);
                let picking = d.color_pick == Some(i);
                div()
                    .id(("lo", i))
                    .relative()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .when(d.drag == Some(i), |d| d.opacity(0.6))
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        // 拖拽排序：拖到哪一行就换到哪
                        if ev.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        if let Some(Dlg::List(d)) = &mut this.dlg {
                            if let Some(from) = d.drag.filter(|f| *f != i) {
                                let o = d.opts.remove(from);
                                d.opts.insert(i, o);
                                d.drag = Some(i);
                                cx.notify();
                            }
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if let Some(Dlg::List(d)) = &mut this.dlg {
                                d.drag = None;
                            }
                            cx.notify();
                        }),
                    )
                    .child(div().id(("lo-drag", i)).cursor(CursorStyle::OpenHand).child(icon("grip", 14., t.text_muted)).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(Dlg::List(d)) = &mut this.dlg {
                                d.drag = Some(i);
                            }
                            cx.notify();
                        }),
                    ))
                    .when(colored, |d| {
                        // 圆点 + 小三角，选中时外圈高亮
                        d.child(
                            div()
                                .id(("lo-c", i))
                                .size(px(22.))
                                .rounded_full()
                                .border_2()
                                .border_color(if picking { t.accent } else { gpui::transparent_black() })
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .child(div().size(px(16.)).rounded_full().bg(rgb(bg)).flex().items_center().justify_center().child(icon(
                                    "sh-tri-down",
                                    8.,
                                    rgb(fg).into(),
                                )))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(Dlg::List(d)) = &mut this.dlg {
                                        d.color_pick = if d.color_pick == Some(i) { None } else { Some(i) };
                                    }
                                    this.picker = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(input_box(t).flex_1().child(inp.clone()))
                    .child(Self::ib(("lo-del", i), "trash", "删除", false, t).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(Dlg::List(d)) = &mut this.dlg {
                            if i < d.opts.len() {
                                d.opts.remove(i);
                            }
                            d.color_pick = None;
                        }
                        cx.notify();
                    })))
            })
            .collect::<Vec<_>>();
        let more = d.more;
        let reject = d.reject;
        let src = d.src.clone();
        Self::dlg_frame("下拉列表", 560., t, cx)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(16.))
                    .child(radio("lr-single", "单选", !d.multi, false, cx))
                    .child(radio("lr-multi", "多选", d.multi, true, cx))
                    .child(div().w(px(1.)).h(px(14.)).bg(t.border))
                    .child(
                        div()
                            .id("lr-color")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .child(check_box(colored, t))
                            .child("选项颜色")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(Dlg::List(d)) = &mut this.dlg {
                                    d.colored = !d.colored;
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("lr-more")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .text_color(t.accent)
                            .cursor_pointer()
                            .child("更多选项设置")
                            .child(icon(if more { "chevron-down" } else { "chevron-right" }, 14., t.accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(Dlg::List(d)) = &mut this.dlg {
                                    d.more = !d.more;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(div().id("lo-list").flex().flex_col().gap(px(8.)).max_h(px(320.)).overflow_y_scroll().children(rows))
            .child(
                div()
                    .id("lo-add")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .text_color(t.text_secondary)
                    .cursor_pointer()
                    .child(icon("plus", 14., t.text_secondary))
                    .child("新增选项")
                    .on_click(cx.listener(|this, _, w, cx| this.list_add_opt(w, cx))),
            )
            .when(more, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(10.))
                        .rounded(px(6.))
                        .bg(t.sidebar_bg)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.))
                                .child(div().text_color(t.text_secondary).child("从单元格区域获取选项"))
                                .child(input_box(t).flex_1().child(src)),
                        )
                        .child(
                            div()
                                .id("lr-rej")
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .child(check_box(reject, t))
                                .child("输入非选项值时拒绝输入")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(Dlg::List(d)) = &mut this.dlg {
                                        d.reject = !d.reject;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(ghost_btn("lr-rm", "移除列表", t).on_click(cx.listener(|this, _, w, cx| this.list_confirm(true, w, cx))))
                    .child(div().flex_1())
                    .child(ghost_btn("lr-cancel", "取消", t).on_click(cx.listener(|this, _, w, cx| this.close_dlg(w, cx))))
                    .child(primary_btn("lr-ok", "确认", t).on_click(cx.listener(|this, _, w, cx| this.list_confirm(false, w, cx)))),
            )
            .relative()
            .when(d.color_pick.is_some(), |frame| {
                // 选项颜色面板：浮在对话框左侧，不遮挡选项；左侧放不下时放右侧，都放不下才盖在对话框上
                let room = (f32::from(self.bounds.size.width) - 560.) / 2.;
                let pal = div().id("lo-pal").child(self.color_panel("opt-color", t, cx)).on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.picker.is_none() {
                        if let Some(Dlg::List(d)) = &mut this.dlg {
                            d.color_pick = None;
                        }
                        cx.notify();
                    }
                }));
                let wrap = div().absolute().top(px(-20.));
                frame.child(if room >= 300. {
                    wrap.right(relative(1.)).pr(px(8.)).child(pal)
                } else if room >= 150. {
                    wrap.left(relative(1.)).pl(px(8.)).child(pal)
                } else {
                    wrap.left(px(56.)).top(px(110.)).child(pal)
                })
            })
            .into_any_element()
    }

    /// 迷你图配置面板（选中迷你图后右侧「迷你图」：基础配置 / 高级配置）。
    pub(crate) fn spark_panel(&self, cell: Cell, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let sp = self.sx().cell(cell.r, cell.c)?.spark.clone()?;
        let open = self.spark_open;
        let seg = |id: &'static str, label: &'static str, ic: &'static str, on: bool, kind: u8, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .h(px(28.))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent_soft).text_color(t.accent))
                .when(!on, |d| d.hover(|s| s.bg(t.hover)))
                .child(icon(ic, 14., if on { t.accent } else { t.text }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.spark_open = 0;
                    this.edit_spark(cell, |s| s.kind = kind, cx)
                }))
        };
        let th = SPARK_THEMES[sp.theme as usize % SPARK_THEMES.len()];
        let swatch2 = |a: u32, b: u32| {
            div()
                .flex()
                .flex_row()
                .rounded(px(2.))
                .overflow_hidden()
                .child(div().w(px(14.)).h(px(10.)).bg(rgb(a)))
                .child(div().w(px(14.)).h(px(10.)).bg(rgb(b)))
        };
        // 下拉框：点击展开 / 收起 `which`
        let toggle = move |which: u8| {
            move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.spark_open = if this.spark_open == which { 0 } else { which };
                this.picker = None;
                cx.notify();
            }
        };
        // 下拉内容：紧贴在下拉框下方浮出（侧栏不在 deferred 里，可以用 deferred 盖住下面的内容）
        let drop = |el: AnyElement| deferred(anchored().snap_to_window_with_margin(px(8.)).child(div().mt(px(34.)).child(el))).with_priority(6);
        let mark = |id: &'static str, label: &'static str, bit: u8, enabled: bool, cx: &mut Context<Self>| {
            let on = sp.marks & bit != 0 && enabled;
            let k = bit.trailing_zeros() as u8;
            let col = sp.mark_color(k as usize).and_then(hex_to_hsla).unwrap_or(rgb(th.2).into());
            let which = 8 + k;
            div()
                .id(id)
                .relative()
                .w(px(116.))
                .h(px(26.))
                .pr(px(4.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .rounded(px(5.))
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| d.hover(|s| s.bg(t.hover)))
                .child(
                    div()
                        .id((id, 1usize))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .flex_1()
                        .when(enabled, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.edit_spark(cell, |s| s.marks ^= bit, cx))))
                        .child(check_box(on, t))
                        .child(label),
                )
                // 勾选后右侧出现颜色点，点开选颜色
                .when(on, |d| {
                    d.child(
                        div()
                            .id((id, 2usize))
                            .size(px(16.))
                            .rounded_full()
                            .bg(col)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(icon("sh-tri-down", 8., white()))
                            .on_click(cx.listener(toggle(which))),
                    )
                    .when(open == which, |d| {
                        d.child(drop(
                            div().id("sp-mk-pal").child(self.color_panel("spark-mark", t, cx)).on_mouse_down_out(spark_drop_close(cx)).into_any_element(),
                        ))
                    })
                })
        };
        let title = |s: &'static str| div().mt(px(8.)).font_weight(FontWeight::MEDIUM).child(s);
        let sub = |s: &'static str| div().text_size(px(12.)).text_color(t.text_secondary).child(s);
        let line = sp.kind == 0;
        let adv = self.spark_adv;
        let tab = |id: &'static str, label: &'static str, on: bool, adv: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .h(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .border_b_2()
                .border_color(if on { t.accent } else { gpui::transparent_black() })
                .text_color(if on { t.accent } else { t.text_secondary })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.spark_adv = adv;
                    this.spark_open = 0;
                    cx.notify();
                }))
        };
        let mut body = div().id("sp-body").flex_1().min_h(px(0.)).overflow_y_scroll().flex().flex_col().gap(px(8.)).px(px(16.)).pb(px(16.));
        if !adv {
            let color_label: AnyElement = if sp.color.is_empty() {
                div().child("默认").into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(14.)).rounded(px(3.)).bg(hex_to_hsla(&sp.color).unwrap_or(t.accent)))
                    .child(sp.color.to_uppercase())
                    .into_any_element()
            };
            let sel = |id: &'static str, which: u8, cx: &mut Context<Self>| {
                crate::base::widgets::select_box(id, "", None, t)
                    .w_full()
                    .when(open == which, |d| d.border_color(t.accent))
                    .on_click(cx.listener(toggle(which)))
            };
            body = body
                .child(
                    div().mt(px(8.)).flex().flex_row().items_center().child(div().flex_1().font_weight(FontWeight::MEDIUM).child("数据范围与位置")).child(
                        Self::ib("sp-edit", "pencil", "编辑数据范围与位置", false, t)
                            .size(px(24.))
                            .on_click(cx.listener(move |this, _, w, cx| this.open_spark_dlg_for(Some(cell), w, cx))),
                    ),
                )
                .child(div().text_color(t.text_secondary).child(format!("{}  →  {}{}", sp.src, col_name(cell.c), cell.r)))
                .child(title("类型"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .p(px(2.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(t.border)
                        .child(seg("sp-line", "折线", "sh-line", sp.kind == 0, 0, cx))
                        .child(seg("sp-col", "柱状", "sh-bars", sp.kind == 1, 1, cx))
                        .child(seg("sp-wl", "盈亏", "sh-winloss", sp.kind == 2, 2, cx)),
                )
                .child(title("主题"))
                .child(
                    div()
                        .relative()
                        .child(
                            crate::base::widgets::select_box("sp-theme", "", None, t)
                                .relative()
                                .w_full()
                                .when(open == 1, |d| d.border_color(t.accent))
                                .child(div().absolute().left(px(8.)).flex().flex_row().items_center().gap(px(8.)).child(swatch2(th.1, th.2)).child(th.0))
                                .on_click(cx.listener(toggle(1))),
                        )
                        .when(open == 1, |d| {
                            let items = SPARK_THEMES.iter().enumerate().map(|(i, (name, a, b))| {
                                let on = i == sp.theme as usize;
                                div()
                                    .id(("sp-th", i))
                                    .h(px(30.))
                                    .px(px(8.))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .when(on, |d| d.bg(t.hover).text_color(t.accent))
                                    .hover(|s| s.bg(t.hover))
                                    .child(swatch2(*a, *b))
                                    .child(div().flex_1().child(*name))
                                    .when(on, |d| d.child(icon("check", 14., t.accent)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.spark_open = 0;
                                        this.edit_spark(
                                            cell,
                                            |s| {
                                                s.theme = i as u8;
                                                s.color.clear();
                                                s.mark_colors.clear();
                                            },
                                            cx,
                                        )
                                    }))
                            });
                            d.child(drop(
                                Self::panel(t)
                                    .id("sp-th-list")
                                    .w(px(248.))
                                    .p(px(4.))
                                    .children(items)
                                    .on_mouse_down_out(spark_drop_close(cx))
                                    .into_any_element(),
                            ))
                        }),
                )
                .child(title("调整样式"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(10.))
                        .child(div().flex_1().flex().flex_col().gap(px(4.)).child(sub("迷你图颜色")).child(
                            div().relative().child(sel("sp-color", 2, cx).relative().child(div().absolute().left(px(8.)).child(color_label))).when(
                                open == 2,
                                |d| {
                                    d.child(drop(
                                        div()
                                            .id("sp-pal")
                                            .child(self.color_panel("spark-color", t, cx))
                                            .on_mouse_down_out(spark_drop_close(cx))
                                            .into_any_element(),
                                    ))
                                },
                            ),
                        ))
                        .when(line, |d| {
                            d.child(div().flex_1().flex().flex_col().gap(px(4.)).child(sub("折线粗细")).child(
                                div().relative().child(sel("sp-lw", 3, cx).relative().child(div().absolute().left(px(8.)).child(format!("{}px", sp.lw)))).when(
                                    open == 3,
                                    |d| {
                                        let items = (1..=5u8).map(|w| {
                                            let on = w == sp.lw;
                                            div()
                                                .id(("sp-lw", w as usize))
                                                .h(px(30.))
                                                .px(px(8.))
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(8.))
                                                .rounded(px(5.))
                                                .cursor_pointer()
                                                .when(on, |d| d.bg(t.hover).text_color(t.accent))
                                                .hover(|s| s.bg(t.hover))
                                                .child(div().w(px(24.)).h(px(w as f32 * 0.75)).bg(t.text))
                                                .child(div().flex_1().child(format!("{w}px")))
                                                .when(on, |d| d.child(icon("check", 14., t.accent)))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.spark_open = 0;
                                                    this.edit_spark(cell, |s| s.lw = w, cx)
                                                }))
                                        });
                                        d.child(drop(
                                            Self::panel(t)
                                                .id("sp-lw-list")
                                                .w(px(118.))
                                                .p(px(4.))
                                                .children(items)
                                                .on_mouse_down_out(spark_drop_close(cx))
                                                .into_any_element(),
                                        ))
                                    },
                                ),
                            ))
                        }),
                )
                .child(
                    div()
                        .id("sp-fancy")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .child(check_box(sp.fancy, t))
                        .child(if line { "渐变填充" } else { "显示圆角" })
                        .on_click(cx.listener(move |this, _, _, cx| this.edit_spark(cell, |s| s.fancy = !s.fancy, cx))),
                )
                .child(title("标记"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_x(px(8.))
                        .gap_y(px(2.))
                        .child(mark("mk-hi", "高点", 1, true, cx))
                        .child(mark("mk-first", "首点", 8, true, cx))
                        .child(mark("mk-lo", "低点", 2, true, cx))
                        .child(mark("mk-last", "尾点", 16, true, cx))
                        .child(mark("mk-neg", "负点", 4, true, cx))
                        .child(mark("mk-all", "标记", 32, line, cx)),
                );
        } else {
            let radio = |id: &'static str, label: &'static str, v: u8, cx: &mut Context<Self>| {
                let on = sp.empty == v;
                div()
                    .id(id)
                    .h(px(26.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(16.))
                            .rounded_full()
                            .border_1()
                            .border_color(if on { t.accent } else { t.border_strong })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(on, |d| d.child(div().size(px(8.)).rounded_full().bg(t.accent))),
                    )
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.edit_spark(cell, |s| s.empty = v, cx)))
            };
            body = body
                .child(title("空单元格显示为"))
                .child(radio("sp-e0", "空距", 0, cx))
                .child(radio("sp-e1", "零值", 1, cx))
                .when(line, |d| d.child(radio("sp-e2", "用直线连接数据点", 2, cx)))
                .child(title("坐标轴"))
                .child(
                    div()
                        .id("sp-axis")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .child(check_box(sp.axis, t))
                        .child("显示横坐标轴")
                        .on_click(cx.listener(move |this, _, _, cx| this.edit_spark(cell, |s| s.axis = !s.axis, cx))),
                )
                .child(
                    div()
                        .id("sp-rtl")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .child(check_box(sp.rtl, t))
                        .child("从右到左绘制数据")
                        .on_click(cx.listener(move |this, _, _, cx| this.edit_spark(cell, |s| s.rtl = !s.rtl, cx))),
                );
        }
        let group = sp.group.clone();
        Some(
            div()
                .w(px(280.))
                .h_full()
                .flex_none()
                .border_l_1()
                .border_color(t.border)
                .bg(t.bg)
                .flex()
                .flex_col()
                .text_size(px(13.))
                .child(
                    div()
                        .px(px(16.))
                        .pt(px(12.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::MEDIUM).child("迷你图"))
                        .child(
                            div()
                                .id("sp-help")
                                .child(icon("info", 14., t.text_muted))
                                .tooltip(|_, cx| cx.new(|_| crate::ui::Tooltip("在单元格内嵌入迷你图，趋势走向一目了然".into())).into()),
                        )
                        .child(div().flex_1())
                        .child(Self::ib("sp-close", "x", "关闭", false, t).on_click(cx.listener(|this, _, _, cx| {
                            this.side = None;
                            this.spark_open = 0;
                            cx.notify();
                        }))),
                )
                .child(div().mx(px(16.)).flex().flex_row().border_b_1().border_color(t.border).child(tab("sp-tab0", "基础配置", !adv, false, cx)).child(tab(
                    "sp-tab1",
                    "高级配置",
                    adv,
                    true,
                    cx,
                )))
                .child(body)
                .child(self.spark_panel_part(cell, t, cx, group.clone())),
        )
    }
    /// 从 `spark_panel` 拆出。
    fn spark_panel_part(&self, cell: Cell, t: &Theme, cx: &mut Context<Self>, group: String) -> AnyElement {
        div()
            .flex_none()
            .mx(px(16.))
            .py(px(12.))
            .border_t_1()
            .border_color(t.border)
            .flex()
            .flex_row()
            .gap(px(10.))
            .child(
                ghost_btn("sp-del", "删除迷你图", t)
                    .flex_1()
                    .flex_row()
                    .gap(px(4.))
                    .child(icon("trash", 13., t.text_secondary))
                    .on_click(Self::cmd(format!("del-spark:{},{}", cell.r, cell.c), cx)),
            )
            .child(ghost_btn("sp-delg", "删除迷你图组", t).flex_1().flex_row().gap(px(4.)).child(icon("trash", 13., t.text_secondary)).on_click(cx.listener(
                move |this, _, _, cx| {
                    let g = group.clone();
                    this.side = None;
                    this.spark_open = 0;
                    this.update_sx(
                        |x| {
                            for c in x.cells.iter_mut() {
                                if c.spark.as_ref().is_some_and(|s| (!g.is_empty() && s.group == g) || (c.r == cell.r && c.c == cell.c)) {
                                    c.spark = None;
                                }
                            }
                        },
                        cx,
                    );
                },
            )))
            .into_any_element()
    }
}
