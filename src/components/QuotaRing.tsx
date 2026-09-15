// The ø38 quota ring. Presentational: features/quotas decides what the arc means.

import type { JSX } from "react";

import { cx } from "../styles/classes";
import { RING_GEOMETRY } from "./geometry";
import styles from "./QuotaRing.module.css";

/** Stroke colour for a quota. `unreadable` is separate so "no number" never looks like "empty". */
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
  /** Without a reading the track is dashed, so it cannot be mistaken for an exhausted quota. */
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
