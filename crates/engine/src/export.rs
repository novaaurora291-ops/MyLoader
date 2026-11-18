//! Export pipeline: [decode threads] -> GPU render (main thread) -> [encode thread]
//!
//! Each stage runs in parallel connected by bounded channels, so the GPU,
//! the hardware decoder and NVENC/AMF/QSV are all busy at the same time.

use crate::ffmpeg::{self, Encoder, EncoderOptions, FrameDecoder, FrameEncoder};
use crate::model::*;
use crate::render::{create_renderer, RenderSize, VideoFrame};
use anyhow::{anyhow, bail, Result};
use crossbeam_channel::{bounded, Receiver};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    pub output: String,
    /// Short edge of the output: 1080 (1080p), 1440, 2160 (4K). `None` = composition size.
    #[serde(default)]
    pub short_edge: Option<u32>,
    /// Output frame rate. `None` = composition fps.
    #[serde(default)]
    pub fps: Option<f64>,
    /// `None` = auto (fastest working hardware encoder).
    #[serde(default)]
    pub encoder: Option<Encoder>,
    /// 0 (best) .. 51. 18-20 is visually lossless.
    #[serde(default = "default_quality")]
    pub quality: u32,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    #[serde(default = "default_true")]
    pub include_audio: bool,
    /// Skip wgpu and render on the CPU.
    #[serde(default)]
    pub force_cpu_render: bool,
}

fn default_quality() -> u32 {
    19
}
fn default_true() -> bool {
    true
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            output: "export.mp4".into(),
            short_edge: Some(1080),
            fps: None,
            encoder: None,
            quality: 19,
            bitrate_kbps: None,
            include_audio: true,
            force_cpu_render: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub frame: u64,
    pub total_frames: u64,
    pub render_fps: f64,
    pub elapsed: f64,
    pub eta: f64,
    pub renderer: String,
    pub encoder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: u64,
    pub seconds: f64,
    pub average_fps: f64,
    pub renderer: String,
    pub encoder: String,
}

struct DecodeTrack {
    layer_id: String,
    start: f64,
    rx: Receiver<Vec<u8>>,
    w: u32,
    h: u32,
    consumed: i64,
    eof: bool,
    current: Option<VideoFrame>,
}

/// Decode size: the size the layer is displayed at in the output, but never
/// larger than the source (no point upscaling before the GPU samples it).
fn decode_size(layer: &Layer, size: RenderSize) -> (u32, u32) {
    let (sw, sh) = match &layer.kind {
        LayerKind::Video { src_width, src_height, .. } => (*src_width.max(&2), *src_height.max(&2)),
        _ => (2, 2),
    };
    let dw = (layer.transform.width * size.scale).abs().max(2.0);
    let dh = (layer.transform.height * size.scale).abs().max(2.0);
    let f = (dw / sw as f32).max(dh / sh as f32).min(1.0);
    let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
    (even(sw as f32 * f), even(sh as f32 * f))
}

pub fn run_export(
    project: &Project,
    settings: &ExportSettings,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&ExportProgress),
) -> Result<ExportReport> {
    let comp = &project.composition;
    let size = RenderSize::for_comp(comp, settings.short_edge);
    let fps = settings.fps.unwrap_or(comp.fps).clamp(1.0, 240.0);
    let duration = project.duration();
    let total = (duration * fps).ceil().max(1.0) as u64;

    let encoder = match settings.encoder {
        Some(e) => e,
        None => ffmpeg::best_encoder(&ffmpeg::detect_encoders()),
    };
    let mut renderer = create_renderer(size, settings.force_cpu_render);
    let renderer_name = renderer.name();

    // ---- decoders ------------------------------------------------------------
    let mut tracks = vec![];
    for layer in project.layers.iter().filter(|l| l.visible && l.opacity > 0.0) {
        if let LayerKind::Video { path, .. } = &layer.kind {
            let (w, h) = decode_size(layer, size);
            let mut dec = FrameDecoder::open(path, fps, w, h)?;
            let (tx, rx) = bounded::<Vec<u8>>(6);
            std::thread::spawn(move || {
                while let Ok(Some(f)) = dec.next_frame() {
                    if tx.send(f).is_err() {
                        break;
                    }
                }
            });
            tracks.push(DecodeTrack { layer_id: layer.id.clone(), start: layer.start, rx, w, h, consumed: 0, eof: false, current: None });
        }
    }

    // ---- encoder ---------------------------------------------------------------
    let audio_layer = if settings.include_audio {
        project.layers.iter().find(|l| l.visible && matches!(l.kind, LayerKind::Video { has_audio: true, .. }))
    } else {
        None
    };
    let opts = EncoderOptions {
        output: settings.output.clone(),
        width: size.width,
        height: size.height,
        fps,
        encoder,
        quality: settings.quality,
        bitrate_kbps: settings.bitrate_kbps,
        audio_from: audio_layer.and_then(|l| match &l.kind {
            LayerKind::Video { path, .. } => Some(path.clone()),
            _ => None,
        }),
        audio_offset: audio_layer.map(|l| l.start).unwrap_or(0.0),
        duration: total as f64 / fps,
    };
    if let Some(dir) = std::path::Path::new(&settings.output).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut enc = FrameEncoder::start(&opts)?;
    let (etx, erx) = bounded::<Vec<u8>>(4);
    let writer = std::thread::spawn(move || -> Result<()> {
        for frame in erx {
            enc.write_frame(&frame)?;
        }
        enc.finish()
    });

    // ---- render loop -------------------------------------------------------------
    let started = Instant::now();
    let mut last_report = Instant::now();
    let mut frames: HashMap<String, VideoFrame> = HashMap::new();
    let mut result: Result<()> = Ok(());
    for i in 0..total {
        if cancel.load(Ordering::Relaxed) {
            result = Err(anyhow!("export cancelled"));
            break;
        }
        let t = i as f64 / fps;
        for tr in tracks.iter_mut() {
            let local = t - tr.start;
            frames.remove(&tr.layer_id);
            if local < 0.0 {
                continue;
            }
            let want = (local * fps + 1e-6).floor() as i64;
            while tr.consumed <= want && !tr.eof {
                match tr.rx.recv() {
                    Ok(data) => {
                        tr.current = Some(VideoFrame { width: tr.w, height: tr.h, data });
                        tr.consumed += 1;
                    }
                    Err(_) => {
                        tr.eof = true;
                        tr.current = None;
                    }
                }
            }
            if let Some(f) = &tr.current {
                frames.insert(tr.layer_id.clone(), f.clone());
            }
        }
        let rgba = match renderer.render(project, t, &frames) {
            Ok(p) => p,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        if etx.send(rgba).is_err() {
            result = Err(anyhow!("encoder exited early"));
            break;
        }
        if last_report.elapsed().as_millis() >= 200 || i + 1 == total {
            last_report = Instant::now();
            let elapsed = started.elapsed().as_secs_f64();
            let rfps = (i + 1) as f64 / elapsed.max(1e-6);
            on_progress(&ExportProgress {
                frame: i + 1,
                total_frames: total,
                render_fps: rfps,
                elapsed,
                eta: (total - i - 1) as f64 / rfps.max(1e-6),
                renderer: renderer_name.clone(),
                encoder: encoder.label().into(),
            });
        }
    }
    drop(etx);
    drop(tracks); // stops decoder threads
    let write_res = writer.join().map_err(|_| anyhow!("encoder thread panicked"))?;
    if let Err(e) = result {
        let _ = std::fs::remove_file(&settings.output);
        return Err(e);
    }
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&settings.output);
        bail!("{e:#}");
    }
    let seconds = started.elapsed().as_secs_f64();
    Ok(ExportReport {
        output: settings.output.clone(),
        width: size.width,
        height: size.height,
        fps,
        frames: total,
        seconds,
        average_fps: total as f64 / seconds.max(1e-6),
        renderer: renderer_name,
        encoder: encoder.label().into(),
    })
}

/// Render a single frame to a PNG (thumbnail / regression testing).
pub fn render_still(project: &Project, t: f64, short_edge: Option<u32>, force_cpu: bool, out_png: &str) -> Result<String> {
    let size = RenderSize::for_comp(&project.composition, short_edge);
    let mut frames = HashMap::new();
    for layer in project.layers.iter().filter(|l| l.active_at(t)) {
        if let LayerKind::Video { path, .. } = &layer.kind {
            let (w, h) = decode_size(layer, size);
            let local = (t - layer.start).max(0.0);
            let data = ffmpeg::extract_frame(path, local, w, h)?;
            frames.insert(layer.id.clone(), VideoFrame { width: w, height: h, data });
        }
    }
    let mut r = create_renderer(size, force_cpu);
    let px = r.render(project, t, &frames)?;
    ffmpeg::write_png(out_png, size.width, size.height, &px)?;
    Ok(r.name())
}
