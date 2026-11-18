// MyLoader GPU compositor.
// Same maths as src/engine/shaders.ts (WebGL2 preview) and cpu.rs (reference).
// All passes are full-screen triangles; intermediate targets are rgba16float.

struct Params {
    comp: vec4<f32>,     // target width, target height, unused, blend mode id
    xform: vec4<f32>,    // centre x, centre y, width, height   (target pixels)
    xform2: vec4<f32>,   // rotation (rad, clockwise), opacity, corner radius (px), source kind (0 texture, 1 solid)
    color: vec4<f32>,    // solid colour, straight RGBA
    cc1: vec4<f32>,      // brightness, contrast, exposure (stops), saturation
    cc2: vec4<f32>,      // highlights, shadows, vignette, vignette softness
    cc3: vec4<f32>,      // bottom fade, bottom fade height, blur radius (px), unused
    blur_dir: vec4<f32>, // blur direction (1,0) or (0,1)
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var t_dst: texture_2d<f32>;
@group(0) @binding(2) var t_src: texture_2d<f32>;
@group(0) @binding(3) var s_lin: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    o.uv = vec2<f32>(x, y);
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return o;
}

// ---------------------------------------------------------------- helpers ---

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// pixel -> layer UV (0..1 inside the layer rectangle)
fn layer_uv(px: vec2<f32>) -> vec2<f32> {
    let d = px - P.xform.xy;
    let a = -P.xform2.x;
    let c = cos(a);
    let s = sin(a);
    let r = vec2<f32>(d.x * c - d.y * s, d.x * s + d.y * c);
    return r / max(P.xform.zw, vec2<f32>(1e-4)) + 0.5;
}

// anti-aliased rounded-rectangle coverage
fn coverage(luv: vec2<f32>, radius: f32) -> f32 {
    let size = P.xform.zw;
    let half_size = size * 0.5;
    let p = (luv - 0.5) * size;
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d, 0.0, 1.0);
}

fn blend_ch(cb: f32, cs: f32, mode: i32) -> f32 {
    let b = clamp(cb, 0.0, 1.0);
    let s = clamp(cs, 0.0, 1.0);
    switch mode {
        case 1: { return b * s; }                                   // multiply
        case 2: { return b + s - b * s; }                           // screen
        case 3: {                                                   // overlay
            if (b <= 0.5) { return 2.0 * b * s; }
            return 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
        }
        case 4: { return cb + cs; }                                 // add
        case 5: {                                                   // soft light (W3C)
            if (s <= 0.5) { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
            var dd = sqrt(b);
            if (b <= 0.25) { dd = ((16.0 * b - 12.0) * b + 4.0) * b; }
            return b + (2.0 * s - 1.0) * (dd - b);
        }
        case 6: {                                                   // hard light
            if (s <= 0.5) { return 2.0 * b * s; }
            return 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
        }
        case 7: { return min(b, s); }                               // darken
        case 8: { return max(b, s); }                               // lighten
        case 9: {                                                   // colour dodge
            if (b <= 0.0) { return 0.0; }
            if (s >= 1.0) { return 1.0; }
            return min(1.0, b / (1.0 - s));
        }
        case 10: {                                                  // colour burn
            if (b >= 1.0) { return 1.0; }
            if (s <= 0.0) { return 0.0; }
            return 1.0 - min(1.0, (1.0 - b) / s);
        }
        case 11: { return abs(b - s); }                             // difference
        default: { return cs; }                                     // normal
    }
}

fn blend(dst: vec4<f32>, src: vec3<f32>, alpha: f32) -> vec4<f32> {
    let mode = i32(P.comp.w + 0.5);
    let mixed = vec3<f32>(
        blend_ch(dst.r, src.r, mode),
        blend_ch(dst.g, src.g, mode),
        blend_ch(dst.b, src.b, mode),
    );
    let a = clamp(alpha, 0.0, 1.0);
    return vec4<f32>(mix(dst.rgb, mixed, a), dst.a + a * (1.0 - dst.a));
}

fn grade(c0: vec3<f32>, luv: vec2<f32>) -> vec3<f32> {
    var c = c0 * exp2(P.cc1.z);                                 // exposure
    c = c + vec3<f32>(P.cc1.x);                                 // brightness
    c = (c - vec3<f32>(0.5)) * (1.0 + P.cc1.y) + vec3<f32>(0.5);// contrast
    let l = luma(c);                                            // HDR highlights / shadows
    let sh = 1.0 - smoothstep(0.0, 0.5, l);
    let hi = smoothstep(0.5, 1.0, l);
    c = c + vec3<f32>(P.cc2.y * sh * 0.5 + P.cc2.x * hi * 0.5);
    let l2 = luma(c);                                           // saturation
    c = mix(vec3<f32>(l2), c, 1.0 + P.cc1.w);
    let d = length(luv - vec2<f32>(0.5)) * 1.41421356;          // vignette
    let inner = 0.8 - 0.6 * P.cc2.w;
    c = c * (1.0 - P.cc2.z * smoothstep(inner, 1.15, d));
    let h = max(P.cc3.y, 0.001);                                // bottom black fade
    c = c * (1.0 - P.cc3.x * smoothstep(1.0 - h, 1.0, luv.y));
    return max(c, vec3<f32>(0.0));
}

// ----------------------------------------------------------------- passes ---

// Composite a video/image texture (kind 0) or a solid rectangle (kind 1).
@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    let ip = vec2<i32>(floor(in.pos.xy));
    let dst = textureLoad(t_dst, ip, 0);
    let luv = layer_uv(in.pos.xy);
    var src = P.color;
    var radius = P.xform2.z;
    if (P.xform2.w < 0.5) {
        src = textureSampleLevel(t_src, s_lin, clamp(luv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
        radius = 0.0;
    }
    let cov = coverage(luv, radius);
    return blend(dst, src.rgb, src.a * P.xform2.y * cov);
}

// Separable gaussian blur of the whole frame (t_src) along blur_dir.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let radius = P.cc3.z;
    let size = P.comp.xy;
    if (radius < 0.5) {
        return textureLoad(t_src, vec2<i32>(floor(in.pos.xy)), 0);
    }
    let taps = min(ceil(radius), 48.0);
    let step_px = radius / taps;
    let sigma = max(radius / 2.5, 0.001);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    let n = i32(taps);
    for (var i = -n; i <= n; i = i + 1) {
        let off = f32(i) * step_px;
        let w = exp(-0.5 * (off / sigma) * (off / sigma));
        let uv = (in.pos.xy + P.blur_dir.xy * off) / size;
        acc = acc + textureSampleLevel(t_src, s_lin, uv, 0.0) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

// Adjustment layer: grade the (optionally blurred) frame below and blend it back.
// t_dst = composite so far, t_src = blurred composite (or a copy of it).
@fragment
fn fs_adjust(in: VsOut) -> @location(0) vec4<f32> {
    let ip = vec2<i32>(floor(in.pos.xy));
    let dst = textureLoad(t_dst, ip, 0);
    let below = textureLoad(t_src, ip, 0);
    let luv = layer_uv(in.pos.xy);
    let cov = coverage(luv, 0.0);
    let graded = grade(below.rgb, luv);
    return blend(dst, graded, P.xform2.y * cov);
}

// Final pass: rgba16float -> rgba8unorm.
@fragment
fn fs_present(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureLoad(t_src, vec2<i32>(floor(in.pos.xy)), 0);
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
