import { useEffect, useState } from "react";
import type { EncoderInfo, ExportProgress, ExportReport, ExportSettings, Project } from "../types";
import { cancelExport, detectEncoders, isTauri, onExportEvents, pickExportPath, startExport } from "../lib/api";

interface Props {
  project: Project;
  onClose: () => void;
}

const RES = [
  { label: "1080p (Full HD)", value: 1080 },
  { label: "1440p (2K)", value: 1440 },
  { label: "2160p (4K UHD)", value: 2160 },
  { label: "Composition size", value: 0 },
];

export function ExportDialog({ project, onClose }: Props) {
  const [encoders, setEncoders] = useState<EncoderInfo[] | null>(null);
  const [settings, setSettings] = useState<ExportSettings>({
    output: "",
    shortEdge: 1080,
    fps: 60,
    encoder: null,
    quality: 19,
    bitrateKbps: null,
    includeAudio: true,
    forceCpuRender: false,
  });
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [report, setReport] = useState<ExportReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    detectEncoders().then(setEncoders).catch((e) => setError(String(e)));
    const un = onExportEvents({
      progress: setProgress,
      done: (r) => {
        setReport(r);
        setRunning(false);
      },
      error: (m) => {
        setError(m);
        setRunning(false);
      },
    });
    return () => {
      un.then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  const set = <K extends keyof ExportSettings>(k: K, v: ExportSettings[K]) => setSettings((s) => ({ ...s, [k]: v }));
  const hasVideo = project.layers.some((l) => l.kind.type === "video");
  const best = encoders?.find((e) => e.available && e.hardware);

  const go = async () => {
    setError(null);
    setReport(null);
    let out = settings.output;
    if (!out) {
      const p = await pickExportPath();
      if (!p) return;
      out = p;
      set("output", p);
    }
    setRunning(true);
    setProgress(null);
    try {
      await startExport(project, { ...settings, output: out, shortEdge: settings.shortEdge || null });
    } catch (e) {
      setError(String(e));
      setRunning(false);
    }
  };

  const pct = progress ? (progress.frame / progress.totalFrames) * 100 : 0;

  return (
    <div className="modal-backdrop" onClick={() => !running && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Export MP4</h3>
        {!isTauri() && <div className="warn">Export runs in the desktop app (npm run tauri dev). The browser preview cannot encode.</div>}

        <label className="row">
          <span>Resolution</span>
          <select value={settings.shortEdge ?? 0} disabled={running} onChange={(e) => set("shortEdge", parseInt(e.target.value) || null)}>
            {RES.map((r) => (
              <option key={r.value} value={r.value}>{r.label}</option>
            ))}
          </select>
        </label>
        <label className="row">
          <span>Frame rate</span>
          <select value={settings.fps ?? 0} disabled={running} onChange={(e) => set("fps", parseFloat(e.target.value) || null)}>
            {[24, 25, 30, 50, 60].map((f) => (
              <option key={f} value={f}>{f} fps</option>
            ))}
            <option value={0}>Composition ({project.composition.fps})</option>
          </select>
        </label>
        <label className="row">
          <span>Encoder</span>
          <select value={settings.encoder ?? ""} disabled={running} onChange={(e) => set("encoder", (e.target.value || null) as ExportSettings["encoder"])}>
            <option value="">Auto {best ? `(${best.label})` : encoders ? "(CPU x264)" : "(detecting…)"}</option>
            {encoders?.map((e) => (
              <option key={e.id} value={e.id} disabled={!e.available}>
                {e.label} {e.available ? "" : "- not available"}
              </option>
            ))}
          </select>
        </label>
        <label className="row">
          <span>Quality ({settings.quality})</span>
          <input type="range" min={14} max={30} value={settings.quality} disabled={running} onChange={(e) => set("quality", parseInt(e.target.value))} />
        </label>
        <div className="dim small">Lower = better. 18–20 is visually lossless.</div>
        <label className="check">
          <input type="checkbox" checked={settings.includeAudio} disabled={running} onChange={(e) => set("includeAudio", e.target.checked)} />
          Include audio from the video layer
        </label>
        <label className="check">
          <input type="checkbox" checked={settings.forceCpuRender} disabled={running} onChange={(e) => set("forceCpuRender", e.target.checked)} />
          Render on CPU (only if the GPU renderer fails)
        </label>
        <label className="row">
          <span>Output</span>
          <input className="path" value={settings.output} placeholder="choose…" disabled={running} onChange={(e) => set("output", e.target.value)} />
          <button disabled={running} onClick={async () => { const p = await pickExportPath(); if (p) set("output", p); }}>…</button>
        </label>

        {(running || progress) && (
          <div className="progress">
            <div className="bar"><div style={{ width: `${pct}%` }} /></div>
            {progress && (
              <div className="dim small">
                {progress.frame}/{progress.totalFrames} frames · {progress.renderFps.toFixed(1)} fps · ETA {progress.eta.toFixed(0)}s
                <br />
                {progress.renderer} → {progress.encoder}
              </div>
            )}
          </div>
        )}
        {report && (
          <div className="ok">
            ✔ {report.output}<br />
            {report.width}×{report.height} @ {report.fps} fps · {report.frames} frames in {report.seconds.toFixed(1)}s ({report.averageFps.toFixed(1)} fps)
          </div>
        )}
        {error && <div className="err">{error}</div>}

        <div className="actions">
          {running ? (
            <button className="danger" onClick={() => cancelExport()}>Cancel</button>
          ) : (
            <>
              <button onClick={onClose}>Close</button>
              <button className="primary" disabled={!isTauri() || !hasVideo} onClick={go}>Export</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
