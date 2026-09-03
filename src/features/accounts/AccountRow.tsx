// One account in the panel: 348 × 90, with a distinct treatment for the active account.
//
// Clicking a row asks to switch to it; the confirmation is the overlay's job, not this one's.
// The active row is not clickable - switching to the account you are already on would touch the
// authentication for no reason, so it is not offered.

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
  /** The last row has no divider under it. */
  last: boolean;
  /** Asks to switch to this account. Absent on the active row, which cannot be switched to. */
  onSelect?: (account: AccountView) => void;
}

export function AccountRow({
  account,
  quota,
  nowSeconds,
  last,
  onSelect,
}: AccountRowProps): JSX.Element {
  const notice = noticeKeyFor(account);
  const switching = account.status === "switching";
  // The active account is already in use, an account that cannot be managed has nothing to switch
  // to, and one already being switched to is under way. None is offered as an action rather than
  // being offered and then refused.
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
      {/* The 2px brand rail down the left edge - one of the four things that make the active
          account recognisable in a second. */}
      <span className={styles["rail"]} aria-hidden="true" />

      <span className={styles["avatar"]} data-accent={accentOf(account)}>
        <span className={styles["initial"]} aria-hidden="true">
          {initialOf(account)}
        </span>
        {account.status === "reauth_required" && (
          <span className={styles["badge"]} role="img" aria-label={t("row.reauth")} />
        )}
      </span>

      <span className={styles["body"]}>
        <span className={styles["heading"]}>
          <span className={styles["name"]}>{account.displayName}</span>
          {/* Plan and address are user data. An unknown plan says so rather than being blank. */}
          <span className={styles["plan"]}>{account.planType ?? t("accounts.planUnknown")}</span>
          {account.isActive && <span className={styles["chip"]}>{t("accounts.active")}</span>}
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
          // A row that needs signing in again has no quota worth showing, and the row is a fixed
          // 90 tall: the notice takes the space the two quota lines would have used.
          <span className={styles["notice"]}>{t(notice)}</span>
        )}
      </span>

      {/* The column is always there, so the arrow appearing on hover cannot reflow the row. While
          a switch runs it carries the spinner instead - the quota beside it is still true, so the
          design leaves the numbers alone and says "working" here. */}
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

/**
 * A reading still in flight has no number either, so it draws the same dashed line. What it is
 * not is a failure, and the description is where that difference is stated.
 */
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
 * The one line that replaces the quota block.
 *
 * Only the states where a quota reading would be meaningless qualify. "Weekly not returned" and
 * "could not be read" do **not**: the lines themselves say those, and pushing them up here would
 * hide the five-hour number that is perfectly readable. Neither does a switch in progress - the
 * numbers are still true while it runs, and the design says so in the arrow column instead.
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
