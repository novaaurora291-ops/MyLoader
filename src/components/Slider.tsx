interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  defaultValue?: number;
  unit?: string;
  disabled?: boolean;
  onChange: (v: number) => void;
}

/** Live slider: drag for real-time GPU preview, double-click to reset, type an exact value. */
export function Slider({ label, value, min, max, step = 0.01, defaultValue = 0, unit = "", disabled, onChange }: Props) {
  const decimals = step >= 1 ? 0 : step >= 0.1 ? 1 : 2;
  const pct = ((value - min) / (max - min)) * 100;
  return (
    <div className={`slider ${disabled ? "disabled" : ""}`} onDoubleClick={() => !disabled && onChange(defaultValue)} title="Double-click to reset">
      <div className="slider-head">
        <span>{label}</span>
        <input
          type="number"
          value={Number(value.toFixed(decimals))}
          step={step}
          min={min}
          max={max}
          disabled={disabled}
          onChange={(e) => {
            const v = parseFloat(e.target.value);
            if (!Number.isNaN(v)) onChange(Math.min(max, Math.max(min, v)));
          }}
        />
        {unit && <span className="unit">{unit}</span>}
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        style={{ ["--pct" as string]: `${pct}%` }}
        onChange={(e) => onChange(parseFloat(e.target.value))}
      />
    </div>
  );
}
