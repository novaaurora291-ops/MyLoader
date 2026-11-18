//! `mlcli` - command line front-end for the MyLoader engine.
//!
//! ```text
//! mlcli info
//! mlcli parse  <preset.xml> [--json]
//! mlcli new    <video> -o project.json [--aspect 9:16] [--preset a.xml ...] [--fit cover|contain|stretch]
//! mlcli fill   <project.json> [--aspect 16:9] [--fit cover]
//! mlcli still  <project.json> -o frame.png [--t 1.5] [--res 1080] [--cpu]
//! mlcli export <project.json> -o out.mp4 [--res 1080|1440|2160|native] [--fps 60]
//!              [--encoder auto|nvenc|hevc_nvenc|amf|qsv|cpu] [--quality 19] [--bitrate 40000] [--no-audio] [--cpu]
//! mlcli bench  [--res 2160] [--frames 120] [--cpu]
//! ```

use anyhow::{anyhow, bail, Context, Result};
use myloader_engine::{alight, export, ffmpeg, layout, model::*, render};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

struct Args {
    pos: Vec<String>,
    flags: HashMap<String, Vec<String>>,
}

impl Args {
    fn parse() -> Self {
        let mut pos = vec![];
        let mut flags: HashMap<String, Vec<String>> = HashMap::new();
        let mut it = std::env::args().skip(1).peekable();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--").or_else(|| a.strip_prefix('-').filter(|s| s.len() == 1)) {
                let takes_value = it.peek().map_or(false, |n| !n.starts_with('-') || n.parse::<f64>().is_ok());
                let v = if takes_value { it.next().unwrap() } else { "true".into() };
                flags.entry(name.to_string()).or_default().push(v);
            } else {
                pos.push(a);
            }
        }
        Self { pos, flags }
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.flags.get(k).and_then(|v| v.last()).map(|s| s.as_str())
    }
    fn all(&self, k: &str) -> Vec<String> {
        self.flags.get(k).cloned().unwrap_or_default()
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
    fn out(&self) -> Option<&str> {
        self.get("o").or_else(|| self.get("out")).or_else(|| self.get("output"))
    }
}

fn parse_res(s: Option<&str>) -> Result<Option<u32>> {
    Ok(match s.unwrap_or("1080").to_ascii_lowercase().trim_end_matches('p') {
        "native" | "comp" => None,
        "4k" | "2160" => Some(2160),
        "2k" | "1440" => Some(1440),
        "720" => Some(720),
        "1080" => Some(1080),
        other => Some(other.parse().map_err(|_| anyhow!("bad --res {other}"))?),
    })
}

fn parse_fit(s: Option<&str>) -> Result<layout::FitMode> {
    Ok(match s.unwrap_or("cover") {
        "cover" | "fill" => layout::FitMode::Cover,
        "contain" | "fit" => layout::FitMode::Contain,
        "stretch" => layout::FitMode::Stretch,
        o => bail!("bad --fit {o} (cover|contain|stretch)"),
    })
}

fn load_project(path: &str) -> Result<Project> {
    Project::from_json(&std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let a = Args::parse();
    let cmd = a.pos.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "info" => {
            println!("FFmpeg : {}", ffmpeg::ffmpeg_version().unwrap_or_else(|e| format!("NOT FOUND ({e})")));
            let size = render::RenderSize { width: 64, height: 64, scale: 1.0 };
            match myloader_engine::gpu::GpuRenderer::new(size) {
                Ok(g) => println!("GPU    : {}", g.adapter_name()),
                Err(e) => println!("GPU    : unavailable ({e:#}) - CPU renderer will be used"),
            }
            println!("Encoders (tested):");
            for e in ffmpeg::detect_encoders() {
                println!("  {:<12} {:<28} {}", e.name, e.label, if e.available { "OK" } else { "-" });
            }
        }
        "parse" => {
            let path = a.pos.get(1).ok_or_else(|| anyhow!("usage: mlcli parse <preset.xml>"))?;
            let p = alight::parse_preset_file(std::path::Path::new(path))?;
            if a.has("json") {
                println!("{}", serde_json::to_string_pretty(&p)?);
                return Ok(());
            }
            println!("'{}'  {}x{} @ {} fps, {:.2}s", p.title, p.width, p.height, p.fps, p.duration);
            println!(
                "elements {}  adjustments {}  rectangles {}  skipped {}  effects mapped {}/{}",
                p.stats.elements, p.stats.adjustments, p.stats.rectangles, p.stats.skipped, p.stats.effects_mapped, p.stats.effects_total
            );
            for (i, l) in p.layers.iter().enumerate() {
                let kind = match &l.kind {
                    LayerKind::Adjustment => "adjustment".to_string(),
                    LayerKind::Rectangle { color, .. } => format!("rect rgba({:.2},{:.2},{:.2},{:.2})", color[0], color[1], color[2], color[3]),
                    LayerKind::Video { .. } => "video".into(),
                };
                println!(
                    "  [{i}] {:<16} {:<34} blend={:?} opacity={:.2} {:.0}x{:.0}@({:.0},{:.0})",
                    l.name, kind, l.blend, l.opacity, l.transform.width, l.transform.height, l.transform.x, l.transform.y
                );
                if !matches!(l.kind, LayerKind::Rectangle { .. }) {
                    let c = &l.cc;
                    println!(
                        "        bri {:+.2} con {:+.2} exp {:+.2} sat {:+.2} hi {:+.2} sh {:+.2} blur {:.1}px vig {:.2} fade {:.2}",
                        c.brightness, c.contrast, c.exposure, c.saturation, c.highlights, c.shadows, c.blur, c.vignette, c.bottom_fade
                    );
                }
            }
            for w in &p.warnings {
                println!("  ! {w}");
            }
        }
        "new" => {
            let video = a.pos.get(1).ok_or_else(|| anyhow!("usage: mlcli new <video> -o project.json"))?;
            let out = a.out().unwrap_or("project.json");
            let info = ffmpeg::probe(video)?;
            let aspect = match a.get("aspect") {
                Some(s) => layout::AspectRatio::parse(s).ok_or_else(|| anyhow!("bad --aspect {s}"))?,
                None if info.height > info.width => layout::AspectRatio::Portrait9x16,
                None if info.height == info.width => layout::AspectRatio::Square1x1,
                None => layout::AspectRatio::Landscape16x9,
            };
            let (w, h) = aspect.size();
            let comp = Composition { width: w, height: h, fps: info.fps.round().max(1.0), ..Default::default() };
            let mut p = Project::new(comp.clone());
            p.layers.push(Layer::new_video("video-1", &info, &comp));
            for preset in a.all("preset") {
                let parsed = alight::parse_preset_file(std::path::Path::new(&preset))?;
                let ids = alight::import_into(&mut p, &parsed);
                println!("imported {} layers from {preset} ({} warnings)", ids.len(), parsed.warnings.len());
            }
            layout::fill_all_layers(&mut p, parse_fit(a.get("fit"))?);
            std::fs::write(out, p.to_json())?;
            println!("wrote {out}: {}x{} {:.2} fps, {} layers, {:.2}s", w, h, comp.fps, p.layers.len(), p.duration());
        }
        "fill" => {
            let path = a.pos.get(1).ok_or_else(|| anyhow!("usage: mlcli fill <project.json>"))?;
            let mut p = load_project(path)?;
            if let Some(s) = a.get("aspect") {
                layout::set_aspect(&mut p, layout::AspectRatio::parse(s).ok_or_else(|| anyhow!("bad --aspect {s}"))?);
            }
            let n = layout::fill_all_layers(&mut p, parse_fit(a.get("fit"))?);
            std::fs::write(a.out().unwrap_or(path), p.to_json())?;
            println!("fitted {n} layers to {}x{}", p.composition.width, p.composition.height);
        }
        "still" => {
            let path = a.pos.get(1).ok_or_else(|| anyhow!("usage: mlcli still <project.json> -o frame.png"))?;
            let p = load_project(path)?;
            let t: f64 = a.get("t").unwrap_or("0").parse()?;
            let out = a.out().unwrap_or("frame.png");
            let r = export::render_still(&p, t, parse_res(a.get("res"))?, a.has("cpu"), out)?;
            println!("wrote {out} ({r})");
        }
        "export" => {
            let path = a.pos.get(1).ok_or_else(|| anyhow!("usage: mlcli export <project.json> -o out.mp4"))?;
            let p = load_project(path)?;
            let encoder = match a.get("encoder").unwrap_or("auto") {
                "auto" => None,
                s => Some(ffmpeg::Encoder::parse(s).ok_or_else(|| anyhow!("unknown encoder {s}"))?),
            };
            let settings = export::ExportSettings {
                output: a.out().unwrap_or("export.mp4").to_string(),
                short_edge: parse_res(a.get("res"))?,
                fps: a.get("fps").map(|s| s.parse()).transpose()?,
                encoder,
                quality: a.get("quality").map(|s| s.parse()).transpose()?.unwrap_or(19),
                bitrate_kbps: a.get("bitrate").map(|s| s.parse()).transpose()?,
                include_audio: !a.has("no-audio"),
                force_cpu_render: a.has("cpu"),
            };
            let cancel = AtomicBool::new(false);
            let rep = export::run_export(&p, &settings, &cancel, |pr| {
                eprint!(
                    "\r[{}/{}] {:5.1} fps  eta {:5.1}s  {} | {}   ",
                    pr.frame, pr.total_frames, pr.render_fps, pr.eta, pr.renderer, pr.encoder
                );
            })?;
            eprintln!();
            println!(
                "done: {} {}x{} @ {} fps, {} frames in {:.1}s ({:.1} fps) [{} / {}]",
                rep.output, rep.width, rep.height, rep.fps, rep.frames, rep.seconds, rep.average_fps, rep.renderer, rep.encoder
            );
        }
        "bench" => {
            let res = parse_res(a.get("res").or(Some("2160")))?;
            let frames: u32 = a.get("frames").unwrap_or("120").parse()?;
            let comp = Composition { width: 1080, height: 1920, ..Default::default() };
            let mut p = Project::new(comp.clone());
            let info = ffmpeg::VideoInfo {
                path: "synthetic".into(),
                width: 1920,
                height: 1080,
                fps: 60.0,
                duration: 100.0,
                frames: 6000,
                codec: "raw".into(),
                has_audio: false,
                rotation: 0,
            };
            p.layers.push(Layer::new_video("v", &info, &comp));
            let preset = alight::parse_preset_str(include_str!("../../../../presets/hdr-cc-lyrics.xml"))?;
            alight::import_into(&mut p, &preset);
            let size = render::RenderSize::for_comp(&comp, res);
            let mut r = render::create_renderer(size, a.has("cpu"));
            let vf = render::VideoFrame { width: 1920, height: 1080, data: (0..1920 * 1080 * 4).map(|i| (i % 251) as u8).collect() };
            let mut map = HashMap::new();
            map.insert("v".to_string(), vf);
            r.render(&p, 0.0, &map)?; // warm-up
            let t0 = std::time::Instant::now();
            for i in 0..frames {
                r.render(&p, i as f64 / 60.0, &map)?;
            }
            let dt = t0.elapsed().as_secs_f64();
            println!(
                "{}: {} frames {}x{} with {} layers in {:.2}s = {:.1} fps (incl. upload + readback)",
                r.name(),
                frames,
                size.width,
                size.height,
                p.layers.len(),
                dt,
                frames as f64 / dt
            );
        }
        _ => {
            println!("{}", include_str!("usage.txt"));
        }
    }
    Ok(())
}
