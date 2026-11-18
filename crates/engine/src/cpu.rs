//! CPU reference renderer (rayon-parallel). Implements exactly the same maths
//! as `gpu/composite.wgsl`. Used as a fallback on machines without a usable
//! GPU and by the test-suite (CI has no GPU).

use crate::render::*;
use crate::model::Project;
use rayon::prelude::*;
use std::collections::HashMap;

pub struct CpuRenderer {
    size: RenderSize,
    a: Vec<[f32; 4]>,
    b: Vec<[f32; 4]>,
    t1: Vec<[f32; 4]>,
    t2: Vec<[f32; 4]>,
}

impl CpuRenderer {
    pub fn new(size: RenderSize) -> Self {
        let n = (size.width * size.height) as usize;
        Self { size, a: vec![[0.0; 4]; n], b: vec![[0.0; 4]; n], t1: vec![[0.0; 4]; n], t2: vec![[0.0; 4]; n] }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn layer_uv(p: &Params, px: f32, py: f32) -> (f32, f32) {
    let (dx, dy) = (px - p.xform[0], py - p.xform[1]);
    let a = -p.xform2[0];
    let (s, c) = a.sin_cos();
    let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
    (rx / p.xform[2].max(1e-4) + 0.5, ry / p.xform[3].max(1e-4) + 0.5)
}

fn coverage(p: &Params, u: f32, v: f32, radius: f32) -> f32 {
    let (w, h) = (p.xform[2], p.xform[3]);
    let (hx, hy) = (w * 0.5, h * 0.5);
    let (px, py) = ((u - 0.5) * w, (v - 0.5) * h);
    let r = radius.min(hx.min(hy));
    let (qx, qy) = (px.abs() - hx + r, py.abs() - hy + r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let d = outside + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

pub(crate) fn blend_ch(cb: f32, cs: f32, mode: i32) -> f32 {
    let b = cb.clamp(0.0, 1.0);
    let s = cs.clamp(0.0, 1.0);
    match mode {
        1 => b * s,
        2 => b + s - b * s,
        3 => {
            if b <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        4 => cb + cs,
        5 => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        6 => {
            if s <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        7 => b.min(s),
        8 => b.max(s),
        9 => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        10 => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        11 => (b - s).abs(),
        _ => cs,
    }
}

fn blend(p: &Params, dst: [f32; 4], src: [f32; 3], alpha: f32) -> [f32; 4] {
    let mode = (p.comp[3] + 0.5) as i32;
    let a = alpha.clamp(0.0, 1.0);
    let mut o = [0.0; 4];
    for i in 0..3 {
        let m = blend_ch(dst[i], src[i], mode);
        o[i] = dst[i] + (m - dst[i]) * a;
    }
    o[3] = dst[3] + a * (1.0 - dst[3]);
    o
}

pub(crate) fn grade(p: &Params, c0: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let e = p.cc1[2].exp2();
    let mut c = c0.map(|x| ((x * e + p.cc1[0]) - 0.5) * (1.0 + p.cc1[1]) + 0.5);
    let l = luma(c);
    let sh = 1.0 - smoothstep(0.0, 0.5, l);
    let hi = smoothstep(0.5, 1.0, l);
    let add = p.cc2[1] * sh * 0.5 + p.cc2[0] * hi * 0.5;
    c = c.map(|x| x + add);
    let l2 = luma(c);
    c = c.map(|x| l2 + (x - l2) * (1.0 + p.cc1[3]));
    let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * std::f32::consts::SQRT_2;
    let inner = 0.8 - 0.6 * p.cc2[3];
    let vig = 1.0 - p.cc2[2] * smoothstep(inner, 1.15, d);
    let h = p.cc3[1].max(0.001);
    let fade = 1.0 - p.cc3[0] * smoothstep(1.0 - h, 1.0, v);
    c.map(|x| (x * vig * fade).max(0.0))
}

/// Bilinear sample with clamp-to-edge, `u,v` in 0..1 (texel centres at (i+0.5)/w).
fn sample_rgba8(f: &VideoFrame, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (f.width as i32, f.height as i32);
    let x = u * f.width as f32 - 0.5;
    let y = v * f.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |xi: i32, yi: i32| -> [f32; 4] {
        let xi = xi.clamp(0, w - 1) as usize;
        let yi = yi.clamp(0, h - 1) as usize;
        let i = (yi * f.width as usize + xi) * 4;
        [f.data[i] as f32 / 255.0, f.data[i + 1] as f32 / 255.0, f.data[i + 2] as f32 / 255.0, f.data[i + 3] as f32 / 255.0]
    };
    let (x0, y0) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    let mut o = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        o[i] = top + (bot - top) * fy;
    }
    o
}

fn sample_f32(buf: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    // x,y in pixel space (centre = i + 0.5); bilinear + clamp, like the GPU sampler
    let x = x - 0.5;
    let y = y - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: i32, yi: i32| buf[(yi.clamp(0, h as i32 - 1) as usize) * w + xi.clamp(0, w as i32 - 1) as usize];
    let (x0, y0) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut o = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        o[i] = top + (bot - top) * fy;
    }
    o
}

fn blur_pass(p: &Params, src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize) {
    let radius = p.cc3[2];
    if radius < 0.5 {
        dst.copy_from_slice(src);
        return;
    }
    let taps = radius.ceil().min(48.0);
    let step = radius / taps;
    let sigma = (radius / 2.5).max(0.001);
    let n = taps as i32;
    let weights: Vec<(f32, f32)> = (-n..=n)
        .map(|i| {
            let off = i as f32 * step;
            (off, (-0.5 * (off / sigma).powi(2)).exp())
        })
        .collect();
    let wsum: f32 = weights.iter().map(|w| w.1).sum();
    let (dx, dy) = (p.blur_dir[0], p.blur_dir[1]);
    dst.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut acc = [0.0f32; 4];
            for (off, wt) in &weights {
                let s = sample_f32(src, w, h, cx + dx * off, cy + dy * off);
                for i in 0..4 {
                    acc[i] += s[i] * wt;
                }
            }
            *out = acc.map(|v| v / wsum);
        }
    });
}

impl Renderer for CpuRenderer {
    fn name(&self) -> String {
        "CPU (rayon)".into()
    }

    fn size(&self) -> RenderSize {
        self.size
    }

    fn render(&mut self, project: &Project, t: f64, frames: &HashMap<String, VideoFrame>) -> anyhow::Result<Vec<u8>> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let bg = project.composition.background;
        self.a.iter_mut().for_each(|p| *p = bg);

        for pass in plan_frame(project, t, self.size, frames) {
            match pass {
                Pass::Video { layer_id, params } => {
                    let f = &frames[&layer_id];
                    let src = &self.a;
                    self.b.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                        for (x, out) in row.iter_mut().enumerate() {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let dst = src[y * w + x];
                            let (u, v) = layer_uv(&params, px, py);
                            let cov = coverage(&params, u, v, 0.0);
                            if cov <= 0.0 {
                                *out = dst;
                                continue;
                            }
                            let s = sample_rgba8(f, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
                            *out = blend(&params, dst, [s[0], s[1], s[2]], s[3] * params.xform2[1] * cov);
                        }
                    });
                    std::mem::swap(&mut self.a, &mut self.b);
                }
                Pass::Solid { params } => {
                    let src = &self.a;
                    self.b.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                        for (x, out) in row.iter_mut().enumerate() {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let (u, v) = layer_uv(&params, px, py);
                            let cov = coverage(&params, u, v, params.xform2[2]);
                            let c = params.color;
                            *out = blend(&params, src[y * w + x], [c[0], c[1], c[2]], c[3] * params.xform2[1] * cov);
                        }
                    });
                    std::mem::swap(&mut self.a, &mut self.b);
                }
                Pass::Adjust { params, blur_h, blur_v } => {
                    let below: &Vec<[f32; 4]> = if let (Some(bh), Some(bv)) = (blur_h, blur_v) {
                        blur_pass(&bh, &self.a, &mut self.t1, w, h);
                        blur_pass(&bv, &self.t1, &mut self.t2, w, h);
                        &self.t2
                    } else {
                        &self.a
                    };
                    let src = &self.a;
                    self.b.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                        for (x, out) in row.iter_mut().enumerate() {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let (u, v) = layer_uv(&params, px, py);
                            let cov = coverage(&params, u, v, 0.0);
                            let dst = src[y * w + x];
                            if cov <= 0.0 {
                                *out = dst;
                                continue;
                            }
                            let b = below[y * w + x];
                            let g = grade(&params, [b[0], b[1], b[2]], u, v);
                            *out = blend(&params, dst, g, params.xform2[1] * cov);
                        }
                    });
                    std::mem::swap(&mut self.a, &mut self.b);
                }
            }
        }

        let mut out = vec![0u8; w * h * 4];
        out.par_chunks_mut(4).zip(self.a.par_iter()).for_each(|(o, c)| {
            o[0] = (c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            o[1] = (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            o[2] = (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            o[3] = 255;
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn params_with(mode: BlendMode) -> Params {
        Params { comp: [1.0, 1.0, 0.0, mode.shader_id() as f32], ..Default::default() }
    }

    #[test]
    fn blend_modes_known_values() {
        let p = params_with(BlendMode::Multiply);
        assert_eq!(blend(&p, [0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5], 1.0)[0], 0.25);
        let p = params_with(BlendMode::Screen);
        assert_eq!(blend(&p, [0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5], 1.0)[0], 0.75);
        let p = params_with(BlendMode::Overlay);
        assert_eq!(blend(&p, [0.25, 0.25, 0.25, 1.0], [0.5, 0.5, 0.5], 1.0)[0], 0.25);
        let p = params_with(BlendMode::Normal);
        assert_eq!(blend(&p, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 0.5)[0], 0.5);
    }

    #[test]
    fn grade_identity_and_exposure() {
        let p = Params { cc2: [0.0, 0.0, 0.0, 0.5], cc3: [0.0, 0.35, 0.0, 0.0], ..Default::default() };
        let g = grade(&p, [0.3, 0.5, 0.7], 0.5, 0.5);
        assert!((g[0] - 0.3).abs() < 1e-5 && (g[2] - 0.7).abs() < 1e-5);
        let p = Params { cc1: [0.0, 0.0, 1.0, 0.0], cc3: [0.0, 0.35, 0.0, 0.0], ..Default::default() };
        let g = grade(&p, [0.25, 0.25, 0.25], 0.5, 0.5);
        assert!((g[0] - 0.5).abs() < 0.01, "{g:?}");
    }

    #[test]
    fn bottom_fade_darkens_bottom_only() {
        let comp = Composition { width: 64, height: 64, background: [1.0, 1.0, 1.0, 1.0], ..Default::default() };
        let mut proj = Project::new(comp.clone());
        let mut a = Layer::new_adjustment("f", "fade", &comp);
        a.cc.bottom_fade = 1.0;
        a.cc.bottom_fade_height = 0.5;
        proj.layers.push(a);
        let mut r = CpuRenderer::new(RenderSize::for_comp(&comp, None));
        let px = r.render(&proj, 0.0, &HashMap::new()).unwrap();
        let at = |x: usize, y: usize| px[(y * 64 + x) * 4];
        assert_eq!(at(32, 2), 255);
        assert!(at(32, 63) < 10, "bottom should be ~black, got {}", at(32, 63));
    }
}
