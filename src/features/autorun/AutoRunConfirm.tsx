// The confirmation that turns automatic continuation on. Nothing is sent to Rust until the
// primary action is pressed.

import type { JSX } from "react";

import { Dialog } from "../../components/Dialog";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import styles from "./AutoRunConfirm.module.css";
import type { BindDraft } from "./draft";
import type { AutoRunFailure } from "./store";
import { optionsSummary } from "./summary";

export interface AutoRunConfirmProps {
  draft: BindDraft;
  accounts: Loadable<readonly AccountView[]>;
  nowSeconds: number;
  /** True while the plan is being bound and turned on. */
  committing: boolean;
  failure: AutoRunFailure | null;
  onConfirm: () => void;
  onCancel: () => void;
}

export function AutoRunConfirm({
  draft,
  accounts,
  nowSeconds,
  committing,
  failure,
  onConfirm,
  onCancel,
}: AutoRunConfirmProps): JSX.Element {
  const names = draft.participants.map((id) => nameOf(accounts, id));

  return (
    <Dialog label={t("autorun.confirmTitle")} testId="autorun-confirm">
      <p className={styles["heading"]}>{t("autorun.confirmTitle")}</p>
      <p className={styles["subject"]}>
        {draft.thread?.projectLabel ?? t("autorun.unknownProject")}
        {" · "}
        {draft.thread?.title ?? draft.thread?.preview ?? t("autorun.untitledThread")}
      </p>
      <p className={styles["body"]}>{t("autorun.confirmPlan", { accounts: names.join(" → ") })}</p>
      <p className={styles["fine"]}>{optionsSummary(draft, nowSeconds)}</p>
      <p className={styles["fine"]}>{t("autorun.confirmCaveat")}</p>

      {failure !== null && (
        <p className={cx(styles["body"], styles["alert"])} role="alert">
          {failureText(failure)}
        </p>
      )}

      <div className={styles["actions"]}>
        <button
          type="button"
          className={styles["secondary"]}
          disabled={committing}
          onClick={onCancel}
        >
          {t("autorun.confirmBack")}
        </button>
        <button
          type="button"
          className={styles["primary"]}
          disabled={committing}
          onClick={onConfirm}
        >
          {t(committing ? "autorun.enabling" : "autorun.confirmAction")}
        </button>
      </div>
    </Dialog>
  );
}

/** The account's name, or its random internal id when the list does not hold it. */
function nameOf(accounts: Loadable<readonly AccountView[]>, id: string): string {
  if (accounts.state !== "ready") {
    return id;
  }
  return accounts.value.find((account) => account.id === id)?.displayName ?? id;
}

/** A session dropped from the listing gets its own line: the fix is to choose again, not retry. */
function failureText(failure: AutoRunFailure): string {
  const error = failure.failure.error;
  if (error === null) {
    return t("autorun.enableUnreported");
  }
  if (failure.step === "bind" && error.code === "thread_unavailable") {
    return t("autorun.rebindSession");
  }
  return t("autorun.enableFailed", { code: error.code });
}
