//! wgpu renderer (DirectX 12 / Vulkan on Windows). Executes the pass plan
//! from [`crate::render::plan_frame`] with the shaders in `composite.wgsl`.
//!
//! Textures: two rgba16float ping-pong accumulators (HDR headroom), two blur
//! scratch targets, one rgba8unorm output that is copied back for encoding.

use crate::model::Project;
use crate::render::*;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;

pub const SHADER_SRC: &str = include_str!("composite.wgsl");
const PARAMS_SIZE: u64 = std::mem::size_of::<Params>() as u64;
const ACCUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Target {
    _tex: wgpu::Texture,
    view: wgpu::TextureView,
}

struct VideoTex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    size: RenderSize,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    p_layer: wgpu::RenderPipeline,
    p_blur: wgpu::RenderPipeline,
    p_adjust: wgpu::RenderPipeline,
    p_present: wgpu::RenderPipeline,
    accum: [Target; 2],
    scratch: [Target; 2],
    out: wgpu::Texture,
    out_view: wgpu::TextureView,
    dummy: Target,
    readback: wgpu::Buffer,
    padded_row: u32,
    uniforms: wgpu::Buffer,
    uniform_slots: u64,
    uniform_stride: u64,
    videos: HashMap<String, VideoTex>,
}

fn make_target(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, label: &str) -> Target {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Target { _tex: tex, view }
}

impl GpuRenderer {
    pub fn new(size: RenderSize) -> Result<Self> {
        pollster::block_on(Self::new_async(size))
    }

    async fn new_async(size: RenderSize) -> Result<Self> {
        // Honours WGPU_BACKEND=dx12|vulkan|gl etc.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && std::env::var("MYLOADER_ALLOW_SOFTWARE_GPU").is_err() {
            return Err(anyhow!("only a software adapter ({}) is available", info.name));
        }
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        let limits = wgpu::Limits {
            max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.max(8192),
            ..wgpu::Limits::downlevel_defaults()
        }
        .using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("myloader"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            })
            .await
            .context("creating GPU device")?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite layout"),
            bind_group_layouts: &[Some(&bgl)],
            ..Default::default()
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let p_layer = pipeline("fs_layer", ACCUM_FORMAT);
        let p_blur = pipeline("fs_blur", ACCUM_FORMAT);
        let p_adjust = pipeline("fs_adjust", ACCUM_FORMAT);
        let p_present = pipeline("fs_present", OUT_FORMAT);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (w, h) = (size.width, size.height);
        let rw = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let accum = [make_target(&device, w, h, ACCUM_FORMAT, rw, "accum0"), make_target(&device, w, h, ACCUM_FORMAT, rw, "accum1")];
        let scratch = [make_target(&device, w, h, ACCUM_FORMAT, rw, "blur0"), make_target(&device, w, h, ACCUM_FORMAT, rw, "blur1")];
        let dummy = make_target(&device, 1, 1, OUT_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING, "dummy");
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("out"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let uniform_stride = PARAMS_SIZE.div_ceil(align) * align;
        let uniform_slots = 64;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: uniform_stride * uniform_slots,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            adapter_name,
            size,
            bgl,
            sampler,
            p_layer,
            p_blur,
            p_adjust,
            p_present,
            accum,
            scratch,
            out,
            out_view,
            dummy,
            readback,
            padded_row,
            uniforms,
            uniform_slots,
            uniform_stride,
            videos: HashMap::new(),
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    fn upload_video(&mut self, id: &str, f: &VideoFrame) {
        let need_new = self.videos.get(id).map_or(true, |v| v.w != f.width || v.h != f.height);
        if need_new {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video"),
                size: wgpu::Extent3d { width: f.width, height: f.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.videos.insert(id.to_string(), VideoTex { tex, view, w: f.width, h: f.height });
        }
        let v = &self.videos[id];
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &v.tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &f.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(f.width * 4), rows_per_image: Some(f.height) },
            wgpu::Extent3d { width: f.width, height: f.height, depth_or_array_layers: 1 },
        );
    }

    fn ensure_uniform_slots(&mut self, n: u64) {
        if n <= self.uniform_slots {
            return;
        }
        self.uniform_slots = n.next_power_of_two();
        self.uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: self.uniform_stride * self.uniform_slots,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn bind(&self, dst: &wgpu::TextureView, src: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(PARAMS_SIZE),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(dst) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }
}

/// One recorded draw: pipeline, bind group, uniform slot, render target.
enum Rt {
    Accum(usize),
    Scratch(usize),
    Out,
}

impl Renderer for GpuRenderer {
    fn name(&self) -> String {
        format!("GPU wgpu: {}", self.adapter_name)
    }

    fn size(&self) -> RenderSize {
        self.size
    }

    fn render(&mut self, project: &Project, t: f64, frames: &HashMap<String, VideoFrame>) -> Result<Vec<u8>> {
        let plan = plan_frame(project, t, self.size, frames);
        for pass in &plan {
            if let Pass::Video { layer_id, .. } = pass {
                let f = &frames[layer_id];
                self.upload_video(layer_id, f);
            }
        }

        // Flatten the plan into draws (and collect their uniform blocks).
        let mut draws: Vec<(u8, wgpu::BindGroup, Params, Rt)> = vec![];
        let mut cur = 0usize; // index of the accumulator holding the composite so far
        let present_params = Params {
            comp: [self.size.width as f32, self.size.height as f32, 0.0, 0.0],
            ..Default::default()
        };
        for pass in &plan {
            match pass {
                Pass::Video { layer_id, params } => {
                    let bg = self.bind(&self.accum[cur].view, &self.videos[layer_id].view);
                    draws.push((0, bg, *params, Rt::Accum(1 - cur)));
                    cur = 1 - cur;
                }
                Pass::Solid { params } => {
                    let bg = self.bind(&self.accum[cur].view, &self.dummy.view);
                    draws.push((0, bg, *params, Rt::Accum(1 - cur)));
                    cur = 1 - cur;
                }
                Pass::Adjust { params, blur_h, blur_v } => {
                    let below_is_blur = if let (Some(bh), Some(bv)) = (blur_h, blur_v) {
                        let b1 = self.bind(&self.dummy.view, &self.accum[cur].view);
                        draws.push((1, b1, *bh, Rt::Scratch(0)));
                        let b2 = self.bind(&self.dummy.view, &self.scratch[0].view);
                        draws.push((1, b2, *bv, Rt::Scratch(1)));
                        true
                    } else {
                        false
                    };
                    let src = if below_is_blur { &self.scratch[1].view } else { &self.accum[cur].view };
                    let bg = self.bind(&self.accum[cur].view, src);
                    draws.push((2, bg, *params, Rt::Accum(1 - cur)));
                    cur = 1 - cur;
                }
            }
        }
        let bg = self.bind(&self.dummy.view, &self.accum[cur].view);
        draws.push((3, bg, present_params, Rt::Out));

        self.ensure_uniform_slots(draws.len() as u64);
        if self.uniforms.size() < self.uniform_stride * draws.len() as u64 {
            return Err(anyhow!("uniform buffer too small"));
        }
        let mut staging = vec![0u8; (self.uniform_stride * draws.len() as u64) as usize];
        for (i, d) in draws.iter().enumerate() {
            let o = i * self.uniform_stride as usize;
            staging[o..o + PARAMS_SIZE as usize].copy_from_slice(bytemuck::bytes_of(&d.2));
        }
        self.queue.write_buffer(&self.uniforms, 0, &staging);

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        // Clear the first accumulator to the background colour.
        {
            let b = project.composition.background;
            let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.accum[0].view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: b[0] as f64, g: b[1] as f64, b: b[2] as f64, a: b[3] as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        for (i, (pipe, bg, _, rt)) in draws.iter().enumerate() {
            let view = match rt {
                Rt::Accum(k) => &self.accum[*k].view,
                Rt::Scratch(k) => &self.scratch[*k].view,
                Rt::Out => &self.out_view,
            };
            let pipeline = match pipe {
                0 => &self.p_layer,
                1 => &self.p_blur,
                2 => &self.p_adjust,
                _ => &self.p_present,
            };
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, bg, &[(i as u64 * self.uniform_stride) as u32]);
            rp.draw(0..3, 0..1);
        }
        let (w, h) = (self.size.width, self.size.height);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.out, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.padded_row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);

        let slice = self.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("GPU poll failed: {e}"))?;
        rx.recv().context("map callback dropped")?.map_err(|e| anyhow!("readback failed: {e}"))?;
        let mut out = vec![0u8; (w * h * 4) as usize];
        {
            let data = slice.get_mapped_range().map_err(|e| anyhow!("readback map: {e:?}"))?;
            let row = (w * 4) as usize;
            for y in 0..h as usize {
                let s = y * self.padded_row as usize;
                out[y * row..(y + 1) * row].copy_from_slice(&data[s..s + row]);
            }
        }
        self.readback.unmap();
        Ok(out)
    }
}

/// Validate the WGSL with naga (no GPU needed). Used by tests / `mlcli check`.
#[cfg(test)]
mod tests {
    #[test]
    fn wgsl_is_valid() {
        let module = naga::front::wgsl::parse_str(super::SHADER_SRC).expect("WGSL parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("WGSL validates");
    }
}
