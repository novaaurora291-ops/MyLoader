import { useEffect, useRef, useState, type Dispatch } from "react";
import type { Project } from "../types";
import type { Action } from "../lib/store";
import { PreviewRenderer } from "../engine/PreviewRenderer";
import { activeAt, layerEnd, projectDuration } from "../lib/layout";

interface Props {
  project: Project;
  time: number;
  playing: boolean;
  previewScale: number;
  mediaUrls: Record<string, string>;
  dispatch: Dispatch<Action>;
  onTainted: (layerId: string) => void;
}

/**
 * Real-time GPU preview. Hidden <video> elements decode (hardware accelerated
 * by WebView2), the WebGL2 renderer composites + grades every frame.
 */
export function Viewer({ project, time, playing, previewScale, mediaUrls, dispatch, onTainted }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<PreviewRenderer | null>(null);
  const videosRef = useRef(new Map<string, HTMLVideoElement>());
  const stateRef = useRef({ project, time, playing, previewScale });
  stateRef.current = { project, time, playing, previewScale };
  const [stats, setStats] = useState({ ms: 0, gpu: "" });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!canvasRef.current) return;
    try {
      const r = new PreviewRenderer(canvasRef.current);
      r.onTaintedVideo = onTainted;
      rendererRef.current = r;
      setStats((s) => ({ ...s, gpu: r.gpuInfo }));
    } catch (e) {
      setError(String(e));
    }
    return () => rendererRef.current?.dispose();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Main loop: advance the clock while playing, sync videos, render.
  useEffect(() => {
    let raf = 0;
    let last = performance.now();
    let frameCount = 0;
    let msAcc = 0;
    const loop = (now: number) => {
      raf = requestAnimationFrame(loop);
      const dt = (now - last) / 1000;
      last = now;
      const st = stateRef.current;
      const p = st.project;
      const duration = projectDuration(p);
      let t = st.time;

      if (st.playing) {
        // Use the first active video as master clock (no A/V drift), else wall clock.
        const master = p.layers.find((l) => l.kind.type === "video" && activeAt(l, t) && videosRef.current.get(l.id));
        const mv = master ? videosRef.current.get(master.id) : undefined;
        if (master && mv && !mv.paused && mv.readyState >= 2) t = master.start + mv.currentTime;
        else t = t + dt;
        if (t >= duration) t = 0; // loop
        if (Math.abs(t - st.time) > 1e-4) dispatch({ type: "setTime", time: t });
      }

      for (const l of p.layers) {
        if (l.kind.type !== "video") continue;
        const v = videosRef.current.get(l.id);
        if (!v) continue;
        const local = t - l.start;
        const end = layerEnd(l);
        const inRange = local >= 0 && (end == null || t < end) && l.visible;
        if (!inRange) {
          if (!v.paused) v.pause();
          continue;
        }
        if (st.playing) {
          if (Math.abs(v.currentTime - local) > 0.25) v.currentTime = local;
          if (v.paused) v.play().catch(() => {});
        } else {
          if (!v.paused) v.pause();
          if (Math.abs(v.currentTime - local) > 0.01 && !v.seeking) v.currentTime = local;
        }
      }

      const r = rendererRef.current;
      if (r) {
        msAcc += r.render(p, t, videosRef.current, st.previewScale);
        if (++frameCount === 30) {
          const ms = msAcc / frameCount;
          setStats((s) => ({ ...s, ms }));
          frameCount = 0;
          msAcc = 0;
        }
      }
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [dispatch]);

  const videoLayers = project.layers.filter((l) => l.kind.type === "video");
  const c = project.composition;

  return (
    <div className="viewer">
      <div className="canvas-wrap">
        <canvas ref={canvasRef} style={{ aspectRatio: `${c.width} / ${c.height}` }} />
        {error && <div className="viewer-error">{error}</div>}
      </div>
      <div className="viewer-bar">
        <button className="play" onClick={() => dispatch({ type: "setPlaying", playing: !playing })}>{playing ? "⏸" : "▶"}</button>
        <button onClick={() => dispatch({ type: "setTime", time: 0 })}>⏮</button>
        <span className="dim">{c.width}×{c.height} @ {c.fps} fps</span>
        <span className="spacer" />
        <label className="dim">
          Preview
          <select value={previewScale} onChange={(e) => dispatch({ type: "setPreviewScale", scale: parseFloat(e.target.value) })}>
            <option value={0.25}>¼</option>
            <option value={0.5}>½</option>
            <option value={1}>Full</option>
          </select>
        </label>
        <span className="dim" title={stats.gpu}>GPU {stats.ms.toFixed(1)} ms</span>
      </div>
      <div style={{ display: "none" }}>
        {videoLayers.map((l) =>
          mediaUrls[l.id] ? (
            <video
              key={l.id + mediaUrls[l.id]}
              src={mediaUrls[l.id]}
              crossOrigin="anonymous"
              preload="auto"
              playsInline
              muted={l !== videoLayers.find((v) => v.visible)}
              ref={(el) => {
                if (el) videosRef.current.set(l.id, el);
                else videosRef.current.delete(l.id);
              }}
              onError={() => onTainted(l.id)}
            />
          ) : null,
        )}
      </div>
    </div>
  );
}
