//! 「视频或文件」里的音频：圆形播放键 + 文件名 + 波形进度条 + 时长 / 倍速。
//! - 播放用 AVPlayer（与视频块共用 crate::media 的播放器表，同一时刻只播一个）；
//! - 波形用 symphonia 在后台解码算峰值，结果缓存到 cache 目录；
//! - AVFoundation 不支持的格式（ogg / oga）在算波形时顺带转成 WAV 缓存，再交给 AVPlayer 播放。

use super::*;
use crate::theme::Theme;
use crate::ui::icon;
use std::cell::Cell;
use std::path::{Path, PathBuf};

/// 缓存的波形柱数（渲染时按实际宽度再合并）
const BARS: usize = 600;
const WAVE_H: f32 = 32.;

#[derive(Clone)]
pub(super) struct Wave {
    /// 0..1 归一化峰值，BARS 个
    peaks: Rc<Vec<f32>>,
    duration: f64,
    /// 实际播放的文件（原文件或转码后的 WAV）
    play: String,
}

#[derive(Clone)]
enum WaveState {
    Pending,
    Ready(Wave),
    Failed,
}

thread_local! {
    static WAVES: RefCell<crate::lru::Lru<String, WaveState>> = RefCell::new(crate::lru::Lru::new(128));
    /// 播放中定时刷新进度（不用逐帧重绘整篇文档）
    static TICKING: Cell<bool> = const { Cell::new(false) };
}

fn needs_transcode(path: &str) -> bool {
    matches!(Path::new(path).extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("ogg" | "oga"))
}

/// 取波形（已缓存直接返回，否则后台解码）。
fn wave(path: &str, cx: &mut App) -> WaveState {
    let stamp = std::fs::metadata(path).ok().map(|m| format!("{}-{:?}", m.len(), m.modified().ok())).unwrap_or_default();
    let key = crate::webview::hash_key(&[path, &stamp, "wave1"]);
    if let Some(s) = WAVES.with(|m| m.borrow().get(&key).cloned()) {
        return s;
    }
    let dir = crate::webview::cache_dir();
    let meta = dir.join(format!("wave-{key}.json"));
    let wav = needs_transcode(path).then(|| dir.join(format!("wave-{key}.wav")));
    let play_of = |wav: &Option<PathBuf>| wav.as_ref().map(|w| w.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    if let Some((peaks, duration)) = std::fs::read_to_string(&meta).ok().and_then(|s| serde_json::from_str::<(Vec<f32>, f64)>(&s).ok()) {
        if wav.as_ref().is_none_or(|w| w.exists()) {
            let st = WaveState::Ready(Wave { peaks: Rc::new(peaks), duration, play: play_of(&wav) });
            WAVES.with(|m| m.borrow_mut().insert(key, st.clone()));
            return st;
        }
    }
    WAVES.with(|m| m.borrow_mut().insert(key.clone(), WaveState::Pending));
    let src = PathBuf::from(path);
    let play = play_of(&wav);
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = cx.background_executor().spawn(async move { analyze(&src, wav.as_deref()) }).await;
        let st = match res {
            Some((peaks, duration)) => {
                std::fs::write(&meta, serde_json::to_string(&(&peaks, duration)).unwrap_or_default()).ok();
                WaveState::Ready(Wave { peaks: Rc::new(peaks), duration, play })
            }
            None => WaveState::Failed,
        };
        WAVES.with(|m| m.borrow_mut().insert(key, st));
        cx.update(|cx| cx.refresh_windows()).ok();
    })
    .detach();
    WaveState::Pending
}

/// 解码整个文件：返回 (BARS 个归一化峰值, 时长秒)；wav_out 有值时同时写出 16 位 PCM WAV。
fn analyze(path: &Path, wav_out: Option<&Path>) -> Option<(Vec<f32>, f64)> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate?;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    // 先按 20ms 一格取峰值，最后再合并成 BARS 格
    let bucket = (rate / 50).max(1) as usize;
    let (mut fine, mut cur, mut n, mut frames) = (Vec::new(), 0f32, 0usize, 0u64);
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut wav: Option<(std::io::BufWriter<std::fs::File>, u16)> = None;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        if sbuf.as_ref().is_none_or(|b| b.capacity() < decoded.capacity() * ch) {
            sbuf = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let buf = sbuf.as_mut()?;
        buf.copy_interleaved_ref(decoded);
        if let Some(out) = wav_out {
            if wav.is_none() {
                let mut f = std::io::BufWriter::new(std::fs::File::create(out).ok()?);
                write_wav_header(&mut f, ch as u16, spec.rate, 0).ok()?;
                wav = Some((f, ch as u16));
            }
            if let Some((f, _)) = wav.as_mut() {
                use std::io::Write;
                for s in buf.samples() {
                    f.write_all(&((s.clamp(-1., 1.) * 32767.) as i16).to_le_bytes()).ok()?;
                }
            }
        }
        for fr in buf.samples().chunks(ch) {
            cur = fr.iter().fold(cur, |a, s| a.max(s.abs()));
            n += 1;
            if n == bucket {
                fine.push(cur);
                (cur, n) = (0., 0);
            }
        }
        frames += (buf.samples().len() / ch) as u64;
    }
    if n > 0 {
        fine.push(cur);
    }
    if fine.is_empty() {
        if let Some(out) = wav_out {
            std::fs::remove_file(out).ok();
        }
        return None;
    }
    if let Some((mut f, ch)) = wav {
        use std::io::{Seek, SeekFrom, Write};
        f.flush().ok()?;
        let mut f = f.into_inner().ok()?;
        f.seek(SeekFrom::Start(0)).ok()?;
        let data = frames * ch as u64 * 2;
        write_wav_header(&mut f, ch, rate, data.min(u32::MAX as u64 - 36) as u32).ok()?;
    }
    let peaks = pool(&fine, BARS);
    let max = peaks.iter().cloned().fold(0f32, f32::max).max(1e-4);
    Some((peaks.iter().map(|p| p / max).collect(), frames as f64 / rate as f64))
}

fn write_wav_header(w: &mut impl std::io::Write, ch: u16, rate: u32, data: u32) -> std::io::Result<()> {
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&ch.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * ch as u32 * 2).to_le_bytes())?;
    w.write_all(&(ch * 2).to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())
}

/// 把峰值序列按最大值合并成 n 格。
fn pool(src: &[f32], n: usize) -> Vec<f32> {
    if src.is_empty() || n == 0 {
        return vec![0.; n];
    }
    (0..n)
        .map(|i| {
            let a = i * src.len() / n;
            let b = ((i + 1) * src.len() / n).max(a + 1).min(src.len());
            src[a.min(src.len() - 1)..b].iter().cloned().fold(0f32, f32::max)
        })
        .collect()
}

impl Editor {
    /// 文档里的音频播放条：点击时选中所在块。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_audio(
        &mut self,
        ix: usize,
        id: u64,
        path: &str,
        name: &str,
        width: f32,
        selected: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let press = self.block_press(ix, cx);
        let ent = cx.entity().downgrade();
        let file = path.to_string();
        let stamp: StampFn = Rc::new(move |secs, _w, cx| {
            ent.update(cx, |e, cx| e.insert_timestamp(&file, secs, cx)).ok();
        });
        audio_player(id, path, name, width, selected, Some(press), Some(stamp), t, cx)
    }
}

/// 「插入时间戳」回调：参数为当前播放位置（秒）。
pub(crate) type StampFn = Rc<dyn Fn(f64, &mut Window, &mut App)>;

/// 音频播放条（不依赖编辑器，文档「视频或文件」与电子表格附件预览共用）。
/// width 为整个卡片宽度；`on_press` 在点击播放键 / 波形时先调用（文档里用来选中块）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn audio_player(
    id: u64,
    path: &str,
    name: &str,
    width: f32,
    selected: bool,
    on_press: Option<super::attach::Press>,
    on_stamp: Option<StampFn>,
    t: &Theme,
    cx: &mut App,
) -> AnyElement {
    let exists = Path::new(path).exists();
    let wave = if exists { wave(path, cx) } else { WaveState::Failed };
    // 需要转码的格式要等 WAV 生成后才能播放
    let play_path = match &wave {
        WaveState::Ready(w) => Some(w.play.clone()),
        _ if exists && !needs_transcode(path) => Some(path.to_string()),
        _ => None,
    };
    let started = crate::media::exists(id);
    let (playing, pos, mut total, rate) = match (&play_path, started) {
        (Some(p), true) => crate::media::with(id, p, |pl| {
            pl.tick();
            (pl.playing, pl.position(), pl.duration(), pl.rate)
        })
        .unwrap_or((false, 0., 0., 1.)),
        _ => (false, 0., 0., 1.),
    };
    if total <= 0. {
        if let WaveState::Ready(w) = &wave {
            total = w.duration;
        }
    }
    if playing && !TICKING.with(|c| c.replace(true)) {
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            TICKING.with(|c| c.set(false));
            cx.update(|cx| cx.refresh_windows()).ok();
        })
        .detach();
    }
    let frac = if total > 0. { (pos / total).clamp(0., 1.) as f32 } else { 0. };
    // 波形区宽度 = 卡片宽 - 内边距 - 播放键 - 间距 - 倍速键
    let stamp_w = if on_stamp.is_some() && play_path.is_some() { 28. + 14. } else { 0. };
    let wave_w = (width - 2. - 32. - 40. - 14. - 14. - 36. - stamp_w).max(60.);
    let bars = ((wave_w + 2.) / 4.).floor().max(1.) as usize;
    let peaks = match &wave {
        WaveState::Ready(w) => pool(&w.peaks, bars),
        _ => vec![0.; bars],
    };
    let played = (frac * bars as f32).round() as usize;
    let idle_col = t.text_muted.opacity(0.35);
    let (p1, p2, p3, p4) = (play_path.clone(), play_path.clone(), play_path.clone(), play_path.clone());
    let press2 = on_press.clone();
    let can_play = play_path.is_some();
    let status = match (&wave, exists) {
        (_, false) => Some("文件不存在"),
        (WaveState::Pending, _) if !can_play => Some("正在解析音频…"),
        (WaveState::Failed, _) if !can_play => Some("无法解析该音频"),
        _ => None,
    };
    div()
        .w(px(width))
        .max_w_full()
        .h(px(84.))
        .px(px(16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(if selected { t.accent } else { t.border })
        .bg(t.bg)
        .child(
            div()
                .id(("au-play", id as usize))
                .size(px(40.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if can_play { t.accent } else { t.text_muted.opacity(0.4) })
                .when(can_play, |d| d.cursor_pointer().hover(|s| s.opacity(0.88)))
                .child(div().when(!playing, |d| d.pl(px(2.))).child(icon(if playing { "pause-fill" } else { "play-fill" }, 18., gpui::white())))
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(f) = &on_press {
                        f(ev, window, cx);
                    }
                    if let Some(p) = &p1 {
                        crate::media::pause_others(id);
                        crate::media::with(id, p, |pl| pl.toggle());
                    }
                    window.refresh();
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(14.))
                                .text_color(t.text)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(name.to_string()),
                        )
                        .child(div().flex_none().text_size(px(12.)).text_color(t.text_muted).child(match status {
                            Some(s) => s.to_string(),
                            None => format!("{} / {}", crate::media::fmt_time(pos), crate::media::fmt_time(total)),
                        })),
                )
                .child(
                    // 波形：点击跳转
                    div()
                        .id(("au-wave", id as usize))
                        .relative()
                        .h(px(WAVE_H))
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.))
                        .overflow_hidden()
                        .when(can_play, |d| d.cursor_pointer())
                        .children(peaks.iter().enumerate().map(|(i, p)| {
                            div().w(px(2.)).flex_none().h(px((p * WAVE_H).max(2.))).rounded(px(1.)).bg(if i < played { t.accent } else { idle_col })
                        }))
                        .child(super::attach::bounds_probe(id))
                        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if let Some(f) = &press2 {
                                f(ev, window, cx);
                            }
                            let (Some(p), Some(b)) = (&p2, super::attach::player_bounds(id)) else { return };
                            let f = (f32::from(ev.position.x - b.left()) / f32::from(b.size.width)).clamp(0., 1.);
                            crate::media::pause_others(id);
                            crate::media::with(id, p, |pl| {
                                let d = pl.duration();
                                pl.seek(d * f as f64);
                                if !pl.playing {
                                    pl.play();
                                }
                            });
                            window.refresh();
                        }),
                ),
        )
        .when_some(on_stamp.filter(|_| can_play), |d, f| {
            d.child(
                div()
                    .id(("au-stamp", id as usize))
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .tooltip(|_, cx| cx.new(|_| crate::ui::Tooltip("插入时间戳（⌥⌘T）".into())).into())
                    .child(icon("clock", 15., t.text_secondary))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        // 不选中音频块：光标留在正在记笔记的段落里
                        cx.stop_propagation();
                        let Some(p) = &p4 else { return };
                        let secs = crate::media::with(id, p, |pl| pl.position()).unwrap_or(0.);
                        f(secs, window, cx);
                    }),
            )
        })
        .child(
            div()
                .id(("au-rate", id as usize))
                .w(px(36.))
                .h(px(24.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(t.text_secondary)
                .when(can_play, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                .child(format!("{rate}x"))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let Some(p) = &p3 else { return };
                    crate::media::with(id, p, |pl| {
                        let i = crate::media::RATES.iter().position(|r| (*r - pl.rate).abs() < 0.01).unwrap_or(1);
                        pl.set_rate(crate::media::RATES[(i + 1) % crate::media::RATES.len()]);
                    });
                    window.refresh();
                }),
        )
        .into_any_element()
}

/// 实际交给播放器的文件（ogg 等转码后的 WAV）；波形还没解析完时为 None。
pub(crate) fn play_path(path: &str, cx: &mut App) -> Option<String> {
    if !Path::new(path).exists() {
        return None;
    }
    match wave(path, cx) {
        WaveState::Ready(w) => Some(w.play),
        _ if !needs_transcode(path) => Some(path.to_string()),
        _ => None,
    }
}
