import { useRef, useState, type Dispatch } from "react";
import type { Layer, Project } from "../types";
import type { Action } from "../lib/store";
import { layerEnd, projectDuration } from "../lib/layout";

interface Props {
  project: Project;
  selectedId: string | null;
  time: number;
  dispatch: Dispatch<Action>;
}

const ICON: Record<Layer["kind"]["type"], string> = { video: "🎬", adjustment: "🎚", rectangle: "▭" };

function fmt(t: number) {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(2).padStart(5, "0")}`;
}

/**
 * Multi-layer timeline. Rows are drawn top layer first (like Alight Motion);
 * the raw video sits on the bottom track. Drag the ⠿ handle to reorder,
 * drag a clip to move it in time, click the ruler to scrub.
 */
export function Timeline({ project, selectedId, time, dispatch }: Props) {
  const duration = Math.max(projectDuration(project), 1);
  const trackRef = useRef<HTMLDivElement>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [overIdx, setOverIdx] = useState<number | null>(null);
  const rows = [...project.layers].map((l, i) => ({ l, i })).reverse();

  const xToTime = (clientX: number) => {
    const r = trackRef.current?.getBoundingClientRect();
    if (!r) return 0;
    return Math.max(0, Math.min(duration, ((clientX - r.left) / r.width) * duration));
  };

  const scrub = (e: React.PointerEvent) => {
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    dispatch({ type: "setTime", time: xToTime(e.clientX) });
    const move = (ev: PointerEvent) => dispatch({ type: "setTime", time: xToTime(ev.clientX) });
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const dragClip = (e: React.PointerEvent, l: Layer) => {
    dispatch({ type: "select", id: l.id });
    if (l.locked) return;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const t0 = xToTime(e.clientX);
    const s0 = l.start;
    const e0 = l.end;
    const move = (ev: PointerEvent) => {
      const d = xToTime(ev.clientX) - t0;
      const start = Math.max(0, s0 + d);
      dispatch({ type: "updateLayer", id: l.id, patch: { start, end: e0 == null ? null : e0 + (start - s0) }, key: `${l.id}:drag` });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const ticks = Array.from({ length: Math.floor(duration) + 1 }, (_, i) => i).filter((i) => duration < 40 || i % 5 === 0);

  return (
    <div className="timeline">
      <div className="tl-header">
        <div className="tl-labels-head">
          <span>Layers ({project.layers.length})</span>
          <span className="timecode">{fmt(time)} / {fmt(duration)}</span>
        </div>
        <div className="tl-ruler" ref={trackRef} onPointerDown={scrub}>
          {ticks.map((s) => (
            <span key={s} className="tick" style={{ left: `${(s / duration) * 100}%` }}>{s}s</span>
          ))}
          <div className="playhead" style={{ left: `${(time / duration) * 100}%` }} />
        </div>
      </div>
      <div className="tl-rows">
        {rows.length === 0 && <div className="tl-empty">Import a video to start. It becomes the bottom track; CC adjustment layers stack above it.</div>}
        {rows.map(({ l, i }) => {
          const end = layerEnd(l) ?? duration;
          return (
            <div
              key={l.id}
              className={`tl-row ${selectedId === l.id ? "selected" : ""} ${!l.visible ? "hidden" : ""} ${overIdx === i ? "drop-target" : ""} ${l.kind.type}`}
              onClick={() => dispatch({ type: "select", id: l.id })}
              onDragOver={(e) => {
                if (dragId) {
                  e.preventDefault();
                  setOverIdx(i);
                }
              }}
              onDragLeave={() => setOverIdx(null)}
              onDrop={(e) => {
                e.preventDefault();
                if (dragId) dispatch({ type: "moveLayer", id: dragId, toIndex: i });
                setDragId(null);
                setOverIdx(null);
              }}
            >
              <div className="tl-label">
                <span
                  className="handle"
                  draggable
                  title="Drag to reorder"
                  onDragStart={(e) => {
                    setDragId(l.id);
                    e.dataTransfer.effectAllowed = "move";
                  }}
                  onDragEnd={() => {
                    setDragId(null);
                    setOverIdx(null);
                  }}
                >
                  ⠿
                </span>
                <button
                  className={`icon ${l.visible ? "" : "off"}`}
                  title={l.visible ? "Hide layer" : "Show layer"}
                  onClick={(e) => {
                    e.stopPropagation();
                    dispatch({ type: "toggleVisible", id: l.id });
                  }}
                >
                  {l.visible ? "👁" : "◌"}
                </button>
                <button
                  className={`icon ${l.locked ? "on" : ""}`}
                  title={l.locked ? "Unlock layer" : "Lock layer"}
                  onClick={(e) => {
                    e.stopPropagation();
                    dispatch({ type: "toggleLock", id: l.id });
                  }}
                >
                  {l.locked ? "🔒" : "🔓"}
                </button>
                <span className="kind">{ICON[l.kind.type]}</span>
                <span className="lname" title={l.name}>{l.name}</span>
                <span className="blend">{l.blend !== "normal" ? l.blend : ""}</span>
                <button className="icon" title="Move up" disabled={i === project.layers.length - 1}
                  onClick={(e) => { e.stopPropagation(); dispatch({ type: "moveLayer", id: l.id, toIndex: i + 1 }); }}>▲</button>
                <button className="icon" title="Move down" disabled={i === 0}
                  onClick={(e) => { e.stopPropagation(); dispatch({ type: "moveLayer", id: l.id, toIndex: i - 1 }); }}>▼</button>
                <button className="icon danger" title="Delete layer" disabled={l.locked}
                  onClick={(e) => { e.stopPropagation(); dispatch({ type: "removeLayer", id: l.id }); }}>✕</button>
              </div>
              <div className="tl-track">
                <div
                  className={`clip ${l.locked ? "locked" : ""}`}
                  style={{ left: `${(l.start / duration) * 100}%`, width: `${(Math.max(0.05, end - l.start) / duration) * 100}%` }}
                  onPointerDown={(e) => dragClip(e, l)}
                >
                  {l.name}
                </div>
                <div className="playhead" style={{ left: `${(time / duration) * 100}%` }} />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
