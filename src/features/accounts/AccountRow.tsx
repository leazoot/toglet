import type { JSX } from "react";

import { QuotaLine } from "../../components/QuotaLine";
import { Spinner } from "../../components/Spinner";
import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, QuotaView, QuotaWindowKind } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { compactReset, percentLabel, tone, windowValue } from "../quotas/format";
import type { QuotaValue } from "../quotas/format";
import styles from "./AccountRow.module.css";
import { accentOf, initialOf } from "./identity";

export interface AccountRowProps {
  account: AccountView;
  quota: Loadable<QuotaView>;
  /** Unix seconds. Passed in so every countdown in the panel reads one clock. */
  nowSeconds: number;
  last: boolean;
  /** Asks to switch to this account. Absent on the active row, which cannot be switched to. */
  onSelect?: (account: AccountView) => void;
  /** Takes part in automatic continuation. A read-only mark by the avatar. */
  participating?: boolean;
  /** The account automatic continuation is running as right now. */
  executing?: boolean;
}

export function AccountRow({
  account,
  quota,
  nowSeconds,
  last,
  onSelect,
  participating = false,
  executing = false,
}: AccountRowProps): JSX.Element {
  const notice = noticeKeyFor(account);
  const switching = account.status === "switching";
  const selectable =
    onSelect !== undefined && !account.isActive && account.status !== "unsupported" && !switching;

  return (
    <li
      className={cx(
        styles["row"],
        account.isActive && styles["active"],
        switching && styles["switching"],
        selectable && styles["selectable"],
        !last && styles["divided"],
      )}
      data-testid="account-row"
      onClick={
        selectable
          ? () => {
              onSelect(account);
            }
          : undefined
      }
      onKeyDown={
        selectable
          ? (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(account);
              }
            }
          : undefined
      }
      role={selectable ? "button" : undefined}
      tabIndex={selectable ? 0 : undefined}
      aria-label={selectable ? t("row.switchTo", { name: account.displayName }) : undefined}
    >
      <span className={styles["rail"]} aria-hidden="true" />

      <span className={styles["avatar"]} data-accent={accentOf(account)}>
        <span className={styles["initial"]} aria-hidden="true">
          {initialOf(account)}
        </span>
        {account.status === "reauth_required" && (
          <span className={styles["badge"]} role="img" aria-label={t("row.reauth")} />
        )}
        {/* Positioned on the avatar so the row's height and columns never change. */}
        {participating && (
          <svg
            viewBox="0 0 12 12"
            className={styles["participant"]}
            role="img"
            aria-label={t("row.participant")}
            data-testid="participant-mark"
          >
            <title>{t("row.participant")}</title>
            <circle cx="6" cy="6" r="5.4" fill="var(--tg-surface-base)" />
            <path
              d="M3.6 6.6 A2.5 2.5 0 1 0 4.4 4.2"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
            />
            <path
              d="M3.3 3.4 L4.4 4.2 L3.5 5.4"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        )}
      </span>

      <span className={styles["body"]}>
        <span className={styles["heading"]}>
          <span className={styles["name"]}>{account.displayName}</span>
          <span className={styles["plan"]}>{account.planType ?? t("accounts.planUnknown")}</span>
          {account.isActive && <span className={styles["chip"]}>{t("accounts.active")}</span>}
          {/* Who the scheduler runs as, which is not always who Codex is signed in as. */}
          {executing && <span className={styles["chip"]}>{t("row.continuing")}</span>}
        </span>

        <span className={styles["email"]}>
          {account.maskedEmail ?? t("accounts.addressUnknown")}
        </span>

        {notice === null ? (
          <span className={styles["quota"]}>
            <Line
              window="five_hour"
              account={account}
              quota={quota}
              nowSeconds={nowSeconds}
              label={t("bar.fiveHour")}
            />
            <Line
              window="weekly"
              account={account}
              quota={quota}
              nowSeconds={nowSeconds}
              label={t("bar.weekly")}
            />
          </span>
        ) : (
          // The row has a fixed height, so the notice takes the quota lines' space.
          <span className={styles["notice"]}>{t(notice)}</span>
        )}
      </span>

      {/* Always present so the hover arrow cannot reflow the row; holds the spinner while
          switching, leaving the (still true) quota numbers in place. */}
      <span className={styles["arrow"]}>
        {switching ? (
          <>
            <Spinner />
            {/* The spin is the only visual carrier, so the word goes to assistive technology. */}
            <span className={styles["hidden"]}>{t("row.switching")}</span>
          </>
        ) : (
          selectable && (
            <svg viewBox="0 0 15 15" className={styles["chevron"]} aria-hidden="true">
              <path
                d="M5.4 3.2 L9.8 7.5 L5.4 11.8"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          )
        )}
      </span>
    </li>
  );
}

interface LineProps {
  window: QuotaWindowKind;
  account: AccountView;
  quota: Loadable<QuotaView>;
  nowSeconds: number;
  label: string;
}

function Line({ window, account, quota, nowSeconds, label }: LineProps): JSX.Element {
  const value = valueOf(quota, window);
  const reading = value.kind === "value";
  return (
    <QuotaLine
      label={label}
      percent={reading ? value.remainingPercent : 0}
      percentLabel={percentLabel(value)}
      reset={
        value.kind === "value" && value.resetsAt !== null
          ? compactReset(value.resetsAt, nowSeconds)
          : ""
      }
      tone={tone(value)}
      hasReading={reading}
      dimmed={!account.isActive}
      description={describe(window, value, quota, nowSeconds)}
    />
  );
}

/** Loading draws the same dashed line as unreadable; only the description tells them apart. */
function valueOf(quota: Loadable<QuotaView>, window: QuotaWindowKind): QuotaValue {
  switch (quota.state) {
    case "ready":
      return windowValue(quota.value, window);
    case "failed":
    case "loading":
      return { kind: "unreadable" };
  }
}

function describe(
  window: QuotaWindowKind,
  value: QuotaValue,
  quota: Loadable<QuotaView>,
  nowSeconds: number,
): string {
  const name = t(window === "five_hour" ? "quota.fiveHourName" : "quota.weeklyName");

  if (quota.state === "loading") {
    return t("quota.reading", { window: name });
  }
  if (value.kind === "unreadable") {
    return t("quota.unreadable", { window: name });
  }
  if (value.kind === "not_returned") {
    return t("quota.notReturned", { window: name });
  }

  const parts = [t("quota.remaining", { window: name, percent: percentLabel(value) })];
  if (value.resetsAt !== null) {
    parts.push(t("quota.resets", { when: compactReset(value.resetsAt, nowSeconds) }));
  }
  return parts.join(" ");
}

/**
 * Replaces the quota block only when a reading would be meaningless. Missing or unreadable
 * windows stay on their own lines so a readable five-hour number is not hidden.
 */
function noticeKeyFor(account: AccountView): MessageKey | null {
  switch (account.status) {
    case "reauth_required":
      return "row.reauthNotice";
    case "unsupported":
      return "row.unsupported";
    default:
      return null;
  }
}
