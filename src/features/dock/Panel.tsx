// The expanded panel: 348 wide, a toolbar, the account list, a status bar.
//
// All three toolbar buttons are here now: refresh, add account, settings.
//
// It takes no `side`. Docking left mirrors the geometry around the panel - which side of the bar
// it sits on, which way the nub points - and the design keeps the panel's own contents reading
// left to right on both edges. Mirroring it here as well as in the dock cancelled out and put
// the panel against the screen edge with the bar pushed inward.

import { forwardRef } from "react";
import type { JSX, KeyboardEvent } from "react";

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { AccountRow } from "../accounts/AccountRow";
import { quotaOf } from "../quotas/store";
import { AddIcon } from "./AddIcon";
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
   * The settings sheet, or `null`.
   *
   * In flow rather than over the list, because it is taller than the panel it would cover: an
   * absolutely positioned sheet cannot make its container grow, so the window would be sized for
   * a panel that no longer describes what is in it and the sheet would be clipped.
   */
  sheet: JSX.Element | null;
  onOpenSettings: () => void;
  onAddAccount: () => void;
}

/**
 * The ref goes on the outer element so the shell can measure how tall the panel rendered and ask
 * Rust for a window that size. Nothing else about the window can know it: the height depends on
 * the account count, the notice lines and the user's text size.
 */
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
  },
  ref,
): JSX.Element {
  const rows = accounts.state === "ready" ? accounts.value : [];

  return (
    <div ref={ref} className={styles["panel"]} data-testid="panel">
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
        {/* A 1px line sweeping under the toolbar. The list keeps its numbers while it runs. */}
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
        // Arrow keys move between rows; each row carries its own Enter handling.
        <ul className={styles["list"]} onKeyDown={moveFocus}>
          {rows.map((account, index) => (
            <AccountRow
              key={account.id}
              account={account}
              quota={quotaOf(quotas, account.id)}
              nowSeconds={nowSeconds}
              last={index === rows.length - 1}
              onSelect={onSelect}
            />
          ))}
        </ul>
      )}

      <div className={styles["footer"]}>
        <span className={cx(styles["dot"], styles[status.tone])} aria-hidden="true" />
        <span className={styles["status"]}>{t(status.key, status.params)}</span>
      </div>

      {overlay}
    </div>
  );
});

/**
 * `↑` and `↓` move the focus between rows.
 *
 * Read off the DOM rather than tracked in state: the rows are already focusable, the browser
 * already knows which one has the focus, and a second copy of that fact could disagree with it.
 */
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
  // Wraps: with at most a handful of rows, running off the end is more annoying than useful.
  const next = at === -1 ? 0 : (at + step + rows.length) % rows.length;
  rows[next]?.focus();
}

/**
 * The Toglet mark: two arcs of one circle, one brand, one plain.
 *
 * Drawn on a viewBox of 18 but rendered at 17, exactly as the design does - the arcs are inset
 * enough that the extra unit only trims the empty margin, and matching it keeps the stroke on the
 * same subpixels the design lands on.
 */
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

/** The same mark at 36, held back, above the empty state's first sentence. */
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
