import { useCallback, useEffect, useRef, useState } from "react";
import type { AspectRatio, Layer, Project } from "./types";
import { DEFAULT_CC } from "./types";
import { newAdjustment, newId, newLyricsFade, newRectangle, useEditor } from "./lib/store";
import { aspectOf, fitLayer } from "./lib/layout";
import * as api from "./lib/api";
import { Viewer } from "./components/Viewer";
import { Timeline } from "./components/Timeline";
import { Inspector } from "./components/Inspector";
import { ExportDialog } from "./components/ExportDialog";

const ASPECTS: AspectRatio[] = ["9:16", "16:9", "1:1", "4:5"];

export default function App() {
  const [state, dispatch] = useEditor();
  const { project, selectedId, time, playing, videoFit, previewScale } = state;
  const [mediaUrls, setMediaUrls] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("Ready");
  const [showExport, setShowExport] = useState(false);
  const [projectPath, setProjectPath] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const tauri = api.isTauri();
  const selected = project.layers.find((l) => l.id === selectedId) ?? null;

  // ---- media -------------------------------------------------------------
  const addVideoLayer = useCallback(
    (layer: Layer, url: string) => {
      setMediaUrls((m) => ({ ...m, [layer.id]: url }));
      const hasVideo = project.layers.some((l) => l.kind.type === "video");
      // Raw video goes on the BOTTOM track.
      dispatch({ type: "addLayer", layer: fitLayer(layer, project.composition, videoFit), index: hasVideo ? undefined : 0 });
    },
    [project, videoFit, dispatch],
  );

  const importVideo = async () => {
    if (!tauri) return fileInput.current?.click();
    const path = await api.pickVideo();
    if (!path) return;
    setStatus("Probing video…");
    try {
      const info = await api.probeVideo(path);
      const layer: Layer = {
        id: newId("video"),
        name: path.split(/[\\/]/).pop() ?? "Video",
        kind: { type: "video", path, srcWidth: info.width, srcHeight: info.height, duration: info.duration, fps: info.fps, hasAudio: info.hasAudio },
        visible: true, locked: false, opacity: 1, blend: "normal",
        transform: { x: 0, y: 0, width: info.width, height: info.height, rotation: 0 },
        cc: { ...DEFAULT_CC }, start: 0, end: null, rawEffects: [],
      };
      addVideoLayer(layer, api.videoUrl(path));
      setStatus(`Imported ${info.width}×${info.height} ${info.fps.toFixed(2)} fps ${info.codec} (${info.duration.toFixed(1)} s)`);
    } catch (e) {
      setStatus(`Video import failed: ${e}`);
    }
  };

  // Browser (non-Tauri) dev mode: read the file locally.
  const onBrowserFile = (f: File) => {
    const url = URL.createObjectURL(f);
    const v = document.createElement("video");
    v.preload = "metadata";
    v.onloadedmetadata = () => {
      const layer: Layer = {
        id: newId("video"), name: f.name,
        kind: { type: "video", path: f.name, srcWidth: v.videoWidth, srcHeight: v.videoHeight, duration: v.duration, fps: 30, hasAudio: true },
        visible: true, locked: false, opacity: 1, blend: "normal",
        transform: { x: 0, y: 0, width: v.videoWidth, height: v.videoHeight, rotation: 0 },
        cc: { ...DEFAULT_CC }, start: 0, end: null, rawEffects: [],
      };
      addVideoLayer(layer, url);
      setStatus(`Imported ${f.name} (browser preview mode)`);
    };
    v.src = url;
  };

  const onTainted = useCallback(
    async (layerId: string) => {
      const l = project.layers.find((x) => x.id === layerId);
      if (!tauri || !l || l.kind.type !== "video" || mediaUrls[layerId]?.startsWith("blob:")) return;
      setStatus("Switching preview to in-memory video…");
      try {
        const url = await api.videoBlobUrl(l.kind.path);
        setMediaUrls((m) => ({ ...m, [layerId]: url }));
      } catch (e) {
        setStatus(`Preview could not load the video: ${e}`);
      }
    },
    [project.layers, mediaUrls, tauri],
  );

  const importPreset = async () => {
    if (!tauri) return setStatus("Preset import needs the desktop app (npm run tauri dev)");
    const path = await api.pickPreset();
    if (!path) return;
    try {
      const preset = await api.parsePreset(path);
      const sx = project.composition.width / preset.width;
      const sy = project.composition.height / preset.height;
      const layers = preset.layers.map((l) => ({
        ...l,
        id: newId("am"),
        transform: { ...l.transform, x: l.transform.x * sx, y: l.transform.y * sy, width: l.transform.width * sx, height: l.transform.height * sy },
        cc: { ...l.cc, blur: l.cc.blur * Math.min(sx, sy) },
      }));
      dispatch({ type: "addLayers", layers });
      const s = preset.stats;
      setStatus(
        `Preset "${preset.title}": ${s.adjustments} adjustment + ${s.rectangles} rectangle layers, ${s.effectsMapped}/${s.effectsTotal} effects mapped` +
          (preset.warnings.length ? ` · ${preset.warnings.length} warnings (see console)` : ""),
      );
      if (preset.warnings.length) console.warn("Preset warnings:\n" + preset.warnings.join("\n"));
    } catch (e) {
      setStatus(`Preset import failed: ${e}`);
    }
  };

  // ---- project files -------------------------------------------------------------
  const saveAs = async () => {
    if (!tauri) {
      const blob = new Blob([JSON.stringify(project, null, 2)], { type: "application/json" });
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = "project.mlproj";
      a.click();
      return;
    }
    const path = projectPath ?? (await api.pickProjectSave());
    if (!path) return;
    await api.saveProject(path, project);
    setProjectPath(path);
    setStatus(`Saved ${path}`);
  };

  const openProject = async () => {
    if (!tauri) return;
    const path = await api.pickProjectOpen();
    if (!path) return;
    try {
      const p: Project = await api.loadProject(path);
      const urls: Record<string, string> = {};
      for (const l of p.layers) if (l.kind.type === "video") urls[l.id] = api.videoUrl(l.kind.path);
      setMediaUrls(urls);
      dispatch({ type: "setProject", project: p });
      setProjectPath(path);
      setStatus(`Opened ${path}`);
    } catch (e) {
      setStatus(`Open failed: ${e}`);
    }
  };

  // ---- keyboard ------------------------------------------------------------------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      if (e.code === "Space") {
        e.preventDefault();
        dispatch({ type: "setPlaying", playing: !playing });
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        dispatch({ type: e.shiftKey ? "redo" : "undo" });
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
        dispatch({ type: "redo" });
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        saveAs();
      } else if (selectedId && (e.key === "Delete" || e.key === "Backspace")) {
        dispatch({ type: "removeLayer", id: selectedId });
      } else if (selectedId && e.key.toLowerCase() === "h") {
        dispatch({ type: "toggleVisible", id: selectedId });
      } else if (selectedId && e.key.toLowerCase() === "l") {
        dispatch({ type: "toggleLock", id: selectedId });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const currentAspect = aspectOf(project.composition);

  return (
    <div className="app">
      <header className="toolbar">
        <div className="brand">MyLoader <span>GPU CC</span></div>
        <div className="group">
          <button onClick={importVideo}>＋ Video</button>
          <button onClick={importPreset} title="Import Alight Motion .xml preset">＋ AM Preset (.xml)</button>
          <button onClick={() => dispatch({ type: "addLayer", layer: newAdjustment(project) })}>＋ Adjustment</button>
          <button onClick={() => dispatch({ type: "addLayer", layer: newLyricsFade(project) })}>＋ Lyrics Fade</button>
          <button onClick={() => dispatch({ type: "addLayer", layer: newRectangle(project) })}>＋ Rectangle</button>
        </div>
        <div className="group aspect">
          {ASPECTS.map((a) => (
            <button key={a} className={currentAspect === a ? "active" : ""} onClick={() => dispatch({ type: "setAspect", aspect: a })}>
              {a}
            </button>
          ))}
        </div>
        <div className="group">
          <select value={videoFit} onChange={(e) => dispatch({ type: "setVideoFit", fit: e.target.value as typeof videoFit })} title="How the video is fitted">
            <option value="cover">Video: Fill (crop)</option>
            <option value="contain">Video: Fit (bars)</option>
            <option value="stretch">Video: Stretch</option>
          </select>
          <button className="accent" onClick={() => { dispatch({ type: "fillAll" }); setStatus("All unlocked layers fitted to the composition"); }}>
            ⤢ Fill All Layers to Composition
          </button>
        </div>
        <span className="spacer" />
        <div className="group">
          <button onClick={() => dispatch({ type: "undo" })} disabled={!state.past.length} title="Ctrl+Z">↶</button>
          <button onClick={() => dispatch({ type: "redo" })} disabled={!state.future.length} title="Ctrl+Y">↷</button>
          {tauri && <button onClick={openProject}>Open</button>}
          <button onClick={saveAs}>Save</button>
          <button className="primary" onClick={() => setShowExport(true)}>Export ⇪</button>
        </div>
        <input ref={fileInput} type="file" accept="video/*" hidden onChange={(e) => e.target.files?.[0] && onBrowserFile(e.target.files[0])} />
      </header>

      <main className="workspace">
        <Viewer project={project} time={time} playing={playing} previewScale={previewScale} mediaUrls={mediaUrls} dispatch={dispatch} onTainted={onTainted} />
        <Inspector layer={selected} dispatch={dispatch} />
      </main>

      <Timeline project={project} selectedId={selectedId} time={time} dispatch={dispatch} />
      <footer className="status">{status}</footer>
      {showExport && <ExportDialog project={project} onClose={() => setShowExport(false)} />}
    </div>
  );
}
