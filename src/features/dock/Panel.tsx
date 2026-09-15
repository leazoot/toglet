// The expanded panel: toolbar, account list, status bar.
// Takes no `side`: the dock mirrors the geometry; the contents read left to right on both edges.

import { forwardRef, useLayoutEffect, useRef, useState } from "react";
import type { ForwardedRef, JSX, KeyboardEvent, RefObject } from "react";

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, AutoRunView, QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { AccountRow } from "../accounts/AccountRow";
import { isExecuting } from "../autorun/status";
import type { AutoRunControl, AutoRunFailure } from "../autorun/store";
import { quotaOf } from "../quotas/store";
import { AutoRunMark } from "../settings/AutoRunSheet";
import { AddIcon } from "./AddIcon";
import { AutoRunLine } from "./AutoRunLine";
import styles from "./Panel.module.css";

/** The colour of the dot in the status bar. Never the only carrier - the text says it too. */
export type StatusTone = "ok" | "warn" | "bad" | "mute";

export interface PanelStatus {
  tone: StatusTone;
  key: MessageKey;
  /** Filled into the message's slots, when it has any. */
  params?: Readonly<Record<string, string | number>>;
}

export interface PanelProps {
  accounts: Loadable<readonly AccountView[]>;
  quotas: Readonly<Record<string, Loadable<QuotaView>>>;
  /** Turns the refresh icon and the scan line on. The panel itself never enters a loading state. */
  refreshing: boolean;
  status: PanelStatus;
  nowSeconds: number;
  onRefresh: () => void;
  /** Asks to switch to an account. The confirmation is the overlay's job. */
  onSelect: (account: AccountView) => void;
  /** The switch overlay, or `null`. Covers the list but leaves it visible. */
  overlay: JSX.Element | null;
  /**
   * The settings sheet, or `null`. In flow rather than over the list: it can be taller than the
   * list, and an absolutely positioned sheet cannot make its container grow.
   */
  sheet: JSX.Element | null;
  onOpenSettings: () => void;
  onAddAccount: () => void;
  onOpenAutoRun: () => void;
  /** The automatic-continuation plan mirrored from Rust, or `null` while unknown. */
  autorun: AutoRunView | null;
  /** True while a pause, continue or cancel is on its way. */
  autorunBusy: boolean;
  autorunFailure: AutoRunFailure | null;
  onAutoRunControl: (action: AutoRunControl) => void;
}

/** The ref goes on the outer element so the dock can measure the rendered height. */
export const Panel = forwardRef<HTMLDivElement, PanelProps>(function Panel(
  {
    accounts,
    quotas,
    refreshing,
    status,
    nowSeconds,
    onRefresh,
    onSelect,
    overlay,
    sheet,
    onOpenSettings,
    onAddAccount,
    onOpenAutoRun,
    autorun,
    autorunBusy,
    autorunFailure,
    onAutoRunControl,
  },
  ref,
): JSX.Element {
  const rows = accounts.state === "ready" ? accounts.value : [];
  const inner = useRef<HTMLDivElement | null>(null);
  const height = useContentHeight(ref, inner);
  // A disabled plan marks nothing: its participants are only a draft.
  const participants = new Set(
    autorun?.enabled === true
      ? autorun.participants.map((participant) => participant.accountId)
      : [],
  );

  return (
    <div
      ref={ref}
      className={styles["panel"]}
      data-testid="panel"
      style={height === null ? undefined : { height }}
    >
      <div ref={inner} className={styles["inner"]}>
        <div className={styles["toolbar"]}>
          <BrandMark />
          <div className={styles["heading"]}>
            <span className={styles["title"]}>{t("app.name")}</span>
            {accounts.state === "ready" && (
              <span className={styles["count"]}>
                {t(rows.length === 1 ? "panel.countOne" : "panel.count", { count: rows.length })}
              </span>
            )}
          </div>
          <div className={styles["actions"]}>
            <button
              type="button"
              className={styles["iconButton"]}
              onClick={onRefresh}
              disabled={refreshing}
              aria-label={t("panel.refresh")}
              title={t("panel.refresh")}
            >
              <RefreshIcon spinning={refreshing} />
            </button>
            <button
              type="button"
              className={styles["iconButton"]}
              onClick={onOpenAutoRun}
              aria-label={t("autorun.open")}
              title={t("autorun.open")}
            >
              <AutoRunMark className={styles["icon"]} />
            </button>
            <button
              type="button"
              className={styles["iconButton"]}
              onClick={onAddAccount}
              aria-label={t("add.open")}
              title={t("add.open")}
            >
              <AddIcon className={styles["icon"]} />
            </button>
            <button
              type="button"
              className={styles["iconButton"]}
              onClick={onOpenSettings}
              aria-label={t("settings.open")}
              title={t("settings.open")}
            >
              <SettingsIcon />
            </button>
          </div>
          {refreshing && <span className={styles["scan"]} aria-hidden="true" />}
        </div>

        {sheet}

        {sheet === null && accounts.state === "failed" && (
          <p className={styles["message"]}>{t("bar.notice.unreadable")}</p>
        )}
        {sheet === null && accounts.state === "loading" && (
          <p className={styles["message"]}>{t("panel.loading")}</p>
        )}
        {sheet === null && accounts.state === "ready" && rows.length === 0 && (
          <div className={styles["empty"]}>
            <EmptyMark />
            <p className={styles["emptyTitle"]}>{t("panel.emptyTitle")}</p>
            <p className={styles["emptyBody"]}>{t("panel.emptyBody")}</p>
            <button type="button" className={styles["emptyAction"]} onClick={onAddAccount}>
              {t("panel.emptyAction")}
            </button>
          </div>
        )}

        {sheet === null && rows.length > 0 && (
          // Beyond five rows the list scrolls and the panel stops growing.
          <ul className={styles["list"]} onKeyDown={moveFocus}>
            {rows.map((account, index) => (
              <AccountRow
                key={account.id}
                account={account}
                quota={quotaOf(quotas, account.id)}
                nowSeconds={nowSeconds}
                last={index === rows.length - 1}
                onSelect={onSelect}
                participating={participants.has(account.id)}
                executing={isExecuting(autorun, account.id)}
              />
            ))}
          </ul>
        )}

        <div className={styles["footer"]}>
          <span className={cx(styles["dot"], styles[status.tone])} aria-hidden="true" />
          <span className={styles["status"]}>{t(status.key, status.params)}</span>
        </div>
        {/* Quota and scheduling are two facts, so the second line is added, not swapped in. */}
        {autorun !== null && (
          <AutoRunLine
            plan={autorun}
            accounts={accounts}
            nowSeconds={nowSeconds}
            busy={autorunBusy}
            failure={autorunFailure}
            onControl={onAutoRunControl}
          />
        )}
      </div>

      {overlay}
    </div>
  );
});

/**
 * The contents' height as an explicit number, because `height: auto` cannot be transitioned.
 * The first measurement lands before paint; without `ResizeObserver` (tests) the height stays auto.
 */
function useContentHeight(
  outer: ForwardedRef<HTMLDivElement>,
  inner: RefObject<HTMLDivElement | null>,
): number | null {
  const [value, setValue] = useState<number | null>(null);
  useLayoutEffect(() => {
    const content = inner.current;
    const panel = typeof outer === "function" || outer === null ? null : outer.current;
    if (content === null || panel === null || typeof ResizeObserver === "undefined") {
      return undefined;
    }
    // Border-box: add the border, which is offset height minus client height.
    const measure = (): void => {
      setValue(content.offsetHeight + (panel.offsetHeight - panel.clientHeight));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => {
      observer.disconnect();
    };
  }, [outer, inner]);
  return value;
}

/** `↑` and `↓` move focus between rows, read off the DOM rather than duplicated in state. */
function moveFocus(event: KeyboardEvent<HTMLUListElement>): void {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") {
    return;
  }
  const rows = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="button"]')];
  const at = rows.indexOf(document.activeElement as HTMLElement);
  if (rows.length === 0) {
    return;
  }
  event.preventDefault();
  const step = event.key === "ArrowDown" ? 1 : -1;
  // Wraps around at either end.
  const next = at === -1 ? 0 : (at + step + rows.length) % rows.length;
  rows[next]?.focus();
}

/** The Toglet mark. Drawn on a viewBox of 18 but rendered at 17, which only trims empty margin. */
function BrandMark(): JSX.Element {
  return (
    <svg viewBox="0 0 18 18" className={styles["brand"]} aria-hidden="true">
      <path
        d="M3.6 6.4 A5.9 5.9 0 0 1 14.4 6.4"
        fill="none"
        stroke="var(--tg-brand)"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
      <path
        d="M14.4 11.6 A5.9 5.9 0 0 1 3.6 11.6"
        fill="none"
        stroke="var(--tg-text-primary)"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** The same mark, muted, for the empty state. */
function EmptyMark(): JSX.Element {
  return (
    <svg viewBox="0 0 22 22" className={styles["emptyMark"]} aria-hidden="true">
      <path
        d="M5.4 8.4 A6.3 6.3 0 0 1 16.6 8.4"
        fill="none"
        stroke="var(--tg-brand)"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <path
        d="M16.6 13.6 A6.3 6.3 0 0 1 5.4 13.6"
        fill="none"
        stroke="var(--tg-mark-muted)"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SettingsIcon(): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={styles["icon"]} aria-hidden="true">
      <path
        d="M2.5 4.5 H12.5 M2.5 10.5 H12.5"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
      />
      <circle
        cx="9.5"
        cy="4.5"
        r="1.6"
        fill="var(--tg-surface-base)"
        stroke="currentColor"
        strokeWidth="1.35"
      />
      <circle
        cx="5.5"
        cy="10.5"
        r="1.6"
        fill="var(--tg-surface-base)"
        stroke="currentColor"
        strokeWidth="1.35"
      />
    </svg>
  );
}

function RefreshIcon({ spinning }: { spinning: boolean }): JSX.Element {
  return (
    <svg
      viewBox="0 0 15 15"
      className={cx(styles["icon"], spinning && styles["spinning"])}
      aria-hidden="true"
    >
      <path
        d="M12.2 5.2 A5 5 0 1 0 12.6 9.4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
      />
      <path
        d="M12.6 2.2 L12.6 5.4 L9.4 5.4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
