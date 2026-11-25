//! 用 GPUI 绘制场景。所有绘制都在 canvas() 的 paint 阶段进行。

use super::geom::*;
use super::*;
use gpui::{fill, point, px, size, App, Bounds, Corners, Font, FontWeight, Hsla, Path, PathBuilder, Pixels, Point, SharedString, TextAlign, TextRun, Window};
use std::sync::Arc;

/// 世界坐标 → 屏幕坐标的变换。
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub origin: Point<Pixels>,
    pub scale: f32,
}

impl View {
    pub fn p(&self, p: Pt) -> Point<Pixels> {
        point(self.origin.x + px(p.x * self.scale), self.origin.y + px(p.y * self.scale))
    }
    pub fn r(&self, r: Rect) -> Bounds<Pixels> {
        Bounds::new(self.p(pt(r.x, r.y)), size(px(r.w * self.scale), px(r.h * self.scale)))
    }
    /// 屏幕坐标 → 世界坐标
    pub fn inv(&self, p: Point<Pixels>) -> Pt {
        pt(f32::from(p.x - self.origin.x) / self.scale, f32::from(p.y - self.origin.y) / self.scale)
    }
}

pub const FONT: &str = "PingFang SC";

fn font(bold: bool) -> Font {
    let mut f = gpui::font(FONT);
    if bold {
        f.weight = FontWeight::SEMIBOLD;
    }
    f
}

fn build(cmds: &[PathCmd], view: &View, stroke: Option<f32>, dash: Option<f32>) -> Option<Path<Pixels>> {
    if cmds.is_empty() {
        return None;
    }
    let mut b = match stroke {
        Some(w) => PathBuilder::stroke(px((w * view.scale).max(0.6))),
        None => PathBuilder::fill(),
    };
    if let Some(d) = dash {
        let d = d * view.scale;
        b = b.dash_array(&[px(d), px(d * 0.7)]);
    }
    for c in cmds {
        match *c {
            PathCmd::Move(p) => b.move_to(view.p(p)),
            PathCmd::Line(p) => b.line_to(view.p(p)),
            PathCmd::Cubic(a, c2, p) => b.cubic_bezier_to(view.p(p), view.p(a), view.p(c2)),
            PathCmd::Close => b.close(),
        }
    }
    b.build().ok()
}

fn fill_cmds(window: &mut Window, cmds: &[PathCmd], view: &View, color: Col) {
    if color.alpha() <= 0. {
        return;
    }
    if let Some(p) = build(cmds, view, None, None) {
        window.paint_path(p, color.hsla());
    }
}

fn stroke_cmds(window: &mut Window, cmds: &[PathCmd], view: &View, color: Col, w: f32, dash: Option<f32>) {
    if color.alpha() <= 0. || w <= 0. {
        return;
    }
    if let Some(p) = build(cmds, view, Some(w), dash) {
        window.paint_path(p, color.hsla());
    }
}

/// 把含曲线的路径打平成折线（手绘风格需要点列）。
pub fn flatten(cmds: &[PathCmd]) -> Vec<(Vec<Pt>, bool)> {
    let mut out = vec![];
    let mut cur: Vec<Pt> = vec![];
    let mut closed = false;
    let mut last = Pt::default();
    for c in cmds {
        match *c {
            PathCmd::Move(p) => {
                if cur.len() > 1 {
                    out.push((std::mem::take(&mut cur), closed));
                }
                cur.clear();
                cur.push(p);
                closed = false;
                last = p;
            }
            PathCmd::Line(p) => {
                cur.push(p);
                last = p;
            }
            PathCmd::Cubic(a, b, p) => {
                for i in 1..=8 {
                    let t = i as f32 / 8.;
                    let mt = 1. - t;
                    let x = mt * mt * mt * last.x + 3. * mt * mt * t * a.x + 3. * mt * t * t * b.x + t * t * t * p.x;
                    let y = mt * mt * mt * last.y + 3. * mt * mt * t * a.y + 3. * mt * t * t * b.y + t * t * t * p.y;
                    cur.push(pt(x, y));
                }
                last = p;
            }
            PathCmd::Close => closed = true,
        }
    }
    if cur.len() > 1 {
        out.push((cur, closed));
    }
    out
}

/// 手绘风格：用 roughr 生成抖动的描边与排线填充。
pub fn rough_cmds(cmds: &[PathCmd], seed: u64, fill: bool, stroke_w: f32) -> (Vec<PathCmd>, Vec<PathCmd>) {
    use roughr::core::{FillStyle, OpSetType, OpType, OptionsBuilder};
    use roughr::generator::Generator;
    use roughr::Point2D;
    let gen = Generator::default();
    let mut stroke_out = vec![];
    let mut fill_out = vec![];
    for (pts, closed) in flatten(cmds) {
        let mut ob = OptionsBuilder::default();
        ob.seed(seed).roughness(1.1).stroke_width(stroke_w).hachure_gap(stroke_w * 4. + 2.).fill_weight(stroke_w * 0.7).fill_style(FillStyle::Hachure);
        if fill && closed {
            ob.fill(roughr::Srgba::new(0., 0., 0., 1.));
        }
        let Ok(opts) = ob.build() else { continue };
        let ps: Vec<Point2D<f32>> = pts.iter().map(|p| Point2D::new(p.x, p.y)).collect();
        let d = if closed { gen.polygon::<f32>(&ps, &Some(opts)) } else { gen.linear_path::<f32>(&ps, false, &Some(opts)) };
        for set in d.sets {
            let target = match set.op_set_type {
                OpSetType::Path => &mut stroke_out,
                OpSetType::FillSketch => &mut fill_out,
                OpSetType::FillPath => continue,
            };
            for op in set.ops {
                match op.op {
                    OpType::Move => target.push(PathCmd::Move(pt(op.data[0], op.data[1]))),
                    OpType::LineTo => target.push(PathCmd::Line(pt(op.data[0], op.data[1]))),
                    OpType::BCurveTo => target.push(PathCmd::Cubic(pt(op.data[0], op.data[1]), pt(op.data[2], op.data[3]), pt(op.data[4], op.data[5]))),
                }
            }
        }
    }
    (stroke_out, fill_out)
}

/// 解码后的位图缓存：后台线程解码（不卡界面），按总字节数做 LRU 淘汰。
mod images {
    use gpui::RenderImage;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, Mutex, OnceLock};

    /// 缓存上限（解码后的 BGRA 字节数）
    const BUDGET: usize = 256 << 20;

    enum Slot {
        Loading,
        Ready { img: Option<Arc<RenderImage>>, bytes: usize, tick: u64 },
    }

    #[derive(Default)]
    struct Cache {
        map: HashMap<String, Slot>,
        bytes: usize,
        tick: u64,
    }

    thread_local! {
        static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    }

    type Done = (String, Option<Arc<RenderImage>>, usize);
    static DONE: Mutex<Vec<Done>> = Mutex::new(Vec::new());
    /// 每完成一张图 +1（带缓存的绘制数据把它算进缓存键，图到了就重建）
    static GENERATION: AtomicU64 = AtomicU64::new(0);

    fn worker() -> &'static Mutex<Sender<String>> {
        static TX: OnceLock<Mutex<Sender<String>>> = OnceLock::new();
        TX.get_or_init(|| {
            let (tx, rx) = channel::<String>();
            std::thread::Builder::new()
                .name("stonebrook-images".into())
                .spawn(move || {
                    for path in rx {
                        let (img, bytes) = decode(&path);
                        DONE.lock().unwrap_or_else(|e| e.into_inner()).push((path, img, bytes));
                        GENERATION.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .expect("image thread");
            Mutex::new(tx)
        })
    }

    fn decode(path: &str) -> (Option<Arc<RenderImage>>, usize) {
        let Some(mut buf) = image::open(path).ok().map(|i| i.to_rgba8()) else { return (None, 0) };
        for px in buf.pixels_mut() {
            px.0.swap(0, 2); // RGBA → BGRA
        }
        let bytes = buf.as_raw().len();
        (Some(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(buf)]))), bytes)
    }

    pub fn generation() -> u64 {
        GENERATION.load(Ordering::Relaxed)
    }

    /// 还有没解码完的图吗。
    pub fn pending() -> bool {
        CACHE.with(|c| c.borrow().map.values().any(|s| matches!(s, Slot::Loading)))
    }

    /// 取图；还没解码好时返回 `None` 并在后台开始解码，`loading` 置为真（调用方据此请求下一帧）。
    pub fn get(path: &str, loading: &mut bool) -> Option<Arc<RenderImage>> {
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let done = std::mem::take(&mut *DONE.lock().unwrap_or_else(|e| e.into_inner()));
            for (p, img, bytes) in done {
                c.tick += 1;
                c.bytes += bytes;
                let tick = c.tick;
                c.map.insert(p, Slot::Ready { img, bytes, tick });
            }
            evict(&mut c);
            c.tick += 1;
            let now = c.tick;
            match c.map.get_mut(path) {
                Some(Slot::Ready { img, tick, .. }) => {
                    *tick = now;
                    img.clone()
                }
                Some(Slot::Loading) => {
                    *loading = true;
                    None
                }
                None => {
                    c.map.insert(path.to_string(), Slot::Loading);
                    worker().lock().unwrap_or_else(|e| e.into_inner()).send(path.to_string()).ok();
                    *loading = true;
                    None
                }
            }
        })
    }

    fn evict(c: &mut Cache) {
        while c.bytes > BUDGET {
            let oldest = c.map.iter().filter_map(|(k, s)| match s {
                Slot::Ready { tick, .. } => Some((*tick, k.clone())),
                Slot::Loading => None,
            });
            let Some((_, k)) = oldest.min() else { break };
            if let Some(Slot::Ready { bytes, .. }) = c.map.remove(&k) {
                c.bytes -= bytes;
            }
        }
    }
}

pub use images::{generation as image_generation, pending as image_pending};

/// 同 `load_image`，但不请求重绘（拿不到 `Window` 的地方用；调用方自己看 `image_pending`）。
pub fn try_load_image(path: &str) -> Option<Arc<gpui::RenderImage>> {
    images::get(path, &mut false)
}

/// 本地图片的 GPU 位图。首次请求时在后台解码，解码完成前返回 `None` 并请求下一帧重绘。
pub fn load_image(path: &str, window: &gpui::Window) -> Option<Arc<gpui::RenderImage>> {
    let mut loading = false;
    let img = images::get(path, &mut loading);
    if loading {
        window.request_animation_frame();
    }
    img
}

/// 按画布绘制同样的字体与宽度换行，返回总行数（文字撑高图形 / 表格行时用）。
pub fn count_lines(ts: &std::sync::Arc<gpui::TextSystem>, text: &str, size: f32, bold: bool, width: f32) -> usize {
    let mut w = ts.line_wrapper(font(bold), px(size));
    text.split('\n').map(|p| if p.is_empty() { 1 } else { 1 + w.wrap_line(&[gpui::LineFragment::text(p)], px(width)).count() }).sum()
}

fn text_run(len: usize, color: Hsla, bold: bool) -> TextRun {
    TextRun { len, font: font(bold), color, background_color: None, underline: None, strikethrough: None }
}

/// 带修饰（斜体 / 下划线 / 删除线）的文字样式。
pub fn deco_run(len: usize, color: Hsla, bold: bool, deco: u8) -> TextRun {
    let mut r = text_run(len, color, bold);
    if deco & super::DECO_ITALIC != 0 {
        r.font.style = gpui::FontStyle::Italic;
    }
    if deco & super::DECO_UNDERLINE != 0 {
        r.underline = Some(gpui::UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false });
    }
    if deco & super::DECO_STRIKE != 0 {
        r.strikethrough = Some(gpui::StrikethroughStyle { color: Some(color), thickness: px(1.) });
    }
    r
}

/// 在矩形内绘制多行文字（自动换行）。
#[allow(clippy::too_many_arguments)]
pub fn paint_text_block(window: &mut Window, cx: &mut App, text: &str, area: Bounds<Pixels>, size_px: f32, color: Hsla, bold: bool, align: Align) {
    paint_text_block_deco(window, cx, text, area, size_px, color, bold, align, 0)
}

/// 同 [`paint_text_block`]，可带斜体 / 下划线 / 删除线。
#[allow(clippy::too_many_arguments)]
pub fn paint_text_block_deco(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    area: Bounds<Pixels>,
    size_px: f32,
    color: Hsla,
    bold: bool,
    align: Align,
    deco: u8,
) {
    let fmt = super::Fmt { bold, deco, color: None };
    paint_text_rich(window, cx, text, &[(text.len(), fmt)], area, size_px, color, 1., align)
}

/// 带样式分段的多行文字（选中部分文字改样式，见 `canvas::rich`）。
/// 分段颜色为 None 时用 `color`；分段里的颜色乘上 `opacity`。
#[allow(clippy::too_many_arguments)]
pub fn paint_text_rich(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    segs: &[(usize, super::Fmt)],
    area: Bounds<Pixels>,
    size_px: f32,
    color: Hsla,
    opacity: f32,
    align: Align,
) {
    if text.is_empty() || size_px < 3.5 {
        return;
    }
    let lh = px(size_px * 1.38);
    let mut lines = vec![];
    for (para, segs) in super::rich::paragraphs(text, segs) {
        let s: SharedString = para.into();
        let runs: Vec<TextRun> =
            segs.iter().map(|(l, f)| deco_run(*l, f.color.map(|c| c.with_alpha(c.alpha() * opacity).hsla()).unwrap_or(color), f.bold, f.deco)).collect();
        if let Ok(wrapped) = window.text_system().shape_text(s, px(size_px), &runs, Some(area.size.width.max(px(4.))), None) {
            lines.extend(wrapped);
        }
    }
    let total: Pixels = lines.iter().map(|l| l.size(lh).height).fold(px(0.), |a, b| a + b);
    let mut y = if align.is_top() {
        area.origin.y
    } else if align.is_bottom() {
        area.origin.y + area.size.height - total
    } else {
        area.origin.y + (area.size.height - total) / 2.
    };
    let ta = match align.h() {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    };
    for l in lines {
        let h = l.size(lh).height;
        let b = Bounds::new(point(area.origin.x, y), size(area.size.width, h));
        l.paint(point(area.origin.x, y), lh, ta, Some(b), window, cx).ok();
        y += h;
    }
}

/// 单行文字，按锚点对齐。
pub fn paint_label(window: &mut Window, cx: &mut App, text: &str, at: Point<Pixels>, size_px: f32, color: Hsla, bold: bool, anchor: Anchor, bg: Option<Hsla>) {
    if text.is_empty() || size_px < 3.5 {
        return;
    }
    let lh = px(size_px * 1.3);
    let lines: Vec<&str> = text.split('\n').collect();
    let n = lines.len() as f32;
    for (i, line) in lines.iter().enumerate() {
        let s: SharedString = line.to_string().into();
        let shaped = window.text_system().shape_line(s.clone(), px(size_px), &[text_run(s.len(), color, bold)], None);
        let w = shaped.width;
        let x = match anchor {
            Anchor::Start => at.x,
            Anchor::Middle => at.x - w / 2.,
            Anchor::End => at.x - w,
        };
        let y = at.y - lh * (n / 2.) + lh * i as f32;
        if let Some(bg) = bg {
            window.paint_quad(fill(Bounds::new(point(x - px(4.), y - px(1.)), size(w + px(8.), lh + px(2.))), bg).corner_radii(px(4.)));
        }
        shaped.paint(point(x, y), lh, window, cx).ok();
    }
}

pub fn paint_scene(scene: &Scene, view: View, clip: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if let Some(bg) = scene.bg {
        window.paint_quad(fill(view.r(rect(0., 0., scene.width, scene.height)), bg.hsla()));
    }
    // 连续的连线先统一画线体与箭头，再统一画标签，防止交叉连线压住其他边的文字
    let mut i = 0;
    while i < scene.items.len() {
        if matches!(scene.items[i], Item::Line(_)) {
            let start = i;
            while i < scene.items.len() && matches!(scene.items[i], Item::Line(_)) {
                if let Item::Line(l) = &scene.items[i] {
                    if view.r(scene.items[i].bounds().inflate(4.)).intersects(&clip) {
                        paint_line_stroke(l, scene.rough, &view, window);
                    }
                }
                i += 1;
            }
            for item in &scene.items[start..i] {
                if let Item::Line(l) = item {
                    if view.r(item.bounds().inflate(4.)).intersects(&clip) {
                        paint_line_labels(l, &view, window, cx);
                    }
                }
            }
            continue;
        }
        let item = &scene.items[i];
        i += 1;
        let b = view.r(item.bounds().inflate(4.));
        if !b.intersects(&clip) {
            continue;
        }
        match item {
            Item::Shape(s) => paint_shape(s, scene.rough, &view, window, cx),
            Item::Line(_) => {}
            Item::Text(t) => {
                let at = view.p(t.pos);
                paint_label(
                    window,
                    cx,
                    &t.text,
                    point(at.x, at.y - px(t.size * view.scale * 0.35)),
                    t.size * view.scale,
                    t.color.hsla(),
                    t.bold,
                    t.anchor,
                    None,
                );
            }
            Item::Path(p) => {
                if let Some(f) = p.fill {
                    fill_cmds(window, &p.cmds, &view, f);
                }
                if let Some(s) = p.stroke {
                    stroke_cmds(window, &p.cmds, &view, s, p.width, p.dash.then_some(5.));
                }
            }
            Item::Image(i) => {
                if let Some(img) = load_image(&i.path, window) {
                    window.paint_image(view.r(i.rect), Corners::all(px(4. * view.scale)), img, 0, false).ok();
                }
            }
            Item::Icon(i) => {
                if crate::board::iconlib::is_color(&i.name) {
                    // 彩色图标（表情、头像、技术图标……）按显示尺寸栅格化
                    let device = i.rect.w.max(i.rect.h) * view.scale * window.scale_factor();
                    if let Some(img) = crate::board::iconlib::raster_for(&i.name, device) {
                        window.paint_image(view.r(i.rect), Corners::default(), img, 0, false).ok();
                    }
                } else {
                    let path: SharedString = format!("icons/lib/{}.svg", i.name).into();
                    window.paint_svg(view.r(i.rect), path, gpui::TransformationMatrix::unit(), i.color.hsla(), cx).ok();
                }
            }
        }
    }
}

pub fn paint_shape(s: &ShapeItem, rough: bool, view: &View, window: &mut Window, cx: &mut App) {
    let r = s.rect;
    let (body, deco) = shape_cmds(s.kind, r, s.radius);
    let fill_col = s.fill.map(|c| c.with_alpha(c.alpha() * s.opacity));
    let stroke_col = s.stroke.map(|c| c.with_alpha(c.alpha() * s.opacity));
    let dash = if s.dotted { Some(2.) } else { s.dash.then_some(5.) };
    if s.shadow && !rough {
        window.paint_shadows(
            view.r(r),
            Corners::all(px(s.radius * view.scale)),
            &[gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.12),
                offset: point(px(0.), px(3. * view.scale)),
                blur_radius: px(8. * view.scale),
                spread_radius: px(0.),
            }],
        );
    }
    match s.kind {
        // 激活条不在这里：，填充管里面、边框管描边，走普通图形的画法
        ShapeKind::StartDot | ShapeKind::Bar => {
            let c = stroke_col.or(fill_col).unwrap_or(Col::rgb(0x333333));
            fill_cmds(window, &body, view, c);
        }
        ShapeKind::EndDot => {
            let c = stroke_col.unwrap_or(Col::rgb(0x333333));
            stroke_cmds(window, &body, view, c, s.stroke_w, None);
            fill_cmds(window, &deco, view, c);
        }
        // 便签与分区底色在手绘风格下也保持实心，否则斜线填充会让文字看不清
        _ if rough && !(s.kind == ShapeKind::Sticky || s.id.starts_with("sec:")) => {
            let (stroke, hatch) = rough_cmds(&body, s.seed, fill_col.is_some(), s.stroke_w);
            if let Some(f) = fill_col {
                stroke_cmds(window, &hatch, view, f, s.stroke_w * 0.8, None);
            }
            if let Some(c) = stroke_col {
                stroke_cmds(window, &stroke, view, c, s.stroke_w, None);
                let (d, _) = rough_cmds(&deco, s.seed + 7, false, s.stroke_w);
                stroke_cmds(window, &d, view, c, s.stroke_w, None);
            }
        }
        _ => {
            if let Some(f) = fill_col {
                fill_cmds(window, &body, view, f);
            }
            if let Some(c) = stroke_col {
                stroke_cmds(window, &body, view, c, s.stroke_w, dash);
                stroke_cmds(window, &deco, view, c, s.stroke_w, None);
            }
        }
    }
    let text_col = s.text_color.with_alpha(s.text_color.alpha() * s.opacity).hsla();
    let fs = s.font_size * view.scale;
    if matches!(s.kind, ShapeKind::Frame | ShapeKind::FrameAlt) {
        // 左上角标签页
        if !s.text.is_empty() {
            let w = estimate_width(&s.text, s.font_size, true) + 16.;
            let tab = rect(r.x, r.y, w, s.font_size * 1.7);
            let tab_cmds = polygon(&[
                pt(tab.x, tab.y),
                pt(tab.right(), tab.y),
                pt(tab.right(), tab.bottom() - 5.),
                pt(tab.right() - 6., tab.bottom()),
                pt(tab.x, tab.bottom()),
            ]);
            if let Some(c) = stroke_col {
                fill_cmds(window, &tab_cmds, view, c.with_alpha(0.08));
                stroke_cmds(window, &tab_cmds, view, c, s.stroke_w * 0.8, None);
            }
            paint_text_block(window, cx, &s.text, view.r(tab), fs, text_col, true, Align::Center);
        }
        return;
    }
    if s.kind == ShapeKind::Actor {
        paint_label(window, cx, &s.text, view.p(pt(r.center().x, r.bottom() + s.font_size * 0.9)), fs, text_col, false, Anchor::Middle, None);
        return;
    }
    if !s.sections.is_empty() {
        let lh = s.font_size * 1.45;
        let mut y = r.y + 6.;
        for (i, sec) in s.sections.iter().enumerate() {
            if i > 0 {
                if let Some(c) = stroke_col {
                    stroke_cmds(window, &[PathCmd::Move(pt(r.x, y)), PathCmd::Line(pt(r.right(), y))], view, c, s.stroke_w * 0.8, None);
                }
                y += 4.;
            }
            for line in &sec.lines {
                let area = view.r(rect(r.x + 10., y, r.w - 20., lh));
                paint_text_block(window, cx, line, area, fs, text_col, sec.bold, if sec.left { Align::Left } else { Align::Center });
                y += lh;
            }
            y += 4.;
        }
        return;
    }
    if !s.text.is_empty() {
        let segs = super::rich::resolve(&s.spans, s.text.len(), super::Fmt { bold: s.bold, deco: s.deco, color: None });
        paint_text_rich(window, cx, &s.text, &segs, view.r(text_area(s.kind, r, s.align)), fs, text_col, s.opacity, s.align);
    }
    for (c, t) in compartments(s.kind, r).into_iter().zip(&s.rows) {
        if !t.is_empty() {
            paint_text_block(window, cx, t, view.r(compartment_text_area(c)), fs, text_col, false, Align::TopLeft);
        }
    }
}

/// 形状内文字的排版区域（世界坐标）。画板原位编辑文字时用同一个区域，保证所见即所得。
pub fn text_area(kind: ShapeKind, r: Rect, align: Align) -> Rect {
    // 先确定文字所在的框：类图 / 实体写在标题栏，生命线写在头部，图标类写在图标下方
    let below = |top: f32| rect(r.x - 30., top, r.w + 60., 24.);
    let (r, kind) = match kind {
        ShapeKind::ClassBox
        | ShapeKind::InterfaceBox
        | ShapeKind::Entity
        | ShapeKind::Entity2Key
        | ShapeKind::Entity2
        | ShapeKind::Entity3
        | ShapeKind::DfdProcess => (rect(r.x, r.y, r.w, header_h(r)), ShapeKind::Rect),
        ShapeKind::LifeBox | ShapeKind::LifeDb | ShapeKind::LifeQueue | ShapeKind::LifeCollections => (rect(r.x, r.y, r.w, life_head(r)), ShapeKind::Rect),
        ShapeKind::LifeActor | ShapeKind::LifeBoundary | ShapeKind::LifeControl | ShapeKind::LifeEntity => (below(r.y + life_icon(r)), ShapeKind::Rect),
        ShapeKind::ProvidedIface | ShapeKind::AssemblyIface | ShapeKind::RequiredIface => (below(r.bottom() + 2.), ShapeKind::Rect),
        ShapeKind::BraceLeft => (rect(r.x, r.y, r.w * 0.8, r.h), ShapeKind::Rect),
        ShapeKind::BraceRight => (rect(r.x + r.w * 0.2, r.y, r.w * 0.8, r.h), ShapeKind::Rect),
        ShapeKind::Cube => {
            let d = r.w.min(r.h) * 0.22;
            (rect(r.x, r.y + d, r.w - d, r.h - d), ShapeKind::Rect)
        }
        ShapeKind::BubbleRound => (rect(r.x, r.y, r.w, r.h * 0.84), ShapeKind::Ellipse),
        ShapeKind::Bubble => (rect(r.x, r.y, r.w, r.h - (r.h * 0.22).min(18.)), ShapeKind::Rect),
        ShapeKind::Frame | ShapeKind::FrameAlt => (rect(r.x, r.y, r.w.min(140.), 26f32.min(r.h)), ShapeKind::Rect),
        ShapeKind::DataStoreId => {
            let d = (r.w * 0.2).min(36.);
            (rect(r.x + d, r.y, r.w - d, r.h), ShapeKind::Rect)
        }
        ShapeKind::MultiDoc => {
            let o = (r.w.min(r.h) * 0.08).max(3.);
            (rect(r.x, r.y + 2. * o, r.w - 2. * o, r.h - 2. * o), ShapeKind::Document)
        }
        _ => (r, kind),
    };
    let pad = match kind {
        ShapeKind::Diamond | ShapeKind::Star | ShapeKind::Sort => r.w * 0.18,
        ShapeKind::Triangle | ShapeKind::RightTriangle | ShapeKind::Merge => r.w * 0.2,
        ShapeKind::Hexagon
        | ShapeKind::Parallelogram
        | ShapeKind::ParallelogramAlt
        | ShapeKind::Trapezoid
        | ShapeKind::TrapezoidAlt
        | ShapeKind::Pentagon
        | ShapeKind::Octagon
        | ShapeKind::Chevron
        | ShapeKind::HomePlate
        | ShapeKind::Display
        | ShapeKind::ManualInput => 14.,
        ShapeKind::Circle | ShapeKind::DoubleCircle | ShapeKind::Ellipse | ShapeKind::Cloud | ShapeKind::BlockArc | ShapeKind::Pie => r.w * 0.12,
        ShapeKind::Note | ShapeKind::Sticky => 12.,
        _ => 8.,
    };
    match align {
        Align::TopLeft | Align::TopCenter | Align::TopRight => rect(r.x + pad, r.y + pad * 0.8, r.w - 2. * pad, r.h - pad),
        Align::BottomLeft | Align::BottomCenter | Align::BottomRight => rect(r.x + pad, r.y + pad * 0.2, r.w - 2. * pad, r.h - pad),
        _ => rect(r.x + pad, r.y + 2., r.w - 2. * pad, r.h - 4.),
    }
}

pub fn line_cmds(l: &LineItem) -> Vec<PathCmd> {
    // 箭头处缩短线段，避免线头戳出箭头尖
    let mut pts = l.points.clone();
    let n = pts.len();
    if n >= 2 {
        let size = (l.width * 4.).max(9.);
        let ie = arrow_inset(l.end, size);
        let (end_from, start_from) = l.arrow_from();
        if ie > 0. {
            let (a, b) = (end_from, pts[n - 1]);
            let d = a.dist(b).max(1e-3);
            pts[n - 1] = b.lerp(a, (ie / d).min(0.9));
        }
        let is = arrow_inset(l.start, size);
        if is > 0. {
            let (a, b) = (start_from, pts[0]);
            let d = a.dist(b).max(1e-3);
            pts[0] = b.lerp(a, (is / d).min(0.9));
        }
    }
    match l.tangents {
        Some((da, db)) if l.curved && l.basis => crate::canvas::geom::basis_cmds_dir(&pts, da, db),
        Some((da, db)) if l.curved => crate::canvas::geom::smooth_cmds_dir(&pts, da, db),
        _ if l.curved && l.basis && pts.len() > 2 => basis_cmds(&pts),
        _ if l.curved && pts.len() > 2 => smooth_cmds(&pts),
        _ if !l.curved && l.corner_radius > 0. && pts.len() > 2 => rounded_polyline_cmds(&pts, l.corner_radius),
        _ => polyline_cmds(&pts),
    }
}

pub fn paint_line_stroke(l: &LineItem, rough: bool, view: &View, window: &mut Window) {
    if l.points.len() < 2 {
        return;
    }
    let cmds = line_cmds(l);
    let dash = if l.dotted {
        Some(2.)
    } else if l.dash {
        Some(5.)
    } else {
        None
    };
    if rough && dash.is_none() {
        let (s, _) = rough_cmds(&cmds, l.seed, false, l.width);
        stroke_cmds(window, &s, view, l.stroke, l.width, None);
    } else {
        stroke_cmds(window, &cmds, view, l.stroke, l.width, dash);
    }
    let size = (l.width * 4.).max(9.);
    let n = l.points.len();
    let (end_from, start_from) = l.arrow_from();
    for (kind, tip, from) in [(l.end, l.points[n - 1], end_from), (l.start, l.points[0], start_from)] {
        for (cmds, filled) in arrow_cmds(kind, tip, from, size) {
            if filled {
                fill_cmds(window, &cmds, view, l.stroke);
            } else {
                if matches!(kind, Arrow::Hollow | Arrow::DiamondHollow | Arrow::Circle) {
                    fill_cmds(window, &cmds, view, l.label_bg.unwrap_or(Col::rgb(0xffffff)));
                }
                stroke_cmds(window, &cmds, view, l.stroke, l.width, None);
            }
        }
    }
}

pub fn paint_line_labels(l: &LineItem, view: &View, window: &mut Window, cx: &mut App) {
    if l.points.len() < 2 {
        return;
    }
    let n = l.points.len();
    let fs = l.font_size * view.scale;
    let bg = l.label_bg.map(|c| c.hsla());
    if !l.label.is_empty() {
        let m = l.label_pos.unwrap_or_else(|| if l.curved || l.corner_radius > 0. { cmds_point_at(&line_cmds(l), 0.5) } else { polyline_mid(&l.points) });
        paint_label(window, cx, &l.label, view.p(m), fs, l.label_color.hsla(), false, Anchor::Middle, bg);
    }
    for (label, a, b) in [(&l.start_label, l.points[0], l.points[1]), (&l.end_label, l.points[n - 1], l.points[n - 2])] {
        if !label.is_empty() {
            let d = b.sub(a);
            let len = (d.x * d.x + d.y * d.y).sqrt().max(1e-3);
            let p = a.add(d.scale(18. / len)).add(pt(-d.y / len * 10., d.x / len * 10.));
            paint_label(window, cx, label, view.p(p), fs * 0.92, l.label_color.hsla(), false, Anchor::Middle, None);
        }
    }
}

#[allow(dead_code)]
pub fn paint_line(l: &LineItem, rough: bool, view: &View, window: &mut Window, cx: &mut App) {
    paint_line_stroke(l, rough, view, window);
    paint_line_labels(l, view, window, cx);
}
