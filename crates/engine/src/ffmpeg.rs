//! FFmpeg integration: probing, hardware-encoder detection, a raw-RGBA frame
//! decoder (stdout pipe) and a raw-RGBA encoder (stdin pipe).
//!
//! FFmpeg is used as an external process (no C linking), so the Windows build
//! needs nothing but `ffmpeg.exe` / `ffprobe.exe` on PATH (or next to the app,
//! or pointed to by `MYLOADER_FFMPEG`).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command(bin: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(bin);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW); // no console flashing from the GUI
    c
}

/// Locate `ffmpeg` (or `ffprobe`): env override, next to the executable, PATH.
pub fn find_tool(name: &str) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    if let Ok(dir) = std::env::var("MYLOADER_FFMPEG") {
        let p = PathBuf::from(&dir);
        let cand = if p.is_dir() { p.join(&exe) } else { p.with_file_name(&exe) };
        if cand.exists() {
            return cand;
        }
    }
    if let Ok(me) = std::env::current_exe() {
        for dir in [me.parent().map(|p| p.to_path_buf()), me.parent().map(|p| p.join("ffmpeg")), me.parent().map(|p| p.join("bin"))]
            .into_iter()
            .flatten()
        {
            let cand = dir.join(&exe);
            if cand.exists() {
                return cand;
            }
        }
    }
    PathBuf::from(exe)
}

pub fn ffmpeg_path() -> PathBuf {
    find_tool("ffmpeg")
}
pub fn ffprobe_path() -> PathBuf {
    find_tool("ffprobe")
}

pub fn ffmpeg_version() -> Result<String> {
    let out = command(&ffmpeg_path()).arg("-version").output().context("ffmpeg not found - install it and add it to PATH")?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("ffmpeg").to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub frames: u64,
    pub codec: String,
    pub has_audio: bool,
    /// Display rotation in degrees (phone videos), already applied to width/height.
    pub rotation: i32,
}

fn parse_rate(s: &str) -> Option<f64> {
    let mut it = s.split('/');
    let n: f64 = it.next()?.trim().parse().ok()?;
    let d: f64 = it.next().map(|d| d.trim().parse().unwrap_or(1.0)).unwrap_or(1.0);
    if d == 0.0 || n == 0.0 {
        None
    } else {
        Some(n / d)
    }
}

pub fn probe(path: &str) -> Result<VideoInfo> {
    if !Path::new(path).exists() {
        bail!("file not found: {path}");
    }
    let out = command(&ffprobe_path())
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format", path])
        .output()
        .context("ffprobe not found - install FFmpeg and add it to PATH")?;
    if !out.status.success() {
        bail!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let streams = j["streams"].as_array().cloned().unwrap_or_default();
    let v = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0)
        .ok_or_else(|| anyhow!("no video stream in {path}"))?;
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let mut width = v["width"].as_u64().unwrap_or(0) as u32;
    let mut height = v["height"].as_u64().unwrap_or(0) as u32;

    // Rotation: side_data_list[].rotation (new) or tags.rotate (old)
    let mut rotation = v["tags"]["rotate"].as_str().and_then(|r| r.parse::<i32>().ok()).unwrap_or(0);
    if let Some(sd) = v["side_data_list"].as_array() {
        for d in sd {
            if let Some(r) = d["rotation"].as_i64() {
                rotation = r as i32;
            }
        }
    }
    if rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }

    let fps = v["avg_frame_rate"].as_str().and_then(parse_rate).or_else(|| v["r_frame_rate"].as_str().and_then(parse_rate)).unwrap_or(30.0);
    let duration = v["duration"]
        .as_str()
        .and_then(|d| d.parse().ok())
        .or_else(|| j["format"]["duration"].as_str().and_then(|d| d.parse().ok()))
        .unwrap_or(0.0);
    let frames = v["nb_frames"].as_str().and_then(|n| n.parse().ok()).unwrap_or((duration * fps).round() as u64);
    Ok(VideoInfo {
        path: path.to_string(),
        width,
        height,
        fps,
        duration,
        frames,
        codec: v["codec_name"].as_str().unwrap_or("?").to_string(),
        has_audio,
        rotation,
    })
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Encoder {
    H264Nvenc,
    HevcNvenc,
    H264Amf,
    HevcAmf,
    H264Qsv,
    HevcQsv,
    Libx264,
}

impl Encoder {
    pub const PREFERENCE: [Encoder; 7] =
        [Encoder::H264Nvenc, Encoder::HevcNvenc, Encoder::H264Amf, Encoder::HevcAmf, Encoder::H264Qsv, Encoder::HevcQsv, Encoder::Libx264];

    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Encoder::H264Nvenc => "h264_nvenc",
            Encoder::HevcNvenc => "hevc_nvenc",
            Encoder::H264Amf => "h264_amf",
            Encoder::HevcAmf => "hevc_amf",
            Encoder::H264Qsv => "h264_qsv",
            Encoder::HevcQsv => "hevc_qsv",
            Encoder::Libx264 => "libx264",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Encoder::H264Nvenc => "NVIDIA NVENC (H.264)",
            Encoder::HevcNvenc => "NVIDIA NVENC (HEVC)",
            Encoder::H264Amf => "AMD AMF (H.264)",
            Encoder::HevcAmf => "AMD AMF (HEVC)",
            Encoder::H264Qsv => "Intel Quick Sync (H.264)",
            Encoder::HevcQsv => "Intel Quick Sync (HEVC)",
            Encoder::Libx264 => "CPU x264 (software)",
        }
    }

    /// Pixel format the encoder accepts natively (QSV/AMF want NV12).
    pub fn pix_fmt(self) -> &'static str {
        match self {
            Encoder::H264Qsv | Encoder::HevcQsv | Encoder::H264Amf | Encoder::HevcAmf => "nv12",
            _ => "yuv420p",
        }
    }

    pub fn is_hardware(self) -> bool {
        self != Encoder::Libx264
    }

    pub fn parse(s: &str) -> Option<Encoder> {
        let s = s.to_ascii_lowercase();
        Some(match s.as_str() {
            "nvenc" | "h264_nvenc" => Encoder::H264Nvenc,
            "hevc_nvenc" | "nvenc_hevc" => Encoder::HevcNvenc,
            "amf" | "h264_amf" => Encoder::H264Amf,
            "hevc_amf" => Encoder::HevcAmf,
            "qsv" | "h264_qsv" => Encoder::H264Qsv,
            "hevc_qsv" => Encoder::HevcQsv,
            "cpu" | "x264" | "libx264" => Encoder::Libx264,
            _ => return None,
        })
    }

    /// Quality-oriented rate control. `quality` 0 (best) .. 51 (worst), ~18-23 is visually lossless.
    pub fn args(self, quality: u32, bitrate_kbps: Option<u32>) -> Vec<String> {
        let q = quality.min(51).to_string();
        let mut a: Vec<String> = match self {
            Encoder::H264Nvenc | Encoder::HevcNvenc => {
                vec!["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &q, "-spatial-aq", "1", "-temporal-aq", "1"]
            }
            Encoder::H264Amf | Encoder::HevcAmf => {
                vec!["-quality", "quality", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-qp_b", &q]
            }
            Encoder::H264Qsv | Encoder::HevcQsv => vec!["-preset", "slower", "-global_quality", &q, "-look_ahead", "1"],
            Encoder::Libx264 => vec!["-preset", "medium", "-crf", &q],
        }
        .into_iter()
        .map(String::from)
        .collect();
        match bitrate_kbps {
            Some(b) if b > 0 => {
                a.extend(["-b:v".into(), format!("{b}k"), "-maxrate".into(), format!("{}k", b * 3 / 2), "-bufsize".into(), format!("{}k", b * 2)]);
            }
            _ if matches!(self, Encoder::H264Nvenc | Encoder::HevcNvenc) => a.extend(["-b:v".into(), "0".into()]),
            _ => {}
        }
        if matches!(self, Encoder::HevcNvenc | Encoder::HevcAmf | Encoder::HevcQsv) {
            a.extend(["-tag:v".into(), "hvc1".into()]); // plays in QuickTime / Windows Photos
        }
        a
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub id: Encoder,
    pub name: String,
    pub label: String,
    pub hardware: bool,
    /// Listed by `ffmpeg -encoders` AND a 0.1 s test encode succeeded on this PC.
    pub available: bool,
}

/// Lists encoders and actually test-encodes with each one, because FFmpeg
/// builds list NVENC/AMF/QSV even when the GPU/driver does not support them.
pub fn detect_encoders() -> Vec<EncoderInfo> {
    let ff = ffmpeg_path();
    let listed = command(&ff)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let handles: Vec<_> = Encoder::PREFERENCE
        .iter()
        .map(|&e| {
            let ff = ff.clone();
            let in_list = listed.contains(e.ffmpeg_name());
            std::thread::spawn(move || {
                let ok = in_list && test_encoder(&ff, e);
                EncoderInfo { id: e, name: e.ffmpeg_name().into(), label: e.label().into(), hardware: e.is_hardware(), available: ok }
            })
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

fn test_encoder(ff: &Path, e: Encoder) -> bool {
    command(ff)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=1280x720:r=30:d=0.2", "-frames:v", "5"])
        .args(["-c:v", e.ffmpeg_name(), "-pix_fmt", e.pix_fmt(), "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn best_encoder(list: &[EncoderInfo]) -> Encoder {
    Encoder::PREFERENCE.into_iter().find(|e| list.iter().any(|i| i.id == *e && i.available)).unwrap_or(Encoder::Libx264)
}

// ---------------------------------------------------------------------------
// Decoder: ffmpeg -> raw RGBA frames on stdout
// ---------------------------------------------------------------------------

pub struct FrameDecoder {
    child: Child,
    stdout: ChildStdout,
    pub width: u32,
    pub height: u32,
    frame_bytes: usize,
}

impl FrameDecoder {
    /// Decode `path` resampled to `fps`, output `width`x`height` RGBA.
    /// Uses hardware decoding when available (`-hwaccel auto`).
    pub fn open(path: &str, fps: f64, width: u32, height: u32) -> Result<Self> {
        let mut child = command(&ffmpeg_path())
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-hwaccel", "auto", "-i", path])
            .args(["-an", "-sn", "-vf", &format!("fps={fps},scale={width}:{height}:flags=bicubic,format=rgba")])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting ffmpeg decoder")?;
        let stdout = child.stdout.take().unwrap();
        Ok(Self { child, stdout, width, height, frame_bytes: (width * height * 4) as usize })
    }

    /// Next frame, or `None` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; self.frame_bytes];
        let mut filled = 0;
        while filled < buf.len() {
            let n = self.stdout.read(&mut buf[filled..])?;
            if n == 0 {
                return Ok(None);
            }
            filled += n;
        }
        Ok(Some(buf))
    }
}

impl Drop for FrameDecoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Encoder: raw RGBA frames on stdin -> MP4
// ---------------------------------------------------------------------------

pub struct EncoderOptions {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub encoder: Encoder,
    pub quality: u32,
    pub bitrate_kbps: Option<u32>,
    /// Mux audio from this file (first audio stream), starting at `audio_offset` seconds of the comp.
    pub audio_from: Option<String>,
    pub audio_offset: f64,
    pub duration: f64,
}

pub struct FrameEncoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_log: std::sync::Arc<std::sync::Mutex<String>>,
}

impl FrameEncoder {
    pub fn args(o: &EncoderOptions) -> Vec<String> {
        let mut a: Vec<String> = vec!["-hide_banner", "-loglevel", "error", "-y"].into_iter().map(String::from).collect();
        a.extend(["-f", "rawvideo", "-pix_fmt", "rgba"].map(String::from));
        a.extend(["-s".into(), format!("{}x{}", o.width, o.height), "-r".into(), format!("{}", o.fps), "-i".into(), "-".into()]);
        if let Some(audio) = &o.audio_from {
            if o.audio_offset > 0.0 {
                a.extend(["-itsoffset".into(), format!("{:.3}", o.audio_offset)]);
            }
            a.extend(["-i".into(), audio.clone()]);
            a.extend(["-map", "0:v:0", "-map", "1:a:0?", "-c:a", "aac", "-b:a", "192k"].map(String::from));
        }
        a.extend(["-c:v".into(), o.encoder.ffmpeg_name().into()]);
        a.extend(o.encoder.args(o.quality, o.bitrate_kbps));
        // BT.709 tagging so players show the colours we rendered.
        a.extend(["-pix_fmt".into(), o.encoder.pix_fmt().into()]);
        a.extend(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"].map(String::from));
        a.extend(["-vf".into(), "scale=out_color_matrix=bt709:out_range=tv".into()]);
        a.extend(["-t".into(), format!("{:.3}", o.duration), "-movflags".into(), "+faststart".into(), o.output.clone()]);
        a
    }

    pub fn start(o: &EncoderOptions) -> Result<Self> {
        let mut child = command(&ffmpeg_path())
            .args(Self::args(o))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting ffmpeg encoder")?;
        let stdin = child.stdin.take();
        let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mut err = child.stderr.take().unwrap();
        let log2 = log.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            *log2.lock().unwrap() = s;
        });
        Ok(Self { child, stdin, stderr_log: log })
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| anyhow!("encoder closed"))?;
        if let Err(e) = stdin.write_all(rgba) {
            let _ = self.child.wait();
            std::thread::sleep(std::time::Duration::from_millis(100));
            bail!("encoder stopped ({e}): {}", self.stderr_log.lock().unwrap().trim());
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        std::thread::sleep(std::time::Duration::from_millis(50));
        if !status.success() {
            bail!("ffmpeg encoder failed: {}", self.stderr_log.lock().unwrap().trim());
        }
        Ok(())
    }

    pub fn abort(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Grab a single RGBA frame at `t` seconds, scaled to `width`x`height`.
pub fn extract_frame(path: &str, t: f64, width: u32, height: u32) -> Result<Vec<u8>> {
    let out = command(&ffmpeg_path())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{t:.3}"), "-i", path, "-frames:v", "1"])
        .args(["-vf", &format!("scale={width}:{height}:flags=bicubic,format=rgba"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .context("running ffmpeg")?;
    let need = (width * height * 4) as usize;
    if out.stdout.len() < need {
        bail!("could not decode a frame at {t:.2}s: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout[..need].to_vec())
}

/// Write RGBA pixels as PNG using ffmpeg (keeps the engine free of image crates).
pub fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let mut child = command(&ffmpeg_path())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{width}x{height}"), "-i", "-", "-frames:v", "1", path])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running ffmpeg")?;
    child.stdin.take().unwrap().write_all(rgba)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("png write failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(parse_rate("30000/1001").map(|r| (r * 100.0).round()), Some(2997.0));
        assert_eq!(parse_rate("60/1"), Some(60.0));
        assert_eq!(parse_rate("0/0"), None);
    }

    #[test]
    fn encoder_args_contain_hw_codec() {
        let o = EncoderOptions {
            output: "out.mp4".into(),
            width: 2160,
            height: 3840,
            fps: 60.0,
            encoder: Encoder::H264Nvenc,
            quality: 19,
            bitrate_kbps: None,
            audio_from: Some("in.mp4".into()),
            audio_offset: 0.0,
            duration: 10.0,
        };
        let a = FrameEncoder::args(&o).join(" ");
        assert!(a.contains("-c:v h264_nvenc"));
        assert!(a.contains("-s 2160x3840 -r 60"));
        assert!(a.contains("-map 1:a:0?"));
    }
}
