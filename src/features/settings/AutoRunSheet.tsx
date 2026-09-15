// The automatic-continuation sheet, in the settings sheet's frame and stylesheet. Two pages: the
// group, and the session chooser that replaces it; the header shows "Done" or "Back" to match.

import { useState } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import { AutoRunSection } from "./AutoRunSection";
import type { AutoRunSectionProps } from "./AutoRunSection";
import styles from "./SettingsSheet.module.css";

export interface AutoRunSheetProps extends Omit<
  AutoRunSectionProps,
  "choosing" | "onChooseSession" | "onChosen"
> {
  onClose: () => void;
  /** Asks for the session list. Called each time the chooser page opens. */
  onListThreads: () => void;
}

export function AutoRunSheet({
  onClose,
  onListThreads,
  ...section
}: AutoRunSheetProps): JSX.Element {
  const [choosing, setChoosing] = useState(false);

  return (
    <div className={styles["scrim"]} data-testid="autorun-sheet">
      <div
        className={styles["sheet"]}
        role="dialog"
        aria-modal="true"
        aria-label={t(choosing ? "autorun.pickSession" : "autorun.section")}
      >
        <div className={styles["header"]}>
          <AutoRunMark className={styles["headerIcon"]} />
          <span className={styles["title"]}>
            {t(choosing ? "autorun.pickSession" : "autorun.section")}
          </span>
          {choosing ? (
            <button
              type="button"
              className={styles["close"]}
              onClick={() => {
                setChoosing(false);
              }}
            >
              {t("autorun.back")}
            </button>
          ) : (
            <button type="button" className={styles["close"]} onClick={onClose}>
              {t("settings.done")}
            </button>
          )}
        </div>
        <AutoRunSection
          {...section}
          choosing={choosing}
          onChooseSession={() => {
            onListThreads();
            setChoosing(true);
          }}
          onChosen={() => {
            setChoosing(false);
          }}
        />
      </div>
    </div>
  );
}

/**
 * A track with a gap (the quota running out) and an arc hopping over it. Deliberately not
 * circular, so it cannot be mistaken for the refresh icon beside it.
 */
export function AutoRunMark({ className }: { className: string | undefined }): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={className} aria-hidden="true">
      <path
        d="M2 9.7 H4.9 M10.1 9.7 H12.2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
      />
      <path
        d="M4.9 9.7 Q7.5 -1.5 10.1 9.7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
      />
      <path
        d="M11 8.5 L12.5 9.7 L11 10.9"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
