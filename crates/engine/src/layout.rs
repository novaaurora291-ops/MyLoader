//! Canvas aspect ratio presets and the one-click "Fill All Layers to
//! Composition" logic. Mirrored 1:1 in `src/lib/layout.ts` for the UI.

use crate::model::{Composition, Layer, LayerKind, Project, Transform};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AspectRatio {
    #[serde(rename = "9:16")]
    Portrait9x16,
    #[serde(rename = "16:9")]
    Landscape16x9,
    #[serde(rename = "1:1")]
    Square1x1,
    #[serde(rename = "4:5")]
    Portrait4x5,
}

impl AspectRatio {
    /// Composition size at 1080p base resolution.
    pub fn size(self) -> (u32, u32) {
        match self {
            AspectRatio::Portrait9x16 => (1080, 1920),
            AspectRatio::Landscape16x9 => (1920, 1080),
            AspectRatio::Square1x1 => (1080, 1080),
            AspectRatio::Portrait4x5 => (1080, 1350),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "9:16" | "9x16" | "portrait" | "tiktok" | "reels" => Some(Self::Portrait9x16),
            "16:9" | "16x9" | "landscape" | "youtube" => Some(Self::Landscape16x9),
            "1:1" | "1x1" | "square" => Some(Self::Square1x1),
            "4:5" | "4x5" => Some(Self::Portrait4x5),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    /// Scale uniformly until the canvas is fully covered (crops overflow).
    #[default]
    Cover,
    /// Scale uniformly until the whole layer is visible (letterbox).
    Contain,
    /// Non-uniform stretch to exactly the canvas size.
    Stretch,
}

/// Change the canvas aspect ratio. Layer centres move proportionally so the
/// layout stays recognisable; press "Fill" afterwards to re-fit sizes.
pub fn set_aspect(project: &mut Project, aspect: AspectRatio) {
    let (w, h) = aspect.size();
    resize_composition(project, w, h);
}

pub fn resize_composition(project: &mut Project, w: u32, h: u32) {
    let sx = w as f32 / project.composition.width.max(1) as f32;
    let sy = h as f32 / project.composition.height.max(1) as f32;
    for l in &mut project.layers {
        l.transform.x *= sx;
        l.transform.y *= sy;
    }
    project.composition.width = w;
    project.composition.height = h;
}

/// Fit a single layer to the composition.
/// * video: uniform scale using `video_mode` (Cover by default)
/// * adjustment / rectangle: stretch to exactly the canvas
pub fn fit_layer(layer: &mut Layer, comp: &Composition, video_mode: FitMode) {
    let cw = comp.width as f32;
    let ch = comp.height as f32;
    let (w, h) = match &layer.kind {
        LayerKind::Video { src_width, src_height, .. } if *src_width > 0 && *src_height > 0 => {
            let (sw, sh) = (*src_width as f32, *src_height as f32);
            match video_mode {
                FitMode::Stretch => (cw, ch),
                FitMode::Cover => {
                    let s = (cw / sw).max(ch / sh);
                    (sw * s, sh * s)
                }
                FitMode::Contain => {
                    let s = (cw / sw).min(ch / sh);
                    (sw * s, sh * s)
                }
            }
        }
        _ => (cw, ch),
    };
    layer.transform = Transform { x: cw / 2.0, y: ch / 2.0, width: w, height: h, rotation: 0.0 };
}

/// One click: fit the video and every adjustment / rectangle layer to the
/// canvas. Locked layers are left untouched. Returns how many were changed.
pub fn fill_all_layers(project: &mut Project, video_mode: FitMode) -> usize {
    let comp = project.composition.clone();
    let mut n = 0;
    for l in project.layers.iter_mut().filter(|l| !l.locked) {
        fit_layer(l, &comp, video_mode);
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn video(w: u32, h: u32) -> Layer {
        Layer {
            id: "v".into(),
            name: "v".into(),
            kind: LayerKind::Video { path: "a.mp4".into(), src_width: w, src_height: h, duration: 10.0, fps: 30.0, has_audio: true },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: Transform { x: 0.0, y: 0.0, width: w as f32, height: h as f32, rotation: 0.0 },
            cc: CcParams::default(),
            start: 0.0,
            end: None,
            raw_effects: vec![],
        }
    }

    #[test]
    fn cover_landscape_video_into_portrait() {
        let comp = Composition { width: 1080, height: 1920, ..Default::default() };
        let mut l = video(1920, 1080);
        fit_layer(&mut l, &comp, FitMode::Cover);
        assert!((l.transform.height - 1920.0).abs() < 0.01);
        assert!((l.transform.width - 3413.333).abs() < 0.01);
        assert_eq!((l.transform.x, l.transform.y), (540.0, 960.0));
    }

    #[test]
    fn contain_landscape_video_into_portrait() {
        let comp = Composition { width: 1080, height: 1920, ..Default::default() };
        let mut l = video(1920, 1080);
        fit_layer(&mut l, &comp, FitMode::Contain);
        assert!((l.transform.width - 1080.0).abs() < 0.01);
        assert!((l.transform.height - 607.5).abs() < 0.01);
    }

    #[test]
    fn fill_all_skips_locked_and_stretches_adjustments() {
        let mut p = Project::new(Composition::default());
        p.layers.push(video(1920, 1080));
        let mut adj = Layer::new_adjustment("a", "CC", &p.composition);
        adj.transform = Transform { x: 10.0, y: 10.0, width: 5.0, height: 5.0, rotation: 30.0 };
        p.layers.push(adj.clone());
        let mut locked = adj.clone();
        locked.id = "locked".into();
        locked.locked = true;
        p.layers.push(locked);

        set_aspect(&mut p, AspectRatio::Square1x1);
        let n = fill_all_layers(&mut p, FitMode::Cover);
        assert_eq!(n, 2);
        assert_eq!(p.composition.width, 1080);
        assert_eq!(p.composition.height, 1080);
        let a = &p.layers[1].transform;
        assert_eq!((a.x, a.y, a.width, a.height, a.rotation), (540.0, 540.0, 1080.0, 1080.0, 0.0));
        assert_eq!(p.layers[2].transform.width, 5.0, "locked layer must not move");
    }
}
