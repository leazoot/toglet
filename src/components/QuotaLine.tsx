// The 4px quota bar used inside an account row.
//
// Presentational only. Like QuotaRing it is handed a width and a tone rather than a reading, so
// the decision "is there a number at all" stays in features/quotas.

import type { JSX } from "react";

import { cx } from "../styles/classes";
import type { QuotaTone } from "./QuotaRing";
import styles from "./QuotaLine.module.css";

export interface QuotaLineProps {
  /** `5H` or `W`. */
  label: string;
  /** How much of the track to fill, 0-100. Ignored when there is no reading. */
  percent: number;
  /** `68%`, or an em dash when there is no reading. */
  percentLabel: string;
  /** The compact reset time, or an empty string when there is none to show. */
  reset: string;
  tone: QuotaTone;
  hasReading: boolean;
  /** Dims the fill on rows that are not the active account. */
  dimmed: boolean;
  /** The sentence a screen reader hears in place of the three fragments above. */
  description: string;
}

export function QuotaLine({
  label,
  percent,
  percentLabel,
  reset,
  tone,
  hasReading,
  dimmed,
  description,
}: QuotaLineProps): JSX.Element {
  return (
    <div className={styles["row"]} role="img" aria-label={description}>
      <span className={styles["label"]} aria-hidden="true">
        {label}
      </span>
      <div className={cx(styles["track"], !hasReading && styles["trackUnread"])} aria-hidden="true">
        {/* No reading means no fill at all - the dashed track is the whole statement. A fill of
            zero width on a solid track would be indistinguishable from 0%. */}
        {hasReading && (
          <div
            className={cx(styles["fill"], styles[tone], dimmed && styles["dimmed"])}
            style={{ width: `${percent.toString()}%` }}
          />
        )}
      </div>
      <span className={styles["percent"]} aria-hidden="true">
        {percentLabel}
      </span>
      <span className={styles["reset"]} aria-hidden="true">
        {reset}
      </span>
    </div>
  );
}
