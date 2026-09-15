// The collapsed bar: the active account and its two quota windows.
// A window without a reading draws a dashed track and an em dash, never `0%`; "not returned" and
// "unreadable" are told apart only in the screen-reader and tooltip text.

import type { JSX } from "react";

import { RING_GEOMETRY } from "../../components/geometry";
import { QuotaRing } from "../../components/QuotaRing";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import { accentOf, initialOf } from "../accounts/identity";
import type { AccountView, QuotaView, QuotaWindowKind } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { isStale, percentLabel, ringDash, tone } from "../quotas/format";
import { AddIcon } from "./AddIcon";
import { describe, emptyKey, noticeKey, valueOf } from "./barText";
import type { BarNotice } from "./barText";
import styles from "./EdgeBar.module.css";
import type { DragHandlers } from "./useDragToSnap";

export type { BarNotice } from "./barText";

export interface EdgeBarProps {
  /** The bar mirrors; the text does not. */
  side: "left" | "right";
  /** `ready` with `null` means Rust verified no account as current, unlike "still loading". */
  account: Loadable<AccountView | null>;
  /** With `account` at `null`, tells "nothing added" apart from "accounts exist, none current". */
  hasAccounts: boolean;
  quota: Loadable<QuotaView>;
  notice: BarNotice | null;
  /** Unix seconds, so the countdown and the staleness check read one clock. */
  nowSeconds: number;
  drag?: DragHandlers;
  onAddAccount?: () => void;
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
  // Known states only: while loading the plain avatar well is kept.
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
        /* No rings without an account: two empty rings would suggest a blank reading. */
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

const RING_CENTRE = RING_GEOMETRY.box / 2;
const RING_VIEW_BOX = `0 0 ${RING_GEOMETRY.box.toString()} ${RING_GEOMETRY.box.toString()}`;

/** The bar's only control when no account is current, drawn where the five-hour ring would be. */
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

/** The row's switch chevron (AccountRow), at the add icon's size. */
export function PickIcon(): JSX.Element {
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
