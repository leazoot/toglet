import type { JSX } from "react";

import { Dialog } from "../../components/Dialog";
import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type {
  AccountView,
  IpcFailure,
  ResetCreditOutcomeView,
  ResetOutcome,
} from "../../types/ipc";
import styles from "./ResetOverlay.module.css";
import type { ResetPhase } from "./resetStore";

/** The stable code Rust sends when the installed Codex cannot serve a method. */
const INCOMPATIBLE = "runtime_incompatible";

export interface ResetOverlayProps {
  phase: ResetPhase;
  target: AccountView | null;
  /** Credits held when the confirmation opened. */
  held: number;
  result: ResetCreditOutcomeView | null;
  failure: IpcFailure | null;
  onConfirm: () => void;
  onDismiss: () => void;
}

export function ResetOverlay({
  phase,
  target,
  held,
  result,
  failure,
  onConfirm,
  onDismiss,
}: ResetOverlayProps): JSX.Element | null {
  if (phase === "idle" || target === null) {
    return null;
  }

  return (
    <Dialog label={t("reset.title")} testId="reset-overlay">
      {phase === "confirm" && (
        <>
          <p className={styles["heading"]}>
            {t("reset.confirmTitle", { name: target.displayName })}
          </p>
          <p className={styles["body"]}>{t("reset.confirmBody", { count: held })}</p>
          <div className={styles["actions"]}>
            <button type="button" className={styles["secondary"]} onClick={onDismiss}>
              {t("reset.cancel")}
            </button>
            <button type="button" className={styles["primary"]} onClick={onConfirm}>
              {t("reset.confirmAction")}
            </button>
          </div>
        </>
      )}

      {phase === "running" && <p className={styles["heading"]}>{t("reset.working")}</p>}

      {phase === "done" && result !== null && (
        <Answer
          tone={result.succeeded ? "ok" : "warn"}
          title={DONE_TITLES[result.outcome]}
          body={DONE_BODIES[result.outcome]}
          onDismiss={onDismiss}
        />
      )}

      {phase === "failed" && (
        <Answer
          tone="warn"
          title={
            failure?.error?.code === INCOMPATIBLE ? "reset.unsupportedTitle" : "reset.failedTitle"
          }
          body={
            failure?.error?.code === INCOMPATIBLE ? "reset.unsupportedBody" : "reset.failedBody"
          }
          onDismiss={onDismiss}
        />
      )}
    </Dialog>
  );
}

/** Every outcome gets its own words; only `reset` reads as a success. */
const DONE_TITLES: Record<ResetOutcome, MessageKey> = {
  reset: "reset.doneTitle",
  nothingToReset: "reset.nothingTitle",
  noCredit: "reset.noCreditTitle",
  alreadyRedeemed: "reset.alreadyTitle",
  unknown: "reset.unknownTitle",
};

const DONE_BODIES: Record<ResetOutcome, MessageKey> = {
  reset: "reset.doneBody",
  nothingToReset: "reset.nothingBody",
  noCredit: "reset.noCreditBody",
  alreadyRedeemed: "reset.alreadyBody",
  unknown: "reset.unknownBody",
};

function Answer({
  tone,
  title,
  body,
  onDismiss,
}: {
  tone: "ok" | "warn";
  title: MessageKey;
  body: MessageKey;
  onDismiss: () => void;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>
        <span className={cx(styles["dot"], styles[tone])} aria-hidden="true" />
        {t(title)}
      </p>
      <p className={styles["body"]}>{t(body)}</p>
      <div className={styles["actions"]}>
        <button type="button" className={styles["primary"]} onClick={onDismiss}>
          {t("reset.close")}
        </button>
      </div>
    </>
  );
}
