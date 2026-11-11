//! v2 新增块的渲染与交互：文件、目录、公式、文本绘图、HTML、网页卡片、视频、嵌入对象。

use super::render::{content_w, SlotSpec};
use super::*;
use crate::canvas::paint::{paint_scene, View};
use crate::canvas::{Measure, Scene};
use crate::model::{Block, BlockKind};
use crate::theme::{theme, Theme};
use crate::ui::icon;

/// 文本绘图控件内容区的最大高度（并排 / 仅预览）；上下排列时源码区另有上限。
const DIAGRAM_MAX_H: f32 = 480.;
const DIAGRAM_SRC_MAX_H: f32 = 300.;
use gpui::prelude::FluentBuilder;
use std::path::PathBuf;

const HTML_FILE_MAX_H: f32 = 640.;

fn is_html_file(p: &std::path::Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("html" | "htm"))
}

/// 用 GPUI 文字系统精确测量（图表布局用）。
pub struct GpuiMeasure<'a> {
    pub window: &'a Window,
}

impl Measure for GpuiMeasure<'_> {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        if text.is_empty() {
            return 0.;
        }
        let mut f = font(crate::canvas::paint::FONT);
        if bold {
            f.weight = FontWeight::SEMIBOLD;
        }
        let run = TextRun { len: text.len(), font: f, color: gpui::black(), background_color: None, underline: None, strikethrough: None };
        f32::from(self.window.text_system().shape_line(text.to_string().into(), px(size), &[run], None).width)
    }
}

/// 视频封面信息：(封面 PNG 路径, 宽, 高, 时长)
type VideoMeta = Option<(String, u32, u32, f64)>;

thread_local! {
    /// `None` 表示正在后台生成
    static VIDEO_META: RefCell<HashMap<String, Option<VideoMeta>>> = RefCell::new(HashMap::new());
}
static VIDEO_DONE: std::sync::Mutex<Vec<(String, VideoMeta)>> = std::sync::Mutex::new(Vec::new());

/// 视频封面与尺寸。首次请求时在后台线程抽帧（解码视频很慢，不能放在渲染里），完成前返回 `None` 并请求下一帧。
pub fn video_meta(path: &str, window: &Window) -> VideoMeta {
    VIDEO_META.with(|m| {
        let mut m = m.borrow_mut();
        for (p, meta) in std::mem::take(&mut *VIDEO_DONE.lock().unwrap_or_else(|e| e.into_inner())) {
            m.insert(p, Some(meta));
        }
        match m.get(path) {
            Some(Some(meta)) => return meta.clone(),
            Some(None) => {}
            None => {
                m.insert(path.to_string(), None);
                let p = path.to_string();
                std::thread::spawn(move || {
                    let meta = crate::media::poster(&p, &crate::webview::cache_dir());
                    VIDEO_DONE.lock().unwrap_or_else(|e| e.into_inner()).push((p, meta));
                });
            }
        }
        window.request_animation_frame();
        None
    })
}

impl Editor {
    pub(super) fn render_v2(&mut self, ix: usize, blk: &Block, focused: bool, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let selected = focused && self.block_selected(ix);
        let select_on_click = cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
            window.focus(&this.focus);
            this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
        });
        match &blk.kind {
            BlockKind::File { .. } | BlockKind::Video { .. } => self.render_attachment(ix, blk, focused, t, window, cx),
            BlockKind::Toc => {
                let heads: Vec<(usize, u8, String)> = self
                    .content
                    .blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| match b.kind {
                        BlockKind::Heading { level } if level <= 3 && !b.text.is_empty() => Some((i, level, b.text.text.replace('\n', " "))),
                        _ => None,
                    })
                    .collect();
                div()
                    .relative()
                    .w_full()
                    .py_2()
                    .px_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if selected { t.accent } else { t.border })
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pb_1()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(icon("outline", 13., t.text_muted))
                            .child(crate::i18n::tr("目录")),
                    )
                    .when(heads.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(t.placeholder).child(crate::i18n::tr("文档中还没有标题"))))
                    .children(heads.into_iter().map(|(i, level, text)| {
                        div()
                            .id(("toc", i))
                            .pl(px((level as f32 - 1.) * 18. + 6.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .text_size(px(if level == 1 { 14.5 } else { 14. }))
                            .text_color(if level == 1 { t.text } else { t.text_secondary })
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover).text_color(t.accent))
                            .child(text)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.scroll_to_block(i, cx);
                                }),
                            )
                    }))
                    .child(self.void_marker(ix))
                    .on_mouse_down(MouseButton::Left, select_on_click)
                    .into_any_element()
            }
            BlockKind::Countdown { .. } => self.render_countdown(ix, blk, selected, t, window, cx),
            BlockKind::Button { .. } => self.render_button(ix, blk, selected, t, window, cx),
            BlockKind::Timeline { .. } => self.render_timeline(ix, blk, focused, t, window, cx),
            BlockKind::Math => self.render_math_block(ix, blk, focused, t, cx),
            BlockKind::Diagram { .. } => self.render_diagram(ix, blk, focused, t, window, cx),
            BlockKind::Html { scripts, file, network } => self.render_html(ix, blk, *scripts, file, *network, focused, t, cx),
            BlockKind::LinkCard { url } => self.render_link_card(ix, blk, url, selected, t, cx),
            BlockKind::Embed { kind, id } => self.render_embed(ix, blk, kind, id, selected, t, window, cx),
            _ => div()
                .relative()
                .w_full()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .bg(t.hover)
                .text_size(px(13.))
                .text_color(t.text_muted)
                .child(crate::i18n::tr("不支持的块类型（由更新版本的石渠创建），内容已原样保留"))
                .child(self.void_marker(ix))
                .on_mouse_down(MouseButton::Left, select_on_click)
                .into_any_element(),
        }
    }

    pub(super) fn update_kind(&mut self, ix: usize, f: impl FnOnce(&mut BlockKind), cx: &mut Context<Self>) {
        self.checkpoint(EditKind::Other);
        f(&mut self.block_mut(ix).kind);
        self.changed(cx);
    }

    /// 打开块属性编辑浮层。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_prop_edit(
        &mut self,
        ix: usize,
        field: &'static str,
        value: &str,
        placeholder: &str,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| LineInput::new(placeholder, cx));
        input.update(cx, |i, cx| i.set_text(value, cx));
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &crate::line_input::InputEvent, window, cx| match ev {
            crate::line_input::InputEvent::Submit { .. } => {
                let v = input.read(cx).text();
                this.apply_prop(ix, field, v.trim(), cx);
                this.pop.prop_edit = None;
                window.focus(&this.focus);
                cx.notify();
            }
            crate::line_input::InputEvent::Cancel => {
                this.pop.prop_edit = None;
                window.focus(&this.focus);
                cx.notify();
            }
            _ => {}
        });
        window.focus(&input.focus_handle(cx));
        self.pop.prop_edit = Some(PropEdit { input, at, _sub: sub });
        cx.notify();
    }

    fn apply_prop(&mut self, ix: usize, field: &str, v: &str, cx: &mut Context<Self>) {
        match field {
            "inline_math" => {
                let Some((b, c, start)) = self.sess.inline_math_at.take() else { return };
                self.checkpoint(EditKind::Other);
                let slot = self.slot_mut((b, c));
                if start + 3 <= slot.len() {
                    if v.is_empty() {
                        slot.delete(start..start + 3);
                    } else {
                        let tex = v.to_string();
                        slot.update_marks(start..start + 3, |m| m.math = Some(tex.clone()));
                    }
                }
                self.core.head = self.clamp(Pos::new(b, c, start + if v.is_empty() { 0 } else { 3 }));
                self.core.anchor = self.core.head;
                self.changed(cx);
            }
            "insert_link" => {
                if v.is_empty() {
                    return;
                }
                let url = if v.contains("://") || v.starts_with("mailto:") { v.to_string() } else { format!("https://{v}") };
                self.checkpoint(EditKind::Other);
                let h = self.core.head;
                let rt = crate::model::RichText::styled_link(&url, &url);
                self.slot_mut(h.slot()).insert_rich(h.offset, &rt);
                self.core.head.offset += rt.len();
                self.core.anchor = self.core.head;
                self.changed(cx);
            }
            "link_url" => {
                let url = if v.contains("://") { v.to_string() } else { format!("https://{v}") };
                self.update_kind(
                    ix,
                    |k| {
                        if let BlockKind::LinkCard { url: u } = k {
                            *u = url;
                        }
                    },
                    cx,
                );
            }
            "image_alt" => {
                self.checkpoint(EditKind::Other);
                if let BlockKind::Image { alt, .. } = &mut self.block_mut(ix).kind {
                    *alt = v.to_string();
                }
                self.changed(cx);
            }
            _ => {}
        }
    }

    // ───────────── 公式 ─────────────

    fn render_math_block(&mut self, ix: usize, blk: &Block, focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let src = blk.text.text.clone();
        let editing = focused && self.core.head.block == ix;
        let state = if src.trim().is_empty() { None } else { Some(crate::math::request(&src, true, t.dark, cx)) };
        let preview: AnyElement = match state {
            None => div().text_size(px(13.)).text_color(t.placeholder).child(crate::i18n::tr("在下方输入 LaTeX 公式，例如 E = mc^2")).into_any_element(),
            Some(crate::math::MathState::Pending) => div().text_size(px(13.)).text_color(t.text_muted).child(crate::i18n::tr("公式渲染中…")).into_any_element(),
            Some(crate::math::MathState::Error(e)) => div().text_size(px(13.)).text_color(t.fg_palette[1]).child(format!("公式有误：{e}")).into_any_element(),
            Some(crate::math::MathState::Ready { path, width, height }) => {
                let maxw = content_w() - 40.;
                let s = (maxw / width).min(1.);
                img(path).w(px(width * s)).h(px(height * s)).into_any_element()
            }
        };
        div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .py_2()
                    .rounded(px(6.))
                    .flex()
                    .justify_center()
                    .when(editing, |d| d.bg(t.accent_soft))
                    .hover(|s| s.bg(t.hover))
                    .cursor_pointer()
                    .child(preview)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            window.focus(&this.focus);
                            let len = this.slot_len((ix, 0));
                            this.set_cursor(Pos::new(ix, 0, len), false, cx);
                        }),
                    ),
            )
            .when(editing || src.trim().is_empty(), |d| {
                let spec = SlotSpec {
                    size: 13.5,
                    line_h: 22.,
                    mono: true,
                    lang: Some("LaTeX".into()),
                    placeholder: Some("\\int_0^1 x^2 dx = \\frac{1}{3}".into()),
                    ..SlotSpec::body(t)
                };
                d.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(t.code_bg)
                        .border_1()
                        .border_color(t.border)
                        .child(div().text_size(px(11.)).text_color(t.text_muted).pb_1().child(crate::i18n::tr("LaTeX · 支持 KaTeX 全部语法")))
                        .child(self.render_slot(ix, 0, &blk.text, spec, focused, t))
                        .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
                )
            })
            .into_any_element()
    }

    // ───────────── 文本绘图 ─────────────

    pub(super) fn diagram_scene(&self, blk: &Block, dark: bool, window: &Window) -> (Option<Arc<Scene>>, Option<crate::diagram::DiagramError>) {
        let BlockKind::Diagram { offsets, .. } = &blk.kind else { return (None, None) };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            blk.text.text.hash(&mut h);
            dark.hash(&mut h);
            for (k, (x, y)) in offsets {
                k.hash(&mut h);
                x.to_bits().hash(&mut h);
                y.to_bits().hash(&mut h);
            }
            h.finish()
        };
        let cached = self.caches.diagram_cache.borrow().get(&key).cloned();
        let result = match cached {
            Some(r) => r,
            None => {
                let r = crate::diagram::render(&blk.text.text, dark, None, &mut GpuiMeasure { window }, offsets).map(Arc::new);
                let mut c = self.caches.diagram_cache.borrow_mut();
                if c.len() > 300 {
                    c.clear();
                }
                c.insert(key, r.clone());
                r
            }
        };
        match result {
            Ok(s) => {
                self.caches.diagram_good.borrow_mut().insert(blk.id, s.clone());
                (Some(s), None)
            }
            Err(e) => (self.caches.diagram_good.borrow().get(&blk.id).cloned(), Some(e)),
        }
    }

    fn render_diagram(&mut self, ix: usize, blk: &Block, focused: bool, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let BlockKind::Diagram { view, offsets } = &blk.kind else { return div().into_any_element() };
        let view_mode = *view;
        let editing = focused && self.core.head.block == ix;
        let (scene, err) = self.diagram_scene(blk, t.dark, window);
        let kind = crate::diagram::detect(&blk.text.text);
        let show_src = view_mode == 2 || view_mode == 3 || (view_mode == 0 && editing) || scene.is_none() && err.is_none();
        let show_preview = view_mode != 2;
        let split = show_src && show_preview && content_w() >= 700.;
        let avail = if split { content_w() * 0.55 } else { content_w() - (blk.indent as f32 * 26.) - 4. };
        // 整个控件限高：预览图按比例缩小塞进去，源码超出时在框内滚动
        let max_h = if split || !show_src { DIAGRAM_MAX_H } else { DIAGRAM_MAX_H * 0.75 };
        let src_max_h = if split { DIAGRAM_MAX_H } else { DIAGRAM_SRC_MAX_H };
        let frame = self.frame.clone();
        let preview: Option<AnyElement> = scene.clone().filter(|_| show_preview).map(|scene| {
            // 保留 min/max 链：两个比值都为 NaN 时 clamp 会返回 NaN，这里要落到 1.0
            #[allow(clippy::manual_clamp)]
            let scale = (avail / scene.width).min(max_h / scene.height).min(1.0).max(0.05);
            let h = scene.height * scale;
            let draggable = kind.is_some_and(|k| k.draggable());
            let sc2 = scene.clone();
            div()
                .id(("dg-pv", blk.id as usize))
                .relative()
                .w(px(scene.width * scale))
                .h(px(h))
                .flex_none()
                .child(
                    canvas(
                        move |bounds, _, _| {
                            let v = View { origin: bounds.origin, scale };
                            frame.borrow_mut().diagrams.insert(ix, (v, sc2.clone()));
                            v
                        },
                        move |bounds, v, window, cx| paint_scene(&scene, v, bounds, window, cx),
                    )
                    .size_full(),
                )
                .cursor(if draggable { CursorStyle::PointingHand } else { CursorStyle::Arrow })
                .tooltip(move |_, cx| {
                    cx.new(|_| {
                        crate::ui::Tooltip(
                            if draggable {
                                "单击放大查看 · 双击编辑源码 · 拖动节点微调位置"
                            } else {
                                "单击放大查看 · 双击编辑源码"
                            }
                            .into(),
                        )
                    })
                    .into()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus);
                        this.pop.diagram_theme_menu = None;
                        if ev.click_count >= 2 {
                            // 双击：取消单击排下的弹框，进入源码编辑
                            this.sess.diagram_click = None;
                            this.diagram_click_epoch = this.diagram_click_epoch.wrapping_add(1);
                            let len = this.slot_len((ix, 0));
                            this.set_cursor(Pos::new(ix, 0, len), false, cx);
                            return;
                        }
                        this.sess.diagram_click = Some((ix, ev.position));
                        if draggable {
                            this.diagram_drag_start(ix, ev.position, cx);
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                        let Some((b, p)) = this.sess.diagram_click.take() else { return };
                        let (dx, dy) = (f32::from(ev.position.x - p.x), f32::from(ev.position.y - p.y));
                        if b == ix && dx * dx + dy * dy < 16. {
                            this.schedule_diagram_preview(ix, cx);
                        }
                    }),
                )
                .into_any_element()
        });
        let err_el = err.map(|e| {
            div().px_3().py_1().rounded(px(6.)).bg(t.bg_palette[1]).text_size(px(12.)).text_color(t.fg_palette[1]).child(if e.line > 0 {
                format!("第 {} 行：{}", e.line, e.msg)
            } else {
                e.msg
            })
        });
        let src_el = show_src.then(|| {
            let spec = SlotSpec {
                size: 13.,
                line_h: 21.,
                mono: true,
                lang: Some("Mermaid".into()),
                placeholder: Some("flowchart LR\n  A[开始] --> B{判断}".into()),
                ..SlotSpec::body(t)
            };
            let (handle, drag) = self.caches.diagram_scroll.borrow_mut().entry(blk.id).or_insert_with(|| (ScrollHandle::new(), Default::default())).clone();
            div()
                .relative()
                .flex_1()
                .min_w(px(0.))
                .rounded(px(6.))
                .bg(t.code_bg)
                .border_1()
                .border_color(t.border)
                .child(
                    div()
                        .id(("dg-src", blk.id as usize))
                        .max_h(px(src_max_h))
                        .overflow_y_scroll()
                        .track_scroll(&handle)
                        .px_3()
                        .py_2()
                        .child(self.render_slot(ix, 0, &blk.text, spec, focused, t))
                        .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
                )
                .child(crate::ui::scrollbar(handle, drag, t.text_muted))
        });
        let has_offsets = !offsets.is_empty();
        let kind_label = kind.map(|k| k.label()).unwrap_or("文本绘图");
        let is_flow = kind == Some(crate::diagram::Kind::Flowchart);
        let menu_open = self.pop.diagram_theme_menu == Some(ix);
        let tb_btn = |id: &'static str, label: String, on: bool| {
            div()
                .id((id, blk.id as usize))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(if on { t.accent } else { t.text_secondary })
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .child(crate::i18n::tr_owned(&label))
        };
        let theme_btn = div()
            .relative()
            .child(
                tb_btn("dg-theme", String::new(), menu_open)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(icon("palette", 13., if menu_open { t.accent } else { t.text_secondary }))
                    .child(crate::i18n::tr("配色"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pop.diagram_theme_menu = if this.pop.diagram_theme_menu == Some(ix) { None } else { Some(ix) };
                            cx.notify();
                        }),
                    ),
            )
            .when(menu_open, |d| d.child(div().absolute().top(px(24.)).left_0().child(self.render_diagram_theme_menu(ix, &blk.text.text, t, cx))));
        let toolbar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .when(!editing && !menu_open, |d| d.invisible().group_hover("dg", |s| s.visible()))
            .child(div().pr_2().text_size(px(12.)).text_color(t.text_muted).child(kind_label))
            .child(tb_btn("dg-v0", "自动".into(), view_mode == 0).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_diagram_view(ix, 0, cx)
                }),
            ))
            .child(tb_btn("dg-v1", "仅预览".into(), view_mode == 1).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_diagram_view(ix, 1, cx)
                }),
            ))
            .child(tb_btn("dg-v2", "仅源码".into(), view_mode == 2).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_diagram_view(ix, 2, cx)
                }),
            ))
            .child(tb_btn("dg-v3", "并排".into(), view_mode == 3).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_diagram_view(ix, 3, cx)
                }),
            ))
            .child(div().w(px(1.)).h(px(14.)).mx_1().bg(t.border))
            .child(theme_btn)
            .when(scene.is_some(), |d| {
                d.child(tb_btn("dg-zoom", "放大查看".into(), false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_diagram_preview(ix, cx)
                    }),
                ))
            })
            .child(tb_btn("dg-png", "导出 PNG".into(), false).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.export_diagram(ix, true, cx)
                }),
            ))
            .child(tb_btn("dg-svg", "导出 SVG".into(), false).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.export_diagram(ix, false, cx)
                }),
            ))
            .when(is_flow, |d| {
                d.child(tb_btn("dg-flow", "转为可编辑流程图".into(), false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.diagram_to_board(ix, cx)
                    }),
                ))
            })
            .when(has_offsets, |d| {
                d.child(tb_btn("dg-reset", "重置位置".into(), false).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.update_kind(
                            ix,
                            |k| {
                                if let BlockKind::Diagram { offsets, .. } = k {
                                    offsets.clear();
                                }
                            },
                            cx,
                        );
                    }),
                ))
            });
        let body = if split {
            div().flex().flex_row().gap_3().items_start().children(src_el).children(preview.map(|p| div().flex_none().child(p))).into_any_element()
        } else {
            div().flex().flex_col().gap_2().children(preview.map(|p| div().w_full().flex().justify_center().child(p))).children(src_el).into_any_element()
        };
        div()
            .group("dg")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(if editing { t.accent } else { gpui::transparent_black() })
            .hover(|s| s.border_color(t.border))
            .child(toolbar)
            .child(body)
            .children(err_el)
            .into_any_element()
    }

    fn set_diagram_view(&mut self, ix: usize, v: u8, cx: &mut Context<Self>) {
        self.update_kind(
            ix,
            |k| {
                if let BlockKind::Diagram { view, .. } = k {
                    *view = v;
                }
            },
            cx,
        );
    }

    /// 配色下拉菜单（默认 / 灰色 / 绿色 / 黄色……，当前项打勾）。
    fn render_diagram_theme_menu(&self, ix: usize, src: &str, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, cur) = crate::diagram::preprocess(src);
        let cur = cur.unwrap_or_else(|| "default".into());
        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("dg-theme-menu")
                    .occlude()
                    .w(px(132.))
                    .p(px(4.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .text_size(px(13.))
                    .children(crate::diagram::THEMES.iter().enumerate().map(|(i, (name, label))| {
                        let on = cur == *name;
                        let swatch = crate::diagram::Theme::by_name(name, false);
                        div()
                            .id(("dg-theme-it", i))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py(px(6.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .when(on, |d| d.bg(t.hover))
                            .child(div().size(px(14.)).rounded(px(3.)).border_1().border_color(swatch.node_stroke.hsla()).bg(swatch.node_fill.hsla()))
                            .child(div().flex_1().text_color(if on { t.accent } else { t.text }).child(*label))
                            .when(on, |d| d.child(icon("check", 14., t.accent)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.set_diagram_theme(ix, i, cx);
                                }),
                            )
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.pop.diagram_theme_menu = None;
                        cx.notify();
                    })),
            ),
        )
        .with_priority(3)
    }

    /// 通过源码首行的 init 指令设置配色（THEMES 的第 pos 项）。
    fn set_diagram_theme(&mut self, ix: usize, pos: usize, cx: &mut Context<Self>) {
        self.pop.diagram_theme_menu = None;
        let src = self.content.blocks[ix].text.text.clone();
        let next = crate::diagram::THEMES[pos].0;
        if crate::diagram::preprocess(&src).1.as_deref().unwrap_or("default") == next {
            cx.notify();
            return;
        }
        let body: Vec<&str> = src.lines().filter(|l| !l.trim_start().starts_with("%%{")).collect();
        let mut out = String::new();
        if next != "default" {
            out.push_str(&format!("%%{{init: {{\"theme\": \"{next}\"}}}}%%\n"));
        }
        out.push_str(&body.join("\n"));
        self.checkpoint(EditKind::Other);
        self.block_mut(ix).text = crate::model::RichText::plain(out);
        self.core.head = self.clamp(self.core.head);
        self.core.anchor = self.clamp(self.core.anchor);
        self.changed(cx);
    }

    /// 单击预览图：稍等一下再弹框，期间若变成双击（编辑源码）就作废。
    fn schedule_diagram_preview(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.diagram_click_epoch = self.diagram_click_epoch.wrapping_add(1);
        let epoch = self.diagram_click_epoch;
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(280)).await;
            this.update(cx, |this, cx| {
                if this.diagram_click_epoch == epoch {
                    this.open_diagram_preview(ix, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn open_diagram_preview(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(b) = self.content.blocks.get(ix).filter(|b| matches!(b.kind, BlockKind::Diagram { .. })) {
            self.sess.diagram_preview = Some(b.id);
            self.sess.diagram_preview_zoom = 1.;
            cx.notify();
        }
    }

    fn close_diagram_preview(&mut self, cx: &mut Context<Self>) {
        self.sess.diagram_preview = None;
        cx.notify();
    }

    /// 弹框查看大图：暗色遮罩 + 居中面板，图按窗口等比放大；可缩放，点遮罩或按 Esc 关闭。
    pub(super) fn render_diagram_preview(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.sess.diagram_preview?;
        let scene = self.caches.diagram_good.borrow().get(&id).cloned()?;
        let vp = window.viewport_size();
        let (pw, ph) = (f32::from(vp.width) * 0.9, f32::from(vp.height) * 0.9);
        let (cw, ch) = (pw - 48., ph - 96.);
        let fit = (cw / scene.width).min(ch / scene.height).min(2.5);
        let zoom = self.sess.diagram_preview_zoom;
        let scale = fit * zoom;
        let title = self.content.blocks.iter().find(|b| b.id == id).and_then(|b| crate::diagram::detect(&b.text.text)).map(|k| k.label()).unwrap_or("文本绘图");
        let zbtn = |id: &'static str, name: &'static str, tip: &'static str| crate::ui::icon_button(id, name, tip, cx);
        let sc = scene.clone();
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id("dg-preview")
                        .w(vp.width)
                        .h(vp.height)
                        .occlude()
                        .bg(gpui::black().opacity(0.6))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_diagram_preview(cx);
                            }),
                        )
                        .child(
                            div()
                                .id("dg-preview-panel")
                                .w(px(pw))
                                .h(px(ph))
                                .flex()
                                .flex_col()
                                .bg(t.bg)
                                .rounded(px(10.))
                                .shadow_lg()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_1()
                                        .px_4()
                                        .h(px(48.))
                                        .flex_none()
                                        .border_b_1()
                                        .border_color(t.border)
                                        .child(div().flex_1().text_size(px(14.)).text_color(t.text).child(title))
                                        .child(zbtn("dg-pz-out", "zoom-out", "缩小").on_click(cx.listener(|this, _, _, cx| {
                                            this.sess.diagram_preview_zoom = (this.sess.diagram_preview_zoom / 1.25).max(0.2);
                                            cx.notify();
                                        })))
                                        .child(
                                            div()
                                                .w(px(44.))
                                                .text_center()
                                                .text_size(px(12.))
                                                .text_color(t.text_secondary)
                                                .child(format!("{:.0}%", scale * 100.)),
                                        )
                                        .child(zbtn("dg-pz-in", "zoom-in", "放大").on_click(cx.listener(|this, _, _, cx| {
                                            this.sess.diagram_preview_zoom = (this.sess.diagram_preview_zoom * 1.25).min(8.);
                                            cx.notify();
                                        })))
                                        .child(zbtn("dg-pz-fit", "maximize", "适应窗口").on_click(cx.listener(|this, _, _, cx| {
                                            this.sess.diagram_preview_zoom = 1.;
                                            cx.notify();
                                        })))
                                        .child(div().w(px(1.)).h(px(16.)).mx_1().bg(t.border))
                                        .child(zbtn("dg-pz-close", "x", "关闭（Esc）").on_click(cx.listener(|this, _, _, cx| this.close_diagram_preview(cx)))),
                                )
                                .child(
                                    div().id("dg-preview-scroll").flex_1().min_h(px(0.)).overflow_scroll().p_6().child(
                                        // 图比视口小时居中，大时从左上角开始可滚动
                                        div().min_w_full().min_h_full().flex().items_center().justify_center().child(
                                            canvas(
                                                move |bounds, _, _| View { origin: bounds.origin, scale },
                                                move |bounds, v, window, cx| paint_scene(&sc, v, bounds, window, cx),
                                            )
                                            .w(px(scene.width * scale))
                                            .h(px(scene.height * scale))
                                            .flex_none(),
                                        ),
                                    ),
                                ),
                        ),
                ),
            )
            .with_priority(9)
            .into_any_element(),
        )
    }

    fn export_diagram(&mut self, ix: usize, png: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.caches.diagram_good.borrow().get(&self.content.blocks[ix].id).cloned() else { return };
        let mut s = (*scene).clone();
        s.bg = Some(if self.content.blocks.is_empty() { crate::canvas::Col::rgb(0xffffff) } else { crate::canvas::Col::from_hsla(theme(cx).bg) });
        let svg = crate::canvas::svg::to_svg(&s);
        let name = format!("图表.{}", if png { "png" } else { "svg" });
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = if png {
                    crate::canvas::svg::svg_to_png(&svg, 2.).is_some_and(|b| std::fs::write(&path, b).is_ok())
                } else {
                    std::fs::write(&path, svg).is_ok()
                };
                this.update(cx, |_, cx| {
                    cx.emit(EditorEvent::Toast(if ok { format!("已导出 {}", path.display()) } else { "导出失败".into() }));
                    if ok {
                        cx.reveal_path(&path);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// 测试脚本用：把第 ix 块的 Mermaid 流程图转为可编辑流程图。
    #[cfg(feature = "dev")]
    pub fn test_diagram_to_board(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.diagram_to_board(ix, cx);
    }

    fn diagram_to_board(&mut self, ix: usize, cx: &mut Context<Self>) {
        let src = self.content.blocks[ix].text.text.clone();
        let Some(board) = crate::board::from_mermaid(&src, theme(cx).dark) else {
            cx.emit(EditorEvent::Toast("只有流程图可以转换".into()));
            return;
        };
        let id = crate::objects::create(crate::objects::ObjKind::Flow, Some(crate::objects::ObjData::Board(Arc::new(board))));
        self.checkpoint(EditKind::Other);
        self.insert_blocks(ix + 1, vec![Block::new(BlockKind::Embed { kind: "flow".into(), id: id.clone() }, crate::model::RichText::default())]);
        self.core.head = Pos::new(ix + 1, 0, 0);
        self.core.anchor = self.core.head;
        self.changed(cx);
        cx.emit(EditorEvent::OpenEmbed { kind: "flow".into(), id });
    }

    fn diagram_drag_start(&mut self, ix: usize, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((view, scene)) = self.frame.borrow().diagrams.get(&ix).cloned() else { return };
        let p = view.inv(pos);
        let Some(shape) = scene.hit_shape(p) else { return };
        let id = shape.id.clone();
        let base = match &self.content.blocks[ix].kind {
            BlockKind::Diagram { offsets, .. } => offsets.get(&id).copied().unwrap_or((0., 0.)),
            _ => return,
        };
        self.sess.diagram_drag = Some(DiagramDrag { block: ix, node: id, start: p, base, moved: false });
        cx.notify();
    }

    pub(super) fn diagram_drag_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(d) = &self.sess.diagram_drag else { return false };
        let Some((view, _)) = self.frame.borrow().diagrams.get(&d.block).cloned() else { return true };
        let p = view.inv(pos);
        if !d.moved {
            // 按下后没动就是单击（弹框预览），只有真正拖动才记撤销点
            if (p.x - d.start.x).abs() < 3. && (p.y - d.start.y).abs() < 3. {
                return true;
            }
            self.checkpoint(EditKind::Other);
        }
        let Some(d) = &mut self.sess.diagram_drag else { return false };
        d.moved = true;
        let (dx, dy) = (d.base.0 + p.x - d.start.x, d.base.1 + p.y - d.start.y);
        let (b, node) = (d.block, d.node.clone());
        if let BlockKind::Diagram { offsets, .. } = &mut self.block_mut(b).kind {
            offsets.insert(node, ((dx * 2.).round() / 2., (dy * 2.).round() / 2.));
        }
        self.version += 1;
        self.dirty = true;
        cx.notify();
        true
    }

    pub(super) fn diagram_drag_end(&mut self, cx: &mut Context<Self>) {
        if self.sess.diagram_drag.take().is_some_and(|d| d.moved) {
            self.changed(cx);
        }
    }

    // ───────────── HTML ─────────────

    fn render_html(
        &mut self,
        ix: usize,
        blk: &Block,
        scripts: bool,
        file: &str,
        network: bool,
        focused: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let drop_html = cx.listener(move |this, paths: &ExternalPaths, _, cx| {
            cx.stop_propagation();
            match paths.paths().iter().find(|p| is_html_file(p)) {
                Some(p) => this.set_html_file(ix, p.clone(), cx),
                None => this.insert_files(paths.paths().to_vec(), cx),
            }
        });
        if !file.is_empty() {
            return self.render_html_file(ix, blk, file, network, t, cx).on_drop(drop_html).into_any_element();
        }
        if blk.text.text.trim().is_empty() {
            return self.render_html_dropzone(ix, blk, t, cx).on_drop(drop_html).into_any_element();
        }
        self.render_html_source(ix, blk, scripts, focused, t, cx)
    }

    /// 空 HTML 块：拖拽 / 点击上传整页 HTML 文件。
    fn render_html_dropzone(&mut self, ix: usize, blk: &Block, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(("html-drop", blk.id as usize))
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(10.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(crate::i18n::tr("HTML 块")))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(t.text_secondary)
                    .child(crate::i18n::tr("上传整个静态 HTML 页面，在文档中原地渲染；也可以手写 HTML 源码。")),
            )
            .child(
                div()
                    .id(("html-pick", blk.id as usize))
                    .h(px(150.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_dashed()
                    .border_color(t.accent)
                    .bg(t.accent_soft)
                    .cursor_pointer()
                    .drag_over::<ExternalPaths>(|s, _, _, _| s.border_2())
                    .child(icon("upload", 28., t.accent))
                    .child(div().text_size(px(13.)).text_color(t.text_secondary).child(crate::i18n::tr("拖拽 HTML 文件到这里，或点击上传")))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick_html_file(ix, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id(("html-write", blk.id as usize))
                    .text_size(px(12.))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .child(crate::i18n::tr("或手写 HTML 源码"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.checkpoint(EditKind::Other);
                            this.block_mut(ix).text = crate::model::RichText::plain(super::ops::HTML_TPL);
                            this.set_cursor(Pos::new(ix, 0, 0), false, cx);
                            this.changed(cx);
                        }),
                    ),
            )
    }

    /// 上传的整页 HTML：默认显示首屏快照，点击后切换为可交互的真实 WebView。
    fn render_html_file(&mut self, ix: usize, blk: &Block, file: &str, network: bool, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let width = content_w() - blk.indent as f32 * 26.;
        let live = self.sess.html_live == Some(blk.id);
        let path = PathBuf::from(file);
        let state = if path.exists() { Some(crate::webview::request_file(&path, network, width, HTML_FILE_MAX_H, cx)) } else { None };
        let frame = self.frame.clone();
        let bid = blk.id;
        let h = match &state {
            Some(crate::webview::SnapState::Ready { height, .. }) => *height,
            _ => 240.,
        };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let preview: AnyElement = if live {
            // 原生 WebView 滚出视口被隐藏时，底下仍显示快照，不会出现空白
            let under = match &state {
                Some(crate::webview::SnapState::Ready { path, .. }) => Some(path.clone()),
                _ => None,
            };
            div()
                .relative()
                .w(px(width))
                .h(px(h))
                .when_some(under, |d, p| d.child(img(p).absolute().top_0().left_0().w(px(width)).h(px(h))))
                .child(
                    canvas(
                        move |b, _, _| {
                            frame.borrow_mut().html_bounds.insert(bid, b);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .into_any_element()
        } else {
            match state {
                None => div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.fg_palette[1])
                    .child(format!("文件不存在：{name}"))
                    .into_any_element(),
                Some(crate::webview::SnapState::Pending) => div()
                    .h(px(240.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child(crate::i18n::tr("正在渲染…"))
                    .into_any_element(),
                Some(crate::webview::SnapState::Failed) => div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.fg_palette[1])
                    .child(crate::i18n::tr("渲染失败"))
                    .into_any_element(),
                Some(crate::webview::SnapState::Ready { path, width, height }) => img(path).w(px(width)).h(px(height)).into_any_element(),
            }
        };
        let btn = |id: &'static str, label: String, on: bool| {
            div()
                .id((id, blk.id as usize))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(if on { t.accent } else { t.text_secondary })
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .child(crate::i18n::tr_owned(&label))
        };
        let bar = div()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .invisible()
            .group_hover("html", |s| s.visible())
            .flex()
            .flex_row()
            .p(px(3.))
            .rounded(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .child(btn("html-live", if live { "退出交互".into() } else { "交互".into() }, live).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if this.sess.html_live == Some(bid) {
                        this.sess.html_live = None;
                        crate::webview::hide_live();
                    } else {
                        this.sess.html_live = Some(bid);
                    }
                    cx.notify();
                }),
            ))
            .child(btn("html-net", if network { "联网：已允许".into() } else { "联网：已禁用".into() }, network).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.update_kind(
                        ix,
                        |k| {
                            if let BlockKind::Html { network, .. } = k {
                                *network = !*network;
                            }
                        },
                        cx,
                    );
                    crate::webview::hide_live();
                }),
            ))
            .child(btn("html-replace", "换文件".into(), false).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.pick_html_file(ix, cx);
                }),
            ))
            .child(btn("html-open", "浏览器打开".into(), false).on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let f = file.to_string();
                    move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(&format!("file://{f}"));
                    }
                }),
            ));
        div()
            .id(("html-file", blk.id as usize))
            .group("html")
            .relative()
            .w_full()
            .rounded(px(8.))
            .border_1()
            .border_color(if live { t.accent } else { t.border })
            .overflow_hidden()
            .child(preview)
            .child(bar)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus);
                    if this.core.head.block != ix {
                        this.set_cursor(Pos::new(ix, 0, 0), false, cx);
                    }
                    if this.sess.html_live != Some(bid) {
                        this.sess.html_live = Some(bid);
                    }
                    cx.notify();
                }),
            )
    }

    fn pick_html_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 HTML 文件".into()),
            ..Default::default()
        });
        cx.spawn(async move |this: WeakEntity<Editor>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().find(|p| is_html_file(p)) {
                    this.update(cx, |this, cx| this.set_html_file(ix, p, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub fn set_html_file(&mut self, ix: usize, src: PathBuf, cx: &mut Context<Self>) {
        let Some(dst) = super::clip::copy_asset(&src, &self.store.assets_dir()) else {
            cx.emit(EditorEvent::Toast("无法读取该文件".into()));
            return;
        };
        self.checkpoint(EditKind::Other);
        let b = self.block_mut(ix);
        b.text = crate::model::RichText::default();
        if let BlockKind::Html { file, .. } = &mut b.kind {
            *file = dst;
        }
        self.sess.html_live = None;
        crate::webview::hide_live();
        self.changed(cx);
    }

    fn render_html_source(&mut self, ix: usize, blk: &Block, scripts: bool, focused: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let editing = focused && self.core.head.block == ix;
        let src = blk.text.text.clone();
        let width = content_w() - blk.indent as f32 * 26. - 20.;
        let live = self.sess.html_live == Some(blk.id);
        let state = if src.trim().is_empty() { None } else { Some(crate::webview::request_html(&src, scripts, t.dark, width, cx)) };
        let frame = self.frame.clone();
        let bid = blk.id;
        let preview: AnyElement = if live {
            let h = match &state {
                Some(crate::webview::SnapState::Ready { height, .. }) => *height,
                _ => 300.,
            };
            div()
                .w(px(width))
                .h(px(h.max(120.)))
                .rounded(px(6.))
                .border_1()
                .border_color(t.accent)
                .child(
                    canvas(
                        move |b, _, _| {
                            frame.borrow_mut().html_bounds.insert(bid, b);
                        },
                        |_, _, _, _| {},
                    )
                    .size_full(),
                )
                .into_any_element()
        } else {
            match state {
                None => {
                    div().py_4().text_size(px(13.)).text_color(t.placeholder).child(crate::i18n::tr("在下方输入 HTML，预览会显示在这里")).into_any_element()
                }
                Some(crate::webview::SnapState::Pending) => {
                    div().py_4().text_size(px(13.)).text_color(t.text_muted).child(crate::i18n::tr("正在渲染…")).into_any_element()
                }
                Some(crate::webview::SnapState::Failed) => {
                    div().py_4().text_size(px(13.)).text_color(t.fg_palette[1]).child(crate::i18n::tr("渲染失败")).into_any_element()
                }
                Some(crate::webview::SnapState::Ready { path, width, height }) => img(path).w(px(width)).h(px(height)).into_any_element(),
            }
        };
        let btn = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id((id, blk.id as usize))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(if on { t.accent } else { t.text_secondary })
                .when(on, |d| d.bg(t.accent_soft))
                .hover(|s| s.bg(t.hover))
                .child(crate::i18n::tr(label))
        };
        div()
            .group("html")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(if editing || live { t.accent } else { t.border })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(div().pr_2().text_size(px(12.)).text_color(t.text_muted).child("HTML"))
                    .child(btn("html-live", if live { "退出交互" } else { "交互模式" }, live).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if this.sess.html_live == Some(bid) {
                                this.sess.html_live = None;
                                crate::webview::hide_live();
                            } else {
                                this.sess.html_live = Some(bid);
                            }
                            cx.notify();
                        }),
                    ))
                    .child(btn("html-js", if scripts { "脚本：已允许" } else { "脚本：已禁用" }, scripts).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.update_kind(
                                ix,
                                |k| {
                                    if let BlockKind::Html { scripts, .. } = k {
                                        *scripts = !*scripts;
                                    }
                                },
                                cx,
                            );
                            crate::webview::hide_live();
                        }),
                    ))
                    .child(div().flex_1())
                    .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("沙箱运行 · 禁止访问网络"))),
            )
            .child(div().w_full().flex().justify_center().child(preview).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus);
                    if this.core.head.block != ix {
                        let len = this.slot_len((ix, 0));
                        this.set_cursor(Pos::new(ix, 0, len), false, cx);
                    }
                }),
            ))
            .when(editing || src.trim().is_empty(), |d| {
                let spec =
                    SlotSpec { size: 13., line_h: 21., mono: true, lang: Some("HTML".into()), placeholder: Some("<h2>Hello</h2>".into()), ..SlotSpec::body(t) };
                d.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(t.code_bg)
                        .child(self.render_slot(ix, 0, &blk.text, spec, focused, t))
                        .on_mouse_down(MouseButton::Left, self.slot_click(ix, 0, cx)),
                )
            })
            .into_any_element()
    }

    // ───────────── 网页卡片 ─────────────

    fn render_link_card(&mut self, ix: usize, blk: &Block, url: &str, selected: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = crate::linkcard::request(url, cx);
        let (u1, u2, u3) = (url.to_string(), url.to_string(), url.to_string());
        let domain = crate::linkcard::domain(url);
        let action = |id: &'static str, label: &'static str| {
            div()
                .id((id, blk.id as usize))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(crate::i18n::tr(label))
        };
        let actions = div()
            .absolute()
            .top(px(6.))
            .right(px(6.))
            .invisible()
            .group_hover("card", |s| s.visible())
            .flex()
            .flex_row()
            .p(px(3.))
            .rounded(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .child(action("card-refresh", "刷新").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    crate::linkcard::refresh(&u2, cx);
                }),
            ))
            .child(action("card-link", "转为链接").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.card_to_link(ix, cx);
                }),
            ))
            .child(action("card-copy", "复制链接").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(u3.clone()));
                }),
            ));
        let body: AnyElement =
            match state {
                crate::linkcard::CardState::Ready(c) => div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .w_full()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(c.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(t.text_secondary)
                                    .line_height(px(20.))
                                    .max_h(px(40.))
                                    .overflow_hidden()
                                    .child(if c.desc.is_empty() { url.to_string() } else { c.desc.clone() }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .pt_1()
                                    .when(!c.favicon.is_empty(), |d| d.child(img(PathBuf::from(&c.favicon)).size(px(14.)).rounded(px(3.))))
                                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(if c.site.is_empty() {
                                        domain.clone()
                                    } else {
                                        format!("{} · {}", c.site, domain)
                                    })),
                            ),
                    )
                    .when(!c.image.is_empty(), |d| d.child(img(PathBuf::from(&c.image)).w(px(180.)).h(px(112.)).flex_none().object_fit(ObjectFit::Cover)))
                    .into_any_element(),
                crate::linkcard::CardState::Loading => div()
                    .px_4()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child(icon("link", 14., t.text_muted))
                    .child(format!("正在获取网页信息… {domain}"))
                    .into_any_element(),
                crate::linkcard::CardState::Offline => {
                    let letter: String = domain.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_3()
                        .child(
                            div()
                                .size(px(36.))
                                .flex_none()
                                .rounded(px(8.))
                                .bg(t.accent_soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(t.accent)
                                .font_weight(FontWeight::BOLD)
                                .child(letter),
                        )
                        .child(div().flex_1().min_w(px(0.)).flex().flex_col().child(div().text_size(px(14.)).text_color(t.text).child(domain.clone())).child(
                            div().text_size(px(12.)).text_color(t.text_muted).whitespace_nowrap().overflow_hidden().text_ellipsis().child(url.to_string()),
                        ))
                        .into_any_element()
                }
            };
        div()
            .id(("card", blk.id as usize))
            .group("card")
            .relative()
            .w_full()
            .max_w(px(620.))
            .rounded(px(10.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .overflow_hidden()
            .cursor_pointer()
            .hover(|s| s.border_color(t.border_strong))
            .child(body)
            .child(actions)
            .child(self.void_marker(ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                    if !ev.modifiers.shift {
                        cx.open_url(&u1);
                    }
                }),
            )
            .into_any_element()
    }

    fn card_to_link(&mut self, ix: usize, cx: &mut Context<Self>) {
        let BlockKind::LinkCard { url } = self.content.blocks[ix].kind.clone() else { return };
        let (title, _) = crate::linkcard::cached_text(&url);
        let text = if title.is_empty() { url.clone() } else { title };
        self.checkpoint(EditKind::Other);
        let b = self.block_mut(ix);
        b.kind = BlockKind::Text;
        b.text = crate::model::RichText::styled_link(&text, &url);
        self.changed(cx);
    }

    // ───────────── 嵌入对象 ─────────────

    #[allow(clippy::too_many_arguments)]
    /// 取（必要时创建）文档里内嵌的可编辑表格视图。
    pub(crate) fn live_sheet(&mut self, oid: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<crate::sheet::SheetEditor> {
        if let Some((v, _)) = self.live_sheets.get(oid) {
            return v.clone();
        }
        let v = cx.new(|cx| crate::sheet::SheetEditor::new_embedded(oid.to_string(), window, cx));
        let id = oid.to_string();
        let sub = cx.subscribe_in(&v, window, move |this, _, ev: &crate::sheet::SheetEvent, window, cx| match ev {
            crate::sheet::SheetEvent::Fullscreen => cx.emit(EditorEvent::OpenEmbed { kind: "sheet".into(), id: id.clone() }),
            crate::sheet::SheetEvent::Close => {
                window.focus(&this.focus);
                cx.notify();
            }
        });
        self.live_sheets.insert(oid.to_string(), (v.clone(), sub));
        v
    }

    /// 取（必要时创建）文档里内嵌的可交互多维表格：表格 / 看板等视图可直接编辑，「全屏」进入完整编辑页。
    pub(crate) fn live_base(&mut self, oid: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<crate::base::editor::BaseEditor> {
        if let Some((v, _)) = self.live_bases.get(oid) {
            return v.clone();
        }
        let v = cx.new(|cx| crate::base::editor::BaseEditor::new_embedded(oid.to_string(), window, cx));
        let id = oid.to_string();
        let sub = cx.subscribe_in(&v, window, move |this, _, ev: &crate::base::editor::BaseEvent, window, cx| match ev {
            crate::base::editor::BaseEvent::Fullscreen => cx.emit(EditorEvent::OpenEmbed { kind: "base".into(), id: id.clone() }),
            crate::base::editor::BaseEvent::Close => {
                window.focus(&this.focus);
                cx.notify();
            }
        });
        self.live_bases.insert(oid.to_string(), (v.clone(), sub));
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn render_embed(
        &mut self,
        ix: usize,
        blk: &Block,
        kind: &str,
        oid: &str,
        selected: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = blk.kind.label();
        let maxw = content_w() - blk.indent as f32 * 26.;
        let (k1, id1) = (kind.to_string(), oid.to_string());
        let (k2, id2) = (kind.to_string(), oid.to_string());
        let preview: AnyElement = match crate::objects::load(oid).map_err(|e| match e {
            crate::objects::LoadError::Missing => crate::i18n::tr("对象数据丢失").to_string(),
            e => e.message(),
        }) {
            Err(msg) => div().p_4().text_size(px(13.)).text_color(t.fg_palette[1]).child(msg).into_any_element(),
            Ok(crate::objects::ObjData::Sheet(_)) => {
                // 文档里的表格直接可编辑，选中后上方浮出工具栏，「全屏」进入完整编辑页
                let view = self.live_sheet(oid, window, cx);
                view.update(cx, |v, _| v.embed_max_w = maxw - 18.);
                div().max_w(px(maxw - 18.)).overflow_hidden().child(view).into_any_element()
            }
            Ok(crate::objects::ObjData::Base(_)) => {
                // 文档里的多维表格直接可交互（切换视图、编辑单元格、拖看板卡片），「全屏」进入完整编辑页
                let view = self.live_base(oid, window, cx);
                div().w(px(maxw - 16.)).h(px(view.read(cx).embed_h)).overflow_hidden().child(view).into_any_element()
            }
            Ok(crate::objects::ObjData::Sketch(d)) => match crate::sketch::preview_size(&d, maxw - 16.) {
                None => div()
                    .h(px(140.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child(format!("空白{label} · 双击开始编辑"))
                    .into_any_element(),
                Some((w, h)) => {
                    let dark = t.dark;
                    let bg = crate::sketch::paint::bg_hsla(&d, dark);
                    div()
                        .w(px(w))
                        .h(px(h))
                        .bg(bg)
                        .child(canvas(|b, _, _| b, move |b, _, window, cx| crate::sketch::paint::paint_fit(&d, b, dark, window, cx)).size_full())
                        .into_any_element()
                }
            },
            Ok(data) => {
                let mut scene = match &data {
                    crate::objects::ObjData::Board(b) => b.to_scene(t.dark),
                    _ => Scene::default(),
                };
                scene.fit(24.);
                if scene.items.is_empty() {
                    div()
                        .h(px(140.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(t.text_muted)
                        .child(format!("空白{label} · 双击开始编辑"))
                        .into_any_element()
                } else {
                    let scale = (maxw / scene.width).min(1.).min(440. / scene.height).max(0.1);
                    let (w, h) = (scene.width * scale, scene.height * scale);
                    let scene = Arc::new(scene);
                    div()
                        .w(px(w))
                        .h(px(h))
                        .child(
                            canvas(move |b, _, _| View { origin: b.origin, scale }, move |b, v, window, cx| paint_scene(&scene, v, b, window, cx)).size_full(),
                        )
                        .into_any_element()
                }
            }
        };
        let label: String = match crate::objects::base(oid).filter(|_| kind == "base") {
            Some(b) if b.borrow().meta.name != label => format!("{} · {}", label, b.borrow().meta.name),
            _ => label.to_string(),
        };
        let ic = match kind {
            "base" => "⊞",
            "sheet" => "▦",
            "flow" => "⇢",
            "sketch" => "✐",
            _ => "✎",
        };
        div()
            .id(("embed", blk.id as usize))
            .group("embed")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(6.))
                    .bg(t.sidebar_bg)
                    .border_b_1()
                    .border_color(t.border)
                    .text_size(px(12.))
                    .text_color(t.text_secondary)
                    .child(ic)
                    .child(crate::i18n::tr_owned(&label))
                    .child(div().flex_1())
                    .child(div().text_color(t.text_muted).child(match kind {
                        "sheet" => "点击单元格直接编辑",
                        _ => "双击编辑",
                    }))
                    .child(
                        div()
                            .id(("embed-open", blk.id as usize))
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .text_color(t.accent)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(if k1 == "sheet" { "全屏" } else { "打开" })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(EditorEvent::OpenEmbed { kind: k1.clone(), id: id1.clone() });
                                }),
                            ),
                    ),
            )
            .child(div().w_full().p_2().flex().when(kind != "sheet", |d| d.justify_center()).bg(t.bg).child(preview))
            .child(self.void_marker(ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                    if ev.click_count >= 2 {
                        cx.emit(EditorEvent::OpenEmbed { kind: k2.clone(), id: id2.clone() });
                    }
                }),
            )
            .into_any_element()
    }

    /// 视频全屏浮层。
    pub(super) fn render_video_full(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.sess.video_full?;
        // 文本视图下行内视频块不渲染，放大预览的播放器不在 f.videos 里，得自己声明可见，否则会被 pause_except 立刻暂停
        crate::media::mark_visible(id);
        let path = self.content.blocks.iter().find_map(|b| match &b.kind {
            BlockKind::Video { path, .. } if b.id == id => Some(path.clone()),
            _ => None,
        })?;
        let (playing, frame, pos, total, muted, rate) = crate::media::with(id, &path, |p| (p.playing, p.frame(), p.position(), p.duration(), p.muted, p.rate))?;
        if playing {
            window.request_animation_frame();
        }
        let p2 = path.clone();
        let (p3, p4, p5, p6) = (path.clone(), path.clone(), path.clone(), path.clone());
        let seek_key = id ^ 0x5eed_0000_0000_0000;
        let frac = if total > 0. { (pos / total).clamp(0., 1.) as f32 } else { 0. };
        let ctrl_btn = |eid: &'static str, label: String| {
            div()
                .id(eid)
                .px_2()
                .py_1()
                .rounded(px(4.))
                .text_size(px(14.))
                .text_color(gpui::white())
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(0., 0., 1., 0.15)))
                .child(crate::i18n::tr_owned(&label))
        };
        let center_play = (!playing).then(|| {
            div().absolute().inset_0().flex().items_center().justify_center().child(
                div().size(px(64.)).rounded_full().bg(gpui::hsla(0., 0., 0., 0.45)).flex().items_center().justify_center().child(div().pl(px(4.)).child(icon(
                    "play-fill",
                    28.,
                    gpui::white(),
                ))),
            )
        });
        let controls = div()
            .id("vf-controls")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .px(px(24.))
            .pb_3()
            .pt_8()
            .bg(gpui::linear_gradient(180., gpui::linear_color_stop(gpui::hsla(0., 0., 0., 0.), 0.), gpui::linear_color_stop(gpui::hsla(0., 0., 0., 0.6), 1.)))
            .flex()
            .flex_col()
            .gap_1()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .id("vf-seek")
                    .relative()
                    .h(px(16.))
                    .w_full()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .relative()
                            .h(px(4.))
                            .w_full()
                            .rounded(px(2.))
                            .bg(gpui::hsla(0., 0., 1., 0.3))
                            .child(div().absolute().left_0().top_0().bottom_0().w(relative(frac)).rounded(px(2.)).bg(t.accent))
                            .child(super::attach::bounds_only(seek_key)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let Some(b) = super::attach::player_bounds(seek_key) else { return };
                            let f = (f32::from(ev.position.x - b.left()) / f32::from(b.size.width).max(1.)).clamp(0., 1.);
                            crate::media::with(id, &p3, |p| {
                                let d = p.duration();
                                p.seek(d * f as f64);
                            });
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(ctrl_btn("vf-play", if playing { "❚❚".into() } else { "▶".into() }).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            crate::media::with(id, &p4, |p| p.toggle());
                            cx.notify();
                        }),
                    ))
                    .child(div().text_size(px(13.)).text_color(gpui::white()).whitespace_nowrap().child(format!(
                        "{} / {}",
                        crate::media::fmt_time(pos),
                        crate::media::fmt_time(total)
                    )))
                    .child(div().flex_1())
                    .child(ctrl_btn("vf-mute", if muted { "🔇".into() } else { "🔊".into() }).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            crate::media::with(id, &p5, |p| p.toggle_mute());
                            cx.notify();
                        }),
                    ))
                    .child(ctrl_btn("vf-rate", format!("{rate}x")).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            crate::media::with(id, &p6, |p| {
                                let i = crate::media::RATES.iter().position(|r| (*r - p.rate).abs() < 0.01).unwrap_or(1);
                                p.set_rate(crate::media::RATES[(i + 1) % crate::media::RATES.len()]);
                            });
                            cx.notify();
                        }),
                    )),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(gpui::black())
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .children(frame.map(|f| surface(f).size_full()))
                .children(center_play)
                .child(controls)
                .child(
                    div()
                        .id("vf-close")
                        .absolute()
                        .top(px(16.))
                        .right(px(16.))
                        .px_3()
                        .py_1()
                        .rounded(px(6.))
                        .bg(gpui::hsla(0., 0., 1., 0.15))
                        .text_color(gpui::white())
                        .text_size(px(13.))
                        .cursor_pointer()
                        .child(crate::i18n::tr("退出全屏 (Esc)"))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.sess.video_full = None;
                                cx.notify();
                            }),
                        ),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        crate::media::with(id, &p2, |p| p.toggle());
                        cx.notify();
                    }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_prop_edit(&self, t: &Theme) -> Option<impl IntoElement> {
        let pe = self.pop.prop_edit.as_ref()?;
        Some(
            deferred(
                anchored().position(point(pe.at.x - px(20.), pe.at.y + px(18.))).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .occlude()
                        .w(px(320.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(8.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(8.))
                        .shadow_lg()
                        .text_size(px(13.))
                        .child(div().flex_1().line_height(px(22.)).child(pe.input.clone()))
                        .child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("↵ 确认"))),
                ),
            )
            .with_priority(4),
        )
    }
}

pub fn col_name(mut c: i32) -> String {
    let mut s = String::new();
    while c > 0 {
        c -= 1;
        s.insert(0, (b'A' + (c % 26) as u8) as char);
        c /= 26;
    }
    s
}
