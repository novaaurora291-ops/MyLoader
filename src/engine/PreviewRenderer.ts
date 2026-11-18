// Real-time WebGL2 preview. Executes the same pass plan as the Rust/wgpu
// export renderer, on the GPU, every animation frame.
import type { Project } from "../types";
import { FS_ADJUST, FS_BLUR, FS_LAYER, FS_PRESENT, VERT } from "./shaders";
import { planFrame, renderSize, type Params, type RenderSize } from "./plan";

interface Target { tex: WebGLTexture; fbo: WebGLFramebuffer }
interface Prog { p: WebGLProgram; u: Record<string, WebGLUniformLocation | null> }

const UNIFORMS = ["u_comp", "u_xform", "u_xform2", "u_color", "u_cc1", "u_cc2", "u_cc3", "u_blurDir", "u_dst", "u_src"];

export class PreviewRenderer {
  private gl: WebGL2RenderingContext;
  private progs: Record<"layer" | "blur" | "adjust" | "present", Prog>;
  private accum: Target[] = [];
  private scratch: Target[] = [];
  private videoTex = new Map<string, WebGLTexture>();
  private dummy: WebGLTexture;
  private vao: WebGLVertexArrayObject;
  private size: RenderSize = { width: 0, height: 0, scale: 1 };
  private floatTargets: boolean;
  /** Set when a video cannot be uploaded (cross-origin taint) so the caller can switch to a Blob URL. */
  public onTaintedVideo?: (layerId: string) => void;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { antialias: false, premultipliedAlpha: false, preserveDrawingBuffer: true });
    if (!gl) throw new Error("WebGL2 is not available - update your GPU driver / WebView2 runtime");
    this.gl = gl;
    this.floatTargets = !!gl.getExtension("EXT_color_buffer_float");
    gl.getExtension("OES_texture_float_linear");
    this.progs = {
      layer: this.program(FS_LAYER),
      blur: this.program(FS_BLUR),
      adjust: this.program(FS_ADJUST),
      present: this.program(FS_PRESENT),
    };
    this.vao = gl.createVertexArray()!;
    this.dummy = this.texture(1, 1, false);
  }

  get gpuInfo(): string {
    const gl = this.gl;
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    const name = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    return `${name}${this.floatTargets ? " (16-bit float)" : " (8-bit)"}`;
  }

  private program(fs: string): Prog {
    const gl = this.gl;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error("shader: " + gl.getShaderInfoLog(s));
      return s;
    };
    const p = gl.createProgram()!;
    gl.attachShader(p, sh(gl.VERTEX_SHADER, VERT));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error("link: " + gl.getProgramInfoLog(p));
    const u: Prog["u"] = {};
    for (const n of UNIFORMS) u[n] = gl.getUniformLocation(p, n);
    return { p, u };
  }

  private texture(w: number, h: number, float: boolean): WebGLTexture {
    const gl = this.gl;
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    if (float) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
    else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  private target(w: number, h: number): Target {
    const gl = this.gl;
    const tex = this.texture(w, h, this.floatTargets);
    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    return { tex, fbo };
  }

  private resize(size: RenderSize) {
    if (size.width === this.size.width && size.height === this.size.height) {
      this.size = size;
      return;
    }
    const gl = this.gl;
    for (const t of [...this.accum, ...this.scratch]) {
      gl.deleteTexture(t.tex);
      gl.deleteFramebuffer(t.fbo);
    }
    this.accum = [this.target(size.width, size.height), this.target(size.width, size.height)];
    this.scratch = [this.target(size.width, size.height), this.target(size.width, size.height)];
    this.canvas.width = size.width;
    this.canvas.height = size.height;
    this.size = size;
  }

  private uploadVideo(id: string, v: HTMLVideoElement): boolean {
    if (v.readyState < 2 || v.videoWidth === 0) return false;
    const gl = this.gl;
    let t = this.videoTex.get(id);
    if (!t) {
      t = this.texture(1, 1, false);
      this.videoTex.set(id, t);
    }
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    try {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, v);
    } catch (e) {
      this.onTaintedVideo?.(id);
      return false;
    }
    return true;
  }

  private draw(prog: Prog, prm: Params, dst: WebGLTexture, src: WebGLTexture, out: Target | null) {
    const gl = this.gl;
    gl.useProgram(prog.p);
    gl.bindFramebuffer(gl.FRAMEBUFFER, out ? out.fbo : null);
    gl.viewport(0, 0, this.size.width, this.size.height);
    const u = prog.u;
    gl.uniform4fv(u.u_comp, prm.comp);
    gl.uniform4fv(u.u_xform, prm.xform);
    gl.uniform4fv(u.u_xform2, prm.xform2);
    gl.uniform4fv(u.u_color, prm.color);
    gl.uniform4fv(u.u_cc1, prm.cc1);
    gl.uniform4fv(u.u_cc2, prm.cc2);
    gl.uniform4fv(u.u_cc3, prm.cc3);
    gl.uniform4fv(u.u_blurDir, prm.blurDir);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, dst);
    gl.uniform1i(u.u_dst, 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, src);
    gl.uniform1i(u.u_src, 1);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /** Render the project at time t. `scale` = preview pixels per composition pixel. */
  render(project: Project, t: number, videos: Map<string, HTMLVideoElement>, scale: number): number {
    const t0 = performance.now();
    const gl = this.gl;
    const c = project.composition;
    this.resize(renderSize(c.width, c.height, scale));

    const ready = new Set<string>();
    for (const [id, v] of videos) if (this.uploadVideo(id, v)) ready.add(id);

    const bg = c.background;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.accum[0].fbo);
    gl.viewport(0, 0, this.size.width, this.size.height);
    gl.clearColor(bg[0], bg[1], bg[2], bg[3]);
    gl.clear(gl.COLOR_BUFFER_BIT);

    let cur = 0;
    for (const pass of planFrame(project, t, this.size, (id) => ready.has(id))) {
      const a = this.accum[cur], b = this.accum[1 - cur];
      if (pass.kind === "video") {
        this.draw(this.progs.layer, pass.params, a.tex, this.videoTex.get(pass.layerId)!, b);
      } else if (pass.kind === "solid") {
        this.draw(this.progs.layer, pass.params, a.tex, this.dummy, b);
      } else {
        let below = a.tex;
        if (pass.blurH && pass.blurV) {
          this.draw(this.progs.blur, pass.blurH, this.dummy, a.tex, this.scratch[0]);
          this.draw(this.progs.blur, pass.blurV, this.dummy, this.scratch[0].tex, this.scratch[1]);
          below = this.scratch[1].tex;
        }
        this.draw(this.progs.adjust, pass.params, a.tex, below, b);
      }
      cur = 1 - cur;
    }
    const present: Params = {
      comp: [this.size.width, this.size.height, 0, 0], xform: [0, 0, 1, 1], xform2: [0, 1, 0, 0], color: [0, 0, 0, 0],
      cc1: [0, 0, 0, 0], cc2: [0, 0, 0, 0], cc3: [0, 0, 0, 0], blurDir: [0, 0, 0, 0],
    };
    this.draw(this.progs.present, present, this.dummy, this.accum[cur].tex, null);
    return performance.now() - t0;
  }

  dispose() {
    const gl = this.gl;
    for (const t of [...this.accum, ...this.scratch]) {
      gl.deleteTexture(t.tex);
      gl.deleteFramebuffer(t.fbo);
    }
    for (const t of this.videoTex.values()) gl.deleteTexture(t);
  }
}
