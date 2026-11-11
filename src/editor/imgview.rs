//! 「视频或文件」里的图片查看器：
//! - 灰底区域里居中显示原图，默认「适应页面」（缩小到放得下，不放大）；
//! - 悬停时底部浮出工具栏：缩小 / 放大 / 旋转（逆时针 90°）/ 原始尺寸 ⇄ 适应页面；
//! - 缩放时中间短暂显示百分比；图片超出区域时可拖动平移；
//! - 文档内的预览视图与全屏预览共用这一套，全屏时工具栏多一个「下载」。
//!
//! 缩放 / 旋转只是查看状态，不写进文档。旋转用系统 sips 生成缓存图。
//! 电子表格附件的全屏预览也用这一套（`img_viewer` + `ImgHost`）。

use super::*;
use crate::theme::Theme;
use crate::ui::{icon, Tooltip};
use std::path::{Path, PathBuf};

/// 全屏预览的查看状态与文档内预览分开存：块 id 加上这个标记位
pub(crate) const FULL_KEY: u64 = 1 << 63;
const ZOOM_STEP: f32 = 1.25;
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 8.;
const PAD: f32 = 14.;

#[derive(Clone, Default)]
pub struct ImgView {
    /// None = 适应页面；Some(s) = 相对原图的缩放比例
    zoom: Option<f32>,
    /// 逆时针旋转的 90° 次数（0..4）
    rot: u8,
    pan: (f32, f32),
    /// 拖动起点：(鼠标位置, 当时的平移量)
    drag: Option<(Point<Pixels>, (f32, f32))>,
    /// 百分比提示：显示中的那次缩放序号（过期后隐藏）
    toast: Option<u64>,
    toast_seq: u64,
}

thread_local! {
    /// 旋转后的缓存图：(原图, 旋转次数) → 生成结果（None = 生成中 / 失败）
    static ROTATED: RefCell<HashMap<(String, u8), Option<PathBuf>>> = RefCell::new(HashMap::new());
}

/// 旋转后的图片路径：0 次直接用原图；否则后台用 sips 生成，生成好之前返回 None。
fn rotated_path(path: &str, rot: u8, cx: &mut App) -> Option<PathBuf> {
    if rot == 0 {
        return Some(PathBuf::from(path));
    }
    let key = (path.to_string(), rot);
    if let Some(r) = ROTATED.with(|m| m.borrow().get(&key).cloned()) {
        return r;
    }
    ROTATED.with(|m| m.borrow_mut().insert(key.clone(), None));
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let out = crate::webview::cache_dir().join(format!("rot-{}.{ext}", crate::webview::hash_key(&[path, &rot.to_string()])));
    let src = path.to_string();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let dst = out.clone();
        let ok = cx
            .background_executor()
            .spawn(async move {
                if dst.exists() {
                    return true;
                }
                std::fs::create_dir_all(dst.parent().unwrap_or(Path::new("."))).ok();
                // sips 按顺时针角度旋转
                std::process::Command::new("sips")
                    .args(["-r", &((4 - rot as u32) * 90).to_string()])
                    .arg(&src)
                    .arg("--out")
                    .arg(&dst)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
                    && dst.exists()
            })
            .await;
        if ok {
            ROTATED.with(|m| m.borrow_mut().insert(key, Some(out)));
        }
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    None
}

/// 查看器能处理的图片（能读出尺寸）；其他格式仍走 Quick Look 缩略图。
pub fn viewable(name: &str) -> bool {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff")
}

/// 宿主（文档编辑器 / 电子表格）提供查看状态的存放处；缩放、旋转、平移的逻辑共用。
pub(crate) trait ImgHost: Sized + 'static {
    fn img_views(&mut self) -> &mut HashMap<u64, ImgView>;
}

impl ImgHost for Editor {
    fn img_views(&mut self) -> &mut HashMap<u64, ImgView> {
        &mut self.sess.img_views
    }
}

/// 适应页面时的缩放比例（不放大）。
fn img_fit(nat: (f32, f32), bw: f32, bh: f32) -> f32 {
    ((bw - PAD * 2.) / nat.0).min((bh - PAD * 2.) / nat.1).clamp(0.01, 1.)
}

fn img_zoom<V: ImgHost>(this: &mut V, key: u64, factor: Option<f32>, fit: f32, cx: &mut Context<V>) {
    let v = this.img_views().entry(key).or_default();
    let cur = v.zoom.unwrap_or(fit);
    v.zoom = match factor {
        Some(f) => Some((cur * f).clamp(ZOOM_MIN, ZOOM_MAX)),
        // 原始尺寸 ⇄ 适应页面
        None if v.zoom.is_none() => Some(1.),
        None => None,
    };
    if v.zoom.is_none() {
        v.pan = (0., 0.);
    }
    v.toast_seq += 1;
    let seq = v.toast_seq;
    v.toast = Some(seq);
    cx.notify();
    cx.spawn(async move |this: WeakEntity<V>, cx: &mut AsyncApp| {
        cx.background_executor().timer(Duration::from_millis(900)).await;
        this.update(cx, |this, cx| {
            if let Some(v) = this.img_views().get_mut(&key).filter(|v| v.toast == Some(seq)) {
                v.toast = None;
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

fn img_rotate<V: ImgHost>(this: &mut V, key: u64, cx: &mut Context<V>) {
    let v = this.img_views().entry(key).or_default();
    v.rot = (v.rot + 1) % 4;
    v.pan = (0., 0.);
    cx.notify();
}

/// 图片查看区（宽 bw × 高 bh），`raw` 为原图尺寸。`full` 为全屏预览（工具栏常驻、多「下载」）。
/// `on_down`：按下鼠标时先交给宿主，返回 true 表示已处理（不再开始拖动）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn img_viewer<V: ImgHost>(
    this: &mut V,
    key: u64,
    path: &str,
    raw: Option<(u32, u32)>,
    bw: f32,
    bh: f32,
    full: bool,
    t: &Theme,
    cx: &mut Context<V>,
    on_down: impl Fn(&mut V, &MouseDownEvent, &mut Window, &mut Context<V>) -> bool + 'static,
    on_download: impl Fn(&mut V, &mut Context<V>) + 'static,
) -> AnyElement {
    let v = this.img_views().get(&key).cloned().unwrap_or_default();
    let raw = raw.map(|(w, h)| (w.max(1) as f32, h.max(1) as f32));
    let shown = rotated_path(path, v.rot, cx);
    let (Some(raw), Some(shown)) = (raw, shown) else {
        return div().size_full().into_any_element();
    };
    let nat = if v.rot % 2 == 1 { (raw.1, raw.0) } else { raw };
    let fit = img_fit(nat, bw, bh);
    let scale = v.zoom.unwrap_or(fit);
    let (dw, dh) = (nat.0 * scale, nat.1 * scale);
    // 平移只在图片超出区域的方向上生效，且不能拖出边界
    let lim = (((dw - bw) / 2.).max(0.), ((dh - bh) / 2.).max(0.));
    let pan = (v.pan.0.clamp(-lim.0, lim.0), v.pan.1.clamp(-lim.1, lim.1));
    let movable = lim.0 > 0. || lim.1 > 0.;
    let group: SharedString = format!("imgv-{key}").into();
    let btn = |eid: &'static str, ic: &'static str, tip: &'static str| {
        div()
            .id((eid, key as usize))
            .size(px(30.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .tooltip(move |_, cx| cx.new(|_| Tooltip(tip.into())).into())
            .child(icon(ic, 17., t.text_secondary))
    };
    let fitted = v.zoom.is_none();
    let toolbar = div()
        .id(("imgv-tb", key as usize))
        .absolute()
        .bottom(px(12.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .when(!full, |d| d.invisible().group_hover(group.clone(), |s| s.visible()))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .p(px(4.))
                .rounded(px(8.))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .shadow_md()
                // 不能 occlude：会挡住查看区的悬停，工具栏一移上去就被 group_hover 隐藏
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(btn("imgv-out", "zoom-out", "缩小").on_click(cx.listener(move |this, _, _, cx| img_zoom(this, key, Some(1. / ZOOM_STEP), fit, cx))))
                .child(btn("imgv-in", "zoom-in", "放大").on_click(cx.listener(move |this, _, _, cx| img_zoom(this, key, Some(ZOOM_STEP), fit, cx))))
                .child(btn("imgv-rot", "rotate-ccw", "旋转").on_click(cx.listener(move |this, _, _, cx| img_rotate(this, key, cx))))
                .child(
                    btn("imgv-fit", if fitted { "actual-size" } else { "fit-page" }, if fitted { "原始尺寸" } else { "适应页面" })
                        .on_click(cx.listener(move |this, _, _, cx| img_zoom(this, key, None, fit, cx))),
                )
                .when(full, |d| d.child(btn("imgv-dl", "download", "下载").on_click(cx.listener(move |this, _, _, cx| on_download(this, cx))))),
        );
    let toast = v.toast.map(|_| {
        div().absolute().inset_0().flex().items_center().justify_center().child(
            div()
                .px(px(18.))
                .py(px(8.))
                .rounded(px(6.))
                .bg(hsla(0., 0., 0.1, 0.72))
                .text_size(px(15.))
                .text_color(gpui::white())
                .child(format!("{}%", (scale * 100.).round() as i32)),
        )
    });
    div()
        .id(("imgv", key as usize))
        .group(group)
        .relative()
        .w(px(bw))
        .h(px(bh))
        .overflow_hidden()
        .bg(t.code_bg)
        .when(movable, |d| d.cursor(if v.drag.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }))
        .child(img(shown).absolute().left(px((bw - dw) / 2. + pan.0)).top(px((bh - dh) / 2. + pan.1)).w(px(dw)).h(px(dh)).object_fit(ObjectFit::Fill))
        .children(toast)
        .child(toolbar)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if on_down(this, ev, window, cx) {
                    return;
                }
                if movable {
                    this.img_views().entry(key).or_default().drag = Some((ev.position, pan));
                }
            }),
        )
        .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
            let Some(v) = this.img_views().get_mut(&key) else { return };
            let Some((start, p0)) = v.drag else { return };
            if ev.pressed_button != Some(MouseButton::Left) {
                v.drag = None;
                cx.notify();
                return;
            }
            let d = ev.position - start;
            v.pan = ((p0.0 + f32::from(d.x)).clamp(-lim.0, lim.0), (p0.1 + f32::from(d.y)).clamp(-lim.1, lim.1));
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if let Some(v) = this.img_views().get_mut(&key) {
                    if v.drag.take().is_some() {
                        cx.notify();
                    }
                }
            }),
        )
        .into_any_element()
}

impl Editor {
    /// 文件块里的图片查看区。`ix` 为文件块序号；`full` 为全屏预览。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_img_viewer(&mut self, ix: usize, id: u64, path: &str, bw: f32, bh: f32, full: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = if full { id | FULL_KEY } else { id };
        let raw = self.image_size(path);
        let on_down = move |this: &mut Self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| {
            if full {
                return false;
            }
            window.focus(&this.focus);
            this.mouse_select(Pos::new(ix, 0, 0), 1, ev.modifiers.shift, cx);
            if ev.click_count >= 2 {
                this.open_attachment(ix, cx);
                return true;
            }
            false
        };
        img_viewer(self, key, path, raw, bw, bh, full, t, cx, on_down, move |this: &mut Self, cx| this.download_attachment(ix, cx))
    }
}
