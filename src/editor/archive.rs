//! 「视频或文件」里的压缩包：不解压，直接列出包内目录树，文件夹可展开 / 收起。
//! 目录用系统自带的 bsdtar（libarchive）读取：zip / 7z / rar / tar.gz / tar.xz / tar.bz2 / iso 一条命令全覆盖。

use super::attach::file_icon;
use super::*;
use crate::convert::human_size;
use crate::theme::Theme;
use crate::ui::icon;
use std::collections::HashSet;
use std::path::Path;

const ROW_H: f32 = 32.;
/// 一次最多渲染的行数（超大压缩包全部展开时）
const MAX_ROWS: usize = 3000;

struct Node {
    name: String,
    full: String,
    size: u64,
    dir: bool,
    children: Vec<Node>,
}

pub(super) struct Tree {
    root: Vec<Node>,
    files: usize,
    total: u64,
}

#[derive(Clone)]
enum Listing {
    Pending,
    Ready(Rc<Tree>),
    Failed,
}

thread_local! {
    static LISTINGS: RefCell<crate::lru::Lru<String, Listing>> = RefCell::new(crate::lru::Lru::new(64));
    /// 每个文件块展开了哪些文件夹（查看状态，不写进文档）
    static EXPANDED: RefCell<HashMap<u64, HashSet<String>>> = RefCell::new(HashMap::new());
}

fn listing(path: &str, cx: &mut App) -> Listing {
    let stamp = std::fs::metadata(path).ok().map(|m| format!("{}-{:?}", m.len(), m.modified().ok())).unwrap_or_default();
    let key = format!("{path}\u{0}{stamp}");
    if let Some(l) = LISTINGS.with(|m| m.borrow().get(&key).cloned()) {
        return l;
    }
    LISTINGS.with(|m| m.borrow_mut().insert(key.clone(), Listing::Pending));
    let src = path.to_string();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let entries = cx
            .background_executor()
            .spawn(async move {
                // UTF-8 语言环境：否则 bsdtar 会把中文文件名转义成 ?
                let out = std::process::Command::new("/usr/bin/bsdtar")
                    .arg("-tvf")
                    .arg(&src)
                    .env("LC_ALL", "en_US.UTF-8")
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()?;
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let entries = parse(&text);
                (out.status.success() || !entries.is_empty()).then_some(entries)
            })
            .await;
        let l = match entries {
            Some(e) if !e.is_empty() => Listing::Ready(Rc::new(build(e))),
            _ => Listing::Failed,
        };
        LISTINGS.with(|m| m.borrow_mut().insert(key, l));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    Listing::Pending
}

/// 解析 `bsdtar -tv` 的 ls 风格输出：权限 链接数 属主 属组 大小 月 日 时间/年 路径。
fn parse(text: &str) -> Vec<(String, u64, bool)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        let mut fields = Vec::with_capacity(8);
        for _ in 0..8 {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            fields.push(&rest[..end]);
            rest = &rest[end..];
        }
        let name = rest.strip_prefix(' ').unwrap_or(rest);
        if fields.len() < 8 || name.is_empty() {
            continue;
        }
        let perm = fields[0];
        // 符号链接：去掉「 -> 目标」
        let name = if perm.starts_with('l') { name.split(" -> ").next().unwrap_or(name) } else { name };
        let dir = perm.starts_with('d') || name.ends_with('/');
        let name = name.trim_start_matches("./").trim_end_matches('/');
        // macOS 打包时附带的资源分支
        if name.is_empty() || name.starts_with("__MACOSX") || name.rsplit('/').next().is_some_and(|n| n == ".DS_Store") {
            continue;
        }
        out.push((name.to_string(), fields[4].parse().unwrap_or(0), dir));
    }
    out
}

fn build(entries: Vec<(String, u64, bool)>) -> Tree {
    let mut root: Vec<Node> = Vec::new();
    let (mut files, mut total) = (0, 0);
    for (path, size, dir) in entries {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let mut level = &mut root;
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let is_dir = !last || dir;
            let pos = match level.iter().position(|n| n.name == *part && n.dir == is_dir) {
                Some(p) => p,
                None => {
                    level.push(Node { name: part.to_string(), full: parts[..=i].join("/"), size: 0, dir: is_dir, children: Vec::new() });
                    level.len() - 1
                }
            };
            if last && !dir {
                level[pos].size = size;
                files += 1;
                total += size;
            }
            level = &mut level[pos].children;
        }
    }
    fn finish(nodes: &mut [Node]) -> u64 {
        nodes.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        let mut sum = 0;
        for n in nodes.iter_mut() {
            if n.dir {
                n.size = finish(&mut n.children);
            }
            sum += n.size;
        }
        sum
    }
    finish(&mut root);
    Tree { root, files, total }
}

fn flatten<'a>(nodes: &'a [Node], depth: usize, open: &HashSet<String>, out: &mut Vec<(usize, &'a Node)>) {
    for n in nodes {
        if out.len() >= MAX_ROWS {
            return;
        }
        out.push((depth, n));
        if n.dir && open.contains(&n.full) {
            flatten(&n.children, depth + 1, open, out);
        }
    }
}

impl Editor {
    /// 压缩包目录。height：Some = 文档内预览（列表区固定高度），None = 全屏（铺满）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_archive(
        &mut self,
        ix: usize,
        id: u64,
        path: &str,
        name: &str,
        size: u64,
        height: Option<f32>,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list = if Path::new(path).exists() { listing(path, cx) } else { Listing::Failed };
        let tree = match list {
            Listing::Ready(tree) => tree,
            Listing::Pending => {
                let msg = div()
                    .w_full()
                    .h(px(height.unwrap_or(240.).min(240.)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.text_muted)
                    .child("加载中…");
                return self.archive_frame(height.is_some(), msg.into_any_element(), name, size, selected, t);
            }
            Listing::Failed => {
                let body = div().w_full().h(px(360.)).bg(t.code_bg).child(self.render_no_preview(name, Some(size), t));
                return self.archive_frame(height.is_some(), body.into_any_element(), name, size, selected, t);
            }
        };
        // 默认只有一个顶层文件夹时自动展开它
        let open = EXPANDED.with(|e| {
            e.borrow_mut()
                .entry(id)
                .or_insert_with(|| match tree.root.as_slice() {
                    [only] if only.dir => HashSet::from([only.full.clone()]),
                    _ => HashSet::new(),
                })
                .clone()
        });
        let mut rows = Vec::new();
        flatten(&tree.root, 0, &open, &mut rows);
        let header = div()
            .h(px(40.))
            .flex_none()
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border)
            .bg(t.sidebar_bg)
            .text_size(px(12.5))
            .text_color(t.text_secondary)
            .child(file_icon(name, 16.))
            .child(format!("共 {} 个文件", tree.files))
            .child(div().flex_1())
            .child(div().text_color(t.text_muted).child(format!("解压后约 {}", human_size(tree.total))));
        let rows_el = rows.into_iter().map(|(depth, n)| {
            let full = n.full.clone();
            let is_open = n.dir && open.contains(&n.full);
            div()
                .id(SharedString::from(format!("arc-{id}-{}", n.full)))
                .h(px(ROW_H))
                .flex_none()
                .pl(px(12. + depth as f32 * 20.))
                .pr(px(14.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .text_size(px(13.5))
                .text_color(t.text)
                .hover(|s| s.bg(t.hover))
                .when(n.dir, |d| d.cursor_pointer())
                .child(
                    div().size(px(14.)).flex_none().when(n.dir, |d| d.child(icon(if is_open { "chevron-down" } else { "chevron-right" }, 14., t.text_muted))),
                )
                .child(if n.dir { icon("folder-fill", 17., rgb(0x3370ff).into()).into_any_element() } else { file_icon(&n.name, 18.).into_any_element() })
                .child(div().flex_1().min_w(px(0.)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(n.name.clone()))
                .child(div().flex_none().text_size(px(12.)).text_color(t.text_muted).child(human_size(n.size)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus);
                        this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
                        EXPANDED.with(|e| {
                            let mut e = e.borrow_mut();
                            let set = e.entry(id).or_default();
                            if !set.remove(&full) {
                                set.insert(full.clone());
                            }
                        });
                        cx.notify();
                    }),
                )
        });
        let list_el = div()
            .id(("arc-list", id as usize))
            .w_full()
            .flex()
            .flex_col()
            .py(px(4.))
            .overflow_y_scroll()
            .map(|d| match height {
                Some(h) => d.max_h(px(h - 40.)),
                None => d.flex_1().min_h(px(0.)),
            })
            .children(rows_el);
        let body = div().w_full().flex().flex_col().when(height.is_none(), |d| d.flex_1().min_h(px(0.))).child(header).child(list_el);
        self.archive_frame(height.is_some(), body.into_any_element(), name, size, selected, t)
    }

    fn archive_frame(&self, inline: bool, body: AnyElement, name: &str, size: u64, selected: bool, t: &Theme) -> AnyElement {
        if !inline {
            return div().size_full().flex().flex_col().child(body).into_any_element();
        }
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .overflow_hidden()
            .child(body)
            .child(self.file_footer(name, Some(size), t))
            .into_any_element()
    }
}
