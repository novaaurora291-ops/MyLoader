// WebGL2 port of crates/engine/src/gpu/composite.wgsl - identical maths so the
// live preview matches the exported video. Pixel space is y-down like the
// export; GL's y-up framebuffer is handled via `px()`.

export const VERT = /* glsl */ `#version 300 es
void main() {
  float x = float((gl_VertexID << 1) & 2);
  float y = float(gl_VertexID & 2);
  gl_Position = vec4(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}`;

const COMMON = /* glsl */ `#version 300 es
precision highp float;
precision highp int;
uniform vec4 u_comp;    // w, h, -, blend
uniform vec4 u_xform;   // cx, cy, w, h
uniform vec4 u_xform2;  // rot, opacity, radius, kind
uniform vec4 u_color;
uniform vec4 u_cc1;     // bri, con, exp, sat
uniform vec4 u_cc2;     // hi, sh, vig, vigSoft
uniform vec4 u_cc3;     // fade, fadeH, blur, -
uniform vec4 u_blurDir;
uniform sampler2D u_dst;
uniform sampler2D u_src;
out vec4 o;

vec2 px() { return vec2(gl_FragCoord.x, u_comp.y - gl_FragCoord.y); }
float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

vec2 layerUv(vec2 p) {
  vec2 d = p - u_xform.xy;
  float a = -u_xform2.x;
  float c = cos(a), s = sin(a);
  vec2 r = vec2(d.x * c - d.y * s, d.x * s + d.y * c);
  return r / max(u_xform.zw, vec2(1e-4)) + 0.5;
}

float coverage(vec2 luv, float radius) {
  vec2 size = u_xform.zw;
  vec2 hs = size * 0.5;
  vec2 p = (luv - 0.5) * size;
  float r = min(radius, min(hs.x, hs.y));
  vec2 q = abs(p) - hs + vec2(r);
  float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
  return clamp(0.5 - d, 0.0, 1.0);
}

float blendCh(float cb, float cs, int m) {
  float b = clamp(cb, 0.0, 1.0);
  float s = clamp(cs, 0.0, 1.0);
  if (m == 1) return b * s;
  if (m == 2) return b + s - b * s;
  if (m == 3) return b <= 0.5 ? 2.0 * b * s : 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
  if (m == 4) return cb + cs;
  if (m == 5) {
    if (s <= 0.5) return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    float dd = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
    return b + (2.0 * s - 1.0) * (dd - b);
  }
  if (m == 6) return s <= 0.5 ? 2.0 * b * s : 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
  if (m == 7) return min(b, s);
  if (m == 8) return max(b, s);
  if (m == 9) { if (b <= 0.0) return 0.0; if (s >= 1.0) return 1.0; return min(1.0, b / (1.0 - s)); }
  if (m == 10) { if (b >= 1.0) return 1.0; if (s <= 0.0) return 0.0; return 1.0 - min(1.0, (1.0 - b) / s); }
  if (m == 11) return abs(b - s);
  return cs;
}

vec4 blendOver(vec4 dst, vec3 src, float alpha) {
  int m = int(u_comp.w + 0.5);
  vec3 mixed = vec3(blendCh(dst.r, src.r, m), blendCh(dst.g, src.g, m), blendCh(dst.b, src.b, m));
  float a = clamp(alpha, 0.0, 1.0);
  return vec4(mix(dst.rgb, mixed, a), dst.a + a * (1.0 - dst.a));
}

vec3 grade(vec3 c0, vec2 luv) {
  vec3 c = c0 * exp2(u_cc1.z);
  c = c + vec3(u_cc1.x);
  c = (c - 0.5) * (1.0 + u_cc1.y) + 0.5;
  float l = luma(c);
  float sh = 1.0 - smoothstep(0.0, 0.5, l);
  float hi = smoothstep(0.5, 1.0, l);
  c = c + vec3(u_cc2.y * sh * 0.5 + u_cc2.x * hi * 0.5);
  float l2 = luma(c);
  c = mix(vec3(l2), c, 1.0 + u_cc1.w);
  float d = length(luv - 0.5) * 1.41421356;
  float inner = 0.8 - 0.6 * u_cc2.w;
  c = c * (1.0 - u_cc2.z * smoothstep(inner, 1.15, d));
  float h = max(u_cc3.y, 0.001);
  c = c * (1.0 - u_cc3.x * smoothstep(1.0 - h, 1.0, luv.y));
  return max(c, vec3(0.0));
}
`;

export const FS_LAYER = COMMON + /* glsl */ `
void main() {
  ivec2 ip = ivec2(gl_FragCoord.xy);
  vec4 dst = texelFetch(u_dst, ip, 0);
  vec2 luv = layerUv(px());
  vec4 src = u_color;
  float radius = u_xform2.z;
  if (u_xform2.w < 0.5) {
    src = texture(u_src, clamp(luv, 0.0, 1.0));   // video texture is stored top row first
    radius = 0.0;
  }
  float cov = coverage(luv, radius);
  o = blendOver(dst, src.rgb, src.a * u_xform2.y * cov);
}`;

export const FS_BLUR = COMMON + /* glsl */ `
void main() {
  float radius = u_cc3.z;
  if (radius < 0.5) { o = texelFetch(u_src, ivec2(gl_FragCoord.xy), 0); return; }
  float taps = min(ceil(radius), 48.0);
  float stepPx = radius / taps;
  float sigma = max(radius / 2.5, 0.001);
  vec4 acc = vec4(0.0);
  float wsum = 0.0;
  int n = int(taps);
  for (int i = -48; i <= 48; i++) {
    if (i < -n || i > n) continue;
    float off = float(i) * stepPx;
    float w = exp(-0.5 * (off / sigma) * (off / sigma));
    vec2 uv = (gl_FragCoord.xy + u_blurDir.xy * off) / u_comp.xy;
    acc += texture(u_src, uv) * w;
    wsum += w;
  }
  o = acc / wsum;
}`;

export const FS_ADJUST = COMMON + /* glsl */ `
void main() {
  ivec2 ip = ivec2(gl_FragCoord.xy);
  vec4 dst = texelFetch(u_dst, ip, 0);
  vec4 below = texelFetch(u_src, ip, 0);
  vec2 luv = layerUv(px());
  float cov = coverage(luv, 0.0);
  o = blendOver(dst, grade(below.rgb, luv), u_xform2.y * cov);
}`;

export const FS_PRESENT = COMMON + /* glsl */ `
void main() {
  vec4 c = texelFetch(u_src, ivec2(gl_FragCoord.xy), 0);
  o = vec4(clamp(c.rgb, 0.0, 1.0), 1.0);
}`;
