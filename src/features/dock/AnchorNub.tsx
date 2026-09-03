// The connector between panel and bar.
//
// Two paths on purpose. The filled one reaches 4.5px into the panel so it covers the panel's own
// stroke; the outlined one runs along that same stroke for 4px before arcing out and coming back
// to it. Together the panel's border and the nub read as one continuous line with no seam and no
// double line at the root - which is the whole point of drawing it rather than using a triangle.

import type { JSX } from "react";

import styles from "./AnchorNub.module.css";

/**
 * The filled shape, in the 14 × 28 box. Exported because the dock clips the panel's scrim to the
 * same outline when an overlay is open: the nub is outside the panel, and a scrim that stopped at
 * the panel's edge left it at full brightness beside a dimmed panel.
 */
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
