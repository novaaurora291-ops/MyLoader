// UI mirror of crates/engine/src/layout.rs - aspect presets & "Fill All Layers to Composition".
import type { AspectRatio, Composition, FitMode, Layer, Project } from "../types";

export const ASPECT_SIZES: Record<AspectRatio, [number, number]> = {
  "9:16": [1080, 1920],
  "16:9": [1920, 1080],
  "1:1": [1080, 1080],
  "4:5": [1080, 1350],
};

export function aspectOf(c: Composition): AspectRatio | null {
  for (const [k, [w, h]] of Object.entries(ASPECT_SIZES)) if (w === c.width && h === c.height) return k as AspectRatio;
  return null;
}

/** Change canvas size; layer centres move proportionally. */
export function setAspect(p: Project, aspect: AspectRatio): Project {
  const [w, h] = ASPECT_SIZES[aspect];
  const sx = w / Math.max(1, p.composition.width);
  const sy = h / Math.max(1, p.composition.height);
  return {
    ...p,
    composition: { ...p.composition, width: w, height: h },
    layers: p.layers.map((l) => ({ ...l, transform: { ...l.transform, x: l.transform.x * sx, y: l.transform.y * sy } })),
  };
}

export function fitLayer(l: Layer, c: Composition, videoMode: FitMode): Layer {
  const cw = c.width, ch = c.height;
  let w = cw, h = ch;
  if (l.kind.type === "video" && l.kind.srcWidth > 0 && l.kind.srcHeight > 0 && videoMode !== "stretch") {
    const sw = l.kind.srcWidth, sh = l.kind.srcHeight;
    const s = videoMode === "cover" ? Math.max(cw / sw, ch / sh) : Math.min(cw / sw, ch / sh);
    w = sw * s;
    h = sh * s;
  }
  return { ...l, transform: { x: cw / 2, y: ch / 2, width: w, height: h, rotation: 0 } };
}

/** One click: fit video + every adjustment/rectangle layer to the canvas. Locked layers are skipped. */
export function fillAllLayers(p: Project, videoMode: FitMode = "cover"): Project {
  return { ...p, layers: p.layers.map((l) => (l.locked ? l : fitLayer(l, p.composition, videoMode))) };
}

export function layerEnd(l: Layer): number | null {
  if (l.end != null) return l.end;
  if (l.kind.type === "video" && l.kind.duration > 0) return l.start + l.kind.duration;
  return null;
}

export function projectDuration(p: Project): number {
  if (p.composition.duration > 0) return p.composition.duration;
  const d = Math.max(0, ...p.layers.map((l) => layerEnd(l) ?? 0));
  return d > 0 ? d : 5;
}

export function activeAt(l: Layer, t: number): boolean {
  const e = layerEnd(l);
  return l.visible && l.opacity > 0 && t >= l.start && (e == null || t < e);
}
