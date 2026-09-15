// Dialog frame: a sheet rising from the panel's bottom edge over a scrim. Overlays compose the
// classes in Dialog.module.css for their contents so buttons are drawn one way everywhere.

import type { JSX, ReactNode } from "react";

import styles from "./Dialog.module.css";

export interface DialogProps {
  /** The accessible name of the dialog. */
  label: string;
  testId?: string;
  children: ReactNode;
}

export function Dialog({ label, testId, children }: DialogProps): JSX.Element {
  return (
    <div className={styles["scrim"]} data-testid={testId}>
      <div className={styles["sheet"]} role="dialog" aria-modal="true" aria-label={label}>
        {children}
      </div>
    </div>
  );
}
