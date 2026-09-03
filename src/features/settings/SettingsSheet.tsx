// The settings sheet: 340 wide, r13, rows 38 tall, segmented controls.
//
// Only the settings that do something today are here. Launch at login, fullscreen avoidance, the
// diagnostics folder and "stop managing Codex authentication" are absent because the behaviour
// behind each does not exist yet - a switch that changes nothing is worse than no switch. They
// are recorded as stage leftovers, not shipped as decoration.
//
// The account list at the bottom is where an account is removed. Removal could have gone in a
// per-row "more" menu; the design draws no such menu, and a control that only exists on hover in
// a 90px row is easy to hit by accident. A settings row with an explicit confirmation is the
// nearest thing the design does draw.
//
// The account Codex is using can be removed too, by signing Codex out - an explicit choice. Its
// row carries an amber mark whose tooltip says so, and its confirmation says "sign out" where
// the others say "remove".

import { useState } from "react";
import type { JSX } from "react";

import { resolveLanguage, t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type {
  AccountView,
  RollbackReport,
  SettingsPatch,
  SettingsView,
  Theme,
} from "../../types/ipc";
import type { Loadable } from "../../types/load";
import type { Removal } from "../accounts/store";
import styles from "./SettingsSheet.module.css";

/**
 * The refresh intervals the sheet offers, in seconds. Rust accepts 30-3600. The 25-minute step
 * for the current account was asked for by the user.
 */
const ACTIVE_INTERVALS = [30, 60, 300, 1500] as const;
const INACTIVE_INTERVALS = [300, 900, 3600] as const;

const THEMES: readonly { value: Theme; key: MessageKey }[] = [
  { value: "system", key: "settings.themeSystem" },
  { value: "dark", key: "settings.themeDark" },
  { value: "light", key: "settings.themeLight" },
];

export interface SettingsSheetProps {
  settings: Loadable<SettingsView>;
  saving: boolean;
  onChange: (patch: SettingsPatch) => void;
  onClose: () => void;
  /** The account list, for removal. Mirrored from Rust like everywhere else. */
  accounts: Loadable<readonly AccountView[]>;
  removal: Removal | null;
  onRemove: (account: AccountView) => void;
  onDismissRemoval: () => void;
}

export function SettingsSheet({
  settings,
  saving,
  onChange,
  onClose,
  accounts,
  removal,
  onRemove,
  onDismissRemoval,
}: SettingsSheetProps): JSX.Element {
  return (
    <div className={styles["scrim"]} data-testid="settings-sheet">
      <div
        className={styles["sheet"]}
        role="dialog"
        aria-modal="true"
        aria-label={t("settings.title")}
      >
        <div className={styles["header"]}>
          <SettingsMark />
          <span className={styles["title"]}>{t("settings.title")}</span>
          <button type="button" className={styles["close"]} onClick={onClose}>
            {t("settings.done")}
          </button>
        </div>

        {settings.state === "loading" && (
          <p className={styles["message"]}>{t("settings.loading")}</p>
        )}
        {settings.state === "failed" && (
          <p className={styles["message"]}>{t("settings.unreachable")}</p>
        )}

        {settings.state === "ready" && (
          <div className={styles["rows"]} aria-busy={saving}>
            <Segmented
              label={t("settings.dockEdge")}
              value={settings.value.dockEdge}
              options={[
                { value: "left" as const, label: t("settings.edgeLeft") },
                { value: "right" as const, label: t("settings.edgeRight") },
              ]}
              disabled={saving}
              onPick={(dockEdge) => {
                onChange({ dockEdge });
              }}
            />

            <Toggle
              label={t("settings.alwaysOnTop")}
              value={settings.value.alwaysOnTop}
              disabled={saving}
              onPick={(alwaysOnTop) => {
                onChange({ alwaysOnTop });
              }}
            />

            <Segmented
              label={t("settings.theme")}
              value={settings.value.theme}
              options={THEMES.map((one) => ({ value: one.value, label: t(one.key) }))}
              disabled={saving}
              onPick={(theme) => {
                onChange({ theme });
              }}
            />

            {/* The design's control has two buttons and no "System" (Toglet.dc.html, board 08).
                What is stored can still be `system`, which is what a fresh install holds, so the
                control shows whichever language that currently resolves to - and picking either
                is what turns a preference into a choice. */}
            <Segmented
              label={t("settings.language")}
              value={resolveLanguage(settings.value.language)}
              options={[
                { value: "en" as const, label: t("settings.languageEnglish") },
                { value: "zh" as const, label: t("settings.languageChinese") },
              ]}
              disabled={saving}
              onPick={(language) => {
                onChange({ language });
              }}
            />

            <Toggle
              label={t("settings.reduceMotion")}
              value={settings.value.reduceMotion}
              disabled={saving}
              onPick={(reduceMotion) => {
                onChange({ reduceMotion });
              }}
            />

            <Segmented
              label={t("settings.activeInterval")}
              value={settings.value.activeRefreshSeconds}
              options={ACTIVE_INTERVALS.map((seconds) => ({
                value: seconds,
                label: intervalLabel(seconds),
              }))}
              disabled={saving}
              onPick={(activeRefreshSeconds) => {
                onChange({ activeRefreshSeconds });
              }}
            />

            <Segmented
              label={t("settings.inactiveInterval")}
              value={settings.value.inactiveRefreshSeconds}
              options={INACTIVE_INTERVALS.map((seconds) => ({
                value: seconds,
                label: intervalLabel(seconds),
              }))}
              disabled={saving}
              onPick={(inactiveRefreshSeconds) => {
                onChange({ inactiveRefreshSeconds });
              }}
            />

            {/* Still shown as a risk each time it is used, whatever this says. */}
            <Toggle
              label={t("settings.reopenCodex")}
              value={settings.value.reopenCodexAfterSwitch}
              disabled={saving}
              onPick={(reopenCodexAfterSwitch) => {
                onChange({ reopenCodexAfterSwitch });
              }}
            />
          </div>
        )}

        <AccountsSection
          accounts={accounts}
          removal={removal}
          onRemove={onRemove}
          onDismissRemoval={onDismissRemoval}
        />
      </div>
    </div>
  );
}

interface AccountsSectionProps {
  accounts: Loadable<readonly AccountView[]>;
  removal: Removal | null;
  onRemove: (account: AccountView) => void;
  onDismissRemoval: () => void;
}

/**
 * Every account with a way to remove it. The one Codex is using is removed by signing Codex out
 * first, so its row is marked and its confirmation says so.
 */
function AccountsSection({
  accounts,
  removal,
  onRemove,
  onDismissRemoval,
}: AccountsSectionProps): JSX.Element {
  // The account whose "Remove" has been pressed once and is waiting for the second press.
  const [pending, setPending] = useState<string | null>(null);

  return (
    <div className={styles["accounts"]} data-testid="settings-accounts">
      <p className={styles["sectionTitle"]}>{t("settings.accounts")}</p>

      {accounts.state === "loading" && <p className={styles["message"]}>{t("panel.loading")}</p>}
      {accounts.state === "failed" && (
        <p className={styles["message"]}>{t("bar.notice.unreadable")}</p>
      )}
      {accounts.state === "ready" && accounts.value.length === 0 && (
        <p className={styles["message"]}>{t("panel.emptyTitle")}</p>
      )}

      {accounts.state === "ready" && accounts.value.length > 0 && (
        <ul className={styles["accountList"]}>
          {accounts.value.map((account) => (
            <li key={account.id} className={styles["accountRow"]}>
              <span className={styles["accountName"]}>
                <span className={styles["nameLine"]}>
                  <span className={styles["nameText"]}>{account.displayName}</span>
                  {account.isActive && <InUseMark />}
                </span>
                {account.maskedEmail !== null && (
                  <span className={styles["accountEmail"]}>{account.maskedEmail}</span>
                )}
              </span>
              <RowActions
                account={account}
                removal={removal}
                pending={pending === account.id}
                onPending={setPending}
                onRemove={onRemove}
              />
            </li>
          ))}
        </ul>
      )}

      {(removal?.phase === "failed" || removal?.phase === "orphaned") && (
        <p className={styles["message"]} role="alert">
          {removal.phase === "orphaned"
            ? t("settings.removeOrphaned", { name: removal.name })
            : failureText(removal.name, removal.rollback)}{" "}
          <button type="button" className={styles["dismiss"]} onClick={onDismissRemoval}>
            {t("settings.dismiss")}
          </button>
        </p>
      )}
    </div>
  );
}

interface RowActionsProps {
  account: AccountView;
  removal: Removal | null;
  pending: boolean;
  onPending: (accountId: string | null) => void;
  onRemove: (account: AccountView) => void;
}

/** The right-hand side of an account row: remove, confirm / cancel, or what is happening. */
function RowActions({
  account,
  removal,
  pending,
  onPending,
  onRemove,
}: RowActionsProps): JSX.Element {
  const hint = t(account.isActive ? "settings.signOutHint" : "settings.removeHint", {
    name: account.displayName,
  });

  if (removal?.phase === "removing" && removal.accountId === account.id) {
    return (
      <span className={styles["inUse"]}>
        {t(removal.signingOut ? "settings.signingOut" : "settings.removing")}
      </span>
    );
  }
  if (pending) {
    return (
      <span className={styles["confirm"]}>
        <button
          type="button"
          className={styles["remove"]}
          title={hint}
          onClick={() => {
            onPending(null);
            onRemove(account);
          }}
        >
          {t(account.isActive ? "settings.signOutConfirm" : "settings.removeConfirm")}
        </button>
        <button
          type="button"
          className={styles["cancel"]}
          onClick={() => {
            onPending(null);
          }}
        >
          {t("settings.cancel")}
        </button>
      </span>
    );
  }
  return (
    <button
      type="button"
      className={styles["remove"]}
      aria-label={t("settings.removeNamed", { name: account.displayName })}
      title={hint}
      onClick={() => {
        onPending(account.id);
      }}
    >
      {t("settings.remove")}
    </button>
  );
}

/**
 * The amber mark on the account Codex is using. What it means is in its tooltip - the row has
 * no room for a sentence, and the sentence only matters to somebody about to press "Remove".
 */
function InUseMark(): JSX.Element {
  const hint = t("settings.removeActive");
  return (
    <svg
      viewBox="0 0 14 14"
      className={styles["inUseMark"]}
      role="img"
      aria-label={hint}
      data-testid="in-use-mark"
    >
      <title>{hint}</title>
      <circle cx="7" cy="7" r="6" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path d="M7 3.8 V7.6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="7" cy="10" r="0.9" fill="currentColor" />
    </svg>
  );
}

/**
 * What a failed removal means for the user. A sign-out that failed also says what happened to
 * Codex's sign-in, in the same words the switch uses for the same outcomes.
 */
function failureText(name: string, rollback: RollbackReport | null): string {
  if (rollback === null) {
    return t("settings.removeFailed", { name });
  }
  const outcome: MessageKey =
    rollback === "restored"
      ? "switch.failedRestored"
      : rollback === "restored_unverified"
        ? "switch.failedRestoredUnverified"
        : rollback === "failed"
          ? "switch.failedManual"
          : "switch.failedUntouched";
  return `${t("settings.signOutFailed", { name })} ${t(outcome)}`;
}

/** The sliders mark, the same one the panel's settings button carries. */
function SettingsMark(): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={styles["headerIcon"]} aria-hidden="true">
      <path
        d="M2.5 4.5 H12.5 M2.5 10.5 H12.5"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
      <circle
        cx="9.5"
        cy="4.5"
        r="1.6"
        fill="var(--tg-surface-overlay)"
        stroke="currentColor"
        strokeWidth="1.3"
      />
      <circle
        cx="5.5"
        cy="10.5"
        r="1.6"
        fill="var(--tg-surface-overlay)"
        stroke="currentColor"
        strokeWidth="1.3"
      />
    </svg>
  );
}

interface SegmentedProps<T extends string | number> {
  label: string;
  value: T;
  options: readonly { value: T; label: string }[];
  disabled: boolean;
  onPick: (value: T) => void;
}

function Segmented<T extends string | number>({
  label,
  value,
  options,
  disabled,
  onPick,
}: SegmentedProps<T>): JSX.Element {
  return (
    <div className={styles["row"]}>
      <span className={styles["label"]} id={fieldId(label)}>
        {label}
      </span>
      <div
        // Three or more items give up a pixel of padding each so the group still fits the row.
        className={cx(styles["segmented"], options.length > 2 && styles["tight"])}
        role="radiogroup"
        aria-labelledby={fieldId(label)}
      >
        {options.map((option) => (
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={option.value === value}
            className={cx(styles["segment"], option.value === value && styles["picked"])}
            disabled={disabled}
            onClick={() => {
              onPick(option.value);
            }}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function Toggle({
  label,
  value,
  disabled,
  onPick,
}: {
  label: string;
  value: boolean;
  disabled: boolean;
  onPick: (value: boolean) => void;
}): JSX.Element {
  return (
    <div className={styles["row"]}>
      <span className={styles["label"]}>{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        aria-label={label}
        className={cx(styles["switch"], value && styles["on"])}
        disabled={disabled}
        onClick={() => {
          onPick(!value);
        }}
      >
        <span className={styles["knob"]} aria-hidden="true" />
      </button>
    </div>
  );
}

/** `30s`, `5m`, `1h`. The same compact vocabulary the quota resets use. */
function intervalLabel(seconds: number): string {
  if (seconds < 60) {
    return `${seconds.toString()}s`;
  }
  if (seconds < 3600) {
    return `${(seconds / 60).toString()}m`;
  }
  return `${(seconds / 3600).toString()}h`;
}

function fieldId(label: string): string {
  return `setting-${label.replace(/\W+/g, "-").toLowerCase()}`;
}
