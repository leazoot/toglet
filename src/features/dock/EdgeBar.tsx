// The collapsed bar: 60 × 168, flush against the screen edge, showing the
// active account and its two quota windows and nothing else.
//
// What it will not do: show a number it does not have. A quota that was not returned and one
// that could not be read both draw a dashed track and an em dash, never `0%` - the two are told
// apart in the text a screen reader and a tooltip get, which is the only affordance the design
// gives the collapsed bar.
//
// Before any account has been added the bar is the whole interface, and the design has no
// drawing for it (`TogletBar` requires an account). What it shows then is built from the bar's
// own vocabulary - the avatar well with a dashed ring, and where the first quota ring would be,
// a ring with the dashed track of "no reading" around the toolbar's plus. Pressing it
// opens the panel with the add-account sheet.
//
// Accounts held while Codex is signed in to none of them is the same drawing with the row's
// chevron in place of the plus: the next step is to pick one, and pressing it opens
// the panel where the rows are. A plain well with a hidden sentence had left that state looking
// broken right after the first account was added.

import type { JSX } from "react";

import { RING_GEOMETRY } from "../../components/geometry";
import { QuotaRing } from "../../components/QuotaRing";
import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import { accentOf, initialOf } from "../accounts/identity";
import type { AccountView, QuotaView, QuotaWindowKind } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { compactReset, isStale, percentLabel, ringDash, tone, windowValue } from "../quotas/format";
import type { QuotaValue } from "../quotas/format";
import { AddIcon } from "./AddIcon";
import styles from "./EdgeBar.module.css";
import type { DragHandlers } from "./useDragToSnap";

/**
 * Why the amber dot on the avatar is lit.
 *
 * The design defines the dot for one case - an account that needs signing in again. The other
 * three exist because the collapsed bar is currently the whole interface, and a start-up
 * recovery that failed, an unusable Codex installation or a read that did not work must not go
 * unmentioned.
 */
export type BarNotice = "reauth_required" | "unreadable" | "environment_failed" | "recovery_failed";

export interface EdgeBarProps {
  /** Which edge the window is docked to. The bar mirrors; the text does not. */
  side: "left" | "right";
  /**
   * The active account. `ready` with `null` means Rust has verified no account as the one Codex
   * is using, which is a different statement from "the list has not arrived" and must not be
   * shown as one.
   */
  account: Loadable<AccountView | null>;
  /**
   * Whether any account is managed at all. With `account` at `null` this tells "nothing has been
   * added" apart from "accounts exist, but none is current" - two states with two different next
   * steps, and the bar must not describe the second as the first.
   */
  hasAccounts: boolean;
  quota: Loadable<QuotaView>;
  notice: BarNotice | null;
  /** Unix seconds. Passed in so the countdown and the staleness check read one clock. */
  nowSeconds: number;
  /**
   * Pointer handlers for the drag-to-snap gesture: the bar is the surface the window
   * is dragged by. Forwarded to the DOM unchanged - the bar knows nothing about what a drag means.
   */
  drag?: DragHandlers;
  /** Pressed on the bar's empty state: open the panel with the add-account sheet. */
  onAddAccount?: () => void;
  /** Pressed when accounts exist but none is current: open the panel so one can be chosen. */
  onPickAccount?: () => void;
}

export function EdgeBar({
  side,
  account,
  hasAccounts,
  quota,
  notice,
  nowSeconds,
  drag,
  onAddAccount,
  onPickAccount,
}: EdgeBarProps): JSX.Element {
  const stale = quota.state === "ready" && isStale(quota.value, nowSeconds);
  const active = account.state === "ready" ? account.value : null;
  // Known to be empty - not "still loading", which keeps the plain avatar well and says so in the
  // hidden text. The well is dashed for both states in which Codex is using no account.
  const empty = account.state === "ready" && !hasAccounts;
  const none = account.state === "ready" && hasAccounts && active === null;

  return (
    <div
      className={cx(styles["bar"], side === "left" ? styles["left"] : styles["right"])}
      data-testid="edge-bar"
      {...drag}
    >
      <div className={styles["avatar"]} data-accent={accentOf(active)} data-empty={empty || none}>
        {!empty && !none && <span className={styles["initial"]}>{initialOf(active)}</span>}
        {notice !== null && (
          <span
            className={styles["notice"]}
            role="img"
            aria-label={t(noticeKey(notice))}
            title={t(noticeKey(notice))}
          />
        )}
      </div>

      {empty ? (
        <RingButton
          label={t("bar.addAccount")}
          testId="bar-add"
          icon={<AddIcon className={styles["addIcon"]} />}
          onPress={onAddAccount}
        />
      ) : none ? (
        <RingButton
          label={t("bar.pickAccount")}
          testId="bar-pick"
          icon={<PickIcon />}
          onPress={onPickAccount}
        />
      ) : active === null ? (
        /* Without an account there is no quota to draw. Two empty rings would suggest a reading
           was attempted and came back blank, which is not what happened. The bar has no room for
           the explanation, so it goes where a screen reader and a tooltip can reach it. */
        <p className={styles["hidden"]} title={t(emptyKey(account, hasAccounts))}>
          {t(emptyKey(account, hasAccounts))}
        </p>
      ) : (
        <>
          <Ring
            window="five_hour"
            label={t("bar.fiveHour")}
            quota={quota}
            nowSeconds={nowSeconds}
            stale={stale}
          />
          <Ring
            window="weekly"
            label={t("bar.weekly")}
            quota={quota}
            nowSeconds={nowSeconds}
            stale={stale}
          />
        </>
      )}
    </div>
  );
}

function emptyKey(account: Loadable<AccountView | null>, hasAccounts: boolean): MessageKey {
  switch (account.state) {
    case "loading":
      return "bar.loadingAccount";
    case "failed":
      return "bar.notice.unreadable";
    case "ready":
      return hasAccounts ? "status.noCurrentAccount" : "bar.noAccount";
  }
}

const RING_CENTRE = RING_GEOMETRY.box / 2;
const RING_VIEW_BOX = `0 0 ${RING_GEOMETRY.box.toString()} ${RING_GEOMETRY.box.toString()}`;

/**
 * The one control the bar has when Codex is using no account, drawn to the quota ring's geometry
 * so it sits exactly where the five-hour ring will once an account is current. The dashed track
 * is the same one a ring without a reading draws; here it says "nothing yet" rather than
 * "unreadable".
 */
function RingButton({
  label,
  testId,
  icon,
  onPress,
}: {
  label: string;
  testId: string;
  icon: JSX.Element;
  onPress: (() => void) | undefined;
}): JSX.Element {
  return (
    <button
      type="button"
      className={styles["add"]}
      onClick={onPress}
      aria-label={label}
      title={label}
      data-testid={testId}
    >
      <svg className={styles["addRing"]} viewBox={RING_VIEW_BOX} aria-hidden="true">
        <circle
          className={styles["addTrack"]}
          cx={RING_CENTRE}
          cy={RING_CENTRE}
          r={RING_GEOMETRY.radius}
          strokeWidth={RING_GEOMETRY.stroke}
        />
      </svg>
      {icon}
    </button>
  );
}

/** The row's switch chevron (AccountRow), at the add icon's size: "choose one of these". */
function PickIcon(): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={styles["addIcon"]} aria-hidden="true">
      <path
        d="M5.4 3.2 L9.8 7.5 L5.4 11.8"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}

interface RingProps {
  window: QuotaWindowKind;
  label: string;
  quota: Loadable<QuotaView>;
  nowSeconds: number;
  stale: boolean;
}

function Ring({ window, label, quota, nowSeconds, stale }: RingProps): JSX.Element {
  const value = valueOf(quota, window);
  return (
    <QuotaRing
      label={label}
      dash={ringDash(value)}
      percent={percentLabel(value)}
      tone={tone(value)}
      hasReading={value.kind === "value"}
      description={describe(window, value, quota, nowSeconds, stale)}
    />
  );
}

/**
 * A reading still in flight has no number either, so it draws the same dashed ring. What it is
 * not is a *failure*, and the description is where that difference is stated.
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
  stale: boolean,
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
  if (stale) {
    parts.push(t("quota.cached"));
  }
  return parts.join(" ");
}

function noticeKey(notice: BarNotice): MessageKey {
  switch (notice) {
    case "reauth_required":
      return "bar.notice.reauth";
    case "unreadable":
      return "bar.notice.unreadable";
    case "environment_failed":
      return "bar.notice.environment";
    case "recovery_failed":
      return "bar.notice.recoveryFailed";
  }
}
