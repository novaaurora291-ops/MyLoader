import type { Dispatch } from "react";
import type { CcParams, Layer } from "../types";
import { BLEND_LABELS, BLEND_MODES, DEFAULT_CC } from "../types";
import type { Action } from "../lib/store";
import { Slider } from "./Slider";

interface Props {
  layer: Layer | null;
  dispatch: Dispatch<Action>;
}

const CC_SLIDERS: { key: keyof CcParams; label: string; min: number; max: number; step?: number; unit?: string; group: string }[] = [
  { key: "exposure", label: "Exposure", min: -4, max: 4, step: 0.01, unit: "EV", group: "Light" },
  { key: "brightness", label: "Brightness", min: -1, max: 1, group: "Light" },
  { key: "contrast", label: "Contrast", min: -1, max: 3, group: "Light" },
  { key: "highlights", label: "Highlights (HDR)", min: -1, max: 1, group: "HDR" },
  { key: "shadows", label: "Shadows (HDR)", min: -1, max: 1, group: "HDR" },
  { key: "saturation", label: "Saturation", min: -1, max: 1, group: "Colour" },
  { key: "blur", label: "Gaussian Blur", min: 0, max: 100, step: 0.5, unit: "px", group: "Blur" },
  { key: "vignette", label: "Vignette", min: 0, max: 1, group: "Vignette & Fade" },
  { key: "vignetteSoftness", label: "Vignette Softness", min: 0, max: 1, group: "Vignette & Fade" },
  { key: "bottomFade", label: "Bottom Black Fade", min: 0, max: 1, group: "Vignette & Fade" },
  { key: "bottomFadeHeight", label: "Fade Height", min: 0.05, max: 1, group: "Vignette & Fade" },
];

const toHex = (c: number[]) => "#" + c.slice(0, 3).map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("");
const fromHex = (h: string): [number, number, number] => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as [number, number, number];

export function Inspector({ layer, dispatch }: Props) {
  if (!layer) {
    return (
      <aside className="inspector empty">
        <p>Select a layer in the timeline.</p>
        <p className="hint">Tip: import a video, then an Alight Motion preset (.xml) or add CC adjustment layers on top.</p>
      </aside>
    );
  }
  const id = layer.id;
  const locked = layer.locked;
  const upd = (patch: Partial<Layer>, key?: string) => dispatch({ type: "updateLayer", id, patch, key });
  const cc = (patch: Partial<CcParams>, key: string) => dispatch({ type: "updateCc", id, patch, key: `${id}:${key}` });
  const t = layer.transform;
  const setT = (k: keyof typeof t, v: number) => upd({ transform: { ...t, [k]: v } }, `${id}:t:${k}`);
  const groups = [...new Set(CC_SLIDERS.map((s) => s.group))];

  return (
    <aside className="inspector">
      <header>
        <input className="name" value={layer.name} disabled={locked} onChange={(e) => upd({ name: e.target.value }, `${id}:name`)} />
        <span className={`badge ${layer.kind.type}`}>{layer.kind.type}</span>
      </header>
      {locked && <div className="locked-note">🔒 Layer is locked - unlock it in the timeline to edit.</div>}

      <section>
        <h4>Blending</h4>
        <label className="row">
          <span>Mode</span>
          <select value={layer.blend} disabled={locked} onChange={(e) => upd({ blend: e.target.value as Layer["blend"] })}>
            {BLEND_MODES.map((m) => (
              <option key={m} value={m}>{BLEND_LABELS[m]}</option>
            ))}
          </select>
        </label>
        <Slider label="Opacity" value={layer.opacity} min={0} max={1} defaultValue={1} disabled={locked} onChange={(v) => upd({ opacity: v }, `${id}:opacity`)} />
      </section>

      {layer.kind.type === "adjustment" &&
        groups.map((g) => (
          <section key={g}>
            <h4>{g}</h4>
            {CC_SLIDERS.filter((s) => s.group === g).map((s) => (
              <Slider
                key={s.key}
                label={s.label}
                value={layer.cc[s.key]}
                min={s.min}
                max={s.max}
                step={s.step}
                unit={s.unit}
                defaultValue={DEFAULT_CC[s.key]}
                disabled={locked}
                onChange={(v) => cc({ [s.key]: v }, s.key)}
              />
            ))}
          </section>
        ))}

      {layer.kind.type === "adjustment" && (
        <section>
          <button disabled={locked} onClick={() => dispatch({ type: "updateCc", id, patch: { ...DEFAULT_CC } })}>Reset all CC</button>
        </section>
      )}

      {layer.kind.type === "rectangle" && (
        <section>
          <h4>Fill</h4>
          <label className="row">
            <span>Colour</span>
            <input
              type="color"
              disabled={locked}
              value={toHex(layer.kind.color)}
              onChange={(e) => {
                if (layer.kind.type !== "rectangle") return;
                upd({ kind: { ...layer.kind, color: [...fromHex(e.target.value), layer.kind.color[3]] } }, `${id}:color`);
              }}
            />
          </label>
          <Slider
            label="Corner radius"
            value={layer.kind.cornerRadius}
            min={0}
            max={500}
            step={1}
            unit="px"
            disabled={locked}
            onChange={(v) => layer.kind.type === "rectangle" && upd({ kind: { ...layer.kind, cornerRadius: v } }, `${id}:radius`)}
          />
        </section>
      )}

      {layer.kind.type === "video" && (
        <section>
          <h4>Source</h4>
          <div className="meta" title={layer.kind.path}>{layer.kind.path}</div>
          <div className="meta">
            {layer.kind.srcWidth}×{layer.kind.srcHeight} · {layer.kind.fps.toFixed(2)} fps · {layer.kind.duration.toFixed(2)} s{layer.kind.hasAudio ? " · audio" : ""}
          </div>
        </section>
      )}

      <section>
        <h4>Transform</h4>
        <div className="grid2">
          {(["x", "y", "width", "height", "rotation"] as const).map((k) => (
            <label key={k} className="num">
              <span>{k === "rotation" ? "rot°" : k}</span>
              <input type="number" value={Math.round(t[k] * 10) / 10} disabled={locked} onChange={(e) => setT(k, parseFloat(e.target.value) || 0)} />
            </label>
          ))}
        </div>
        <button disabled={locked} onClick={() => dispatch({ type: "fitLayer", id })}>Fit this layer to composition</button>
      </section>

      <section>
        <h4>Timing</h4>
        <div className="grid2">
          <label className="num">
            <span>start s</span>
            <input type="number" step={0.1} value={layer.start} disabled={locked} onChange={(e) => upd({ start: Math.max(0, parseFloat(e.target.value) || 0) }, `${id}:start`)} />
          </label>
          <label className="num">
            <span>end s</span>
            <input
              type="number"
              step={0.1}
              placeholder="auto"
              value={layer.end ?? ""}
              disabled={locked}
              onChange={(e) => upd({ end: e.target.value === "" ? null : parseFloat(e.target.value) }, `${id}:end`)}
            />
          </label>
        </div>
      </section>

      {layer.rawEffects.length > 0 && (
        <section>
          <h4>Unsupported preset effects</h4>
          {layer.rawEffects.map((e, i) => (
            <div key={i} className="meta" title={JSON.stringify(e.params)}>{e.id.replace("com.alightcreative.effects.", "")}</div>
          ))}
        </section>
      )}
    </aside>
  );
}
