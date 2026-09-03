// The ø38 quota ring.
//
// Presentational only: it is handed an arc, a label and a tone, and it draws them. Deciding what
// the arc means - whether a reading exists at all - happens in features/quotas.

import type { JSX } from "react";

import { cx } from "../styles/classes";
import { RING_GEOMETRY } from "./geometry";
import styles from "./QuotaRing.module.css";

/**
 * How a quota reads at a glance.
 *
 * The vocabulary lives here rather than in the feature because it is a drawing instruction: one
 * of five stroke colours. `unreadable` is a tone in its own right so that "no number" can never
 * be drawn in the colour of "no quota left".
 */
export type QuotaTone = "healthy" | "warn" | "low" | "empty" | "unreadable";

const CENTRE = RING_GEOMETRY.box / 2;
const VIEW_BOX = `0 0 ${RING_GEOMETRY.box.toString()} ${RING_GEOMETRY.box.toString()}`;

export interface QuotaRingProps {
  /** `5H` or `W`. Text, never colour alone. */
  label: string;
  /** `stroke-dasharray` for the arc. */
  dash: string;
  /** `68%`, or an em dash when there is no reading. */
  percent: string;
  tone: QuotaTone;
  /**
   * Whether a reading exists. When it does not, the track is dashed so that an empty ring cannot
   * be mistaken for an exhausted one.
   */
  hasReading: boolean;
  /** The full sentence a screen reader hears, and the tooltip a pointer gets. */
  description: string;
}

export function QuotaRing({
  label,
  dash,
  percent,
  tone,
  hasReading,
  description,
}: QuotaRingProps): JSX.Element {
  return (
    <div className={styles["group"]} title={description}>
      <div className={styles["ring"]} role="img" aria-label={description}>
        <svg className={styles["svg"]} viewBox={VIEW_BOX} aria-hidden="true">
          <circle
            className={hasReading ? styles["track"] : styles["trackUnread"]}
            cx={CENTRE}
            cy={CENTRE}
            r={RING_GEOMETRY.radius}
            strokeWidth={RING_GEOMETRY.stroke}
          />
          <circle
            className={cx(styles["arc"], styles[tone])}
            cx={CENTRE}
            cy={CENTRE}
            r={RING_GEOMETRY.radius}
            strokeWidth={RING_GEOMETRY.stroke}
            strokeDasharray={dash}
          />
        </svg>
        <span className={styles["label"]} aria-hidden="true">
          {label}
        </span>
      </div>
      {/* Fixed width and tabular figures so the bar does not shift as the number changes. */}
      <span className={styles["percent"]} aria-hidden="true">
        {percent}
      </span>
    </div>
  );
}
