// The switch overlays: confirm, blocked, progress, success, failure.
//
// Two things the file is built around:
//
// * **`force-quit` is never the default.** The blocked overlay's primary action is "Check again";
//   closing Codex is offered as a weakened secondary, exactly as the design draws it.
// * **A failure says what happened to the previous account.** "Switch failed" on its own leaves
//   the user not knowing whether they can still work; the rollback line is the answer.

import type { JSX } from "react";

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, SwitchView } from "../../types/ipc";
import styles from "./SwitchOverlay.module.css";
import type { SwitchPhase } from "./store";

/** The four steps the panel shows: Check → Switch → Verify → Ready. */
const STEPS: readonly MessageKey[] = [
  "switch.stepCheck",
  "switch.stepSwitch",
  "switch.stepVerify",
  "switch.stepReady",
];

export interface SwitchOverlayProps {
  phase: SwitchPhase;
  target: AccountView | null;
  /** How many steps Rust says have finished, 0 to 4. Never advanced by this component. */
  step: number;
  result: SwitchView | null;
  /** Set when the call did not get through, as opposed to a switch that ran and failed. */
  unreachable: boolean;
  detailsOpen: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  onToggleDetails: () => void;
}

export function SwitchOverlay({
  phase,
  target,
  step,
  result,
  unreachable,
  detailsOpen,
  onConfirm,
  onCancel,
  onToggleDetails,
}: SwitchOverlayProps): JSX.Element | null {
  if (phase === "idle" || phase === "checking" || target === null) {
    return null;
  }

  return (
    <div className={styles["scrim"]} data-testid="switch-overlay">
      <div
        className={styles["sheet"]}
        role="dialog"
        aria-modal="true"
        aria-label={t("switch.title")}
      >
        {phase === "confirm" && (
          <Confirm name={target.displayName} onConfirm={onConfirm} onCancel={onCancel} />
        )}
        {phase === "blocked" && <Blocked onConfirm={onConfirm} onCancel={onCancel} />}
        {phase === "running" && <Progress name={target.displayName} step={step} />}
        {phase === "done" && <Done name={target.displayName} result={result} />}
        {phase === "failed" && (
          <Failed
            result={result}
            unreachable={unreachable}
            detailsOpen={detailsOpen}
            onRetry={onConfirm}
            onDismiss={onCancel}
            onToggleDetails={onToggleDetails}
          />
        )}
      </div>
    </div>
  );
}

function Confirm({
  name,
  onConfirm,
  onCancel,
}: {
  name: string;
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>{t("switch.confirmTitle", { name })}</p>
      <p className={styles["body"]}>{t("switch.confirmBody")}</p>
      <div className={styles["actions"]}>
        <button type="button" className={styles["secondary"]} onClick={onCancel}>
          {t("switch.cancel")}
        </button>
        <button type="button" className={styles["primary"]} onClick={onConfirm}>
          {t("switch.confirmAction")}
        </button>
      </div>
    </>
  );
}

function Blocked({
  onConfirm,
  onCancel,
}: {
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>
        <span className={cx(styles["dot"], styles["warn"])} aria-hidden="true" />
        {t("switch.blockedTitle")}
      </p>
      <p className={styles["body"]}>{t("switch.blockedBody")}</p>
      <div className={styles["actions"]}>
        <button type="button" className={styles["secondary"]} onClick={onCancel}>
          {t("switch.cancel")}
        </button>
        {/* The primary action is to look again, never to close somebody's editor for them. */}
        <button type="button" className={styles["primary"]} onClick={onConfirm}>
          {t("switch.checkAgain")}
        </button>
      </div>
    </>
  );
}

function Progress({ name, step }: { name: string; step: number }): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>{t("switch.progressTitle", { name })}</p>
      <ol className={styles["steps"]} aria-label={t("switch.progressLabel", { done: step })}>
        {STEPS.map((key, index) => (
          <li
            key={key}
            className={cx(styles["step"], index < step && styles["stepDone"])}
            aria-current={index === step ? "step" : undefined}
          >
            <span className={styles["stepDot"]} aria-hidden="true" />
            {t(key)}
          </li>
        ))}
      </ol>
    </>
  );
}

function Done({ name, result }: { name: string; result: SwitchView | null }): JSX.Element {
  return (
    <div className={styles["success"]}>
      <span className={cx(styles["dot"], styles["ok"])} aria-hidden="true" />
      <span>
        <p className={styles["heading"]}>{t("switch.doneTitle", { name })}</p>
        <p className={styles["body"]}>{t(doneKey(result))}</p>
      </span>
    </div>
  );
}

/**
 * What happened to Codex itself.
 *
 * The result is two-part, and the second part has three outcomes that read very differently: it
 * was reopened, it was closed **because the settings say so**, or it was left running the
 * previous account. Only the last is something to act on, and calling the middle one a problem
 * would describe the user's own choice as a fault.
 */
function doneKey(result: SwitchView | null): MessageKey {
  if (result === null || result.clientUpToDate) {
    return "switch.doneBody";
  }
  switch (result.clientOutcome) {
    case "nothing_was_running":
      return "switch.doneBody";
    case "closed_by_choice":
      return "switch.doneClosedByChoice";
    default:
      return "switch.doneClientStale";
  }
}

function Failed({
  result,
  unreachable,
  detailsOpen,
  onRetry,
  onDismiss,
  onToggleDetails,
}: {
  result: SwitchView | null;
  unreachable: boolean;
  detailsOpen: boolean;
  onRetry: () => void;
  onDismiss: () => void;
  onToggleDetails: () => void;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>
        <span className={cx(styles["dot"], styles["bad"])} aria-hidden="true" />
        {t("switch.failedTitle")}
      </p>
      {/* The one thing the user needs first: which account they are on now. */}
      <p className={styles["body"]}>{t(rollbackKey(result, unreachable))}</p>

      {detailsOpen && result?.error != null && (
        // Stable codes, shown verbatim. They are identifiers, not prose, and the design shows
        // them as such. The error's own detail never leaves Rust.
        <p className={styles["details"]}>
          {result.error.code} · {result.error.phase}
        </p>
      )}

      <div className={styles["actions"]}>
        {result?.error != null && (
          <button type="button" className={styles["link"]} onClick={onToggleDetails}>
            {t(detailsOpen ? "switch.hideDetails" : "switch.showDetails")}
          </button>
        )}
        <button type="button" className={styles["secondary"]} onClick={onDismiss}>
          {t("switch.dismiss")}
        </button>
        {/* Offered only when Rust said the failure is worth another attempt. */}
        {(unreachable || result?.error?.retryable === true) && (
          <button type="button" className={styles["primary"]} onClick={onRetry}>
            {t("switch.retry")}
          </button>
        )}
      </div>
    </>
  );
}

/**
 * What happened to the account the user was on.
 *
 * Every branch answers it. "Switch failed" without this line leaves somebody unsure whether they
 * can keep working, which is exactly what this project refuses to do.
 */
function rollbackKey(result: SwitchView | null, unreachable: boolean): MessageKey {
  if (unreachable || result === null) {
    return "switch.failedUnreachable";
  }
  if (result.manualRecoveryRequired) {
    return "switch.failedManual";
  }
  switch (result.rollback) {
    case "not_needed":
      return "switch.failedUntouched";
    case "restored":
      return "switch.failedRestored";
    case "restored_unverified":
      return "switch.failedRestoredUnverified";
    case "failed":
      return "switch.failedManual";
    case null:
      return "switch.failedUnreachable";
  }
}
