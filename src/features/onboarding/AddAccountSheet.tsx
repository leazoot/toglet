// Adding an account: step 1 / step 2.
//
// The warning in step 1 is the whole reason this screen has two steps rather than one button.
// `account/login/start` takes no parameters beyond the type, so there is no way to ask ChatGPT
// for the account chooser: a browser already signed in reuses that session and the user is never
// asked. Toglet cannot prevent that, so it says so before opening the browser - and says plainly
// what happened afterwards when the account turns out to be one already held.
//
// Step 1 no longer asks for a name. The design drew a name field there; the account is named
// after itself instead - the name it carries at ChatGPT, or the local part of its address -
// because asking somebody to name an account before knowing which account the browser will hand
// back was the wrong order.

import type { JSX } from "react";

import { Spinner } from "../../components/Spinner";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView } from "../../types/ipc";
import styles from "./AddAccountSheet.module.css";
import type { AddPhase } from "./store";

export interface AddAccountSheetProps {
  phase: AddPhase;
  account: AccountView | null;
  /**
   * Whether Rust has verified that Codex is using none of the managed accounts - signed out, or
   * signed in outside Toglet to one it does not hold. Then the account just added is the obvious
   * next thing for it to use, and the result offers the switch - which still runs the whole
   * switch flow, confirmation included; nothing is switched by being added.
   */
  noCurrentAccount: boolean;
  onBegin: () => void;
  onCancel: () => void;
  onDone: () => void;
  onSwitch: (account: AccountView) => void;
}

export function AddAccountSheet({
  phase,
  account,
  noCurrentAccount,
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
        <Result
          heading={t("add.failedTitle")}
          body={t("add.failedBody")}
          tone="bad"
          onDone={onDone}
        />
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

      {/* Stated before the browser opens, because afterwards it is too late to act on. */}
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
      {/* The browser is where the work is; this step only has to look alive while it waits. The
          spinner is the same one a row carries mid-switch, and the heading is the status text. */}
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
  /** Offered only when Codex is using no managed account; the switch itself asks before acting. */
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
