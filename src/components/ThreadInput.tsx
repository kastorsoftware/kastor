import { ChevronUp, ChevronDown } from "lucide-react";

interface ThreadInputProps {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
}

export function ThreadInput({ value, onChange, min = 1, max = 1000 }: ThreadInputProps) {
  const clamp = (v: number) => Math.max(min, Math.min(max, v));

  return (
    <div className="thread-input-wrap">
      <button
        type="button"
        onClick={() => onChange(clamp(value - 1))}
        aria-label="Decrease"
      >
        <ChevronDown />
      </button>
      <input
        type="number"
        min={min}
        max={max}
        value={value}
        onChange={(e) => onChange(clamp(parseInt(e.target.value) || min))}
      />
      <button
        type="button"
        onClick={() => onChange(clamp(value + 1))}
        aria-label="Increase"
      >
        <ChevronUp />
      </button>
    </div>
  );
}
