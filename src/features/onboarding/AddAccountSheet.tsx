// Adding an account. The confirmation step exists for its warning: `account/login/start` cannot
// request the account chooser, so a browser already signed in silently reuses that session.
// No name is asked for; the account is named after itself.

import type { JSX } from "react";

import { Spinner } from "../../components/Spinner";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, IpcFailure } from "../../types/ipc";
import styles from "./AddAccountSheet.module.css";
import { signInReason } from "./reason";
import type { AddPhase } from "./store";

export interface AddAccountSheetProps {
  phase: AddPhase;
  account: AccountView | null;
  /**
   * Rust verified that Codex uses none of the managed accounts. The result then offers a switch,
   * which still runs the full switch flow; adding never switches.
   */
  noCurrentAccount: boolean;
  failure: IpcFailure | null;
  onBegin: () => void;
  onCancel: () => void;
  onDone: () => void;
  onSwitch: (account: AccountView) => void;
}

export function AddAccountSheet({
  phase,
  account,
  noCurrentAccount,
  failure,
  onBegin,
  onCancel,
  onDone,
  onSwitch,
}: AddAccountSheetProps): JSX.Element | null {
  if (phase === "idle") {
    return null;
  }

  return (
    <div
      className={styles["sheet"]}
      role="dialog"
      aria-label={t("add.title")}
      data-testid="add-sheet"
    >
      {phase === "confirming" && <Confirming onBegin={onBegin} onCancel={onCancel} />}
      {phase === "waiting" && <Waiting onCancel={onCancel} />}
      {phase === "added" && (
        <Result
          heading={t("add.addedTitle", { name: account?.displayName ?? "" })}
          body={t(noCurrentAccount ? "add.addedNoCurrent" : "add.addedBody")}
          tone="ok"
          onDone={onDone}
          onSwitch={
            noCurrentAccount && account !== null
              ? () => {
                  onSwitch(account);
                }
              : undefined
          }
        />
      )}
      {phase === "duplicate" && (
        <Result
          heading={t("add.duplicateTitle", { name: account?.displayName ?? "" })}
          body={t("add.duplicateBody")}
          tone="warn"
          onDone={onDone}
        />
      )}
      {phase === "failed" && (
        <Result heading={t("add.failedTitle")} body={why(failure)} tone="bad" onDone={onDone} />
      )}
    </div>
  );
}

function Confirming({
  onBegin,
  onCancel,
}: {
  onBegin: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>{t("add.title")}</p>
      <p className={styles["body"]}>{t("add.namingNote")}</p>

      {/* Shown before the browser opens; afterwards it is too late to act on. */}
      <p className={styles["warning"]}>{t("add.browserWarning")}</p>

      <div className={styles["actions"]}>
        <button type="button" className={styles["secondary"]} onClick={onCancel}>
          {t("switch.cancel")}
        </button>
        <button type="button" className={styles["primary"]} autoFocus onClick={onBegin}>
          {t("add.continue")}
        </button>
      </div>
    </>
  );
}

function Waiting({ onCancel }: { onCancel: () => void }): JSX.Element {
  return (
    <>
      {/* The heading is the live status text for the browser wait. */}
      <p className={styles["heading"]} role="status" aria-live="polite" aria-busy="true">
        <Spinner />
        {t("add.waitingTitle")}
      </p>
      <p className={styles["body"]}>{t("add.waitingBody")}</p>
      <div className={styles["actions"]}>
        <button type="button" className={styles["secondary"]} onClick={onCancel}>
          {t("switch.cancel")}
        </button>
      </div>
    </>
  );
}

/** The cause first, then what it means for the accounts. */
function why(failure: IpcFailure | null): string {
  const reason = signInReason(failure);
  return `${t(reason.key, { code: reason.code ?? "" })} ${t("add.failedBody")}`;
}

function Result({
  heading,
  body,
  tone,
  onDone,
  onSwitch,
}: {
  heading: string;
  body: string;
  tone: "ok" | "warn" | "bad";
  onDone: () => void;
  /** Offered only when Codex uses no managed account; the switch still asks before acting. */
  onSwitch?: (() => void) | undefined;
}): JSX.Element {
  return (
    <>
      <p className={styles["heading"]}>
        <span className={cx(styles["dot"], styles[tone])} aria-hidden="true" />
        {heading}
      </p>
      <p className={styles["body"]}>{body}</p>
      <div className={styles["actions"]}>
        {onSwitch === undefined ? (
          <button type="button" className={styles["primary"]} onClick={onDone}>
            {t("switch.dismiss")}
          </button>
        ) : (
          <>
            <button type="button" className={styles["secondary"]} onClick={onDone}>
              {t("switch.dismiss")}
            </button>
            <button type="button" className={styles["primary"]} autoFocus onClick={onSwitch}>
              {t("add.switchNow")}
            </button>
          </>
        )}
      </div>
    </>
  );
}
