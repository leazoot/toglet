// The connector between panel and bar. The fill covers the panel's stroke and the outline runs
// along it before arcing out, so border and nub read as one line with no seam.

import type { JSX } from "react";

import styles from "./AnchorNub.module.css";

/** The filled shape, exported so the dock can clip the panel's scrim to the nub's outline. */
export const NUB_FILL_PATH = "M0 4 L4.5 4 C9.2 4 13 8.5 13 14 C13 19.5 9.2 24 4.5 24 L0 24 Z";

export function AnchorNub(): JSX.Element {
  return (
    <svg className={styles["nub"]} viewBox="0 0 14 28" aria-hidden="true">
      <path className={styles["fill"]} d={NUB_FILL_PATH} />
      <path
        className={styles["stroke"]}
        fill="none"
        strokeWidth="1"
        d="M4.5 0 L4.5 4 C9.2 4 13 8.5 13 14 C13 19.5 9.2 24 4.5 24 L4.5 28"
      />
    </svg>
  );
}
