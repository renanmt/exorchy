import { Progress } from "@ark-ui/solid/progress";
import { createStallDetector } from "../stallDetector";

/** True when either explicitly requested, value is 0, or progress has stalled. */
function useIndeterminate(
  value: () => number,
  explicit: () => boolean | undefined,
): () => boolean {
  const stalled = createStallDetector(value);
  return () => {
    const e = explicit();
    if (e === true) { return true; }
    if (e === false) { return false; }
    return value() === 0 || stalled();
  };
}

// ── Linear progress bar ─────────────────────────────────────────────────────

interface AutoProgressProps {
  /** Progress in 0..1 range. */
  value: number;
  /** Force indeterminate mode; omit to auto-detect from value/stall. */
  indeterminate?: boolean;
  /** Extra classes to pass through (e.g. "mini" for thin bars). */
  class?: string;
}

export function AutoProgress(props: AutoProgressProps) {
  const isIndet = useIndeterminate(() => props.value, () => props.indeterminate);
  return (
    <Progress.Root value={isIndet() ? 0 : props.value * 100} class={`ark-progress ${props.class ?? ""}`}>
      <Progress.Track class="ark-progress-track">
        <Progress.Range class={`ark-progress-range${isIndet() ? " indeterminate" : ""}`} />
      </Progress.Track>
    </Progress.Root>
  );
}

// ── Circular progress ring ──────────────────────────────────────────────────

interface CircularProgressProps {
  value: number;
  size?: number;
  strokeWidth?: number;
  indeterminate?: boolean;
  /** Content rendered inside the circle (e.g. percentage text). */
  children?: any;
  class?: string;
}

export function CircularProgress(props: CircularProgressProps) {
  const isIndet = useIndeterminate(() => props.value, () => props.indeterminate);
  const size = () => props.size ?? 48;
  const stroke = () => props.strokeWidth ?? 4;
  const radius = () => (size() - stroke()) / 2;
  const circumference = () => 2 * Math.PI * radius();
  const dashOffset = () => {
    const clamped = Math.max(0, Math.min(1, props.value));
    return circumference() * (1 - clamped);
  };

  return (
    <div class={`circular-progress${isIndet() ? " indeterminate" : ""} ${props.class ?? ""}`}
         style={{ width: `${size()}px`, height: `${size()}px` }}>
      <svg width={size()} height={size()} viewBox={`0 0 ${size()} ${size()}`}>
        <circle class="circular-progress-track"
                cx={size() / 2} cy={size() / 2} r={radius()}
                fill="none" stroke-width={stroke()} />
        <circle class="circular-progress-range"
                cx={size() / 2} cy={size() / 2} r={radius()}
                fill="none" stroke-width={stroke()}
                stroke-dasharray={isIndet() ? `${circumference() * 0.3} ${circumference()}` : `${circumference()}`}
                stroke-dashoffset={isIndet() ? "0" : `${dashOffset()}`}
                stroke-linecap="round"
                transform={`rotate(-90 ${size() / 2} ${size() / 2})`} />
      </svg>
      {props.children && <div class="circular-progress-content">{props.children}</div>}
    </div>
  );
}
