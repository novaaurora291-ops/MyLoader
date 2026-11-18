//! Project data model. This is the single source of truth shared by the
//! desktop UI (serialised as camelCase JSON) and the export engine.
//!
//! Layer order: `layers[0]` is the BOTTOM of the stack (normally the user's
//! raw video). Higher indices are composited on top.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default = "default_version")]
    pub version: u32,
    pub composition: Composition,
    #[serde(default)]
    pub layers: Vec<Layer>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Composition {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: f64,
    /// Seconds. `0` = derive from the layers (longest layer end).
    #[serde(default)]
    pub duration: f64,
    /// Straight RGBA, 0..1
    #[serde(default = "default_bg")]
    pub background: [f32; 4],
}

fn default_fps() -> f64 {
    30.0
}
fn default_bg() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

impl Default for Composition {
    fn default() -> Self {
        Self { width: 1080, height: 1920, fps: 30.0, duration: 0.0, background: default_bg() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
    SoftLight,
    HardLight,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    Difference,
}

impl BlendMode {
    pub const ALL: [BlendMode; 12] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Add,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::Difference,
    ];

    /// Numeric id used by the shaders (WGSL + GLSL). Keep in sync with
    /// `shaders/composite.wgsl` and `src/engine/shaders.ts`.
    pub fn shader_id(self) -> u32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }

    /// Lenient parser used by the Alight Motion importer.
    pub fn from_loose(name: &str) -> Option<BlendMode> {
        let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        Some(match n.as_str() {
            "" | "normal" | "none" | "srcover" | "sourceover" => BlendMode::Normal,
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "add" | "plus" | "lineardodge" | "lighter" | "additive" => BlendMode::Add,
            "softlight" => BlendMode::SoftLight,
            "hardlight" => BlendMode::HardLight,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "colordodge" | "dodge" => BlendMode::ColorDodge,
            "colorburn" | "burn" => BlendMode::ColorBurn,
            "difference" => BlendMode::Difference,
            _ => return None,
        })
    }
}

/// Centre-based transform in composition pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Degrees, clockwise.
    #[serde(default)]
    pub rotation: f32,
}

impl Transform {
    pub fn full(comp: &Composition) -> Self {
        Self {
            x: comp.width as f32 / 2.0,
            y: comp.height as f32 / 2.0,
            width: comp.width as f32,
            height: comp.height as f32,
            rotation: 0.0,
        }
    }
}

/// Colour-correction parameters of an adjustment layer. All "amount"
/// parameters are roughly -1..1 (0 = no change).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CcParams {
    pub brightness: f32,
    pub contrast: f32,
    /// Photographic stops, -4..4
    pub exposure: f32,
    pub saturation: f32,
    /// HDR tone: negative pulls highlights down (recovery), positive boosts.
    pub highlights: f32,
    /// HDR tone: positive lifts shadows.
    pub shadows: f32,
    /// Gaussian blur radius in composition pixels (0..100)
    pub blur: f32,
    /// Vignette strength 0..1
    pub vignette: f32,
    /// Vignette softness 0..1
    pub vignette_softness: f32,
    /// Bottom black fade strength 0..1 (lyrics readability)
    pub bottom_fade: f32,
    /// Fraction of the layer height covered by the fade, 0..1
    pub bottom_fade_height: f32,
}

impl Default for CcParams {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            exposure: 0.0,
            saturation: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            blur: 0.0,
            vignette: 0.0,
            vignette_softness: 0.5,
            bottom_fade: 0.0,
            bottom_fade_height: 0.35,
        }
    }
}

impl CcParams {
    /// True when the layer would not change the image (softness/height
    /// only matter when vignette/fade are on).
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0
            && self.contrast == 0.0
            && self.exposure == 0.0
            && self.saturation == 0.0
            && self.highlights == 0.0
            && self.shadows == 0.0
            && self.blur <= 0.0
            && self.vignette == 0.0
            && self.bottom_fade == 0.0
    }

    pub fn clamped(mut self) -> Self {
        self.brightness = self.brightness.clamp(-1.0, 1.0);
        self.contrast = self.contrast.clamp(-1.0, 3.0);
        self.exposure = self.exposure.clamp(-4.0, 4.0);
        self.saturation = self.saturation.clamp(-1.0, 1.0);
        self.highlights = self.highlights.clamp(-1.0, 1.0);
        self.shadows = self.shadows.clamp(-1.0, 1.0);
        self.blur = self.blur.clamp(0.0, 200.0);
        self.vignette = self.vignette.clamp(0.0, 1.0);
        self.vignette_softness = self.vignette_softness.clamp(0.0, 1.0);
        self.bottom_fade = self.bottom_fade.clamp(0.0, 1.0);
        self.bottom_fade_height = self.bottom_fade_height.clamp(0.0, 1.0);
        self
    }
}

/// An effect from an imported preset that we could not map onto
/// [`CcParams`]. Kept so the UI can show what the preset contained.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RawEffect {
    pub id: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LayerKind {
    Video {
        path: String,
        #[serde(default)]
        src_width: u32,
        #[serde(default)]
        src_height: u32,
        /// Seconds
        #[serde(default)]
        duration: f64,
        #[serde(default)]
        fps: f64,
        #[serde(default = "default_true")]
        has_audio: bool,
    },
    Adjustment,
    Rectangle {
        /// Straight RGBA 0..1
        color: [f32; 4],
        #[serde(default)]
        corner_radius: f32,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub kind: LayerKind,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: BlendMode,
    pub transform: Transform,
    /// Colour correction. Used by adjustment layers; ignored for others.
    #[serde(default)]
    pub cc: CcParams,
    /// Start time on the composition timeline, seconds.
    #[serde(default)]
    pub start: f64,
    /// End time, seconds. `None` = until the end of the composition
    /// (for video: until the clip ends).
    #[serde(default)]
    pub end: Option<f64>,
    #[serde(default)]
    pub raw_effects: Vec<RawEffect>,
}

fn default_opacity() -> f32 {
    1.0
}

impl Layer {
    pub fn is_video(&self) -> bool {
        matches!(self.kind, LayerKind::Video { .. })
    }

    /// Effective end time on the composition timeline.
    pub fn end_time(&self) -> Option<f64> {
        match (&self.kind, self.end) {
            (_, Some(e)) => Some(e),
            (LayerKind::Video { duration, .. }, None) if *duration > 0.0 => Some(self.start + duration),
            _ => None,
        }
    }

    pub fn active_at(&self, t: f64) -> bool {
        self.visible && self.opacity > 0.0 && t >= self.start && self.end_time().map_or(true, |e| t < e)
    }

    pub fn new_adjustment(id: impl Into<String>, name: impl Into<String>, comp: &Composition) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: LayerKind::Adjustment,
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: Transform::full(comp),
            cc: CcParams::default(),
            start: 0.0,
            end: None,
            raw_effects: vec![],
        }
    }

    pub fn new_video(id: impl Into<String>, info: &crate::ffmpeg::VideoInfo, comp: &Composition) -> Self {
        let mut l = Self {
            id: id.into(),
            name: std::path::Path::new(&info.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Video".into()),
            kind: LayerKind::Video {
                path: info.path.clone(),
                src_width: info.width,
                src_height: info.height,
                duration: info.duration,
                fps: info.fps,
                has_audio: info.has_audio,
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: Transform::full(comp),
            cc: CcParams::default(),
            start: 0.0,
            end: None,
            raw_effects: vec![],
        };
        crate::layout::fit_layer(&mut l, comp, crate::layout::FitMode::Cover);
        l
    }
}

impl Project {
    pub fn new(comp: Composition) -> Self {
        Self { version: 1, composition: comp, layers: vec![] }
    }

    /// Composition duration in seconds (explicit or derived from layers).
    pub fn duration(&self) -> f64 {
        if self.composition.duration > 0.0 {
            return self.composition.duration;
        }
        let d = self.layers.iter().filter_map(|l| l.end_time()).fold(0.0_f64, f64::max);
        if d > 0.0 {
            d
        } else {
            5.0
        }
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serialises")
    }
}
