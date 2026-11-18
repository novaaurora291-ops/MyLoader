// Built-in looks, user presets and Auto CC.
// Everything here only produces ordinary adjustment / rectangle layers, so the
// Rust export engine renders them exactly like hand-made or XML-imported layers.
import type { BlendMode, CcParams, Layer, Project } from "../types";
import { DEFAULT_CC } from "../types";
import { LOOK_PREFIX, newAdjustment, newId, newRectangle } from "./store";
export const isLookLayer = (l: Layer) => l.id.startsWith(LOOK_PREFIX);

interface AdjSpec { name: string; cc: Partial<CcParams>; blend?: BlendMode; opacity?: number }
interface RectSpec { name: string; rect: [number, number, number, number]; blend: BlendMode; opacity: number }
type Spec = AdjSpec | RectSpec;

export interface Look { id: string; name: string; description: string; layers: Spec[] }

/** Bottom → top. */
export const BUILTIN_LOOKS: Look[] = [
  {
    id: "hdr-cc",
    name: "HDR CC (Lyrics)",
    description: "Alight Motion style HDR grade: lifted shadows, recovered highlights, punchy colour + lyrics fade",
    layers: [
      { name: "HDR Base", cc: { exposure: 0.15, contrast: 0.25, highlights: -0.35, shadows: 0.3, saturation: 0.2 } },
      { name: "HDR Pop", cc: { contrast: 0.1, saturation: 0.12, vignette: 0.25, vignetteSoftness: 0.6 } },
      { name: "Dark Overlay", rect: [0, 0, 0, 1], blend: "multiply", opacity: 0.18 },
      { name: "Lyrics Fade", cc: { bottomFade: 0.85, bottomFadeHeight: 0.4 } },
    ],
  },
  {
    id: "cinematic",
    name: "Cinematic",
    description: "Muted colour, deep contrast, soft highlights, heavy vignette",
    layers: [
      { name: "Cine Grade", cc: { exposure: -0.1, contrast: 0.3, highlights: -0.25, shadows: -0.1, saturation: -0.15 } },
      { name: "Cine Vignette", cc: { vignette: 0.45, vignetteSoftness: 0.65 } },
    ],
  },
  {
    id: "vivid",
    name: "Vivid Pop",
    description: "Bright, saturated, social-media pop",
    layers: [{ name: "Vivid", cc: { exposure: 0.05, contrast: 0.2, highlights: -0.15, shadows: 0.15, saturation: 0.45 } }],
  },
  {
    id: "moody",
    name: "Moody Dark",
    description: "Dark, desaturated, strong vignette and bottom fade",
    layers: [
      { name: "Moody Grade", cc: { exposure: -0.35, contrast: 0.35, shadows: -0.15, saturation: -0.3 } },
      { name: "Moody Vignette", cc: { vignette: 0.55, vignetteSoftness: 0.5, bottomFade: 0.6, bottomFadeHeight: 0.35 } },
    ],
  },
  {
    id: "glow",
    name: "Soft Glow",
    description: "Dreamy bloom: blurred copy screened over a soft grade",
    layers: [
      { name: "Glow Base", cc: { brightness: 0.03, contrast: -0.1, highlights: 0.1, saturation: 0.1 } },
      { name: "Glow Bloom", cc: { blur: 18, exposure: 0.2 }, blend: "screen", opacity: 0.35 },
    ],
  },
  {
    id: "bw",
    name: "B&W Contrast",
    description: "Black & white with strong contrast",
    layers: [{ name: "B&W", cc: { saturation: -1, contrast: 0.4, highlights: -0.2, shadows: 0.1, vignette: 0.3 } }],
  },
];

function tag(l: Layer): Layer {
  return { ...l, id: newId(LOOK_PREFIX + (l.kind.type === "rectangle" ? "rect" : "adj")) };
}

export function buildLook(look: Look, p: Project): Layer[] {
  return look.layers.map((s) => {
    if ("rect" in s) {
      const r = newRectangle(p);
      return tag({ ...r, name: s.name, blend: s.blend, opacity: s.opacity, kind: { type: "rectangle", color: s.rect, cornerRadius: 0 } });
    }
    const a = newAdjustment(p, s.name);
    return tag({ ...a, cc: { ...DEFAULT_CC, ...s.cc }, blend: s.blend ?? "normal", opacity: s.opacity ?? 1 });
  });
}

// ---- user presets (stored in the app's local storage) ------------------------

export interface UserPreset { name: string; width: number; height: number; layers: Layer[]; savedAt: string }
const KEY = "myloader.presets.v1";

export function loadUserPresets(): UserPreset[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export function saveUserPresets(list: UserPreset[]) {
  localStorage.setItem(KEY, JSON.stringify(list));
}

/** Snapshot every non-video layer of the project as a preset. */
export function presetFromProject(name: string, p: Project): UserPreset {
  return {
    name,
    width: p.composition.width,
    height: p.composition.height,
    layers: p.layers.filter((l) => l.kind.type !== "video").map((l) => ({ ...l, locked: false })),
    savedAt: new Date().toISOString(),
  };
}

/** Preset layers rescaled to the current composition. */
export function buildPreset(pr: UserPreset, p: Project): Layer[] {
  const sx = p.composition.width / Math.max(1, pr.width);
  const sy = p.composition.height / Math.max(1, pr.height);
  return pr.layers.map((l) =>
    tag({
      ...l,
      transform: { ...l.transform, x: l.transform.x * sx, y: l.transform.y * sy, width: l.transform.width * sx, height: l.transform.height * sy },
      cc: { ...DEFAULT_CC, ...l.cc, blur: (l.cc?.blur ?? 0) * Math.min(sx, sy) },
    }),
  );
}

// ---- Auto CC -------------------------------------------------------------------

export interface FrameStats { median: number; p02: number; p98: number; meanSat: number; samples: number }

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));
const round = (v: number) => Math.round(v * 100) / 100;

/** Accumulates luma histogram + chroma over frames drawn from <video>. Throws if the frame is tainted. */
export class FrameAnalyzer {
  private hist = new Float64Array(256);
  private satSum = 0;
  private n = 0;
  private frames = 0;
  private ctx: CanvasRenderingContext2D;

  constructor() {
    const c = document.createElement("canvas");
    c.width = 96;
    c.height = 96;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) throw new Error("2D canvas unavailable");
    this.ctx = ctx;
  }

  add(v: HTMLVideoElement) {
    if (v.readyState < 2 || !v.videoWidth) return;
    const ar = v.videoWidth / v.videoHeight;
    const w = ar >= 1 ? 96 : Math.max(8, Math.round(96 * ar));
    const h = ar >= 1 ? Math.max(8, Math.round(96 / ar)) : 96;
    this.ctx.drawImage(v, 0, 0, w, h);
    const d = this.ctx.getImageData(0, 0, w, h).data; // throws SecurityError if tainted
    for (let i = 0; i < d.length; i += 4) {
      const r = d[i] / 255, g = d[i + 1] / 255, b = d[i + 2] / 255;
      const y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
      this.hist[Math.min(255, Math.round(y * 255))]++;
      this.satSum += Math.max(r, g, b) - Math.min(r, g, b);
      this.n++;
    }
    this.frames++;
  }

  stats(): FrameStats | null {
    if (!this.n) return null;
    const pct = (q: number) => {
      let acc = 0;
      const target = q * this.n;
      for (let i = 0; i < 256; i++) {
        acc += this.hist[i];
        if (acc >= target) return i / 255;
      }
      return 1;
    };
    return { median: pct(0.5), p02: pct(0.02), p98: pct(0.98), meanSat: this.satSum / this.n, samples: this.frames };
  }
}

/** Turn frame statistics into a balanced HDR-ish grade for this engine's CC model. */
export function autoCcFromStats(s: FrameStats): Partial<CcParams> {
  const exposure = clamp(Math.log2(0.45 / Math.max(s.median, 0.03)) * 0.6, -1.5, 1.5);
  const k = Math.pow(2, exposure);
  const lo = Math.min(1, s.p02 * k);
  const hi = Math.min(1, s.p98 * k);
  const spread = Math.max(0.05, hi - lo);
  const contrast = clamp((0.88 / spread - 1) * 0.5, -0.3, 0.6);
  const highlights = hi > 0.93 ? -0.35 : hi < 0.75 ? 0.15 : -0.15;
  const shadows = lo < 0.04 ? 0.25 : lo > 0.18 ? -0.1 : 0.1;
  const saturation = clamp((0.32 / Math.max(s.meanSat, 0.04) - 1) * 0.45, -0.3, 0.5);
  return {
    exposure: round(exposure),
    contrast: round(contrast),
    highlights: round(highlights),
    shadows: round(shadows),
    saturation: round(saturation),
    vignette: 0.2,
    vignetteSoftness: 0.6,
  };
}

export function autoCcLayer(p: Project, s: FrameStats): Layer {
  const a = newAdjustment(p, "Auto CC");
  return tag({ ...a, cc: { ...DEFAULT_CC, ...autoCcFromStats(s) } });
}

const once = (el: HTMLVideoElement, ev: string, ms = 4000) =>
  new Promise<void>((res, rej) => {
    const t = setTimeout(() => rej(new Error(`video ${ev} timeout`)), ms);
    el.addEventListener(ev, () => { clearTimeout(t); res(); }, { once: true });
    el.addEventListener("error", () => { clearTimeout(t); rej(new Error("video error")); }, { once: true });
  });

/** Analyse up to `count` frames spread across the clip using a private <video>. */
export async function analyzeVideo(url: string, count = 8): Promise<FrameStats | null> {
  const v = document.createElement("video");
  v.crossOrigin = "anonymous";
  v.muted = true;
  v.preload = "auto";
  v.src = url;
  await once(v, "loadeddata", 8000);
  const an = new FrameAnalyzer();
  const dur = isFinite(v.duration) && v.duration > 0 ? v.duration : 0;
  for (let i = 0; i < count; i++) {
    const t = dur ? (dur * (i + 0.5)) / count : 0;
    if (dur) {
      v.currentTime = t;
      await once(v, "seeked");
    }
    an.add(v);
    if (!dur) break;
  }
  v.removeAttribute("src");
  v.load();
  return an.stats();
}
