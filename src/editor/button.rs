//! 按钮块（「打开超链接」）：小色块按钮，点击执行动作；悬停出编辑工具栏；
//! 配置浮层里改按钮文字 / 颜色 / 执行操作 / 超链接地址，按钮实时预览，确定后才写回文档。

use super::*;
use crate::model::{button_color, button_style, Block, BlockKind, BUTTON_COLORS};
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};
use std::cell::Cell;

/// 执行操作：(取值, 名称)
const ACTIONS: [(&str, &str); 2] = [("open_url", "打开超链接"), ("duplicate_doc", "创建副本")];

fn action_name(action: &str) -> &'static str {
    ACTIONS.iter().find(|a| a.0 == action).map(|a| a.1).unwrap_or(ACTIONS[0].1)
}

/// 按钮配置浮层的编辑状态（未确定前不改文档）。
pub struct ButtonEdit {
    pub block: usize,
    label: Entity<LineInput>,
    url: Entity<LineInput>,
    color: usize,
    action: &'static str,
    /// 执行操作下拉是否展开
    action_menu: bool,
    /// 上一帧测得的下拉框 (left, 下沿, 宽) 与浮层原点（窗口坐标），下拉列表据此贴在下拉框下面
    select_at: Rc<Cell<(f32, f32, f32)>>,
    pop_at: Rc<Cell<(f32, f32)>>,
    /// 刚由斜杠菜单插入：插入时已记过撤销点
    fresh: bool,
    _subs: Vec<Subscription>,
}

impl Editor {
    /// 打开按钮配置浮层。
    pub(super) fn open_button_edit(&mut self, ix: usize, fresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(blk) = self.content.blocks.get(ix) else { return };
        let BlockKind::Button { action, url, style } = &blk.kind else { return };
        let action = ACTIONS.iter().find(|a| a.0 == action).map(|a| a.0).unwrap_or(ACTIONS[0].0);
        let (text, url, color) = (blk.text.text.clone(), url.clone(), button_color(*style));
        let label = cx.new(|cx| {
            let mut i = LineInput::new("请输入按钮文字", cx);
            i.set_text(&text, cx);
            i.collapse_to_end(cx);
            i
        });
        let url = cx.new(|cx| {
            let mut i = LineInput::new("请输入链接地址", cx);
            i.set_text(&url, cx);
            i.collapse_to_end(cx);
            i
        });
        let on_input = |this: &mut Self, _: &Entity<LineInput>, ev: &InputEvent, window: &mut Window, cx: &mut Context<Self>| match ev {
            InputEvent::Submit { .. } => this.button_confirm(window, cx),
            InputEvent::Cancel => this.button_cancel(window, cx),
            _ => cx.notify(),
        };
        let subs = vec![cx.subscribe_in(&label, window, on_input), cx.subscribe_in(&url, window, on_input)];
        window.focus(&label.focus_handle(cx));
        self.pop.button_edit =
            Some(ButtonEdit { block: ix, label, url, color, action, action_menu: false, select_at: Rc::default(), pop_at: Rc::default(), fresh, _subs: subs });
        cx.notify();
    }

    /// 确定：把浮层里的设置写回按钮块。
    fn button_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(be) = self.pop.button_edit.take() else { return };
        let ix = be.block;
        let mut label = be.label.read(cx).text().trim().to_string();
        if label.is_empty() {
            label = action_name(be.action).to_string();
        }
        let url = be.url.read(cx).text().trim().to_string();
        let url = if url.is_empty() || url.contains("://") || url.starts_with("mailto:") { url } else { format!("https://{url}") };
        let kind = BlockKind::Button { action: be.action.to_string(), url, style: button_style(be.color) };
        if let Some(blk) = self.content.blocks.get(ix).filter(|b| matches!(b.kind, BlockKind::Button { .. })) {
            if blk.kind != kind || blk.text.text != label {
                if !be.fresh {
                    self.checkpoint(EditKind::Other);
                }
                let b = self.block_mut(ix);
                b.kind = kind;
                b.text = RichText::plain(&label);
                self.changed(cx);
            }
        }
        self.button_edit_done(ix, window, cx);
    }

    fn button_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(be) = self.pop.button_edit.take() {
            self.button_edit_done(be.block, window, cx);
        }
    }

    /// 关闭浮层后光标落到按钮后面，回到正文编辑。
    fn button_edit_done(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.core.head = self.clamp(Pos::new((ix + 1).min(self.content.blocks.len() - 1), 0, 0));
        self.core.anchor = self.core.head;
        window.focus(&self.focus);
        cx.notify();
    }

    fn button_delete(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        self.remove_blocks(ix..ix + 1);
        self.core.head = self.clamp(Pos::new(ix, 0, 0));
        self.core.anchor = self.core.head;
        self.changed(cx);
    }

    /// 鼠标进出按钮 / 工具栏：进入立即显示，移出延迟 250ms 再隐藏，让鼠标有时间从按钮移到工具栏。
    fn button_bar_hover(&mut self, id: u64, over: bool, cx: &mut Context<Self>) {
        self.button_bar_epoch = self.button_bar_epoch.wrapping_add(1);
        if over {
            if self.sess.button_bar != Some(id) {
                self.sess.button_bar = Some(id);
                cx.notify();
            }
            return;
        }
        let epoch = self.button_bar_epoch;
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
            this.update(cx, |this, cx| {
                if this.button_bar_epoch == epoch && this.sess.button_bar == Some(id) {
                    this.sess.button_bar = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_button(&mut self, ix: usize, blk: &Block, selected: bool, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let BlockKind::Button { action, url, style } = &blk.kind else { return div().into_any_element() };
        let editing = self.pop.button_edit.as_ref().filter(|be| be.block == ix);
        // 编辑中按浮层里的内容实时预览
        let (label, color, action, url) = match editing {
            Some(be) => (be.label.read(cx).text(), be.color, be.action, be.url.read(cx).text()),
            None => (blk.text.text.clone(), button_color(*style), if action == "duplicate_doc" { "duplicate_doc" } else { "open_url" }, url.clone()),
        };
        let editing = editing.is_some();
        let (_, bg, fg) = BUTTON_COLORS[color];
        // 还没填链接的「打开超链接」按钮半透明显示
        let unset = action == "open_url" && url.trim().is_empty();
        let shown = if label.trim().is_empty() { action_name(action).to_string() } else { label };
        let id = blk.id as usize;

        let chip = {
            let (action, url) = (action, url.clone());
            div()
                .id(("btn", id))
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if selected { t.accent } else { gpui::transparent_black() })
                .bg(rgb(bg))
                .text_size(px(14.))
                .text_color(rgb(fg))
                .whitespace_nowrap()
                .cursor_pointer()
                .when(unset, |d| d.opacity(0.5))
                .when(!unset && !editing, |d| d.hover(|s| s.opacity(0.85)))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| this.button_bar_hover(id as u64, *over, cx)))
                .child(shown)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if this.pop.button_edit.is_some() {
                            return;
                        }
                        if ev.modifiers.shift {
                            window.focus(&this.focus);
                            this.mouse_select(Pos::new(ix, 0, 0), 1, true, cx);
                        } else if action == "duplicate_doc" {
                            cx.emit(EditorEvent::DuplicateDoc);
                        } else if url.trim().is_empty() {
                            this.open_button_edit(ix, false, window, cx);
                        } else {
                            cx.open_url(&url);
                        }
                    }),
                )
        };

        // 悬停工具栏（按钮上方浮出）
        let tool = |name: &'static str, tip: &'static str, key: &'static str| {
            div()
                .id((key, id))
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(name, 15., t.text_secondary))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
        };
        let toolbar = div()
            .id(("btn-bar", id))
            .absolute()
            .top(px(-36.))
            .left(px(-4.))
            .occlude()
            .on_hover(cx.listener(move |this, over: &bool, _, cx| this.button_bar_hover(id as u64, *over, cx)))
            .flex()
            .flex_row()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .child(tool("pencil", "编辑", "btn-edit").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_button_edit(ix, false, window, cx);
                }),
            ))
            .when(action == "open_url" && !url.trim().is_empty(), |d| {
                let u = url.clone();
                d.child(tool("copy", "复制链接", "btn-copy").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(ClipboardItem::new_string(u.clone()));
                        cx.emit(EditorEvent::Toast("链接已复制".into()));
                    }),
                ))
            })
            .child(tool("trash", "删除", "btn-del").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.button_delete(ix, cx);
                }),
            ));

        div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .child(
                div()
                    .group("btn")
                    .relative()
                    .child(chip)
                    // 悬停状态由 button_bar 记录（group_hover 只认父元素自身范围，够不到上方的工具栏）
                    .when(!editing && self.sess.button_bar == Some(blk.id), |d| d.child(toolbar)),
            )
            // anchored 取父元素原点：套一个贴在按钮下沿的零高占位，浮层就出现在按钮正下方
            .when(editing, |d| d.child(div().w_full().h(px(0.)).child(self.render_button_popover(t, window, cx))))
            .child(self.void_marker(ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                }),
            )
            .into_any_element()
    }

    /// 配置浮层：按钮文字 / 按钮颜色 / 执行操作 / 超链接地址 / 取消·确定，贴在按钮下方。
    fn render_button_popover(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let be = self.pop.button_edit.as_ref().expect("button_edit");
        let caption = |s: &'static str| div().pt(px(12.)).pb(px(6.)).text_size(px(12.)).text_color(t.text_secondary).child(crate::i18n::tr(s));
        let field = |input: &Entity<LineInput>, key: &'static str| {
            let focused = input.focus_handle(cx).is_focused(window);
            let fh = input.focus_handle(cx);
            div()
                .id(key)
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(if focused { t.accent } else { t.border })
                .hover(|s| s.border_color(if focused { t.accent } else { t.border_strong }))
                .text_size(px(13.))
                .text_color(t.text)
                .cursor_text()
                .child(div().flex_1().min_w(px(0.)).line_height(px(20.)).child(input.clone()))
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&fh))
        };

        let swatch = |i: usize| {
            let (name, bg, _) = BUTTON_COLORS[i];
            let on = be.color == i;
            div()
                .id(("btn-color", i))
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_2()
                .border_color(if on { t.accent } else { gpui::transparent_black() })
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(div().size(px(18.)).rounded(px(4.)).bg(rgb(bg)).when(i == 7, |d| d.border_1().border_color(t.border)))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(name.into())).into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(be) = this.pop.button_edit.as_mut() {
                            be.color = i;
                            be.action_menu = false;
                        }
                        cx.notify();
                    }),
                )
        };
        let swatch_row = |r: usize| div().flex().flex_row().justify_between().children((r * 8..r * 8 + 8).map(swatch));

        let select = div()
            .id("btn-action")
            .h(px(30.))
            .px(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(6.))
            .border_1()
            .border_color(if be.action_menu { t.accent } else { t.border })
            .hover(|s| s.border_color(t.accent))
            .text_size(px(13.))
            .text_color(t.text)
            .cursor_pointer()
            .child(action_name(be.action))
            .child(icon(if be.action_menu { "chevron-up" } else { "chevron-down" }, 14., t.text_muted))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(be) = this.pop.button_edit.as_mut() {
                        be.action_menu = !be.action_menu;
                    }
                    cx.notify();
                }),
            );
        let ((sx, sy, mw), (px0, py0)) = (be.select_at.get(), be.pop_at.get());
        // 绝对定位相对浮层的内边距框（扣掉 1px 边框）
        let (mx, my) = (sx - px0 - 1., sy - py0 - 1.);
        let menu = be.action_menu.then(|| {
            div()
                .absolute()
                .left(px(mx))
                .top(px(my + 4.))
                .w(px(mw))
                .occlude()
                .p(px(4.))
                .rounded(px(6.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .shadow_lg()
                .children(ACTIONS.iter().map(|&(key, name)| {
                    let on = be.action == key;
                    div()
                        .id(key)
                        .h(px(30.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(px(4.))
                        .text_size(px(13.))
                        .text_color(if on { t.accent } else { t.text })
                        .cursor_pointer()
                        .hover(|s| s.bg(t.hover))
                        .child(name)
                        .when(on, |d| d.child(icon("check", 14., t.accent)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(be) = this.pop.button_edit.as_mut() {
                                    // 按钮文字还是默认的动作名时跟着切换
                                    let cur = be.label.read(cx).text();
                                    if cur.trim().is_empty() || ACTIONS.iter().any(|a| a.1 == cur.trim()) {
                                        be.label.update(cx, |i, cx| {
                                            i.set_text(name, cx);
                                            i.collapse_to_end(cx);
                                        });
                                    }
                                    be.action = key;
                                    be.action_menu = false;
                                    if key == "open_url" {
                                        window.focus(&be.url.focus_handle(cx));
                                    }
                                }
                                cx.notify();
                            }),
                        )
                }))
        });

        let button = |key: &'static str, label: &'static str, primary: bool| {
            div()
                .id(key)
                .h(px(30.))
                .w(px(68.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .when(primary, |d| d.bg(t.accent).text_color(gpui::white()).hover(|s| s.opacity(0.9)))
                .when(!primary, |d| d.border_1().border_color(t.border).text_color(t.text).hover(|s| s.bg(t.hover)))
                .child(crate::i18n::tr(label))
        };

        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("btn-pop")
                    .occlude()
                    .mt(px(6.))
                    .w(px(272.))
                    .px(px(12.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .child(caption("按钮文字"))
                    .child(field(&be.label, "btn-label"))
                    .child(caption("按钮颜色"))
                    .child(div().flex().flex_col().gap(px(6.)).child(swatch_row(0)).child(swatch_row(1)))
                    .child(caption("执行操作"))
                    .child(div().relative().child(select).child({
                        let at = be.select_at.clone();
                        canvas(move |b, _, _| at.set((b.origin.x.into(), b.bottom().into(), b.size.width.into())), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                    }))
                    .when(be.action == "open_url", |d| d.child(caption("超链接地址")).child(field(&be.url, "btn-url")))
                    .child(
                        div()
                            .pt(px(14.))
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.))
                            .child(button("btn-cancel", "取消", false).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.button_cancel(window, cx);
                                }),
                            ))
                            .child(button("btn-ok", "确定", true).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.button_confirm(window, cx);
                                }),
                            )),
                    )
                    // 下拉列表作为最后一个子元素，才能盖住下面的「超链接地址」
                    .children(menu)
                    .child({
                        let at = be.pop_at.clone();
                        canvas(move |b, _, _| at.set((b.origin.x.into(), b.origin.y.into())), |_, _, _, _| {}).absolute().top_0().left_0().size_full()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            // 点浮层空白处收起执行操作下拉
                            if let Some(be) = this.pop.button_edit.as_mut().filter(|be| be.action_menu) {
                                be.action_menu = false;
                                cx.notify();
                            }
                        }),
                    )
                    // 点浮层外面视为确定。推迟到本次点击分发完再关：点在按钮本身上时它仍看到浮层开着，不会顺手执行动作
                    .on_mouse_down_out(cx.listener(|_, _, window, cx| cx.defer_in(window, |this, window, cx| this.button_confirm(window, cx)))),
            ),
        )
        .with_priority(4)
    }
}
