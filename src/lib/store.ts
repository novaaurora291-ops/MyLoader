import { useReducer } from "react";
import type { AspectRatio, CcParams, FitMode, Layer, Project } from "../types";
import { DEFAULT_CC } from "../types";
import { fillAllLayers, fitLayer, setAspect } from "./layout";

export interface State {
  project: Project;
  selectedId: string | null;
  time: number;
  playing: boolean;
  videoFit: FitMode;
  previewScale: number;
  past: Project[];
  future: Project[];
  lastKey: string | null;
  lastAt: number;
}

export type Action =
  | { type: "setProject"; project: Project }
  | { type: "addLayer"; layer: Layer; index?: number }
  | { type: "addLayers"; layers: Layer[] }
  | { type: "updateLayer"; id: string; patch: Partial<Layer>; key?: string }
  | { type: "updateCc"; id: string; patch: Partial<CcParams>; key?: string }
  | { type: "removeLayer"; id: string }
  | { type: "moveLayer"; id: string; toIndex: number }
  | { type: "toggleVisible"; id: string }
  | { type: "toggleLock"; id: string }
  | { type: "select"; id: string | null }
  | { type: "setTime"; time: number }
  | { type: "setPlaying"; playing: boolean }
  | { type: "setAspect"; aspect: AspectRatio }
  | { type: "fillAll" }
  | { type: "fitLayer"; id: string }
  | { type: "setVideoFit"; fit: FitMode }
  | { type: "setPreviewScale"; scale: number }
  | { type: "undo" }
  | { type: "redo" };

export const newId = (prefix: string) =>
  `${prefix}-${(globalThis.crypto?.randomUUID?.() ?? Math.random().toString(36).slice(2)).slice(0, 8)}`;

export function emptyProject(): Project {
  return {
    version: 1,
    composition: { width: 1080, height: 1920, fps: 30, duration: 0, background: [0, 0, 0, 1] },
    layers: [],
  };
}

export function newAdjustment(p: Project, name = "CC Adjustment"): Layer {
  const c = p.composition;
  return {
    id: newId("adj"), name, kind: { type: "adjustment" }, visible: true, locked: false, opacity: 1, blend: "normal",
    transform: { x: c.width / 2, y: c.height / 2, width: c.width, height: c.height, rotation: 0 },
    cc: { ...DEFAULT_CC }, start: 0, end: null, rawEffects: [],
  };
}

export function newRectangle(p: Project): Layer {
  const c = p.composition;
  return {
    id: newId("rect"), name: "Rectangle", kind: { type: "rectangle", color: [0, 0, 0, 1], cornerRadius: 0 },
    visible: true, locked: false, opacity: 0.4, blend: "multiply",
    transform: { x: c.width / 2, y: c.height / 2, width: c.width, height: c.height, rotation: 0 },
    cc: { ...DEFAULT_CC }, start: 0, end: null, rawEffects: [],
  };
}

/** Lyrics-friendly bottom black fade adjustment layer. */
export function newLyricsFade(p: Project): Layer {
  const l = newAdjustment(p, "Lyrics Fade");
  l.cc = { ...l.cc, bottomFade: 0.85, bottomFadeHeight: 0.4 };
  return l;
}

const COALESCE_MS = 600;

function withHistory(s: State, project: Project, key: string | null = null): State {
  const now = Date.now();
  const coalesce = key !== null && key === s.lastKey && now - s.lastAt < COALESCE_MS;
  return {
    ...s,
    project,
    past: coalesce ? s.past : [...s.past.slice(-79), s.project],
    future: [],
    lastKey: key,
    lastAt: now,
  };
}

function mapLayer(p: Project, id: string, f: (l: Layer) => Layer): Project {
  return { ...p, layers: p.layers.map((l) => (l.id === id ? f(l) : l)) };
}

export function reducer(s: State, a: Action): State {
  const p = s.project;
  switch (a.type) {
    case "setProject":
      return { ...withHistory(s, a.project), selectedId: null, time: 0 };
    case "addLayer": {
      const layers = [...p.layers];
      layers.splice(a.index ?? layers.length, 0, a.layer);
      return { ...withHistory(s, { ...p, layers }), selectedId: a.layer.id };
    }
    case "addLayers":
      return { ...withHistory(s, { ...p, layers: [...p.layers, ...a.layers] }), selectedId: a.layers.at(-1)?.id ?? s.selectedId };
    case "updateLayer": {
      const target = p.layers.find((l) => l.id === a.id);
      const onlyFlags = Object.keys(a.patch).every((k) => k === "locked" || k === "visible");
      if (!target || (target.locked && !onlyFlags)) return s;
      return withHistory(s, mapLayer(p, a.id, (l) => ({ ...l, ...a.patch })), a.key ?? null);
    }
    case "updateCc": {
      const target = p.layers.find((l) => l.id === a.id);
      if (!target || target.locked) return s;
      return withHistory(s, mapLayer(p, a.id, (l) => ({ ...l, cc: { ...l.cc, ...a.patch } })), a.key ?? null);
    }
    case "removeLayer": {
      const target = p.layers.find((l) => l.id === a.id);
      if (!target || target.locked) return s;
      return { ...withHistory(s, { ...p, layers: p.layers.filter((l) => l.id !== a.id) }), selectedId: s.selectedId === a.id ? null : s.selectedId };
    }
    case "moveLayer": {
      const from = p.layers.findIndex((l) => l.id === a.id);
      if (from < 0) return s;
      const to = Math.max(0, Math.min(p.layers.length - 1, a.toIndex));
      if (to === from) return s;
      const layers = [...p.layers];
      const [l] = layers.splice(from, 1);
      layers.splice(to, 0, l);
      return withHistory(s, { ...p, layers });
    }
    case "toggleVisible":
      return withHistory(s, mapLayer(p, a.id, (l) => ({ ...l, visible: !l.visible })));
    case "toggleLock":
      return withHistory(s, mapLayer(p, a.id, (l) => ({ ...l, locked: !l.locked })));
    case "select":
      return { ...s, selectedId: a.id };
    case "setTime":
      return { ...s, time: Math.max(0, a.time) };
    case "setPlaying":
      return { ...s, playing: a.playing };
    case "setAspect":
      return withHistory(s, setAspect(p, a.aspect));
    case "fillAll":
      return withHistory(s, fillAllLayers(p, s.videoFit));
    case "fitLayer":
      return withHistory(s, mapLayer(p, a.id, (l) => (l.locked ? l : fitLayer(l, p.composition, s.videoFit))));
    case "setVideoFit":
      return { ...s, videoFit: a.fit };
    case "setPreviewScale":
      return { ...s, previewScale: a.scale };
    case "undo": {
      const prev = s.past.at(-1);
      if (!prev) return s;
      return { ...s, project: prev, past: s.past.slice(0, -1), future: [p, ...s.future], lastKey: null };
    }
    case "redo": {
      const next = s.future[0];
      if (!next) return s;
      return { ...s, project: next, past: [...s.past, p], future: s.future.slice(1), lastKey: null };
    }
  }
}

export function useEditor() {
  return useReducer(reducer, undefined, (): State => ({
    project: emptyProject(),
    selectedId: null,
    time: 0,
    playing: false,
    videoFit: "cover",
    previewScale: 0.5,
    past: [],
    future: [],
    lastKey: null,
    lastAt: 0,
  }));
}
