// The reset banner above the panel's status bar. Its words come from `bannerLine`, which only
// restates what Rust mirrored from the feed; the feed's own sentence goes to the tooltip alone.
// The credit the feed's terms ask for lives on the settings page (user 2026-09-17): a link here
// made the line long and busy.

import type { JSX } from "react";

import { RESET_GAUGE_GEOMETRY } from "../../components/geometry";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import type { ResetsView } from "../../types/ipc";
import { bannerLine } from "./banner";
import type { BannerTone } from "./banner";
import styles from "./ResetBanner.module.css";

export interface ResetBannerProps {
  view: ResetsView;
  nowSeconds: number;
}

export function ResetBanner({ view, nowSeconds }: ResetBannerProps): JSX.Element {
  const line = bannerLine(view, nowSeconds, t);
  return (
    <div className={styles["line"]} data-testid="reset-banner">
      <Gauge value={line.gauge} tone={line.tone} fresh={line.fresh} label={line.gaugeLabel} />
      <span className={styles["status"]} role="status" title={line.detail ?? undefined}>
        {line.text}
        {line.asOf !== null && <span className={styles["asOf"]}> · {line.asOf}</span>}
      </span>
    </div>
  );
}

interface GaugeProps {
  value: number | null;
  tone: BannerTone;
  fresh: boolean;
  label: string;
}

/** A ring that fills as the average interval passes; dashed when the figures are unknown. */
function Gauge({ value, tone, fresh, label }: GaugeProps): JSX.Element {
  const { box, radius, stroke } = RESET_GAUGE_GEOMETRY;
  const centre = box / 2;
  const circumference = 2 * Math.PI * radius;
  return (
    <svg
      viewBox={`0 0 ${box.toString()} ${box.toString()}`}
      className={styles["gauge"]}
      role="img"
      aria-label={label}
      data-testid="reset-gauge"
    >
      <title>{label}</title>
      <circle
        cx={centre}
        cy={centre}
        r={radius}
        fill="none"
        strokeWidth={stroke}
        className={value === null ? styles["trackUnknown"] : styles["track"]}
      />
      {value !== null && (
        <circle
          cx={centre}
          cy={centre}
          r={radius}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          className={cx(styles["arc"], styles[tone])}
          strokeDasharray={`${(value * circumference).toFixed(2)} ${circumference.toFixed(2)}`}
          transform={`rotate(-90 ${centre.toString()} ${centre.toString()})`}
        />
      )}
      {fresh && <circle cx={centre} cy={centre} r={stroke} className={styles["core"]} />}
    </svg>
  );
}
