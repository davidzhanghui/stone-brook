//! 主窗口：侧边栏文档树 + 顶栏 + 编辑器 + 大纲 + 快速打开 / 回收站 / 关于。

use crate::convert;
use crate::editor::{fmt_num, Editor, EditorEvent};
use crate::line_input::{InputEvent, LineInput};
use crate::model::{BlockKind, DocContent};
use crate::seed;
use crate::store::{DocMeta, Store};
use crate::theme::{theme, Theme, ThemeMode};
use crate::ui::{icon, icon_button, logo, Tooltip};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

mod graph;
mod panels;
mod present;
mod shortcuts_panel;
mod tree;

actions!(
    stonebrook,
    [
        NewDoc,
        NewSubDoc,
        MoveDoc,
        QuickOpen,
        ToggleSidebar,
        ToggleOutline,
        CycleTheme,
        TogglePerf,
        FocusMode,
        ExportMarkdown,
        ExportHtml,
        ImportMarkdown,
        StressTest,
        BaseStressTest,
        About,
        Quit,
        TrashDoc,
        OpenWelcome,
        OpenSettings,
        CommandPalette,
        ExportPdf,
        ExportBackup,
        RestoreBackup,
        Present,
        ToggleReadMode,
        ShowShortcuts,
        ExportDocxXml,
        CheckUpdate,
        InstallUpdate,
        CancelConfirm
    ]
);

/// 文档副本：块的运行期 id 在加载时重新分配；嵌入对象（画板 / 电子表格 / 多维表格…）
/// 逐个深拷贝，不然副本和原件改的是同一个对象。标题加「副本」。
fn duplicate_content(mut c: DocContent) -> DocContent {
    for b in c.blocks.iter_mut() {
        if let BlockKind::Embed { id, .. } = &b.kind {
            if let Some(n) = crate::objects::duplicate(id) {
                if let BlockKind::Embed { id, .. } = &mut std::sync::Arc::make_mut(b).kind {
                    *id = n;
                }
            }
        }
    }
    if let Some(t) = c.blocks.first_mut() {
        let t = std::sync::Arc::make_mut(t);
        let title = format!("{} 副本", t.text.text);
        t.text = crate::model::RichText::plain(title);
    }
    c
}

struct Switcher {
    input: Entity<LineInput>,
    results: Vec<(String, String, String)>,
    selected: usize,
    _sub: Subscription,
}

/// 「移动到…」弹窗状态。
struct MovePicker {
    doc_id: String,
    input: Entity<LineInput>,
    targets: Vec<tree::MoveTarget>,
    selected: usize,
    _sub: Subscription,
}

/// 侧边栏文档右键菜单。
struct DocContextMenu {
    doc_id: String,
    position: Point<Pixels>,
}

/// 侧边栏文档拖拽负载。
#[derive(Clone)]
struct DragDoc {
    id: String,
    title: String,
}

/// 拖拽侧边栏文档时跟随鼠标的半透明预览胶囊。
struct DocDragPreview(String);

impl Render for DocDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px_3()
            .py_1()
            .rounded(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.accent)
            .shadow_md()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .text_color(t.text)
            .opacity(0.92)
            .child(icon("file", 14., t.accent))
            .child(self.0.clone())
    }
}

/// 破坏性操作前的二次确认（目前用于回收站里的「永久删除」）。
/// 移到回收站这类可撤销的操作不弹窗，只用 toast 告知，避免高频打扰。
struct Confirm {
    title: SharedString,
    body: SharedString,
    ok: SharedString,
    action: ConfirmAction,
}

enum ConfirmAction {
    /// 永久删除文档（连同其子文档与评论）
    Purge(String),
}

/// 提示条样式：普通（深色胶囊）/ 成功（绿色底 + 勾选）。
#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Success,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SettingsTab {
    #[default]
    Appearance,
    General,
    Stats,
    Data,
    Updates,
}

pub struct Workspace {
    pub store: Store,
    docs: Vec<DocMeta>,
    current: String,
    pub editor: Entity<Editor>,
    expanded: HashSet<String>,
    show_sidebar: bool,
    show_outline: bool,
    focus_mode: bool,
    theme_mode: ThemeMode,
    /// 配色主题（theme_css::list 的选择值；空串为默认）
    css_theme: String,
    /// 当前配色主题里没生效的条目
    theme_problems: Vec<String>,
    /// 用户主题文件的修改时间（变了就重新加载）
    theme_mtime: Option<std::time::SystemTime>,
    switcher: Option<Switcher>,
    /// 侧边栏拖拽文档时的当前落点指示（无活跃拖拽时自动清空）
    doc_drop_target: Option<tree::DropZone>,
    /// 侧边栏文档右键菜单
    doc_menu: Option<DocContextMenu>,
    /// 「移动到…」目标选择器弹窗
    move_picker: Option<MovePicker>,
    show_trash: bool,
    toast: Option<String>,
    toast_kind: ToastKind,
    /// 每次提示递增：作为动画元素的 id，保证同一条提示再次出现时动画从头开始
    toast_gen: u64,
    toast_task: Option<Task<()>>,
    about: bool,
    more_menu: bool,
    last_title: String,
    outline: (u64, Vec<(usize, u8, SharedString)>),
    /// 反向链接缓存：(文档 ID, 文档列表签名, 链接到它的文档)
    backlinks: (String, u64, Vec<(String, String)>),
    /// 未链接的提及缓存：(文档 ID, 文档列表签名, 结果)
    mentions: (String, u64, Vec<(String, String)>),
    graph: Option<graph::Graph>,
    /// 上次同步给编辑器 `@` 菜单的文档列表签名
    doc_list_sig: u64,
    outline_scroll: UniformListScrollHandle,
    outline_current: Option<usize>,
    focused_once: bool,
    pending_embed: Option<(String, String)>,
    pending_new_doc: Option<DocContent>,
    /// 模板选择器：Some(true) 新建文档，Some(false) 插入到当前文档
    template_picker: Option<bool>,
    /// 「关联已有多维表格」选择框
    base_picker: Option<Vec<(String, String, usize)>>,
    template_list: Vec<(String, String, String)>,
    settings_open: bool,
    settings_tab: SettingsTab,
    /// 待确认的破坏性操作（永久删除）；Some 时显示确认弹窗并拦截点击
    confirm: Option<Confirm>,
    /// 确认弹窗的焦点（弹窗打开时聚焦它，Esc 才能关掉，而不是落到编辑器的 Esc 处理上）
    confirm_focus: FocusHandle,
    /// 已经提示过「新版本已下载」的版本号
    update_notified: Option<String>,
    /// 新版本就绪决策弹窗（Muse / Sparkle 2 风格）
    update_dialog_open: bool,
    /// 更新弹窗里是否展开更新说明
    update_notes_open: bool,
    /// 右侧评论侧栏是否展开
    comments_open: bool,
    /// 设置面板里的「评论作者名」输入框
    author_input: Entity<LineInput>,
    palette: Option<(Entity<LineInput>, Subscription, usize)>,
    /// 快捷键面板（⌘/）
    shortcuts_panel: Option<shortcuts_panel::ShortcutsPanel>,
    slides: Option<(Vec<present::Slide>, usize)>,
    present_ui: present::PresentUi,
    present_focus: FocusHandle,
    present_fullscreen: bool,
    /// 全屏打开的嵌入对象编辑器
    embed: Option<(AnyView, FocusHandle, Subscription)>,
    _subs: Vec<Subscription>,
    _theme_watch: Task<()>,
    _error_poll: Task<()>,
}

/// 透明标题栏：在空白处按下拖动窗口，双击缩放（与系统标题栏行为一致）。
fn title_drag(ev: &MouseDownEvent, window: &mut Window) {
    if ev.click_count >= 2 {
        window.zoom_window();
    } else {
        window.start_window_move();
    }
}

fn window_defer_open(ws: &mut Workspace, kind: String, id: String, cx: &mut Context<Workspace>) {
    // 事件回调里拿不到 Window，留到下一次渲染时打开
    ws.pending_embed = Some((kind, id));
    cx.notify();
}

impl Workspace {
    pub fn open_embed(&mut self, kind: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        // 读不出来（不存在 / 读失败 / 损坏）的对象不进编辑器：否则会打开一份空白默认内容，用户以为数据没了，
        // 而且在空白内容上做的修改也保存不了。损坏的原始数据已在 `objects::load` 里备份。
        match crate::objects::load(id) {
            Ok(_) | Err(crate::objects::LoadError::NoStore) => {}
            Err(e) => {
                self.show_toast_for(e.message(), 8000, cx);
                cx.notify();
                return;
            }
        }
        use crate::objects::ObjKind;
        match ObjKind::parse(kind) {
            Some(ObjKind::Board | ObjKind::Flow) => self.mount_embed::<crate::board::view::BoardEditor>(id, window, cx),
            Some(ObjKind::Sketch) => self.mount_embed::<crate::sketch::view::SketchEditor>(id, window, cx),
            Some(ObjKind::Sheet) => self.mount_embed::<crate::sheet::SheetEditor>(id, window, cx),
            Some(ObjKind::Base) => self.mount_embed::<crate::base::editor::BaseEditor>(id, window, cx),
            None => self.show_toast("暂不支持的对象类型".into(), cx),
        }
        cx.notify();
    }

    /// 打开一个全屏嵌入编辑器：新建视图、关闭事件回到文档、抢焦点。
    fn mount_embed<V: crate::embed::EmbedEditor>(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let v = cx.new(|cx| V::open(id.to_string(), window, cx));
        let sub = cx.subscribe_in(&v, window, |this, _, ev: &V::Event, window, cx| {
            if V::closes(ev) {
                this.close_embed(window, cx);
            }
        });
        let fh = v.focus_handle(cx);
        window.focus(&fh);
        self.embed = Some((v.into(), fh, sub));
    }

    /// 以思维导图查看当前文档：标题为根、各级标题与列表项为分支，生成一个临时画板；
    /// 关掉即丢弃，「插入到文档」则作为画板插到光标处。
    pub fn open_doc_mindmap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let lines = doc_outline(&self.editor.read(cx).content.blocks);
        let doc = crate::board::model::BoardDoc::with_mind(Some(&lines), 1, theme(cx).dark);
        let id = crate::objects::create(crate::objects::ObjKind::Board, Some(crate::objects::ObjData::Board(std::sync::Arc::new(doc))));
        let v = cx.new(|cx| {
            let mut b = crate::board::view::BoardEditor::new(id.clone(), cx);
            b.preview = true;
            b
        });
        let sub = cx.subscribe_in(&v, window, move |this, _, ev: &crate::board::view::BoardEvent, window, cx| {
            use crate::board::view::BoardEvent;
            match ev {
                BoardEvent::Close => crate::objects::remove(&id),
                BoardEvent::Insert => {
                    this.editor.update(cx, |e, cx| e.insert_embed("board", &id, cx));
                    this.show_toast("已作为思维导图插入到光标处".into(), cx);
                }
            }
            this.close_embed(window, cx);
        });
        let fh = v.focus_handle(cx);
        window.focus(&fh);
        self.embed = Some((v.into(), fh, sub));
        cx.notify();
    }

    #[cfg(feature = "dev")]
    pub fn board_view(&self) -> Option<Entity<crate::board::view::BoardEditor>> {
        self.embed.as_ref().and_then(|e| e.0.clone().downcast::<crate::board::view::BoardEditor>().ok())
    }

    #[cfg(feature = "dev")]
    pub fn sketch_view(&self) -> Option<Entity<crate::sketch::view::SketchEditor>> {
        self.embed.as_ref().and_then(|e| e.0.clone().downcast::<crate::sketch::view::SketchEditor>().ok())
    }

    pub fn embed_view(&self) -> Option<AnyView> {
        self.embed.as_ref().map(|e| e.0.clone())
    }

    pub fn open_templates(&mut self, new_doc: bool, cx: &mut Context<Self>) {
        self.template_list = self.store.list_templates();
        self.template_picker = Some(new_doc);
        self.more_menu = false;
        cx.notify();
    }

    /// 应用模板：`builtin` 为内置模板序号，否则为用户模板 JSON。
    pub fn apply_template(&mut self, builtin: Option<usize>, json: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let new_doc = self.template_picker.take().unwrap_or(true);
        let cur_title = self.editor.read(cx).content.blocks.first().map(|b| b.text.text.clone()).unwrap_or_default();
        let (title, blocks) = match (builtin, json) {
            (Some(i), _) => crate::templates::builtin_blocks(&crate::templates::BUILTIN[i], &cur_title),
            (_, Some(j)) => crate::templates::user_blocks(&j, &cur_title),
            _ => return,
        };
        if new_doc {
            self.create_doc(None, crate::templates::to_doc(title, blocks), window, cx);
        } else {
            self.editor.update(cx, |e, cx| e.insert_blocks_after_cursor(blocks, cx));
            window.focus(&self.editor.focus_handle(cx));
        }
        cx.notify();
    }

    pub fn save_as_template(&mut self, cx: &mut Context<Self>) {
        self.more_menu = false;
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let content = self.editor.read(cx).content.clone();
        let name = content.blocks.first().map(|b| b.text.text.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "未命名模板".into());
        // 折叠状态是视图状态，模板里不带
        let blocks: Vec<crate::model::Block> = content
            .blocks
            .iter()
            .map(|b| {
                let mut b = (**b).clone();
                b.collapsed = false;
                b
            })
            .collect();
        self.store.save_template(&name, &crate::model::try_blocks_to_json(&blocks).unwrap_or_default());
        self.show_toast(format!("已保存为模板「{name}」"), cx);
    }

    fn render_templates(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let new_doc = self.template_picker == Some(true);
        let card = |id: ElementId, ic: &'static str, color: u32, name: String, desc: String| {
            div()
                .id(id)
                .w(px(196.))
                .h(px(118.))
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(10.))
                .border_1()
                .border_color(t.border)
                .bg(t.bg)
                .cursor_pointer()
                .hover(|s| s.border_color(t.accent).shadow_md())
                .child(div().size(px(32.)).rounded(px(8.)).flex().items_center().justify_center().bg(gpui::rgba(color << 8 | 0x22)).child(icon(
                    ic,
                    17.,
                    gpui::rgb(color).into(),
                )))
                .child(div().mt_1().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(name))
                .child(div().text_size(px(12.)).text_color(t.text_muted).line_height(px(17.)).child(crate::i18n::tr_owned(&desc)))
        };
        let mut builtin = div().flex().flex_wrap().gap_3();
        for (i, b) in crate::templates::BUILTIN.iter().enumerate() {
            builtin = builtin.child(
                card(("tpl", i).into(), b.icon, b.color, b.name.into(), b.desc.into())
                    .on_click(cx.listener(move |this, _, w, cx| this.apply_template(Some(i), None, w, cx))),
            );
        }
        if new_doc {
            builtin =
                builtin.child(card("tpl-blank".into(), "plus", 0x8f959e, "空白文档".into(), "从零开始".into()).on_click(cx.listener(|this, _, w, cx| {
                    this.template_picker = None;
                    this.create_doc(None, DocContent::new_empty(""), w, cx);
                })));
        }
        let mut mine = div().flex().flex_wrap().gap_3();
        for (i, (id, name, json)) in self.template_list.iter().enumerate() {
            let (json, id2) = (json.clone(), id.clone());
            let n = crate::model::blocks_from_json(json.as_str()).map(|b| b.len()).unwrap_or(0);
            mine = mine.child(
                div()
                    .relative()
                    .child(
                        card(("utpl", i).into(), "layout-template", 0x3b6cf6, name.clone(), format!("{n} 个块"))
                            .on_click(cx.listener(move |this, _, w, cx| this.apply_template(None, Some(json.clone()), w, cx))),
                    )
                    .child(
                        div()
                            .id(("utpl-del", i))
                            .absolute()
                            .top(px(8.))
                            .right(px(8.))
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("x", 12., t.text_muted))
                            .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("删除模板").into())).into())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.store.delete_template(&id2);
                                this.template_list = this.store.list_templates();
                                cx.notify();
                            })),
                    ),
            );
        }
        let has_mine = !self.template_list.is_empty();
        div()
            .absolute()
            .inset_0()
            .bg(t.overlay)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.template_picker = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("tpl-panel")
                    .w(px(680.))
                    .max_h(px(620.))
                    .overflow_y_scroll()
                    .p(px(24.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(16.))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).text_color(t.text).child(if new_doc {
                        "从模板新建"
                    } else {
                        "插入模板"
                    }))
                    .child(div().text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr("{{今天}} {{本周}} {{时间}} 等变量会自动替换为当前日期")))
                    .child(div().mt_1().text_size(px(13.)).text_color(t.text_secondary).child(crate::i18n::tr("内置模板")))
                    .child(builtin)
                    .when(has_mine, |d| d.child(div().mt_2().text_size(px(13.)).text_color(t.text_secondary).child(crate::i18n::tr("我的模板"))).child(mine))
                    .when(!has_mine, |d| {
                        d.child(
                            div()
                                .mt_2()
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child(crate::i18n::tr("提示：在文档右上角「⋯」菜单中选择「另存为模板」，即可把任意文档变成自己的模板")),
                        )
                    }),
            )
    }

    /// 「关联已有多维表格」：列出本机所有多维表格，选中后在文档里插入同一个多维表格的引用（两处数据实时同步）。
    fn render_base_picker(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let list = self.base_picker.as_ref()?;
        let items = list.iter().enumerate().map(|(i, (id, name, n))| {
            let id = id.clone();
            div()
                .id(("base-pick", i))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(div().size(px(32.)).rounded(px(8.)).bg(gpui::rgb(0x7f3bf5)).flex().items_center().justify_center().child(icon(
                    "base",
                    17.,
                    gpui::white(),
                )))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(14.)).text_color(t.text).child(name.clone()))
                        .child(div().text_size(px(12.)).text_color(t.text_muted).child(format!("{n} 张数据表"))),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.base_picker = None;
                    this.editor.update(cx, |e, cx| e.insert_embed("base", &id, cx));
                    this.show_toast("已插入关联的多维表格，两处修改实时同步".into(), cx);
                }))
        });
        let empty = list.is_empty();
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.overlay)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.base_picker = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("base-picker")
                        .w(px(440.))
                        .max_h(px(520.))
                        .overflow_y_scroll()
                        .p(px(20.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(14.))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).text_color(t.text).child(crate::i18n::tr("关联已有多维表格")))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child(crate::i18n::tr("插入的是同一份数据：在任意一处修改，其他文档里的这个多维表格同步更新")),
                        )
                        .when(empty, |d| {
                            d.child(
                                div()
                                    .py_4()
                                    .text_size(px(13.))
                                    .text_color(t.text_muted)
                                    .child(crate::i18n::tr("还没有多维表格，先用斜杠菜单「表格 / 看板 / 画册」新建一个")),
                            )
                        })
                        .children(items),
                ),
        )
    }

    pub fn close_embed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.embed = None;
        window.focus(&self.editor.focus_handle(cx));
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut docs = store.list();
        if docs.is_empty() {
            let id = store.create(None, &seed::welcome());
            store.set_kv("last_doc", &id);
            docs = store.list();
        }
        let current = store
            .get_kv("last_doc")
            .filter(|id| docs.iter().any(|d| &d.id == id && !d.deleted))
            .or_else(|| docs.iter().find(|d| !d.deleted).map(|d| d.id.clone()))
            .unwrap_or_else(|| store.create(None, &DocContent::new_empty("")));
        let docs = store.list();
        let (content, corrupt) = store.load(&current).unwrap_or_else(|| (DocContent::new_empty(""), None));
        let last_title = content.title();
        let editor = cx.new(|cx| Editor::new(store.clone(), current.clone(), content, cx));
        if store.get_kv(&Self::read_mode_key(&current)).as_deref() == Some("1") {
            editor.update(cx, |e, cx| e.set_read_only(true, cx));
        }
        let css_theme = store.get_kv("css_theme").unwrap_or_default();
        let theme_mode = match store.get_kv("theme").as_deref() {
            Some("light") => ThemeMode::Light,
            Some("dark") => ThemeMode::Dark,
            _ => ThemeMode::System,
        };
        let sub = cx.subscribe_in(&editor, window, |this, editor, ev: &EditorEvent, window, cx| match ev {
            EditorEvent::OpenDoc(id) => {
                if this.docs.iter().any(|d| &d.id == id && !d.deleted) {
                    this.open_doc(id, window, cx);
                } else {
                    this.show_toast("链接的文档不存在或已在回收站".into(), cx);
                }
            }
            EditorEvent::Changed => {
                let title = editor.read(cx).content.title();
                if title != this.last_title {
                    this.last_title = title.clone();
                    if let Some(d) = this.docs.iter_mut().find(|d| d.id == this.current) {
                        d.title = title;
                    }
                }
                cx.notify();
            }
            EditorEvent::Toast(msg) => this.show_toast(msg.clone(), cx),
            EditorEvent::ToastSuccess(msg) => this.show_toast_ok(msg.clone(), cx),
            EditorEvent::OpenEmbed { kind, id } => {
                let (kind, id) = (kind.clone(), id.clone());
                window_defer_open(this, kind, id, cx);
            }
            EditorEvent::DuplicateDoc => {
                let content = editor.read(cx).content.clone();
                this.pending_new_doc = Some(duplicate_content(content));
                cx.notify();
            }
            EditorEvent::PickTemplate => {
                this.open_templates(false, cx);
                cx.notify();
            }
            EditorEvent::PickBase => {
                this.base_picker = Some(crate::objects::list_bases());
                cx.notify();
            }
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(window, cx);
        });
        let mut expanded = HashSet::new();
        // 展开当前文档的所有祖先
        let mut p = docs.iter().find(|d| d.id == current).and_then(|d| d.parent.clone());
        while let Some(pid) = p {
            expanded.insert(pid.clone());
            p = docs.iter().find(|d| d.id == pid).and_then(|d| d.parent.clone());
        }
        let show_outline = store.get_kv("outline").as_deref() != Some("0");
        let author_input = cx.new(|cx| crate::line_input::LineInput::new("我", cx));
        author_input.update(cx, |i, cx| i.set_text(&crate::settings::author(), cx));
        // 输入即生效；清空表示恢复默认「我」。写库放到后台线程，不阻塞打字
        let author_sub = cx.subscribe(&author_input, |this, input, ev: &crate::line_input::InputEvent, cx| {
            if matches!(ev, crate::line_input::InputEvent::Submit { .. } | crate::line_input::InputEvent::Changed) {
                let v = input.read(cx).text().trim().to_string();
                let store = this.store.clone();
                crate::settings::set_author_mem(&v);
                cx.background_executor().spawn(async move { store.set_kv("author", &v) }).detach();
            }
        });
        let ws = Workspace {
            store,
            docs,
            current,
            editor,
            expanded,
            show_sidebar: true,
            show_outline,
            focus_mode: false,
            theme_mode,
            css_theme,
            theme_problems: vec![],
            theme_mtime: None,
            switcher: None,
            doc_drop_target: None,
            doc_menu: None,
            move_picker: None,
            show_trash: false,
            toast: corrupt.map(|c| corrupt_msg(&c)),
            toast_kind: ToastKind::Info,
            toast_gen: 0,
            toast_task: None,
            about: false,
            more_menu: false,
            last_title,
            outline: (0, vec![]),
            backlinks: (String::new(), 0, vec![]),
            mentions: (String::new(), 0, vec![]),
            graph: None,
            doc_list_sig: 0,
            outline_scroll: UniformListScrollHandle::new(),
            outline_current: None,
            focused_once: false,
            pending_embed: None,
            pending_new_doc: None,
            template_picker: None,
            base_picker: None,
            template_list: vec![],
            settings_open: false,
            settings_tab: SettingsTab::Appearance,
            confirm: None,
            confirm_focus: cx.focus_handle(),
            update_notified: None,
            update_dialog_open: false,
            update_notes_open: false,
            comments_open: false,
            author_input,
            palette: None,
            shortcuts_panel: None,
            slides: None,
            present_focus: cx.focus_handle(),
            present_fullscreen: false,
            present_ui: Default::default(),
            embed: None,
            _subs: vec![sub, appearance, author_sub],
            _error_poll: Self::poll_store_errors(cx),
            _theme_watch: Self::watch_theme_file(window, cx),
        };
        let mut ws = ws;
        ws.set_theme_global(window, cx);
        ws
    }

    /// 明暗基础色 + 配色主题 CSS → 全局主题。
    fn set_theme_global(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut t = Theme::for_mode(self.theme_mode, window.appearance());
        self.theme_problems = match crate::theme_css::source(&self.css_theme) {
            Some(src) => crate::theme_css::apply(&mut t, &src),
            None => vec!["主题文件读不到了，已用默认配色".into()],
        };
        self.theme_mtime = crate::theme_css::user_path(&self.css_theme).and_then(|p| std::fs::metadata(p).ok()?.modified().ok());
        cx.set_global(t);
    }

    /// 用户主题文件保存后自动重新加载（每秒看一次修改时间）。
    fn watch_theme_file(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this: WeakEntity<Workspace>, cx: &mut AsyncWindowContext| loop {
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            let alive = this.update_in(cx, |this, window, cx| {
                let Some(path) = crate::theme_css::user_path(&this.css_theme) else { return };
                let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
                if mtime != this.theme_mtime {
                    this.apply_theme(window, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        })
    }

    /// 定期取走存储错误并提示（写入在后台线程，失败时界面层不会立刻知道；读失败、对象损坏也走这里）。
    fn poll_store_errors(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            let alive = this.update(cx, |this, cx| {
                if let Some(e) = this.store.take_error() {
                    // 消息自带上下文（保存失败 / 读取失败 / 对象损坏…），多停留一会儿
                    this.show_toast_for(e, 6000, cx);
                }
                if crate::updater::take_changed() {
                    // 后台自动下载好新版本：未开「以后自动下载并安装更新」且当前未打开设置面板时，弹出决策框；
                    // 开启了自动安装则不弹窗打扰，仅做轻提示并在侧栏显示蓝点徽标。
                    if let crate::updater::State::Ready { release, .. } = crate::updater::state() {
                        if this.update_notified.as_deref() != Some(release.version.as_str()) {
                            this.update_notified = Some(release.version.clone());
                            if crate::updater::auto_install_enabled(&this.store) {
                                this.show_toast_for(format!("新版本 {} 已下载好，退出应用时将自动安装", release.version), 6000, cx);
                            } else if !this.settings_open {
                                this.update_dialog_open = true;
                                this.update_notes_open = false;
                            }
                        }
                    }
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// 展开 / 收起右侧评论侧栏。
    #[cfg(feature = "dev")]
    pub fn toggle_comments_sidebar(&mut self, cx: &mut Context<Self>) {
        self.comments_open = !self.comments_open;
        if self.comments_open {
            self.editor.update(cx, |e, cx| {
                e.refresh_comments();
                cx.notify();
            });
        }
        cx.notify();
    }

    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_theme_global(window, cx);
        cx.notify();
        self.editor.update(cx, |_, cx| cx.notify());
    }

    pub fn show_toast(&mut self, msg: String, cx: &mut Context<Self>) {
        self.show_toast_for(msg, 2600, cx)
    }

    /// 成功类提示（复制、保存等）：飞书同款绿色提示条。
    pub fn show_toast_ok(&mut self, msg: String, cx: &mut Context<Self>) {
        self.show_toast_for(msg, 2200, cx);
        self.toast_kind = ToastKind::Success;
    }

    pub fn show_toast_for(&mut self, msg: String, ms: u64, cx: &mut Context<Self>) {
        self.toast = Some(crate::i18n::tr_owned(&msg));
        self.toast_kind = ToastKind::Info;
        self.toast_gen = self.toast_gen.wrapping_add(1);
        self.toast_task = Some(cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// 重新读文档列表（先等写队列写完，标题等才是最新的）。
    /// 文档列表签名：增删、改标题、保存（updated 变化）都会变。
    fn docs_sig(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for d in &self.docs {
            (&d.id, &d.parent, &d.title, d.sort.to_bits(), d.updated, d.deleted).hash(&mut h);
        }
        h.finish()
    }

    /// 文档列表变了：同步给编辑器的 `@` 菜单（最近修改的在前）。
    fn sync_doc_list(&mut self, cx: &mut Context<Self>) {
        let sig = self.docs_sig();
        if sig == self.doc_list_sig {
            return;
        }
        self.doc_list_sig = sig;
        let mut docs: Vec<&DocMeta> = self.docs.iter().filter(|d| !d.deleted).collect();
        docs.sort_by_key(|d| std::cmp::Reverse(d.updated));
        let list = docs.into_iter().map(|d| (d.id.clone(), d.title.clone())).collect();
        self.editor.update(cx, |e, _| e.set_doc_list(list));
    }

    /// 当前文档的反向链接（文档或文档列表变化时重新查询）。
    fn current_backlinks(&mut self) -> &[(String, String)] {
        let sig = self.docs_sig();
        if self.backlinks.0 != self.current || self.backlinks.1 != sig {
            self.backlinks = (self.current.clone(), sig, self.store.backlinks(&self.current));
        }
        &self.backlinks.2
    }

    /// 当前文档的未链接提及。
    fn current_mentions(&mut self) -> &[(String, String)] {
        let sig = self.docs_sig();
        if self.mentions.0 != self.current || self.mentions.1 != sig {
            let title = self.docs.iter().find(|d| d.id == self.current).map(|d| d.title.clone()).unwrap_or_default();
            self.mentions = (self.current.clone(), sig, self.store.unlinked_mentions(&self.current, &title));
        }
        &self.mentions.2
    }

    fn reload_docs(&mut self) {
        self.store.flush();
        self.docs = self.store.list();
    }

    /// 阅读模式按文档记在键值表里，下次打开同一篇仍是阅读模式。
    fn read_mode_key(id: &str) -> String {
        format!("read_only:{id}")
    }

    fn apply_read_mode(&mut self, cx: &mut Context<Self>) {
        let on = self.store.get_kv(&Self::read_mode_key(&self.current)).as_deref() == Some("1");
        self.editor.update(cx, |e, cx| e.set_read_only(on, cx));
    }

    fn toggle_read_mode(&mut self, _: &ToggleReadMode, _: &mut Window, cx: &mut Context<Self>) {
        let on = !self.editor.read(cx).is_read_only();
        self.editor.update(cx, |e, cx| {
            if on {
                // 先把未保存的修改写下去，阅读模式的快照就是磁盘上的内容
                e.save_now(cx);
            }
            e.set_read_only(on, cx)
        });
        self.store.set_kv(&Self::read_mode_key(&self.current), if on { "1" } else { "" });
        self.more_menu = false;
        self.show_toast(if on { "已切换为阅读模式".into() } else { "已切换为编辑模式".into() }, cx);
        cx.notify();
    }

    pub fn open_doc(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.current {
            window.focus(&self.editor.focus_handle(cx));
            return;
        }
        let Some((content, corrupt)) = self.store.load(id) else {
            self.show_toast("文档无法打开".into(), cx);
            return;
        };
        if let Some(c) = corrupt {
            self.show_toast(corrupt_msg(&c), cx);
        }
        self.last_title = content.title();
        self.current = id.to_string();
        self.store.set_kv("last_doc", id);
        self.editor.update(cx, |e, cx| e.load(id.to_string(), content, cx));
        self.apply_read_mode(cx);
        self.reload_docs();
        let mut p = self.docs.iter().find(|d| d.id == id).and_then(|d| d.parent.clone());
        while let Some(pid) = p {
            self.expanded.insert(pid.clone());
            p = self.docs.iter().find(|d| d.id == pid).and_then(|d| d.parent.clone());
        }
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn create_doc(&mut self, parent: Option<String>, content: DocContent, window: &mut Window, cx: &mut Context<Self>) -> String {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let id = self.store.create(parent.as_deref(), &content);
        if let Some(p) = parent {
            self.expanded.insert(p);
        }
        self.reload_docs();
        self.open_doc(&id, window, cx);
        id
    }

    fn new_doc(&mut self, _: &NewDoc, window: &mut Window, cx: &mut Context<Self>) {
        self.create_doc(None, DocContent::new_empty(""), window, cx);
    }

    fn new_sub_doc(&mut self, _: &NewSubDoc, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.current.clone();
        self.create_doc(Some(parent), DocContent::new_empty(""), window, cx);
    }

    /// 复制一篇文档（副本放在原文档同一层级，并打开副本）。
    fn duplicate_doc_id(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let content = if id == self.current {
            self.editor.read(cx).content.clone()
        } else {
            match self.store.load(&id) {
                Some((c, _)) => c,
                None => return,
            }
        };
        let parent = self.docs.iter().find(|d| d.id == id).and_then(|d| d.parent.clone());
        self.create_doc(parent, duplicate_content(content), window, cx);
        self.show_toast(crate::i18n::tr("已创建副本").into(), cx);
    }

    fn trash_doc_id(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        self.store.set_deleted(&id, true);
        self.reload_docs();
        let title = self.docs.iter().find(|d| d.id == id).map(|d| d.title.clone()).unwrap_or_default();
        if self.docs.iter().find(|d| d.id == self.current).is_none_or(|d| d.deleted) {
            let next = self.docs.iter().find(|d| !d.deleted).map(|d| d.id.clone());
            match next {
                Some(n) => self.open_doc(&n, window, cx),
                None => {
                    self.create_doc(None, DocContent::new_empty(""), window, cx);
                }
            }
        }
        let name = if title.is_empty() { "未命名文档" } else { &title };
        self.show_toast(format!("「{name}」已移到回收站，可在回收站恢复"), cx);
    }

    fn trash_doc(&mut self, _: &TrashDoc, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.current.clone();
        self.trash_doc_id(id, window, cx);
    }

    /// 永久删除前先确认：`purge` 会递归删掉所有子孙文档和评论，不可撤销。
    fn ask_purge(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(meta) = self.docs.iter().find(|d| d.id == id) else { return };
        let name = if meta.title.trim().is_empty() { "未命名文档".to_string() } else { meta.title.trim().to_string() };
        let (kids, alive) = tree::subtree_stats(&self.docs, &id);
        let english = crate::i18n::english();
        let title: SharedString = if english { format!("Delete “{name}” permanently?") } else { format!("永久删除「{name}」？") }.into();
        let body: SharedString = match (english, kids, alive) {
            (false, 0, _) => "删除后无法恢复。".into(),
            (false, n, 0) => format!("该文档下还有 {n} 篇子文档，会一起永久删除，删除后无法恢复。").into(),
            (false, n, m) => format!("该文档下还有 {n} 篇子文档会一起永久删除，其中有 {m} 篇不在回收站；删除后无法恢复。").into(),
            (true, 0, _) => "This can’t be undone.".into(),
            (true, n, 0) => format!("Its {n} sub-document(s) will be deleted too. This can’t be undone.").into(),
            (true, n, m) => format!("Its {n} sub-document(s) will be deleted too, including {m} not in the Trash. This can’t be undone.").into(),
        };
        self.confirm = Some(Confirm { title, body, ok: crate::i18n::tr("永久删除").into(), action: ConfirmAction::Purge(id) });
        // 焦点移进弹窗：不然 Esc / 回车会落到编辑器上
        window.focus(&self.confirm_focus);
        cx.notify();
    }

    /// 取消并关掉弹窗，把焦点还给编辑器。
    fn close_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.take().is_some() {
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
        }
    }

    /// 执行确认弹窗里的操作；当前文档被删掉时切到别的文档，避免停在已不存在的内容上。
    fn confirm_ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.confirm.take() else { return };
        match c.action {
            ConfirmAction::Purge(id) => {
                self.store.purge(&id);
                self.reload_docs();
                if self.docs.iter().find(|d| d.id == self.current).is_none_or(|d| d.deleted) {
                    match self.docs.iter().find(|d| !d.deleted).map(|d| d.id.clone()) {
                        Some(next) => self.open_doc(&next, window, cx),
                        None => {
                            self.create_doc(None, DocContent::new_empty(""), window, cx);
                        }
                    }
                }
                self.show_toast("已永久删除".into(), cx);
            }
        }
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar = !self.show_sidebar;
        self.focus_mode = false;
        cx.notify();
    }

    fn toggle_outline(&mut self, _: &ToggleOutline, _: &mut Window, cx: &mut Context<Self>) {
        self.show_outline = !self.show_outline;
        self.store.set_kv("outline", if self.show_outline { "1" } else { "0" });
        cx.notify();
    }

    fn focus_mode(&mut self, _: &FocusMode, _: &mut Window, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        self.show_toast(if self.focus_mode { "专注模式 · ⇧⌘F 退出".into() } else { "已退出专注模式".into() }, cx);
    }

    fn cycle_theme(&mut self, _: &CycleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let dark_now = theme(cx).dark;
        self.theme_mode = match self.theme_mode {
            ThemeMode::System => {
                if dark_now {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                }
            }
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Light,
        };
        self.store.set_kv(
            "theme",
            match self.theme_mode {
                ThemeMode::Light => "light",
                ThemeMode::Dark => "dark",
                ThemeMode::System => "system",
            },
        );
        self.apply_theme(window, cx);
    }

    fn follow_system_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_mode = ThemeMode::System;
        self.store.set_kv("theme", "system");
        self.apply_theme(window, cx);
        self.show_toast("主题已跟随系统".into(), cx);
    }

    fn toggle_perf(&mut self, _: &TogglePerf, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            e.show_perf = !e.show_perf;
            crate::base::stress::SHOW.store(e.show_perf, std::sync::atomic::Ordering::Relaxed);
            e.caches.frame_times.clear();
            cx.notify();
        });
        if let Some(v) = self.embed_view() {
            if let Ok(b) = v.downcast::<crate::base::editor::BaseEditor>() {
                b.update(cx, |_, cx| cx.notify());
            }
        }
    }

    fn stress_test(&mut self, _: &StressTest, window: &mut Window, cx: &mut Context<Self>) {
        let t0 = std::time::Instant::now();
        let content = seed::stress(1_000_000);
        let n = content.char_count();
        let blocks = content.blocks.len();
        self.create_doc(None, content, window, cx);
        self.editor.update(cx, |e, cx| {
            e.show_perf = true;
            e.caches.frame_times.clear();
            cx.notify();
        });
        self.show_toast(format!("已生成 {} 字 / {} 块，用时 {}ms · 滚动试试", fmt_num(n), fmt_num(blocks), t0.elapsed().as_millis()), cx);
    }

    /// 多维表格压测（T6）：10 万行 × 20 字段（5 个公式、2 个查找引用、1 个双向关联），插到文档并全屏打开。
    fn base_stress_test(&mut self, _: &BaseStressTest, window: &mut Window, cx: &mut Context<Self>) {
        let t0 = std::time::Instant::now();
        let id = crate::objects::create_base(crate::base::ViewKind::Grid);
        if let Some(b) = crate::objects::base(&id) {
            let mut b = b.borrow_mut();
            let first = b.meta.tables.first().cloned();
            crate::base::stress::build(&mut b, 100_000);
            if let Some(f) = first {
                b.del_table(&f);
                let mut m = b.meta.clone();
                m.tables.retain(|t| t != &f);
                b.set_meta(m);
            }
            b.meta.name = "多维表格压测".into();
            b.commit();
        }
        crate::objects::touch(&id);
        let gen = t0.elapsed().as_millis();
        self.editor.update(cx, |e, cx| e.insert_embed("base", &id, cx));
        crate::base::stress::SHOW.store(true, std::sync::atomic::Ordering::Relaxed);
        self.open_embed("base", &id, window, cx);
        self.show_toast(format!("已生成 10 万行压测多维表格，用时 {gen}ms · ⌥⌘P 看指标"), cx);
    }

    fn open_welcome(&mut self, _: &OpenWelcome, window: &mut Window, cx: &mut Context<Self>) {
        self.create_doc(None, seed::welcome(), window, cx);
    }

    /// 导出当前文档：`ext` 为 md / html / xml（飞书 DocxXML）。
    fn export(&mut self, ext: &'static str, cx: &mut Context<Self>) {
        let (title, body) = {
            let e = self.editor.read(cx);
            let blocks = &e.content.blocks;
            let title = e.content.title();
            let body = match ext {
                "html" => convert::full_html_page(&title, &convert::blocks_to_html(blocks)),
                "xml" => crate::docxml::export(blocks),
                _ => convert::blocks_to_md(blocks),
            };
            (title, body)
        };
        let name = format!("{}.{ext}", if title.trim().is_empty() { "未命名文档".to_string() } else { title.replace(['/', ':'], "-") });
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = std::fs::write(&path, body).is_ok();
                this.update(cx, |this, cx| {
                    if ok {
                        this.show_toast(format!("已导出到 {}", path.display()), cx);
                        cx.reveal_path(&path);
                    } else {
                        this.show_toast("导出失败".into(), cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn export_md(&mut self, _: &ExportMarkdown, _: &mut Window, cx: &mut Context<Self>) {
        self.export("md", cx)
    }

    fn export_html(&mut self, _: &ExportHtml, _: &mut Window, cx: &mut Context<Self>) {
        self.export("html", cx)
    }

    fn export_docxml(&mut self, _: &ExportDocxXml, _: &mut Window, cx: &mut Context<Self>) {
        self.export("xml", cx)
    }

    /// 导出完整备份（数据库 + assets）为一个 zip。
    fn export_backup(&mut self, _: &ExportBackup, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.save_now(cx));
        crate::objects::flush_dirty(true);
        let name = format!("石渠备份-{}.zip", chrono::Local::now().format("%Y-%m-%d-%H%M"));
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            this.update(cx, |this, cx| this.show_toast("正在导出备份…".into(), cx)).ok();
            let dest = path.clone();
            let res = cx.background_spawn(async move { crate::backup::export(&store, &dest) }).await;
            this.update(cx, |this, cx| match res {
                Ok(s) => {
                    this.show_toast(format!("备份完成：{} 个文件，{:.1} MB", s.files, s.bytes as f64 / 1048576.0), cx);
                    cx.reveal_path(&path);
                }
                Err(e) => this.show_toast_for(format!("备份失败：{e:#}"), 5000, cx),
            })
            .ok();
        })
        .detach();
    }

    /// 选一个备份 zip，校验后放入待恢复目录；重启石渠后生效（旧数据会保留在 recovery/ 下）。
    fn restore_backup(&mut self, _: &RestoreBackup, _: &mut Window, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("恢复".into()), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| this.show_toast("正在校验备份…".into(), cx)).ok();
            let res = cx.background_spawn(async move { crate::backup::stage_restore(&path) }).await;
            this.update(cx, |this, cx| match res {
                Ok(()) => this.show_toast_for("备份已就绪：请退出并重新打开石渠完成恢复（当前数据会保留在 recovery/ 目录）".into(), 12000, cx),
                Err(e) => this.show_toast_for(format!("恢复失败：{e:#}"), 5000, cx),
            })
            .ok();
        })
        .detach();
    }

    fn import_md(&mut self, _: &ImportMarkdown, _: &mut Window, cx: &mut Context<Self>) {
        let parent = self.docs.iter().find(|d| d.id == self.current && !d.deleted).map(|d| d.id.clone());
        self.import_md_into(parent, cx);
    }

    /// 弹出文件选择框，将选中的 Markdown / 飞书 DocxXML 导入到指定父文档下（`None` 为根目录）。
    fn import_md_into(&mut self, parent: Option<String>, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("导入".into()), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = rx.await {
                cx.update(|cx| {
                    let Some(window) = cx.active_window() else { return };
                    window
                        .update(cx, |_, window, cx| {
                            this.update(cx, |this, cx| this.import_files(paths, parent, window, cx)).ok();
                        })
                        .ok();
                })
                .ok();
            }
        })
        .detach();
    }

    /// 从外部拖入侧边栏的 Markdown / DocxXML 文件：仅挑选支持的扩展名导入到对应父文档下。
    fn import_external_paths(&mut self, paths: Vec<std::path::PathBuf>, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let matched: Vec<std::path::PathBuf> = paths
            .into_iter()
            .filter(|p| {
                p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "xml"))
            })
            .collect();
        if !matched.is_empty() {
            self.import_files(matched, parent, window, cx);
        }
    }

    fn import_files(&mut self, paths: Vec<std::path::PathBuf>, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        // 校验父文档仍然有效（未删除），批量导入的全部文件都放到同一个父文档下，避免相互嵌套
        let parent = parent.filter(|pid| self.docs.iter().any(|d| &d.id == pid && !d.deleted));
        let parent_title = parent.as_ref().and_then(|pid| self.docs.iter().find(|d| &d.id == pid)).map(|d| {
            if d.title.trim().is_empty() {
                crate::i18n::tr("未命名文档").to_string()
            } else {
                d.title.trim().to_string()
            }
        });
        let mut n = 0;
        let mut failed = vec![];
        for p in paths {
            let Ok(s) = std::fs::read_to_string(&p) else { continue };
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("导入的文档").to_string();
            // 飞书 / quiet.md 的 DocxXML
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
                match crate::docxml::import(&s) {
                    Ok(mut blocks) => {
                        if blocks[0].text.is_empty() {
                            blocks[0].text = crate::model::RichText::plain(stem);
                        }
                        self.create_doc(parent.clone(), DocContent::from_blocks(blocks), window, cx);
                        n += 1;
                    }
                    Err(e) => failed.push(format!("{stem}：{e}")),
                }
                continue;
            }
            let mut blocks = convert::md_to_blocks(&s);
            // 首个一级标题作为文档标题
            if let Some(first) = blocks.first() {
                if first.kind == (BlockKind::Heading { level: 1 }) {
                    let mut t = blocks.remove(0);
                    t.kind = BlockKind::Title;
                    blocks.insert(0, t);
                }
            }
            if blocks.first().is_none_or(|b| b.kind != BlockKind::Title) {
                blocks.insert(0, crate::model::Block::new(BlockKind::Title, crate::model::RichText::plain(stem)));
            }
            self.create_doc(parent.clone(), DocContent::from_blocks(blocks), window, cx);
            n += 1;
        }
        if let Some(first) = failed.first() {
            self.show_toast_for(format!("有 {} 个文件没能导入（{first}）", failed.len()), 6000, cx);
        } else if n > 0 {
            let msg = match (crate::i18n::english(), parent_title) {
                (false, Some(pt)) => format!("已导入 {n} 篇文档到「{pt}」下"),
                (false, None) => format!("已导入 {n} 篇文档"),
                (true, Some(pt)) => format!("Imported {n} document(s) into “{pt}”"),
                (true, None) => format!("Imported {n} document(s)"),
            };
            self.show_toast(msg, cx);
        }
    }

    fn about(&mut self, _: &About, _: &mut Window, cx: &mut Context<Self>) {
        self.about = true;
        cx.notify();
    }

    pub fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, _| e.save_now_queued());
        crate::flush_all();
        cx.quit();
    }

    // ───────────── 快速打开 ─────────────

    fn quick_open(&mut self, _: &QuickOpen, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_some() {
            self.close_switcher(window, cx);
            return;
        }
        self.editor.update(cx, |e, cx| e.save_now(cx));
        let input = cx.new(|cx| LineInput::new("搜索文档标题或正文…", cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => this.refresh_switcher(cx),
            InputEvent::Up => {
                if let Some(s) = &mut this.switcher {
                    s.selected = s.selected.saturating_sub(1);
                    cx.notify();
                }
            }
            InputEvent::Down => {
                if let Some(s) = &mut this.switcher {
                    s.selected = (s.selected + 1).min(s.results.len().saturating_sub(1));
                    cx.notify();
                }
            }
            InputEvent::Submit { .. } => {
                let id = this.switcher.as_ref().and_then(|s| s.results.get(s.selected)).map(|r| r.0.clone());
                this.close_switcher(window, cx);
                if let Some(id) = id {
                    this.open_doc(&id, window, cx);
                }
            }
            InputEvent::Cancel => this.close_switcher(window, cx),
        });
        window.focus(&input.focus_handle(cx));
        self.switcher = Some(Switcher { input, results: vec![], selected: 0, _sub: sub });
        self.refresh_switcher(cx);
    }

    fn refresh_switcher(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.switcher else { return };
        let q = s.input.read(cx).text();
        let results = if q.trim().is_empty() {
            let mut recent: Vec<&DocMeta> = self.docs.iter().filter(|d| !d.deleted).collect();
            recent.sort_by_key(|d| std::cmp::Reverse(d.updated));
            recent.into_iter().take(12).map(|d| (d.id.clone(), d.title.clone(), String::new())).collect()
        } else {
            self.store.search(q.trim(), 30)
        };
        let s = self.switcher.as_mut().unwrap();
        s.results = results;
        s.selected = 0;
        cx.notify();
    }

    fn close_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = None;
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    // ───────────── 文档层级移动 ─────────────

    fn doc_display_title(&self, id: &str) -> String {
        self.docs
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.title.trim())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| crate::i18n::tr("未命名文档").to_string())
    }

    /// 按指定落点移动文档（支持跨层级移动与同级排序，自动防成环并展开新父文档）。
    fn move_doc_to_zone(&mut self, doc_id: &str, zone: tree::DropZone, cx: &mut Context<Self>) {
        let Some(plan) = tree::plan_move(&self.docs, doc_id, &zone) else { return };
        let old_parent = self.docs.iter().find(|d| d.id == doc_id).and_then(|d| d.parent.clone());
        let doc_title = self.doc_display_title(doc_id);
        let target_title = plan.new_parent.as_deref().map(|pid| self.doc_display_title(pid));
        if !self.store.move_doc(doc_id, plan.new_parent.as_deref(), &plan.sibling_order) {
            return;
        }
        if let Some(pid) = &plan.new_parent {
            self.expanded.insert(pid.clone());
        }
        self.reload_docs();
        if old_parent != plan.new_parent {
            let msg = match (crate::i18n::english(), target_title) {
                (false, Some(pt)) => format!("已将「{doc_title}」移到「{pt}」下"),
                (false, None) => format!("已将「{doc_title}」移到根目录"),
                (true, Some(pt)) => format!("Moved “{doc_title}” into “{pt}”"),
                (true, None) => format!("Moved “{doc_title}” to root"),
            };
            self.show_toast(msg, cx);
        }
        cx.notify();
    }

    fn move_current_doc(&mut self, _: &MoveDoc, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.current.clone();
        self.open_move_picker(id, window, cx);
    }

    fn open_move_picker(&mut self, doc_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.docs.iter().any(|d| d.id == doc_id && !d.deleted) {
            return;
        }
        self.more_menu = false;
        self.doc_menu = None;
        let input = cx.new(|cx| LineInput::new("搜索目标父文档或选择根目录…", cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => this.refresh_move_picker(cx),
            InputEvent::Up => {
                if let Some(p) = &mut this.move_picker {
                    p.selected = p.selected.saturating_sub(1);
                    cx.notify();
                }
            }
            InputEvent::Down => {
                if let Some(p) = &mut this.move_picker {
                    p.selected = (p.selected + 1).min(p.targets.len().saturating_sub(1));
                    cx.notify();
                }
            }
            InputEvent::Submit { .. } => {
                let picked = this.move_picker.as_ref().and_then(|p| p.targets.get(p.selected).map(|t| (p.doc_id.clone(), t.parent.clone(), t.is_current)));
                this.close_move_picker(window, cx);
                if let Some((doc_id, parent, is_current)) = picked {
                    if !is_current {
                        let zone = match parent {
                            Some(pid) => tree::DropZone::Into(pid),
                            None => tree::DropZone::RootEnd,
                        };
                        this.move_doc_to_zone(&doc_id, zone, cx);
                    }
                }
            }
            InputEvent::Cancel => this.close_move_picker(window, cx),
        });
        window.focus(&input.focus_handle(cx));
        self.move_picker = Some(MovePicker { doc_id, input, targets: vec![], selected: 0, _sub: sub });
        self.refresh_move_picker(cx);
    }

    fn refresh_move_picker(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &self.move_picker else { return };
        let q = p.input.read(cx).text();
        let targets = tree::move_targets(&self.docs, &p.doc_id, &q);
        let p = self.move_picker.as_mut().unwrap();
        p.targets = targets;
        p.selected = p.targets.iter().position(|t| !t.is_current).unwrap_or(0);
        cx.notify();
    }

    fn close_move_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.move_picker.take().is_some() {
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
        }
    }

    fn render_move_picker(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.move_picker.as_ref()?;
        let q = p.input.read(cx).text();
        let searching = !q.trim().is_empty();
        let doc_title = self.doc_display_title(&p.doc_id);
        let doc_id = p.doc_id.clone();
        let heading = if crate::i18n::english() { format!("Move “{doc_title}” to…") } else { format!("将「{doc_title}」移动到…") };
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.overlay)
                .flex()
                .flex_col()
                .items_center()
                .pt(px(110.))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, w, cx| this.close_move_picker(w, cx)))
                .child(
                    div()
                        .w(px(540.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(12.))
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .px(px(16.))
                                .pt(px(12.))
                                .pb(px(4.))
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text_muted)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(heading),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(10.))
                                .px(px(16.))
                                .h(px(46.))
                                .border_b_1()
                                .border_color(t.border)
                                .text_size(px(15.))
                                .line_height(px(22.))
                                .child(icon("search", 16., t.text_muted))
                                .child(p.input.clone()),
                        )
                        .child(
                            div()
                                .id("mp-results")
                                .max_h(px(400.))
                                .overflow_y_scroll()
                                .p(px(6.))
                                .when(p.targets.is_empty(), |d| {
                                    d.child(div().px_3().py_4().text_size(px(13.)).text_color(t.text_muted).child(crate::i18n::tr("没有可移动的目标位置")))
                                })
                                .children(p.targets.iter().enumerate().map(|(i, target)| {
                                    let doc_id = doc_id.clone();
                                    let parent = target.parent.clone();
                                    let is_current = target.is_current;
                                    let is_root = target.parent.is_none();
                                    let sel = i == p.selected;
                                    let indent = if searching || is_root { 10. } else { 10. + target.depth as f32 * 18. };
                                    let label = if is_root { crate::i18n::tr("根目录（顶层）").to_string() } else { target.title.clone() };
                                    div()
                                        .id(("mp", i))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(8.))
                                        .pl(px(indent))
                                        .pr(px(10.))
                                        .py(px(8.))
                                        .rounded(px(8.))
                                        .when(!is_current, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                                        .when(is_current, |d| d.opacity(0.6))
                                        .when(sel && !is_current, |d| d.bg(t.active))
                                        .child(icon(if is_root { "folder" } else { "file" }, 15., if sel && !is_current { t.accent } else { t.text_muted }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div()
                                                        .text_size(px(14.))
                                                        .text_color(t.text)
                                                        .whitespace_nowrap()
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(label),
                                                )
                                                .when(searching && !target.path.is_empty(), |d| {
                                                    d.child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .text_color(t.text_muted)
                                                            .whitespace_nowrap()
                                                            .overflow_hidden()
                                                            .text_ellipsis()
                                                            .child(target.path.clone()),
                                                    )
                                                }),
                                        )
                                        .when(is_current, |d| d.child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("当前所在"))))
                                        .when(sel && !is_current, |d| {
                                            d.child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("↵ 移动")))
                                        })
                                        .when(!is_current, |d| {
                                            d.on_click(cx.listener(move |this, _, w, cx| {
                                                this.close_move_picker(w, cx);
                                                let zone = match &parent {
                                                    Some(pid) => tree::DropZone::Into(pid.clone()),
                                                    None => tree::DropZone::RootEnd,
                                                };
                                                this.move_doc_to_zone(&doc_id, zone, cx);
                                            }))
                                        })
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(14.))
                                .px(px(16.))
                                .py(px(8.))
                                .border_t_1()
                                .border_color(t.border)
                                .text_size(px(11.))
                                .text_color(t.text_muted)
                                .child(crate::i18n::tr("↑↓ 选择"))
                                .child(crate::i18n::tr("↵ 移动"))
                                .child(crate::i18n::tr("esc 关闭")),
                        ),
                ),
        )
    }

    fn render_doc_menu(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.doc_menu.as_ref()?;
        let doc = self.docs.iter().find(|d| d.id == menu.doc_id && !d.deleted)?;
        let has_parent = doc.parent.is_some();
        let (id1, id2, id3, id4, id5, id6) = (doc.id.clone(), doc.id.clone(), doc.id.clone(), doc.id.clone(), doc.id.clone(), doc.id.clone());
        let item = |id: &'static str, ic: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 14., t.text_secondary))
                .child(div().flex_1().child(crate::i18n::tr(label)))
        };
        let sep = || div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(t.border);
        Some(
            deferred(
                anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("doc-ctx-menu")
                        .occlude()
                        .w(px(236.))
                        .p(px(6.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(10.))
                        .shadow_lg()
                        .text_size(px(13.))
                        .text_color(t.text)
                        .child(item("ctx-sub", "plus", "新建子文档").on_click(cx.listener(move |this, _, w, cx| {
                            this.doc_menu = None;
                            this.create_doc(Some(id1.clone()), DocContent::new_empty(""), w, cx);
                        })))
                        .child(item("ctx-imp", "upload", "导入 Markdown 到此文档下…").on_click(cx.listener(move |this, _, _, cx| {
                            this.doc_menu = None;
                            this.import_md_into(Some(id2.clone()), cx);
                            cx.notify();
                        })))
                        .child(sep())
                        .child(item("ctx-move", "folder", "移动到…").on_click(cx.listener(move |this, _, w, cx| {
                            this.doc_menu = None;
                            this.open_move_picker(id3.clone(), w, cx);
                        })))
                        .when(has_parent, |d| {
                            d.child(item("ctx-root", "arrow-up-to-line", "移到根目录").on_click(cx.listener(move |this, _, _, cx| {
                                this.doc_menu = None;
                                this.move_doc_to_zone(&id4, tree::DropZone::RootEnd, cx);
                            })))
                        })
                        .child(item("ctx-dup", "copy", "创建副本").on_click(cx.listener(move |this, _, w, cx| {
                            this.doc_menu = None;
                            this.duplicate_doc_id(id5.clone(), w, cx);
                        })))
                        .child(sep())
                        .child(item("ctx-trash", "trash", "移到回收站").on_click(cx.listener(move |this, _, w, cx| {
                            this.doc_menu = None;
                            this.trash_doc_id(id6.clone(), w, cx);
                        })))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.doc_menu = None;
                            cx.notify();
                        })),
                ),
            )
            .with_priority(6),
        )
    }

    // ───────────── 渲染 ─────────────

    fn children_of(&self) -> HashMap<Option<String>, Vec<usize>> {
        let mut map: HashMap<Option<String>, Vec<usize>> = HashMap::new();
        let alive: HashSet<&str> = self.docs.iter().filter(|d| !d.deleted).map(|d| d.id.as_str()).collect();
        for (i, d) in self.docs.iter().enumerate() {
            if d.deleted {
                continue;
            }
            let parent = d.parent.clone().filter(|p| alive.contains(p.as_str()));
            map.entry(parent).or_default().push(i);
        }
        map
    }

    fn render_tree(
        &self,
        parent: Option<String>,
        depth: usize,
        map: &HashMap<Option<String>, Vec<usize>>,
        t: &Theme,
        out: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let Some(kids) = map.get(&parent) else { return };
        let dragging = cx.has_active_drag();
        for &i in kids {
            let d = &self.docs[i];
            let id = d.id.clone();
            let has_kids = map.get(&Some(id.clone())).is_some_and(|k| !k.is_empty());
            let open = self.expanded.contains(&id);
            let active = id == self.current;
            let title_str = if d.title.trim().is_empty() { "未命名文档".to_string() } else { d.title.clone() };
            let title: SharedString = title_str.clone().into();
            let indent = 6. + depth as f32 * 16.;
            let is_before = dragging && self.doc_drop_target.as_ref() == Some(&tree::DropZone::Before(id.clone()));
            let is_into = dragging && self.doc_drop_target.as_ref() == Some(&tree::DropZone::Into(id.clone()));
            let is_after = dragging && self.doc_drop_target.as_ref() == Some(&tree::DropZone::After(id.clone()));
            let (id1, id2, id3, id4, id5, id_move, id_drop, id_ext, id_ctx) =
                (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
            let accent = t.accent;
            let accent_soft = t.accent_soft;
            let expanded_with_kids = has_kids && open;
            out.push(
                div()
                    .id(SharedString::from(format!("doc-{id}")))
                    .relative()
                    .group("doc")
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(32.))
                    .pl(px(indent))
                    .pr(px(6.))
                    .mx(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(14.))
                    .when(active && !is_into, |d| d.bg(t.active).text_color(t.accent).font_weight(FontWeight::MEDIUM))
                    .when(!active && !is_into, |d| d.text_color(t.text).hover(|s| s.bg(t.hover)))
                    .when(is_into, |d| d.bg(accent_soft).border_1().border_color(accent).text_color(accent))
                    .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent_soft).border_1().border_color(accent))
                    .on_drag(DragDoc { id: id.clone(), title: title_str }, |d: &DragDoc, _, _, cx| cx.new(|_| DocDragPreview(d.title.clone())))
                    .on_drag_move::<DragDoc>(cx.listener(move |this, ev: &DragMoveEvent<DragDoc>, _, cx| {
                        if !ev.bounds.contains(&ev.event.position) {
                            return;
                        }
                        let h = f32::from(ev.bounds.size.height).max(1.0);
                        let rel = (f32::from(ev.event.position.y) - f32::from(ev.bounds.origin.y)) / h;
                        let zone = if rel < 0.25 {
                            tree::DropZone::Before(id_move.clone())
                        } else if rel > 0.75 && !expanded_with_kids {
                            tree::DropZone::After(id_move.clone())
                        } else {
                            tree::DropZone::Into(id_move.clone())
                        };
                        let drag_id = ev.drag(cx).id.clone();
                        let next = tree::plan_move(&this.docs, &drag_id, &zone).map(|_| zone);
                        if this.doc_drop_target != next {
                            this.doc_drop_target = next;
                            cx.notify();
                        }
                    }))
                    .on_drop(cx.listener(move |this, d: &DragDoc, _, cx| {
                        let zone = this.doc_drop_target.take().unwrap_or_else(|| tree::DropZone::Into(id_drop.clone()));
                        this.move_doc_to_zone(&d.id, zone, cx);
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.import_external_paths(paths.paths().to_vec(), Some(id_ext.clone()), window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.more_menu = false;
                            this.doc_menu = Some(DocContextMenu { doc_id: id_ctx.clone(), position: ev.position });
                            cx.notify();
                        }),
                    )
                    .when(is_before, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(-1.))
                                .left(px(indent))
                                .right(px(6.))
                                .h(px(2.))
                                .bg(accent)
                                .child(div().absolute().left(px(-3.)).top(px(-2.)).size(px(6.)).rounded_full().bg(accent)),
                        )
                    })
                    .when(is_after, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(-1.))
                                .left(px(indent))
                                .right(px(6.))
                                .h(px(2.))
                                .bg(accent)
                                .child(div().absolute().left(px(-3.)).top(px(-2.)).size(px(6.)).rounded_full().bg(accent)),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("chev-{id}")))
                            .size(px(18.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .when(has_kids, |d| {
                                d.hover(|s| s.bg(t.hover)).child(icon(if open { "chevron-down" } else { "chevron-right" }, 13., t.text_muted)).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !this.expanded.remove(&id1) {
                                            this.expanded.insert(id1.clone());
                                        }
                                        cx.notify();
                                    }),
                                )
                            }),
                    )
                    .child(div().ml(px(2.)).mr(px(6.)).child(icon("file", 15., if active || is_into { t.accent } else { t.text_muted })))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when(d.title.trim().is_empty(), |d| d.text_color(t.text_muted))
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .invisible()
                            .group_hover("doc", |s| s.visible())
                            .child(
                                div()
                                    .id(SharedString::from(format!("add-{id}")))
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("plus", 14., t.text_secondary))
                                    .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("新建子文档").into())).into())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.create_doc(Some(id2.clone()), DocContent::new_empty(""), window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("dup-{id}")))
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("copy", 13., t.text_secondary))
                                    .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("创建副本").into())).into())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.duplicate_doc_id(id5.clone(), window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("del-{id}")))
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .hover(|s| s.bg(t.hover))
                                    .child(icon("trash", 13., t.text_secondary))
                                    .tooltip(|_, cx| cx.new(|_| Tooltip(crate::i18n::tr("移到回收站").into())).into())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.trash_doc_id(id3.clone(), window, cx);
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_doc(&id4, window, cx)))
                    .into_any_element(),
            );
            if has_kids && open {
                self.render_tree(Some(id.clone()), depth + 1, map, t, out, cx);
            }
        }
    }

    fn render_sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let map = self.children_of();
        let mut rows = Vec::new();
        self.render_tree(None, 0, &map, t, &mut rows, cx);
        let trashed: Vec<&DocMeta> = self.docs.iter().filter(|d| d.deleted).collect();
        let trash_count = trashed.len();
        let nav_item = |id: &'static str, ic: &'static str, label: &'static str, hint: String| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .h(px(32.))
                .px(px(10.))
                .mx(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(14.))
                .text_color(t.text)
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 15., t.text_secondary))
                .child(div().flex_1().child(crate::i18n::tr(label)))
                .child(div().text_size(px(11.)).text_color(t.text_muted).child(hint))
        };
        let theme_icon = match self.theme_mode {
            ThemeMode::System => "monitor",
            ThemeMode::Light => "sun",
            ThemeMode::Dark => "moon",
        };
        div()
            .w(px(256.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(t.sidebar_bg)
            .border_r_1()
            .border_color(t.border)
            .child(
                // 顶部：留出红绿灯位置 + Logo
                div()
                    .h(px(52.))
                    .flex()
                    .flex_row()
                    .items_end()
                    .justify_end()
                    .px(px(10.))
                    .pb(px(6.))
                    .child(div().flex_1().h_full().on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| title_drag(ev, window)))
                    .child(
                        icon_button("collapse", "sidebar", "隐藏侧边栏 ⌘\\", cx)
                            .on_click(cx.listener(|this, _, w, cx| this.toggle_sidebar(&ToggleSidebar, w, cx))),
                    ),
            )
            .child(
                div().flex().flex_row().items_center().gap(px(10.)).px(px(18.)).pb(px(14.)).child(logo(30.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(crate::store::APP_NAME))
                        .child(div().text_size(px(11.)).text_color(t.text_muted).child(format!(
                            "{} · {}",
                            crate::store::APP_NAME_EN,
                            crate::i18n::tr("本地文档")
                        ))),
                ),
            )
            .child(
                div()
                    .id("search-box")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .mx(px(12.))
                    .mb(px(6.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .bg(t.bg)
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .hover(|s| s.border_color(t.border_strong))
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child(icon("search", 14., t.text_muted))
                    .child(div().flex_1().child(crate::i18n::tr("搜索")))
                    .child(div().text_size(px(11.)).child(crate::shortcuts::hint("stonebrook::QuickOpen", cx)))
                    .on_click(cx.listener(|this, _, w, cx| this.quick_open(&QuickOpen, w, cx))),
            )
            .child(
                nav_item("nav-new", "plus", "新建文档", crate::shortcuts::hint("stonebrook::NewDoc", cx))
                    .on_click(cx.listener(|this, _, w, cx| this.new_doc(&NewDoc, w, cx))),
            )
            .child({
                let dragging = cx.has_active_drag();
                let root_start = dragging && self.doc_drop_target == Some(tree::DropZone::RootStart);
                let root_end = dragging && self.doc_drop_target == Some(tree::DropZone::RootEnd);
                let (accent, accent_soft) = (t.accent, t.accent_soft);
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .on_drag_move::<DragDoc>(cx.listener(|this, ev: &DragMoveEvent<DragDoc>, _, cx| {
                        if !ev.bounds.contains(&ev.event.position) && this.doc_drop_target.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .id("tree-root-head")
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .mx(px(8.))
                            .mt(px(10.))
                            .mb(px(4.))
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .text_size(px(12.))
                            .text_color(if root_start { accent } else { t.text_muted })
                            .when(root_start, |d| d.bg(accent_soft).border_1().border_color(accent))
                            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent_soft).border_1().border_color(accent))
                            .on_drag_move::<DragDoc>(cx.listener(|this, ev: &DragMoveEvent<DragDoc>, _, cx| {
                                if !ev.bounds.contains(&ev.event.position) {
                                    return;
                                }
                                let drag_id = ev.drag(cx).id.clone();
                                let zone = tree::DropZone::RootStart;
                                let next = tree::plan_move(&this.docs, &drag_id, &zone).map(|_| zone);
                                if this.doc_drop_target != next {
                                    this.doc_drop_target = next;
                                    cx.notify();
                                }
                            }))
                            .on_drop(cx.listener(|this, d: &DragDoc, _, cx| {
                                this.doc_drop_target = None;
                                this.move_doc_to_zone(&d.id, tree::DropZone::RootStart, cx);
                            }))
                            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                this.import_external_paths(paths.paths().to_vec(), None, window, cx);
                            }))
                            .child(crate::i18n::tr("我的文档"))
                            .when(root_start, |d| d.child(div().text_size(px(11.)).text_color(accent).child(crate::i18n::tr("移到根目录")))),
                    )
                    .child(
                        div().id("tree").flex_1().min_h(px(0.)).flex().flex_col().overflow_y_scroll().pb_2().children(rows).child(
                            div()
                                .id("tree-root-drop")
                                .relative()
                                .flex_1()
                                .min_h(px(64.))
                                .mx(px(8.))
                                .mt(px(2.))
                                .rounded(px(6.))
                                .flex()
                                .items_start()
                                .justify_center()
                                .pt(px(10.))
                                .when(root_end, |d| {
                                    d.bg(accent_soft)
                                        .child(
                                            div()
                                                .absolute()
                                                .top(px(0.))
                                                .left(px(6.))
                                                .right(px(6.))
                                                .h(px(2.))
                                                .bg(accent)
                                                .child(div().absolute().left(px(-3.)).top(px(-2.)).size(px(6.)).rounded_full().bg(accent)),
                                        )
                                        .child(div().text_size(px(11.5)).text_color(accent).child(crate::i18n::tr("松手移到根目录")))
                                })
                                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent_soft).border_1().border_color(accent))
                                .on_drag_move::<DragDoc>(cx.listener(|this, ev: &DragMoveEvent<DragDoc>, _, cx| {
                                    if !ev.bounds.contains(&ev.event.position) {
                                        return;
                                    }
                                    let drag_id = ev.drag(cx).id.clone();
                                    let zone = tree::DropZone::RootEnd;
                                    let next = tree::plan_move(&this.docs, &drag_id, &zone).map(|_| zone);
                                    if this.doc_drop_target != next {
                                        this.doc_drop_target = next;
                                        cx.notify();
                                    }
                                }))
                                .on_drop(cx.listener(|this, d: &DragDoc, _, cx| {
                                    this.doc_drop_target = None;
                                    this.move_doc_to_zone(&d.id, tree::DropZone::RootEnd, cx);
                                }))
                                .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                    this.import_external_paths(paths.paths().to_vec(), None, window, cx);
                                })),
                        ),
                    )
            })
            .when(self.show_trash, |d| {
                d.child(
                    div()
                        .id("trash-list")
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(t.border)
                        .py_1()
                        .when(trashed.is_empty(), |d| {
                            d.child(div().px(px(18.)).py_2().text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr("回收站是空的")))
                        })
                        .children(trashed.iter().map(|d| {
                            let (a, b) = (d.id.clone(), d.id.clone());
                            div()
                                .id(SharedString::from(format!("tr-{}", d.id)))
                                .group("tr")
                                .flex()
                                .flex_row()
                                .items_center()
                                .h(px(30.))
                                .mx(px(8.))
                                .px(px(10.))
                                .rounded(px(6.))
                                .text_size(px(13.))
                                .text_color(t.text_secondary)
                                .hover(|s| s.bg(t.hover))
                                .child(div().flex_1().min_w(px(0.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(if d.title.is_empty() {
                                    "未命名文档".to_string()
                                } else {
                                    d.title.clone()
                                }))
                                .child(icon_button(SharedString::from(format!("rs-{}", d.id)), "restore", "恢复", cx).on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.store.set_deleted(&a, false);
                                        this.reload_docs();
                                        this.open_doc(&a.clone(), window, cx);
                                        this.show_toast("已恢复".into(), cx);
                                    },
                                )))
                                .child(
                                    icon_button(SharedString::from(format!("pg-{}", d.id)), "x", "永久删除", cx)
                                        .on_click(cx.listener(move |this, _, window, cx| this.ask_purge(b.clone(), window, cx))),
                                )
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .px(px(10.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .id("trash-toggle")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .flex_1()
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(t.text_secondary)
                            .when(self.show_trash, |d| d.bg(t.hover))
                            .hover(|s| s.bg(t.hover))
                            .child(icon("trash", 14., t.text_secondary))
                            .child(format!("{}{}", crate::i18n::tr("回收站"), if trash_count > 0 { format!(" · {trash_count}") } else { String::new() }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_trash = !this.show_trash;
                                cx.notify();
                            })),
                    )
                    .child({
                        let has_update = matches!(crate::updater::state(), crate::updater::State::Ready { .. });
                        let tip = if has_update { "设置 ⌘,（有新版本可安装）" } else { "设置 ⌘," };
                        div()
                            .relative()
                            .child(icon_button("settings", "settings", tip, cx).on_click(cx.listener(move |this, _, _, cx| {
                                if has_update {
                                    this.show_settings_tab(SettingsTab::Updates, cx);
                                } else {
                                    this.show_settings(cx);
                                }
                            })))
                            .when(has_update, |d| {
                                d.child(
                                    div().absolute().top(px(3.)).right(px(3.)).size(px(7.)).rounded_full().bg(t.accent).border_1().border_color(t.sidebar_bg),
                                )
                            })
                    })
                    .child(
                        icon_button("theme", theme_icon, "切换深浅色 ⇧⌘D（右键跟随系统）", cx)
                            .on_click(cx.listener(|this, _, w, cx| this.cycle_theme(&CycleTheme, w, cx)))
                            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, w, cx| this.follow_system_theme(w, cx))),
                    )
                    .child(icon_button("about", "info", "关于石渠", cx).on_click(cx.listener(|this, _, w, cx| this.about(&About, w, cx)))),
            )
    }

    fn render_topbar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let chars = self.editor.update(cx, |e, _| e.char_count());
        let editor = self.editor.read(cx);
        let saved = editor.is_saved();
        let read_only = editor.is_read_only();
        // 面包屑：祖先文档
        let mut crumbs: Vec<(String, String)> = Vec::new();
        let mut p = self.docs.iter().find(|d| d.id == self.current).and_then(|d| d.parent.clone());
        while let Some(pid) = p {
            if let Some(d) = self.docs.iter().find(|d| d.id == pid) {
                crumbs.insert(0, (d.id.clone(), d.title.clone()));
                p = d.parent.clone();
            } else {
                break;
            }
        }
        let title = editor.content.title();
        let sidebar_hidden = !self.show_sidebar || self.focus_mode;
        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .pl(px(if sidebar_hidden { 84. } else { 16. }))
            .pr(px(12.))
            .when(sidebar_hidden, |d| {
                d.child(icon_button("show-sidebar", "sidebar", "显示侧边栏 ⌘\\", cx).on_click(cx.listener(|this, _, w, cx| {
                    this.focus_mode = false;
                    this.show_sidebar = true;
                    let _ = w;
                    cx.notify();
                })))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(13.))
                    .text_color(t.text_secondary)
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| title_drag(ev, window))
                    .children(crumbs.into_iter().map(|(id, title)| {
                        let id2 = id.clone();
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                div()
                                    .id(SharedString::from(format!("crumb-{id}")))
                                    .px(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.hover))
                                    .whitespace_nowrap()
                                    .child(if title.is_empty() { "未命名文档".to_string() } else { title })
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                    .on_click(cx.listener(move |this, _, w, cx| this.open_doc(&id2, w, cx))),
                            )
                            .child(div().text_color(t.text_muted).child("/"))
                    }))
                    .child(div().px(px(4.)).whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(t.text).child(if title.is_empty() {
                        "未命名文档".to_string()
                    } else {
                        title
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .when(read_only, |d| {
                        d.child(
                            div()
                                .id("read-mode-pill")
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(10.))
                                .bg(t.hover)
                                .text_color(t.text_secondary)
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.accent))
                                .child(crate::i18n::tr("阅读模式"))
                                .on_click(cx.listener(|this, _, w, cx| this.toggle_read_mode(&ToggleReadMode, w, cx))),
                        )
                    })
                    .child(div().size(px(6.)).rounded_full().bg(if saved { t.fg_palette[4] } else { t.fg_palette[2] }))
                    .child(crate::i18n::tr(if saved { "已保存到本地" } else { "保存中…" }))
                    .child(div().text_color(t.border_strong).child("·"))
                    .child(if crate::i18n::english() {
                        format!("{} char{}", fmt_num(chars), if chars == 1 { "" } else { "s" })
                    } else {
                        format!("{} 字", fmt_num(chars))
                    }),
            )
            .child(div().w(px(8.)))
            .child({
                let n_open = self.editor.read(cx).comments.iter().filter(|c| !c.resolved).count();
                let has_any = !self.editor.read(cx).comments.is_empty();
                div()
                    .relative()
                    .child(icon_button("comments-btn", "comment", "评论", cx).when(self.comments_open, |d| d.bg(t.hover)).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.comments_open = !this.comments_open;
                            this.editor.update(cx, |e, cx| {
                                e.refresh_comments();
                                cx.notify();
                            });
                            cx.notify();
                        },
                    )))
                    .when(has_any, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(1.))
                                .right(px(2.))
                                .min_w(px(14.))
                                .h(px(14.))
                                .px(px(3.))
                                .rounded_full()
                                .bg(t.accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(9.5))
                                .text_color(gpui::white())
                                .child(if n_open > 0 { n_open.to_string() } else { "✓".to_string() }),
                        )
                    })
            })
            .child(
                icon_button("outline-btn", "outline", "大纲 ⇧⌘O", cx)
                    .when(self.show_outline, |d| d.bg(t.hover))
                    .on_click(cx.listener(|this, _, w, cx| this.toggle_outline(&ToggleOutline, w, cx))),
            )
            .child(icon_button("focus-btn", "focus", "专注模式 ⇧⌘F", cx).on_click(cx.listener(|this, _, w, cx| this.focus_mode(&FocusMode, w, cx))))
            .child(
                div()
                    .relative()
                    .child(icon_button("more-btn", "more", "更多", cx).when(self.more_menu, |d| d.bg(t.hover)).on_click(cx.listener(|this, _, _, cx| {
                        this.more_menu = !this.more_menu;
                        cx.notify();
                    })))
                    .when(self.more_menu, |d| {
                        d.child(
                            deferred(
                                anchored()
                                    .anchor(Corner::TopRight)
                                    .offset(point(px(28.), px(34.)))
                                    .snap_to_window_with_margin(px(8.))
                                    .child(self.render_more_menu(t, cx)),
                            )
                            .with_priority(5),
                        )
                    }),
            )
    }

    fn render_more_menu(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |id: &'static str, ic: &'static str, label: &'static str, hint: String| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(ic, 15., t.text_secondary))
                .child(div().flex_1().child(crate::i18n::tr(label)))
                .child(div().text_size(px(11.)).text_color(t.text_muted).child(hint))
        };
        let sep = || div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(t.border);
        macro_rules! act {
            ($a:expr) => {
                cx.listener(|this, _, w, cx| {
                    this.more_menu = false;
                    w.dispatch_action(Box::new($a), cx);
                    cx.notify();
                })
            };
        }
        div()
            .id("more-menu")
            .occlude()
            .w(px(264.))
            .p(px(6.))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border)
            .rounded(px(10.))
            .shadow_lg()
            .text_size(px(13.))
            .text_color(t.text)
            .child(item("m-sub", "plus", "新建子文档", crate::shortcuts::hint("stonebrook::NewSubDoc", cx)).on_click(act!(NewSubDoc)))
            .child(item("m-move", "folder", "移动到…", String::new()).on_click(act!(MoveDoc)))
            .child(item("m-dup-doc", "copy", "创建副本", String::new()).on_click(cx.listener(|this, _, w, cx| {
                this.more_menu = false;
                let id = this.current.clone();
                this.duplicate_doc_id(id, w, cx);
            })))
            .child(item("m-img", "image", "插入图片…", String::new()).on_click(cx.listener(|this, _, w, cx| {
                this.more_menu = false;
                this.editor.update(cx, |e, cx| e.pick_images(w, cx));
            })))
            .child(sep())
            .child(item("m-tpl", "layout-template", "从模板新建…", String::new()).on_click(cx.listener(|this, _, _, cx| this.open_templates(true, cx))))
            .child(item("m-save-tpl", "layout-template", "另存为模板", String::new()).on_click(cx.listener(|this, _, _, cx| this.save_as_template(cx))))
            .child(item("m-mind", "mindmap", "以思维导图查看", String::new()).on_click(cx.listener(|this, _, w, cx| {
                this.more_menu = false;
                this.open_doc_mindmap(w, cx);
            })))
            .child(sep())
            .child(
                item("m-imp", "upload", "导入 Markdown / 飞书 DocxXML…", crate::shortcuts::hint("stonebrook::ImportMarkdown", cx))
                    .on_click(act!(ImportMarkdown)),
            )
            .child(item("m-md", "download", "导出为 Markdown", crate::shortcuts::hint("stonebrook::ExportMarkdown", cx)).on_click(act!(ExportMarkdown)))
            .child(item("m-html", "download", "导出为 HTML", String::new()).on_click(act!(ExportHtml)))
            .child(item("m-docxml", "download", "导出为飞书 DocxXML", String::new()).on_click(act!(ExportDocxXml)))
            .child(item("m-pdf", "download", "导出为 PDF", String::new()).on_click(act!(ExportPdf)))
            .child(item("m-bk", "download", "导出完整备份…", String::new()).on_click(act!(ExportBackup)))
            .child(item("m-restore", "upload", "从备份恢复…", String::new()).on_click(act!(RestoreBackup)))
            .child(sep())
            .child(item("m-present", "monitor", "演示模式", crate::shortcuts::hint("stonebrook::Present", cx)).on_click(act!(Present)))
            .child(
                item(
                    "m-read",
                    "book-open",
                    if self.editor.read(cx).is_read_only() { "切换为编辑模式" } else { "切换为阅读模式" },
                    crate::shortcuts::hint("stonebrook::ToggleReadMode", cx),
                )
                .on_click(act!(ToggleReadMode)),
            )
            .child(item("m-cmd", "command", "命令面板", crate::shortcuts::hint("stonebrook::CommandPalette", cx)).on_click(act!(CommandPalette)))
            .child(item("m-keys", "command", "快捷键", crate::shortcuts::hint("stonebrook::ShowShortcuts", cx)).on_click(act!(ShowShortcuts)))
            .child(item("m-set", "settings", "设置", crate::shortcuts::hint("stonebrook::OpenSettings", cx)).on_click(act!(OpenSettings)))
            .child(sep())
            .child(item("m-perf", "activity", "性能面板", crate::shortcuts::hint("stonebrook::TogglePerf", cx)).on_click(act!(TogglePerf)))
            .when(cfg!(feature = "dev"), |d| {
                d.child(item("m-stress", "zap", "生成百万字压测文档", String::new()).on_click(act!(StressTest)))
                    .child(item("m-bstress", "zap", "生成 10 万行多维表格压测", String::new()).on_click(act!(BaseStressTest)))
            })
            .child(item("m-welcome", "feather", "打开欢迎文档", String::new()).on_click(act!(OpenWelcome)))
            .child(sep())
            .child(item("m-trash", "trash", "移到回收站", String::new()).on_click(act!(TrashDoc)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.more_menu = false;
                cx.notify();
            }))
    }

    /// 右侧评论侧栏：列出当前文档全部评论，点击跳到对应块。
    fn render_comments_sidebar(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (list, places): (Vec<_>, Vec<Option<usize>>) = {
            let e = self.editor.read(cx);
            (e.comments.clone(), e.comments.iter().map(|c| e.comment_block_ix(c)).collect())
        };
        let n_open = list.iter().filter(|c| !c.resolved).count();
        let close = icon_button("cm-side-close", "x", "关闭", cx).on_click(cx.listener(|this, _, _, cx| {
            this.comments_open = false;
            cx.notify();
        }));
        div()
            .w(px(280.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px(px(12.))
                    .py(px(10.))
                    .child(div().flex_1().text_size(px(12.)).text_color(t.text_muted).child(format!(
                        "评论 · {} 条{}",
                        list.len(),
                        if n_open > 0 { format!(" · {n_open} 待处理") } else { String::new() }
                    )))
                    .child(close),
            )
            .child(
                div()
                    .id("comments-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(list.is_empty(), |d| {
                        d.child(
                            div()
                                .pt(px(20.))
                                .text_size(px(12.5))
                                .text_color(t.text_muted)
                                .child(crate::i18n::tr("还没有评论。选中文字或图片后，用块菜单或图片工具栏的「评论」添加。")),
                        )
                    })
                    .children(list.into_iter().zip(places).map(|(c, place)| {
                        let id = c.id.clone();
                        let id2 = c.id.clone();
                        let resolved = c.resolved;
                        div()
                            .id(SharedString::from(format!("cs-card-{}", c.id)))
                            // 点卡片任意处定位到原文；原文已删除时不可点
                            .when_some(place, |d, block| {
                                d.cursor_pointer().hover(|s| s.bg(t.hover)).on_click(cx.listener(move |this, _, window, cx| {
                                    this.editor.update(cx, |e, cx| e.jump_to_block(block, cx));
                                    let h = this.editor.focus_handle(cx);
                                    window.focus(&h);
                                    cx.notify();
                                }))
                            })
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if resolved { t.border } else { t.accent.opacity(0.35) })
                            .when(resolved, |d| d.opacity(0.65))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .child(div().flex_1().child(format!("{} · {}", c.author, crate::editor::fmt_comment_time(c.created))))
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("cs-r-{id}")))
                                            .px(px(5.))
                                            .py(px(1.))
                                            .rounded(px(4.))
                                            .cursor_pointer()
                                            .text_color(if resolved { t.text_muted } else { t.accent })
                                            .hover(|s| s.bg(t.hover))
                                            .child(if resolved { "重新打开" } else { "解决" })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.editor.update(cx, |e, cx| e.resolve_comment(&id, !resolved, cx));
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("cs-d-{id2}")))
                                            .px(px(5.))
                                            .py(px(1.))
                                            .rounded(px(4.))
                                            .cursor_pointer()
                                            .text_color(t.text_muted)
                                            .hover(|s| s.bg(t.hover))
                                            .child(crate::i18n::tr("删除"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.editor.update(cx, |e, cx| e.remove_comment(&id2, cx));
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .when(!c.anchor.is_empty(), |d| {
                                d.child(div().text_size(px(11.5)).text_color(t.accent).child(format!("「{}」", c.anchor.chars().take(24).collect::<String>())))
                            })
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .text_color(if resolved { t.text_muted } else { t.text })
                                    .when(resolved, |d| d.line_through())
                                    .child(c.body.clone()),
                            )
                            .when(place.is_none(), |d| {
                                d.child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("原文已删除（撤销可找回）")))
                            })
                    })),
            )
    }

    fn render_outline(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (version, top) = {
            let e = self.editor.read(cx);
            (e.version, e.current_section())
        };
        if self.outline.0 != version {
            let heads = self
                .editor
                .read(cx)
                .content
                .blocks
                .iter()
                .enumerate()
                .filter_map(|(i, b)| match b.kind {
                    BlockKind::Heading { level } if level <= 4 && !b.text.is_empty() => Some((i, level, SharedString::from(b.text.text.replace('\n', " ")))),
                    _ => None,
                })
                .collect();
            self.outline = (version, heads);
        }
        let heads = &self.outline.1;
        // 当前章节：视口顶部所在位置之前的最后一个标题
        let pos = heads.partition_point(|(i, _, _)| *i <= top + 1).checked_sub(1);
        let current = pos.map(|p| heads[p].0);
        if current != self.outline_current {
            self.outline_current = current;
            if let Some(p) = pos {
                self.outline_scroll.scroll_to_item(p, ScrollStrategy::Center);
            }
        }
        let min_level = heads.iter().map(|h| h.1).min().unwrap_or(1);
        let n = heads.len();
        let (accent, secondary, text_color, border) = (t.accent, t.text_secondary, t.text, t.border);
        div()
            .w(px(220.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .pt(px(8.))
            .pr(px(12.))
            .child(div().px(px(12.)).pb(px(8.)).text_size(px(12.)).text_color(t.text_muted).child(if n > 0 {
                format!("{} · {n}", crate::i18n::tr("大纲"))
            } else {
                crate::i18n::tr("大纲").into()
            }))
            .when(n == 0, |d| d.child(div().px(px(12.)).text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr("添加标题后，这里会显示文档大纲"))))
            .when(n > 0, |d| {
                d.child(
                    div().flex_1().min_h(px(0.)).border_l_1().border_color(border).child(
                        uniform_list(
                            "outline",
                            n,
                            cx.processor(move |this: &mut Workspace, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|k| {
                                        let (i, level, text) = this.outline.1[k].clone();
                                        let active = Some(i) == this.outline_current;
                                        div()
                                            .id(("ol", i))
                                            .relative()
                                            .h(px(30.))
                                            .flex()
                                            .items_center()
                                            .pl(px(12. + (level - min_level) as f32 * 14.))
                                            .pr(px(6.))
                                            .text_size(px(13.))
                                            .cursor_pointer()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_color(if active { accent } else { secondary })
                                            .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                                            .hover(|s| s.text_color(text_color))
                                            .when(active, |d| d.child(div().absolute().left(px(-1.)).top(px(7.)).bottom(px(7.)).w(px(2.)).bg(accent)))
                                            .child(text)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.editor.update(cx, |e, cx| e.scroll_to_block(i, cx));
                                                window.focus(&this.editor.focus_handle(cx));
                                            }))
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(self.outline_scroll.clone())
                        .h_full(),
                    ),
                )
            })
            .child(self.render_backlinks(t, cx))
    }

    /// 大纲下方：引用了当前文档的文档，点击打开；再往下是「未链接的提及」，可一键转为链接。
    fn render_backlinks(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let links = self.current_backlinks().to_vec();
        let mentions = self.current_mentions().to_vec();
        let title = self.docs.iter().find(|d| d.id == self.current).map(|d| d.title.clone()).unwrap_or_default();
        div()
            .flex_none()
            .mt(px(12.))
            .pt(px(10.))
            .pb(px(16.))
            .border_t_1()
            .border_color(t.border)
            .child(div().px(px(12.)).pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(if links.is_empty() {
                crate::i18n::tr("反向链接").to_string()
            } else {
                format!("{} · {}", crate::i18n::tr("反向链接"), links.len())
            }))
            .when(links.is_empty(), |d| {
                d.child(div().px(px(12.)).text_size(px(12.)).text_color(t.text_muted).child(crate::i18n::tr("在其他文档里输入 @ 引用本文，会显示在这里")))
            })
            .children(links.into_iter().take(30).enumerate().map(|(k, (id, title))| {
                div()
                    .id(("backlink", k))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .h(px(28.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(t.text_secondary)
                    .hover(|s| s.bg(t.hover).text_color(t.text))
                    .child(icon("file", 13., t.text_muted))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(if title.trim().is_empty() {
                        "未命名文档".to_string()
                    } else {
                        title
                    }))
                    .on_click(cx.listener(move |this, _, w, cx| this.open_doc(&id, w, cx)))
            }))
            .when(!mentions.is_empty(), |d| {
                d.child(div().px(px(12.)).pt(px(12.)).pb(px(6.)).text_size(px(12.)).text_color(t.text_muted).child(format!(
                    "{} · {}",
                    crate::i18n::tr("未链接的提及"),
                    mentions.len()
                )))
                .children(mentions.into_iter().take(20).enumerate().map(|(k, (id, mtitle))| {
                    let (id2, title2) = (id.clone(), title.clone());
                    div()
                        .id(("mention-doc", k))
                        .group("mention-row")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(12.))
                        .h(px(28.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(t.text_secondary)
                        .hover(|s| s.bg(t.hover).text_color(t.text))
                        .child(icon("file", 13., t.text_muted))
                        .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(if mtitle.trim().is_empty() {
                            "未命名文档".to_string()
                        } else {
                            mtitle
                        }))
                        .child(
                            div()
                                .id(("mention-link", k))
                                .invisible()
                                .group_hover("mention-row", |s| s.visible())
                                .px(px(6.))
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .text_color(t.accent)
                                .hover(|s| s.bg(t.accent_soft))
                                .child(crate::i18n::tr("转为链接"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let cur = this.current.clone();
                                    if this.store.link_mention(&id2, &cur, &title2) {
                                        this.reload_docs();
                                        this.backlinks.1 = 0;
                                        this.mentions.1 = 0;
                                        this.show_toast("已转为链接".into(), cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| this.open_doc(&id, w, cx)))
                }))
            })
            .child(
                div()
                    .id("open-graph")
                    .mx(px(12.))
                    .mt(px(12.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(t.text_secondary)
                    .hover(|s| s.bg(t.hover))
                    .child(icon("sitemap", 13., t.text_muted))
                    .child(crate::i18n::tr("文档图谱"))
                    .on_click(cx.listener(|this, _, w, cx| this.open_graph(w, cx))),
            )
    }

    fn render_switcher(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let s = self.switcher.as_ref()?;
        let q = s.input.read(cx).text();
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.overlay)
                .flex()
                .flex_col()
                .items_center()
                .pt(px(110.))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, w, cx| this.close_switcher(w, cx)))
                .child(
                    div()
                        .w(px(580.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(12.))
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(10.))
                                .px(px(16.))
                                .h(px(52.))
                                .border_b_1()
                                .border_color(t.border)
                                .text_size(px(16.))
                                .line_height(px(24.))
                                .child(icon("search", 18., t.text_muted))
                                .child(s.input.clone()),
                        )
                        .child(
                            div()
                                .id("sw-results")
                                .max_h(px(420.))
                                .overflow_y_scroll()
                                .p(px(6.))
                                .when(s.results.is_empty(), |d| {
                                    d.child(div().px_3().py_4().text_size(px(13.)).text_color(t.text_muted).child(if q.trim().is_empty() {
                                        "还没有文档"
                                    } else {
                                        "没有找到匹配的文档"
                                    }))
                                })
                                .when(q.trim().is_empty() && !s.results.is_empty(), |d| {
                                    d.child(div().px_3().pt_1().pb(px(4.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("最近编辑")))
                                })
                                .children(s.results.iter().enumerate().map(|(i, (id, title, snippet))| {
                                    let id = id.clone();
                                    let sel = i == s.selected;
                                    div()
                                        .id(("sw", i))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(10.))
                                        .px(px(10.))
                                        .py(px(8.))
                                        .rounded(px(8.))
                                        .cursor_pointer()
                                        .when(sel, |d| d.bg(t.active))
                                        .hover(|s| s.bg(t.hover))
                                        .child(icon("file", 16., if sel { t.accent } else { t.text_muted }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div()
                                                        .text_size(px(14.))
                                                        .text_color(t.text)
                                                        .whitespace_nowrap()
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(if title.is_empty() { "未命名文档".to_string() } else { title.clone() }),
                                                )
                                                .when(!snippet.is_empty(), |d| {
                                                    d.child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .text_color(t.text_muted)
                                                            .whitespace_nowrap()
                                                            .overflow_hidden()
                                                            .text_ellipsis()
                                                            .child(snippet.clone()),
                                                    )
                                                }),
                                        )
                                        .when(sel, |d| d.child(div().text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("↵ 打开"))))
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.close_switcher(w, cx);
                                            this.open_doc(&id, w, cx);
                                        }))
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(14.))
                                .px(px(16.))
                                .py(px(8.))
                                .border_t_1()
                                .border_color(t.border)
                                .text_size(px(11.))
                                .text_color(t.text_muted)
                                .child(crate::i18n::tr("↑↓ 选择"))
                                .child(crate::i18n::tr("↵ 打开"))
                                .child(crate::i18n::tr("esc 关闭"))
                                .child(div().flex_1())
                                .child(format!("{} 篇文档", self.docs.iter().filter(|d| !d.deleted).count())),
                        ),
                ),
        )
    }

    fn render_about(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let line =
            |ic: &'static str, s: &'static str| div().flex().flex_row().items_center().gap(px(8.)).child(icon(ic, 14., t.accent)).child(crate::i18n::tr(s));
        div()
            .absolute()
            .inset_0()
            .bg(t.overlay)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.about = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(400.))
                    .p(px(32.))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(16.))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .child(logo(96.))
                    .child(div().mt(px(8.)).text_size(px(24.)).font_weight(FontWeight::BOLD).text_color(t.text).child(crate::i18n::tr("石渠")))
                    .child(div().text_size(px(13.)).text_color(t.text_muted).child(format!("Stonebrook {}", env!("CARGO_PKG_VERSION"))))
                    .child(div().mt(px(10.)).text_size(px(15.)).text_color(t.text_secondary).child(crate::i18n::tr("藏书于石，引水成渠")))
                    .child(
                        div()
                            .mt(px(18.))
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .text_size(px(13.))
                            .text_color(t.text_secondary)
                            .child(line("lock", "纯本地存储，无账号、无云端、无追踪"))
                            .child(line("zap", "Rust + GPUI，Metal 直接绘制，百万字不卡顿"))
                            .child(line("feather", "块编辑器：表格、分栏、代码、高亮块、待办"))
                            .child(line("copy", "从网页粘贴，格式原样还原"))
                            .child(line("workflow", "图表、公式、画板、思维导图、电子表格")),
                    )
                    .child(div().mt(px(20.)).text_size(px(11.)).text_color(t.text_muted).child(crate::i18n::tr("点击任意处关闭"))),
            )
    }

    /// 破坏性操作的确认弹窗：Esc、点弹窗外或「取消」都算放弃。
    fn render_confirm(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use crate::base::widgets::{ghost_btn, primary_btn};
        let c = self.confirm.as_ref()?;
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.overlay)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .key_context("Confirm")
                .track_focus(&self.confirm_focus)
                .on_action(cx.listener(|this, _: &CancelConfirm, window, cx| this.close_confirm(window, cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_confirm(window, cx)))
                .child(
                    div()
                        .id("confirm-dlg")
                        .w(px(400.))
                        .p(px(24.))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(14.))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.text).child(c.title.clone()))
                        .child(div().text_size(px(13.)).line_height(px(20.)).text_color(t.text_secondary).child(c.body.clone()))
                        .child(
                            div()
                                .mt(px(8.))
                                .flex()
                                .flex_row()
                                .justify_end()
                                .gap(px(8.))
                                .child(
                                    ghost_btn("confirm-cancel", crate::i18n::tr("取消"), t)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_confirm(window, cx))),
                                )
                                .child(
                                    primary_btn("confirm-ok", c.ok.clone(), t)
                                        .bg(t.fg_palette[1])
                                        .on_click(cx.listener(|this, _, window, cx| this.confirm_ok(window, cx))),
                                ),
                        ),
                ),
        )
    }

    fn render_toast(&self, t: &Theme) -> Option<impl IntoElement> {
        let msg = self.toast.clone()?;
        let gen = self.toast_gen as usize;
        // 飞书同款：顶部居中、从上往下滑入、成功提示是绿色底 + 勾选图标
        let success = self.toast_kind == ToastKind::Success;
        let (bg, border, fg, mark) = if success {
            if t.dark {
                (rgb(0x1e2b22), rgb(0x2f6b45), rgb(0xe6e8ea), rgb(0x3fbf7f))
            } else {
                (rgb(0xeaf9ef), rgb(0xa7e3bb), rgb(0x1f2329), rgb(0x2ba471))
            }
        } else if t.dark {
            (rgb(0xe3e4e6), rgb(0xe3e4e6), rgb(0x1f2329), rgb(0x1f2329))
        } else {
            (rgb(0x1f2329), rgb(0x1f2329), rgb(0xffffff), rgb(0xffffff))
        };
        let card = div()
            .px(px(16.))
            .py(px(9.))
            .rounded(px(10.))
            .bg(bg)
            .border_1()
            .border_color(border)
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .shadow_lg()
            .when(success, |d| {
                d.child(div().size(px(18.)).rounded_full().bg(mark).flex().items_center().justify_center().child(icon("check", 12., gpui::white())))
            })
            .child(div().text_size(px(13.)).text_color(fg).child(msg));
        Some(div().absolute().top(px(14.)).left_0().right_0().flex().justify_center().child(card.with_animation(
            ("toast", gen),
            // 三次缓出：起手快、收尾稳
            Animation::new(Duration::from_millis(180)).with_easing(|d: f32| 1. - (1. - d).powi(3)),
            move |d, delta| d.opacity(delta).mt(px(-10. * (1. - delta))),
        )))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() && self.doc_drop_target.is_some() {
            self.doc_drop_target = None;
        }
        self.sync_doc_list(cx);
        if let Some((kind, id)) = self.pending_embed.take() {
            self.open_embed(&kind, &id, window, cx);
        }
        if let Some(content) = self.pending_new_doc.take() {
            self.create_doc(None, content, window, cx);
            self.show_toast("已创建副本".into(), cx);
        }
        if !self.focused_once {
            // 首帧之后再聚焦编辑器，保证启动即可输入
            self.focused_once = true;
            let handle = self.editor.focus_handle(cx);
            window.on_next_frame(move |window, _| window.focus(&handle));
        }
        let t = theme(cx).clone();
        let show_sidebar = self.show_sidebar && !self.focus_mode;
        let show_outline = self.show_outline && !self.focus_mode;
        let comments_open = self.comments_open && !self.focus_mode;
        div()
            .key_context("Workspace")
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(t.bg)
            .text_color(t.text)
            .font_family("PingFang SC")
            .on_action(cx.listener(Self::new_doc))
            .on_action(cx.listener(Self::new_sub_doc))
            .on_action(cx.listener(Self::move_current_doc))
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::cycle_theme))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(|this, _: &About, _, cx| this.show_settings_tab(SettingsTab::Updates, cx)))
            .on_action(cx.listener(|this, _: &CheckUpdate, _, cx| {
                if matches!(crate::updater::state(), crate::updater::State::Ready { .. }) {
                    this.update_dialog_open = true;
                    this.update_notes_open = false;
                    cx.notify();
                } else {
                    this.show_settings_tab(SettingsTab::Updates, cx);
                    crate::updater::run(true, true);
                }
            }))
            .on_action(cx.listener(|this, _: &InstallUpdate, _, cx| this.install_update(cx)))
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape"
                    && (this.confirm.is_some()
                        || this.settings_open
                        || this.update_dialog_open
                        || this.template_picker.is_some()
                        || this.base_picker.is_some()
                        || this.doc_menu.is_some()
                        || this.move_picker.is_some())
                {
                    if this.confirm.take().is_some() || this.move_picker.take().is_some() {
                        window.focus(&this.editor.focus_handle(cx));
                    }
                    this.settings_open = false;
                    this.update_dialog_open = false;
                    this.template_picker = None;
                    this.base_picker = None;
                    this.doc_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::export_pdf))
            .on_action(cx.listener(Self::export_backup))
            .on_action(cx.listener(Self::restore_backup))
            .on_action(cx.listener(Self::present))
            .on_action(cx.listener(Self::toggle_perf))
            .on_action(cx.listener(Self::toggle_read_mode))
            .on_action(cx.listener(Self::show_shortcuts))
            .on_action(cx.listener(Self::export_docxml))
            .when(cfg!(feature = "dev"), |d| d.on_action(cx.listener(Self::stress_test)).on_action(cx.listener(Self::base_stress_test)))
            .on_action(cx.listener(Self::focus_mode))
            .on_action(cx.listener(Self::export_md))
            .on_action(cx.listener(Self::export_html))
            .on_action(cx.listener(Self::import_md))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::trash_doc))
            .on_action(cx.listener(Self::open_welcome))
            .when(show_sidebar, |d| d.child(self.render_sidebar(&t, cx)))
            .child(
                div().flex_1().min_w(px(0.)).h_full().flex().flex_col().child(self.render_topbar(&t, cx)).child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .h_full()
                                .relative()
                                .child(self.editor.clone())
                                // 提示条盖在正文区上方居中（不含侧栏），和飞书一致
                                .children(self.render_toast(&t)),
                        )
                        .when(show_outline, |d| d.child(self.render_outline(&t, cx)))
                        .when(comments_open, |d| d.child(self.render_comments_sidebar(&t, cx))),
                ),
            )
            .children(self.render_switcher(&t, cx))
            .children(self.render_move_picker(&t, cx))
            .children(self.render_doc_menu(&t, cx))
            .when(self.about, |d| d.child(self.render_about(&t, cx)))
            .when(self.template_picker.is_some(), |d| d.child(self.render_templates(&t, cx)))
            .children(self.render_base_picker(&t, cx))
            .when(self.settings_open, |d| d.child(self.render_settings(&t, cx)))
            .when(self.update_dialog_open, |d| d.child(self.render_update_dialog(&t, cx)))
            .children(self.render_palette(&t, cx))
            .children(self.render_shortcuts_panel(&t, cx))
            .children(self.render_confirm(&t, cx))
            .children(self.render_present(window, cx))
            .children(self.render_graph(window, cx))
            .when_some(self.embed.as_ref().map(|e| e.0.clone()), |d, v| d.child(div().absolute().inset_0().occlude().child(v)))
    }
}

/// 文档结构 → 缩进大纲（层级, 文字）：标题为根（层级 0），各级标题 1..6，列表项 10 + 缩进。
fn corrupt_msg(c: &crate::store::Corrupt) -> String {
    format!("文档数据无法解析，原文已备份到 {}", c.backup.display())
}

fn doc_outline(blocks: &[std::sync::Arc<crate::model::Block>]) -> Vec<(usize, String)> {
    let title = blocks.first().map(|b| b.text.text.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| "未命名文档".into());
    let mut out = vec![(0, title)];
    for b in blocks.iter().skip(1) {
        let level = match b.kind {
            BlockKind::Heading { level } => level as usize,
            BlockKind::Bullet | BlockKind::Ordered | BlockKind::Todo { .. } => 10 + b.indent as usize,
            _ => continue,
        };
        let text = b.text.text.replace('\n', " ");
        if !text.trim().is_empty() {
            out.push((level, text.chars().take(60).collect()));
        }
    }
    out
}
