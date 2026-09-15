// The automatic-continuation line under the panel's status bar. Its words come only from Rust's
// plan: an unknown expected time is said as such, never shown as a countdown.

import type { JSX } from "react";

import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, AutoRunView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { controlsFor, statusLine } from "../autorun/status";
import type { AutoRunControl, AutoRunFailure } from "../autorun/store";
import { clockTime } from "../quotas/format";
import styles from "./AutoRunLine.module.css";

export interface AutoRunLineProps {
  plan: AutoRunView;
  accounts: Loadable<readonly AccountView[]>;
  nowSeconds: number;
  busy: boolean;
  failure: AutoRunFailure | null;
  onControl: (action: AutoRunControl) => void;
}

const CONTROL_LABELS: Record<
  AutoRunControl,
  "autorun.pause" | "autorun.resume" | "autorun.cancel"
> = {
  pause: "autorun.pause",
  resume: "autorun.resume",
  cancel: "autorun.cancel",
};

export function AutoRunLine({
  plan,
  accounts,
  nowSeconds,
  busy,
  failure,
  onControl,
}: AutoRunLineProps): JSX.Element | null {
  const line = statusLine(
    plan,
    (id) =>
      accounts.state === "ready"
        ? (accounts.value.find((account) => account.id === id)?.displayName ?? null)
        : null,
    (at) => clockTime(at, nowSeconds),
    t,
  );
  if (line === null) {
    return null;
  }

  const controlFailure = failure !== null && failure.step in CONTROL_LABELS ? failure : null;

  return (
    <div className={styles["line"]} data-testid="autorun-line">
      <span className={cx(styles["dot"], styles[line.tone])} aria-hidden="true" />
      <span className={styles["status"]} role="status">
        {controlFailure === null
          ? t(line.key, line.params)
          : controlFailure.failure.error === null
            ? t("autorun.controlUnreported")
            : t("autorun.controlFailed", { code: controlFailure.failure.error.code })}
      </span>
      {controlsFor(plan.state).map((action) => (
        <button
          key={action}
          type="button"
          className={styles["iconButton"]}
          disabled={busy}
          aria-label={t(CONTROL_LABELS[action])}
          title={t(CONTROL_LABELS[action])}
          onClick={() => {
            onControl(action);
          }}
        >
          <ControlIcon action={action} />
        </button>
      ))}
    </div>
  );
}

function ControlIcon({ action }: { action: AutoRunControl }): JSX.Element {
  switch (action) {
    case "pause":
      return (
        <svg viewBox="0 0 15 15" className={styles["icon"]} aria-hidden="true">
          <path
            d="M5.2 3.5 V11.5 M9.8 3.5 V11.5"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
          />
        </svg>
      );
    case "resume":
      return (
        <svg viewBox="0 0 15 15" className={styles["icon"]} aria-hidden="true">
          <path
            d="M5 3.4 L11 7.5 L5 11.6 Z"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinejoin="round"
          />
        </svg>
      );
    case "cancel":
      return (
        <svg viewBox="0 0 15 15" className={styles["icon"]} aria-hidden="true">
          <path
            d="M4.2 4.2 L10.8 10.8 M10.8 4.2 L4.2 10.8"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      );
  }
}
