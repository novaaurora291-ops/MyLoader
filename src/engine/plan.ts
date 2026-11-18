// Port of crates/engine/src/render.rs `plan_frame` - turns the project at time t
// into ordered passes with shader parameters (vec4 x 8, same as the WGSL Params).
import type { Layer, Project } from "../types";
import { BLEND_MODES } from "../types";
import { activeAt } from "../lib/layout";

export type Vec4 = [number, number, number, number];
export interface Params { comp: Vec4; xform: Vec4; xform2: Vec4; color: Vec4; cc1: Vec4; cc2: Vec4; cc3: Vec4; blurDir: Vec4 }

export type Pass =
  | { kind: "video"; layerId: string; params: Params }
  | { kind: "solid"; params: Params }
  | { kind: "adjust"; params: Params; blurH: Params | null; blurV: Params | null };

export interface RenderSize { width: number; height: number; scale: number }

export function renderSize(compW: number, compH: number, scale: number): RenderSize {
  const even = (v: number) => Math.max(2, Math.floor(Math.round(v) / 2) * 2);
  return { width: even(compW * scale), height: even(compH * scale), scale };
}

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

function base(l: Layer, s: RenderSize, opacity: number): Params {
  const t = l.transform;
  return {
    comp: [s.width, s.height, 0, BLEND_MODES.indexOf(l.blend)],
    xform: [t.x * s.scale, t.y * s.scale, t.width * s.scale, t.height * s.scale],
    xform2: [(t.rotation * Math.PI) / 180, opacity, 0, 0],
    color: [0, 0, 0, 0],
    cc1: [0, 0, 0, 0],
    cc2: [0, 0, 0, 0],
    cc3: [0, 0, 0, 0],
    blurDir: [0, 0, 0, 0],
  };
}

export function isIdentity(l: Layer): boolean {
  const c = l.cc;
  return c.brightness === 0 && c.contrast === 0 && c.exposure === 0 && c.saturation === 0 && c.highlights === 0 &&
    c.shadows === 0 && c.blur <= 0 && c.vignette === 0 && c.bottomFade === 0;
}

export function planFrame(p: Project, t: number, size: RenderSize, hasFrame: (layerId: string) => boolean): Pass[] {
  const passes: Pass[] = [];
  for (const l of p.layers) {
    if (!activeAt(l, t)) continue;
    const opacity = clamp(l.opacity, 0, 1);
    if (l.kind.type === "video") {
      if (hasFrame(l.id)) passes.push({ kind: "video", layerId: l.id, params: base(l, size, opacity) });
    } else if (l.kind.type === "rectangle") {
      const prm = base(l, size, opacity);
      prm.xform2[2] = l.kind.cornerRadius * size.scale;
      prm.xform2[3] = 1;
      prm.color = [...l.kind.color] as Vec4;
      passes.push({ kind: "solid", params: prm });
    } else {
      if (isIdentity(l) && l.blend === "normal") continue;
      const c = l.cc;
      const prm = base(l, size, opacity);
      prm.cc1 = [clamp(c.brightness, -1, 1), clamp(c.contrast, -1, 3), clamp(c.exposure, -4, 4), clamp(c.saturation, -1, 1)];
      prm.cc2 = [clamp(c.highlights, -1, 1), clamp(c.shadows, -1, 1), clamp(c.vignette, 0, 1), clamp(c.vignetteSoftness, 0, 1)];
      const radius = clamp(c.blur, 0, 200) * size.scale;
      prm.cc3 = [clamp(c.bottomFade, 0, 1), clamp(c.bottomFadeHeight, 0, 1), radius, 0];
      let blurH: Params | null = null, blurV: Params | null = null;
      if (radius >= 0.5) {
        blurH = { ...prm, blurDir: [1, 0, 0, 0] };
        blurV = { ...prm, blurDir: [0, 1, 0, 0] };
      }
      passes.push({ kind: "adjust", params: prm, blurH, blurV });
    }
  }
  return passes;
}
