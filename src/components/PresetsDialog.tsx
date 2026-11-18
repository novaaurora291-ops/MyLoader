import { useState } from "react";
import type { UserPreset } from "../lib/looks";

interface Props {
  presets: UserPreset[];
  layerCount: number;
  onSave: (name: string) => void;
  onDelete: (name: string) => void;
  onApply: (p: UserPreset) => void;
  onExportFile: () => void;
  onImportFile: () => void;
  onClose: () => void;
}

/** Save the current CC stack as a reusable preset, manage saved presets. */
export function PresetsDialog({ presets, layerCount, onSave, onDelete, onApply, onExportFile, onImportFile, onClose }: Props) {
  const [name, setName] = useState(`My Look ${presets.length + 1}`);
  const exists = presets.some((p) => p.name === name.trim());
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>My Presets</h3>
        <div className="dim small">Saves every non-video layer (CC adjustments, rectangles, fades) so you can apply it to any video in one click.</div>
        <label className="row" style={{ marginTop: 10 }}>
          <span>Name</span>
          <input className="name" style={{ flex: 1 }} value={name} onChange={(e) => setName(e.target.value)} autoFocus
            onKeyDown={(e) => e.key === "Enter" && name.trim() && layerCount && onSave(name.trim())} />
          <button className="primary" disabled={!name.trim() || !layerCount} onClick={() => onSave(name.trim())}>
            {exists ? "Overwrite" : "Save"}
          </button>
        </label>
        {!layerCount && <div className="warn">Add some CC layers (or a look) first; there is nothing to save yet.</div>}
        <div className="preset-list">
          {presets.length === 0 && <div className="dim small">No saved presets yet.</div>}
          {presets.map((p) => (
            <div key={p.name} className="preset-item">
              <span className="pname">{p.name}</span>
              <span className="dim small">{p.layers.length} layers · {p.width}×{p.height}</span>
              <button onClick={() => onApply(p)}>Apply</button>
              <button className="danger" onClick={() => onDelete(p.name)} title="Delete">✕</button>
            </div>
          ))}
        </div>
        <div className="actions">
          <button onClick={onImportFile}>📂 Import file…</button>
          <button onClick={onExportFile} disabled={!layerCount}>💾 Export file…</button>
          <span className="spacer" />
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
