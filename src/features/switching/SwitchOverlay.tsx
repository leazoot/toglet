import type { JSX } from "react";

import { Dialog } from "../../components/Dialog";
import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, ClientVerdict, SwitchView } from "../../types/ipc";
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
  verdict: ClientVerdict | null;
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
  verdict,
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
    <Dialog label={t("switch.title")} testId="switch-overlay">
      {phase === "confirm" && (
        <Confirm name={target.displayName} onConfirm={onConfirm} onCancel={onCancel} />
      )}
      {phase === "blocked" && (
        <Blocked verdict={verdict} onConfirm={onConfirm} onCancel={onCancel} />
      )}
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
    </Dialog>
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

/**
 * `blocked` means a Codex session was found; `unknown` means the probe could not answer, so it
 * must not claim Codex is running.
 */
function Blocked({
  verdict,
  onConfirm,
  onCancel,
}: {
  verdict: ClientVerdict | null;
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element {
  const unknown = verdict === "unknown";
  return (
    <>
      <p className={styles["heading"]}>
        <span className={cx(styles["dot"], styles["warn"])} aria-hidden="true" />
        {t(unknown ? "switch.unknownTitle" : "switch.blockedTitle")}
      </p>
      <p className={styles["body"]}>{t(unknown ? "switch.unknownBody" : "switch.blockedBody")}</p>
      <div className={styles["actions"]}>
        <button type="button" className={styles["secondary"]} onClick={onCancel}>
          {t("switch.cancel")}
        </button>
        {/* Force-quitting Codex is never the default action. */}
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

/** Codex left closed by the user's setting is not reported as a problem; only a stale client is. */
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
      {/* Always says which account the user is on now. */}
      <p className={styles["body"]}>{t(rollbackKey(result, unreachable))}</p>

      {detailsOpen && result?.error != null && (
        // Only stable codes; the error's detail never leaves Rust.
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
        {(unreachable || result?.error?.retryable === true) && (
          <button type="button" className={styles["primary"]} onClick={onRetry}>
            {t("switch.retry")}
          </button>
        )}
      </div>
    </>
  );
}

/** Every branch says what happened to the account the user was on. */
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
