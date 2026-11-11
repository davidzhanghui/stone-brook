//! 画板「素材」图标库（画板工具栏的「素材」：IconPark、表情、用户头像、项目管理……）。
//!
//! 素材集来自开源图标库，由 `scripts/iconlib/gen.py` 生成到 `assets/iconlib/*.json.z`（zlib 压缩的 JSON），
//! 首次使用时解压（启动时会在后台线程预热）。来源见 `assets/iconlib/LICENSES.md`，许可全文见 `THIRD_PARTY_LICENSES.txt`。
//!
//! 图标名带素材集前缀，如 `park:user`、`fluent:grinning-face`、`fluentui:person`（Fluent UI System Icons，与 Fluent Emoji 的 `fluent` 区分）；不带前缀的是最早内置的「常用」Lucide 图标（`board::icons`）。
//! 单色图标（mono）可以改颜色，按 GPUI 的 SVG 蒙版绘制；彩色图标用 resvg 栅格化成位图绘制。

use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Deserialize)]
pub struct Pack {
    pub id: String,
    pub title: String,
    /// 单色图标：颜色跟随元素的描边色
    pub mono: bool,
    #[serde(default)]
    w: f32,
    #[serde(default)]
    h: f32,
    pub license: String,
    pub url: String,
    pub cats: Vec<Cat>,
    /// 首页卡片上展示的图标名（不带前缀），空则取前几个
    #[serde(default)]
    preview: Vec<String>,
}

#[derive(Deserialize)]
pub struct Cat {
    pub name: String,
    pub icons: Vec<Def>,
}

#[derive(Deserialize)]
pub struct Def {
    #[serde(rename = "n")]
    name: String,
    /// 中文名（悬停提示）
    #[serde(rename = "t")]
    pub title: String,
    #[serde(rename = "k")]
    keys: String,
    #[serde(rename = "b")]
    body: String,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    /// 完整的图标名（带素材集前缀）
    #[serde(skip)]
    pub id: String,
    /// 搜索用的小写关键词（名字、中文名、标签、分类）
    #[serde(skip)]
    search: String,
}

impl Pack {
    pub fn len(&self) -> usize {
        self.cats.iter().map(|c| c.icons.len()).sum()
    }

    pub fn icons(&self) -> impl Iterator<Item = &Def> {
        self.cats.iter().flat_map(|c| c.icons.iter())
    }

    /// 首页卡片上展示的图标
    pub fn preview(&self) -> Box<dyn Iterator<Item = &Def> + '_> {
        if self.preview.is_empty() {
            return Box::new(self.icons());
        }
        Box::new(self.preview.iter().filter_map(|n| self.icons().find(|d| &d.name == n)))
    }
}

impl Def {
    /// 搜索：多个词用空格隔开，每个词都要命中。
    pub fn matches(&self, words: &[&str]) -> bool {
        words.iter().all(|w| self.search.contains(w))
    }
}

/// 按顺序打包进来的素材集（id, 数据）
const PACK_FILES: &[(&str, &[u8])] = &[
    ("park", include_bytes!("../../assets/iconlib/park.json.z")),
    ("park-solid", include_bytes!("../../assets/iconlib/park-solid.json.z")),
    ("park-twotone", include_bytes!("../../assets/iconlib/park-twotone.json.z")),
    ("pm", include_bytes!("../../assets/iconlib/pm.json.z")),
    ("avatar", include_bytes!("../../assets/iconlib/avatar.json.z")),
    ("logos", include_bytes!("../../assets/iconlib/logos.json.z")),
    ("brands", include_bytes!("../../assets/iconlib/brands.json.z")),
    ("flags", include_bytes!("../../assets/iconlib/flags.json.z")),
    ("tabler", include_bytes!("../../assets/iconlib/tabler.json.z")),
    ("tabler-filled", include_bytes!("../../assets/iconlib/tabler-filled.json.z")),
    ("ph-fill", include_bytes!("../../assets/iconlib/ph-fill.json.z")),
    ("ph-duotone", include_bytes!("../../assets/iconlib/ph-duotone.json.z")),
    ("tdesign", include_bytes!("../../assets/iconlib/tdesign.json.z")),
    ("fluentui", include_bytes!("../../assets/iconlib/fluentui.json.z")),
    ("carbon", include_bytes!("../../assets/iconlib/carbon.json.z")),
    ("fluent", include_bytes!("../../assets/iconlib/fluent.json.z")),
];

/// 素材面板里的顺序：常用（内置 Lucide）、IconPark（线框 / 实心 / 双色）、项目管理、用户头像、技术图标、品牌图标、国旗与地区旗、
/// Tabler（线框 / 实心）、Phosphor（实心 / 双色）、TDesign、Fluent 图标、Carbon 架构、Fluent Emoji。
/// 素材集 id 会作为图标名前缀存进文档，已发布的 id 不能改。
static PACKS: LazyLock<Vec<Pack>> = LazyLock::new(|| {
    let mut packs = vec![builtin()];
    for (id, data) in PACK_FILES {
        let mut json = String::new();
        if flate2::read::ZlibDecoder::new(*data).read_to_string(&mut json).is_err() {
            continue;
        }
        match serde_json::from_str::<Pack>(&json) {
            Ok(p) => packs.push(p),
            Err(e) => eprintln!("图标素材集 {id} 解析失败：{e}"),
        }
    }
    for p in &mut packs {
        let prefix = if p.id == "lucide" { String::new() } else { format!("{}:", p.id) };
        let title = p.title.to_lowercase();
        let (pw, ph) = (p.w, p.h);
        for c in &mut p.cats {
            let cat = c.name.to_lowercase();
            for d in &mut c.icons {
                d.id = format!("{prefix}{}", d.name);
                if d.w == 0. {
                    (d.w, d.h) = (pw, ph);
                }
                d.search = format!("{} {} {} {cat} {title}", d.name, d.title, d.keys).to_lowercase();
            }
        }
    }
    packs
});

static INDEX: LazyLock<HashMap<&'static str, (&'static Pack, &'static Def)>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for p in PACKS.iter() {
        for d in p.icons() {
            m.entry(d.id.as_str()).or_insert((p, d));
        }
    }
    m
});

/// 最早内置的「常用」图标（Lucide，带中文分类与关键词）
fn builtin() -> Pack {
    let cats = super::icons::ICON_CATS
        .iter()
        .map(|(cat, items)| Cat {
            name: cat.to_string(),
            icons: items
                .iter()
                .map(|(n, l, b)| Def {
                    name: n.to_string(),
                    title: l.split(' ').next().unwrap_or(l).to_string(),
                    keys: l.to_string(),
                    body: format!(r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{b}</g>"#),
                    w: 24.,
                    h: 24.,
                    id: String::new(),
                    search: String::new(),
                })
                .collect(),
        })
        .collect();
    Pack {
        id: "lucide".into(),
        title: "常用".into(),
        mono: true,
        w: 24.,
        h: 24.,
        license: "ISC · Lucide".into(),
        url: "https://lucide.dev".into(),
        cats,
        preview: vec![],
    }
}

/// 在后台线程解压素材，避免第一次打开面板或画板时卡顿。
pub fn warm() {
    std::thread::spawn(|| {
        LazyLock::force(&INDEX);
    });
}

pub fn packs() -> &'static [Pack] {
    &PACKS
}

pub fn pack(id: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.id == id)
}

pub fn get(name: &str) -> Option<(&'static Pack, &'static Def)> {
    INDEX.get(name).copied()
}

/// 彩色图标（不能改颜色，按位图绘制）
pub fn is_color(name: &str) -> bool {
    get(name).is_some_and(|(p, _)| !p.mono)
}

/// 完整的 SVG（原始尺寸）；单色图标的 currentColor 换成 color（如 `#1f2329`）。
pub fn svg(name: &str, color: Option<&str>) -> Option<String> {
    let (_, d) = get(name)?;
    svg_with(name, color, &format!(r#"width="{}" height="{}""#, d.w, d.h))
}

/// 同 [`svg`]，根元素的位置 / 尺寸等属性由 attrs 给出（导出时内嵌到画板 SVG 里用）。
pub fn svg_with(name: &str, color: Option<&str>, attrs: &str) -> Option<String> {
    let (p, d) = get(name)?;
    let body = match color {
        Some(c) if p.mono => d.body.replace("currentColor", c),
        _ => d.body.clone(),
    };
    Some(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" {attrs} viewBox="0 0 {} {}">{body}</svg>"#, d.w, d.h))
}

/// 新插入时的尺寸：长边 48（头像 64），保持原图比例。
pub fn insert_size(name: &str) -> (f32, f32) {
    let Some((p, d)) = get(name) else { return (48., 48.) };
    let s = (if p.id == "avatar" { 64. } else { 48. }) / d.w.max(d.h).max(1.);
    (d.w * s, d.h * s)
}

thread_local! {
    static RASTER: RefCell<crate::lru::Lru<(String, u32), Option<Arc<gpui::RenderImage>>>> = RefCell::new(crate::lru::Lru::new(512));
}

/// 彩色图标栅格化为 GPUI 位图（BGRA），长边 px 像素，带缓存。
pub fn raster(name: &str, px: u32) -> Option<Arc<gpui::RenderImage>> {
    let key = (name.to_string(), px);
    if let Some(hit) = RASTER.with(|m| m.borrow().get(&key).cloned()) {
        return hit;
    }
    let img = (|| {
        let (_, d) = get(name)?;
        let svg = svg(name, None)?;
        let opts = resvg::usvg::Options { fontdb: crate::canvas::svg::fontdb(), ..Default::default() };
        let tree = resvg::usvg::Tree::from_str(&svg, &opts).ok()?;
        let s = px as f32 / d.w.max(d.h).max(1.);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(((d.w * s).ceil() as u32).max(1), ((d.h * s).ceil() as u32).max(1))?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
        let (w, h) = (pixmap.width(), pixmap.height());
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for p in pixmap.pixels() {
            let c = p.demultiply();
            bgra.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
        }
        let buf = image::RgbaImage::from_raw(w, h, bgra)?;
        Some(Arc::new(gpui::RenderImage::new(smallvec::smallvec![image::Frame::new(buf)])))
    })();
    RASTER.with(|m| m.borrow_mut().insert(key, img.clone()));
    img
}

/// 画布上按显示尺寸挑选栅格化精度（2 的幂，32–1024）。
pub fn raster_for(name: &str, device_px: f32) -> Option<Arc<gpui::RenderImage>> {
    raster(name, (device_px.ceil() as u32).clamp(32, 1024).next_power_of_two())
}

// ---------------------------------------------------------------- 最近使用

const RECENT_MAX: usize = 16;

static RECENT: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| {
    let list = std::fs::read_to_string(recent_path()).unwrap_or_default();
    Mutex::new(list.lines().filter(|l| !l.is_empty()).take(RECENT_MAX).map(str::to_string).collect())
});

fn recent_path() -> std::path::PathBuf {
    crate::store::data_dir().join("icon-recent.txt")
}

/// 最近使用的素材（图标名，或 `emoji:<表情>`），最新的在前。
pub fn recent() -> Vec<String> {
    RECENT.lock().unwrap().clone()
}

pub fn push_recent(key: &str) {
    let mut r = RECENT.lock().unwrap();
    r.retain(|k| k != key);
    r.insert(0, key.to_string());
    r.truncate(RECENT_MAX);
    std::fs::write(recent_path(), r.join("\n")).ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn packs_load_and_resolve() {
        let packs = super::packs();
        assert!(packs.len() >= 17, "素材集数量：{}", packs.len());
        assert!(packs.iter().map(|p| p.len()).sum::<usize>() > 23000);
        // 每个打包进来的素材集都要能解析出来（解析失败只会打印错误，不会 panic）
        for (id, _) in super::PACK_FILES {
            assert!(super::pack(id).is_some_and(|p| p.len() > 0), "素材集 {id} 没有加载");
        }
        for name in [
            "park-solid:user",
            "park-twotone:user",
            "tabler-filled:user-filled",
            "ph-fill:house-fill",
            "ph-duotone:house-duotone",
            "tdesign:user",
            "fluentui:person",
            "carbon:kubernetes",
            "brands:wechat",
            "flags:cn",
        ] {
            assert!(super::get(name).is_some(), "缺少 {name}");
        }
        assert!(super::is_color("flags:cn"));
        assert!(!super::is_color("brands:wechat"));
        assert!(super::get("flags:tw").is_none());
        assert!(super::get("star").is_some(), "内置图标不带前缀");
        assert!(super::get("park:user").is_some());
        assert!(super::is_color("fluent:grinning-face"));
        assert!(!super::is_color("tabler:user"));
        // 中文搜索：IconPark 自带中文，Tabler 借 IconPark 的英文名补了中文词
        let hits = |q: &str, pack: &str| super::pack(pack).unwrap().icons().filter(|d| d.matches(&[q])).count();
        assert!(hits("用户", "park") > 5);
        assert!(hits("用户", "tabler") > 5);
        assert!(hits("数据库", "logos") > 10);
        assert!(hits("笑", "fluent") > 5);
        assert!(hits("用户", "park-solid") > 5);
        assert!(hits("用户", "tdesign") > 5);
        assert!(hits("数据库", "carbon") > 5);
        assert!(hits("微信", "brands") == 1);
        assert!(hits("中国", "flags") >= 3);
        assert!(hits("用户", "fluentui") > 5);
    }

    #[test]
    fn color_icons_rasterize() {
        for name in ["fluent:grinning-face", "avatar:notionists-1", "logos:docker", "pm:priority-1", "pm:star-half", "flags:cn", "flags:european-union"] {
            let img = super::raster(name, 64).unwrap_or_else(|| panic!("{name} 栅格化失败"));
            assert!(img.size(0).width.0 > 0);
        }
        let svg = super::svg("tabler:user", Some("#ff0000")).unwrap();
        assert!(svg.contains("#ff0000") && !svg.contains("currentColor"));
        // 用 mask 的单色图标（IconPark 实心 / 双色）改色后 mask id 带了素材集前缀，不会和别的图标撞
        let svg = super::svg("park-twotone:user", Some("#ff0000")).unwrap();
        assert!(svg.contains("#ff0000") && svg.contains(r#"id="park-twotone-user-"#), "{svg}");
    }
}

#[cfg(test)]
mod export_tests {
    use crate::canvas::{rect, Col, IconItem, Item, Scene};

    /// 导出 SVG / PNG：单色图标带颜色，彩色图标作为内嵌图片；结果能被 resvg 正常渲染。
    #[test]
    fn export_icons_to_svg_png() {
        let names = [
            "star",
            "park:user",
            "tabler:database",
            "fluent:rocket",
            "avatar:lorelei-2",
            "logos:kubernetes",
            "pm:progress-75",
            "park-twotone:user",
            "ph-duotone:house-duotone",
            "fluentui:person",
            "brands:wechat",
            "flags:cn",
        ];
        let items = names
            .iter()
            .enumerate()
            .map(|(i, n)| Item::Icon(IconItem { rect: rect(10. + i as f32 * 60., 10., 48., 48.), name: n.to_string(), color: Col::rgb(0x3370ff) }))
            .collect();
        let scene = Scene { items, width: 20. + names.len() as f32 * 60., height: 70., bg: Some(Col::rgb(0xffffff)), ..Default::default() };
        let svg = crate::canvas::svg::to_svg(&scene);
        assert!(svg.contains("#3370ff") && svg.matches("data:image/svg+xml;base64").count() == 5, "{svg}");
        let png = crate::canvas::svg::svg_to_png(&svg, 2.).expect("png");
        if let Ok(out) = std::env::var("ICON_EXPORT_PNG") {
            std::fs::write(out, &png).unwrap();
        }
        // 每个图标所在区域都要有非白色像素
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        for i in 0..names.len() {
            let x0 = (10 + i as u32 * 60) * 2;
            let inked =
                (x0..x0 + 96).flat_map(|x| (20..116).map(move |y| (x, y))).filter(|&(x, y)| img.get_pixel(x, y).0[..3].iter().any(|c| *c < 200)).count();
            assert!(inked > 50, "{} 没有画出来", names[i]);
        }
    }
}
