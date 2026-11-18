//! Backend-independent frame planning: turns a [`Project`] at time `t` into an
//! ordered list of passes with their shader parameters. Both the wgpu
//! renderer and the CPU reference renderer execute the same plan.

use crate::model::*;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// Uniform block. Layout must match `Params` in `gpu/composite.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct Params {
    pub comp: [f32; 4],
    pub xform: [f32; 4],
    pub xform2: [f32; 4],
    pub color: [f32; 4],
    pub cc1: [f32; 4],
    pub cc2: [f32; 4],
    pub cc3: [f32; 4],
    pub blur_dir: [f32; 4],
}

pub const SOURCE_TEXTURE: f32 = 0.0;
pub const SOURCE_SOLID: f32 = 1.0;

/// One RGBA8 video frame (tightly packed rows).
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSize {
    pub width: u32,
    pub height: u32,
    /// output pixels per composition pixel
    pub scale: f32,
}

impl RenderSize {
    /// Output size whose SHORT edge is `short_edge` (1080 = 1080p, 2160 = 4K).
    /// `None` = native composition size. Always even (required by yuv420p).
    pub fn for_comp(comp: &Composition, short_edge: Option<u32>) -> Self {
        let (cw, ch) = (comp.width.max(2) as f32, comp.height.max(2) as f32);
        let scale = short_edge.map(|s| s as f32 / cw.min(ch)).unwrap_or(1.0);
        let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
        Self { width: even(cw * scale), height: even(ch * scale), scale }
    }
}

#[derive(Clone, Debug)]
pub enum Pass {
    /// Composite a decoded video frame (looked up by layer id).
    Video { layer_id: String, params: Params },
    /// Composite a solid (rounded) rectangle.
    Solid { params: Params },
    /// Adjustment layer. `blur` passes run first when radius > 0.
    Adjust { params: Params, blur_h: Option<Params>, blur_v: Option<Params> },
}

fn base(layer: &Layer, size: RenderSize, opacity: f32) -> Params {
    let s = size.scale;
    let t = &layer.transform;
    Params {
        comp: [size.width as f32, size.height as f32, 0.0, layer.blend.shader_id() as f32],
        xform: [t.x * s, t.y * s, t.width * s, t.height * s],
        xform2: [t.rotation.to_radians(), opacity, 0.0, SOURCE_TEXTURE],
        ..Default::default()
    }
}

/// Build the pass list for time `t` (seconds). Hidden layers, layers outside
/// their time range and video layers without a decoded frame are skipped.
pub fn plan_frame(project: &Project, t: f64, size: RenderSize, frames: &HashMap<String, VideoFrame>) -> Vec<Pass> {
    let mut passes = vec![];
    for layer in project.layers.iter().filter(|l| l.active_at(t)) {
        let opacity = layer.opacity.clamp(0.0, 1.0);
        match &layer.kind {
            LayerKind::Video { .. } => {
                if frames.contains_key(&layer.id) {
                    passes.push(Pass::Video { layer_id: layer.id.clone(), params: base(layer, size, opacity) });
                }
            }
            LayerKind::Rectangle { color, corner_radius } => {
                let mut p = base(layer, size, opacity);
                p.xform2[2] = corner_radius * size.scale;
                p.xform2[3] = SOURCE_SOLID;
                p.color = *color;
                passes.push(Pass::Solid { params: p });
            }
            LayerKind::Adjustment => {
                let cc = layer.cc.clamped();
                if cc.is_identity() && layer.blend == BlendMode::Normal {
                    continue; // no-op layer, save GPU time
                }
                let mut p = base(layer, size, opacity);
                p.cc1 = [cc.brightness, cc.contrast, cc.exposure, cc.saturation];
                p.cc2 = [cc.highlights, cc.shadows, cc.vignette, cc.vignette_softness];
                let radius = cc.blur * size.scale;
                p.cc3 = [cc.bottom_fade, cc.bottom_fade_height, radius, 0.0];
                let (bh, bv) = if radius >= 0.5 {
                    let mut h = p;
                    h.blur_dir = [1.0, 0.0, 0.0, 0.0];
                    let mut v = p;
                    v.blur_dir = [0.0, 1.0, 0.0, 0.0];
                    (Some(h), Some(v))
                } else {
                    (None, None)
                };
                passes.push(Pass::Adjust { params: p, blur_h: bh, blur_v: bv });
            }
        }
    }
    passes
}

/// Something that turns a project + decoded frames into RGBA8 pixels.
pub trait Renderer {
    fn name(&self) -> String;
    fn size(&self) -> RenderSize;
    /// Render one frame. Returns tightly packed RGBA8 (`width * height * 4`).
    fn render(&mut self, project: &Project, t: f64, frames: &HashMap<String, VideoFrame>) -> anyhow::Result<Vec<u8>>;
}

/// Pick the fastest available renderer: wgpu (DX12/Vulkan) first, CPU fallback.
pub fn create_renderer(size: RenderSize, force_cpu: bool) -> Box<dyn Renderer + Send> {
    if !force_cpu {
        match crate::gpu::GpuRenderer::new(size) {
            Ok(r) => return Box::new(r),
            Err(e) => eprintln!("[myloader] GPU renderer unavailable ({e:#}); falling back to CPU"),
        }
    }
    Box::new(crate::cpu::CpuRenderer::new(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_size_even_and_scaled() {
        let comp = Composition { width: 1080, height: 1920, ..Default::default() };
        let s = RenderSize::for_comp(&comp, Some(2160));
        assert_eq!((s.width, s.height), (2160, 3840));
        let comp = Composition { width: 1001, height: 1001, ..Default::default() };
        let s = RenderSize::for_comp(&comp, None);
        assert_eq!((s.width, s.height), (1000, 1000));
    }

    #[test]
    fn plan_skips_hidden_and_identity() {
        let comp = Composition::default();
        let mut p = Project::new(comp.clone());
        let mut a = Layer::new_adjustment("a", "a", &comp);
        p.layers.push(a.clone());
        a.id = "b".into();
        a.cc.contrast = 0.2;
        p.layers.push(a.clone());
        a.id = "c".into();
        a.visible = false;
        p.layers.push(a);
        let plan = plan_frame(&p, 0.0, RenderSize::for_comp(&comp, None), &HashMap::new());
        assert_eq!(plan.len(), 1);
    }
}
