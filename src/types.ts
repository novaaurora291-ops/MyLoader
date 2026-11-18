// Mirrors crates/engine/src/model.rs (serde camelCase). Keep in sync.

export type BlendMode =
  | "normal" | "multiply" | "screen" | "overlay" | "add" | "softLight"
  | "hardLight" | "darken" | "lighten" | "colorDodge" | "colorBurn" | "difference";

export const BLEND_MODES: BlendMode[] = [
  "normal", "multiply", "screen", "overlay", "add", "softLight",
  "hardLight", "darken", "lighten", "colorDodge", "colorBurn", "difference",
];

export const BLEND_LABELS: Record<BlendMode, string> = {
  normal: "Normal", multiply: "Multiply", screen: "Screen", overlay: "Overlay", add: "Add",
  softLight: "Soft Light", hardLight: "Hard Light", darken: "Darken", lighten: "Lighten",
  colorDodge: "Color Dodge", colorBurn: "Color Burn", difference: "Difference",
};

export interface Transform { x: number; y: number; width: number; height: number; rotation: number }

export interface CcParams {
  brightness: number;
  contrast: number;
  exposure: number;
  saturation: number;
  highlights: number;
  shadows: number;
  blur: number;
  vignette: number;
  vignetteSoftness: number;
  bottomFade: number;
  bottomFadeHeight: number;
}

export const DEFAULT_CC: CcParams = {
  brightness: 0, contrast: 0, exposure: 0, saturation: 0, highlights: 0, shadows: 0,
  blur: 0, vignette: 0, vignetteSoftness: 0.5, bottomFade: 0, bottomFadeHeight: 0.35,
};

export type LayerKind =
  | { type: "video"; path: string; srcWidth: number; srcHeight: number; duration: number; fps: number; hasAudio: boolean }
  | { type: "adjustment" }
  | { type: "rectangle"; color: [number, number, number, number]; cornerRadius: number };

export interface RawEffect { id: string; params: Record<string, string> }

export interface Layer {
  id: string;
  name: string;
  kind: LayerKind;
  visible: boolean;
  locked: boolean;
  opacity: number;
  blend: BlendMode;
  transform: Transform;
  cc: CcParams;
  start: number;
  end: number | null;
  rawEffects: RawEffect[];
}

export interface Composition {
  width: number;
  height: number;
  fps: number;
  duration: number;
  background: [number, number, number, number];
}

export interface Project { version: number; composition: Composition; layers: Layer[] }

export interface VideoInfo {
  path: string; width: number; height: number; fps: number; duration: number;
  frames: number; codec: string; hasAudio: boolean; rotation: number;
}

export interface ParsedPreset {
  title: string; width: number; height: number; fps: number; duration: number;
  background: [number, number, number, number];
  layers: Layer[];
  warnings: string[];
  stats: { elements: number; adjustments: number; rectangles: number; skipped: number; effectsTotal: number; effectsMapped: number };
}

export type EncoderId = "h264_nvenc" | "hevc_nvenc" | "h264_amf" | "hevc_amf" | "h264_qsv" | "hevc_qsv" | "libx264";

export interface EncoderInfo { id: EncoderId; name: string; label: string; hardware: boolean; available: boolean }

export interface ExportSettings {
  output: string;
  shortEdge: number | null;
  fps: number | null;
  encoder: EncoderId | null;
  quality: number;
  bitrateKbps: number | null;
  includeAudio: boolean;
  forceCpuRender: boolean;
}

export interface ExportProgress {
  frame: number; totalFrames: number; renderFps: number; elapsed: number; eta: number; renderer: string; encoder: string;
}

export interface ExportReport {
  output: string; width: number; height: number; fps: number; frames: number;
  seconds: number; averageFps: number; renderer: string; encoder: string;
}

export type AspectRatio = "9:16" | "16:9" | "1:1" | "4:5";
export type FitMode = "cover" | "contain" | "stretch";
