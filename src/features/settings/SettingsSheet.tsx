// The settings sheet, with only settings whose behaviour exists. Accounts are removed here, with
// an explicit confirmation; removing the one Codex is using signs Codex out. Three pages
// (settings, notification channels, remote control); the header shows "Done" or "Back".

import { useEffect, useState } from "react";
import type { JSX } from "react";

import { resolveLanguage, t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import type {
  AccountView,
  RollbackReport,
  SettingsPatch,
  SettingsView,
  Theme,
} from "../../types/ipc";
import type { Loadable } from "../../types/load";
import type { Removal } from "../accounts/store";
import { NotifyRow } from "../notify/NotifyRow";
import { NotifySection } from "../notify/NotifySection";
import { RemoteRow } from "../remote/RemoteRow";
import { RemoteSection } from "../remote/RemoteSection";
import { Segmented, Toggle } from "./controls";
import styles from "./SettingsSheet.module.css";

/** Refresh intervals offered, in seconds. Rust accepts 30-3600. */
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
  /** The account list, for removal. */
  accounts: Loadable<readonly AccountView[]>;
  removal: Removal | null;
  onRemove: (account: AccountView) => void;
  onDismissRemoval: () => void;
  /**
   * Called when the sheet moves onto or off a sub-page. Sub-pages pin the panel: their secret
   * fields start empty, so closing on pointer drift would lose a half-typed secret.
   */
  onPinned: (pinned: boolean) => void;
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
  onPinned,
}: SettingsSheetProps): JSX.Element {
  const [page, setPage] = useState<"settings" | "notify" | "remote">("settings");
  const onSettings = page === "settings";

  // The cleanup matters: if the sheet unmounts with a pin set, the panel would stay open for good.
  useEffect(() => {
    onPinned(!onSettings);
    return () => {
      onPinned(false);
    };
  }, [onSettings, onPinned]);
  const label = t(
    page === "notify" ? "notify.section" : page === "remote" ? "remote.section" : "settings.title",
  );

  return (
    <div className={styles["scrim"]} data-testid="settings-sheet">
      <div className={styles["sheet"]} role="dialog" aria-modal="true" aria-label={label}>
        <div className={styles["header"]}>
          <SettingsMark />
          <span className={styles["title"]}>{label}</span>
          {onSettings ? (
            <button type="button" className={styles["close"]} onClick={onClose}>
              {t("settings.done")}
            </button>
          ) : (
            <button
              type="button"
              className={styles["close"]}
              onClick={() => {
                setPage("settings");
              }}
            >
              {t("settings.back")}
            </button>
          )}
        </div>

        {page === "notify" && <NotifySection />}
        {page === "remote" && <RemoteSection />}

        {onSettings && settings.state === "loading" && (
          <p className={styles["message"]}>{t("settings.loading")}</p>
        )}
        {onSettings && settings.state === "failed" && (
          <p className={styles["message"]}>{t("settings.unreachable")}</p>
        )}

        {onSettings && settings.state === "ready" && (
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

            <Segmented
              label={t("settings.dockShape")}
              value={settings.value.dockShape}
              options={[
                { value: "bar" as const, label: t("settings.shapeBar") },
                { value: "ring" as const, label: t("settings.shapeRing") },
              ]}
              disabled={saving}
              onPick={(dockShape) => {
                onChange({ dockShape });
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

            {/* No "System" option; a stored `system` shows as the language it resolves to. */}
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

        {onSettings && (
          <>
            {/* Outside the settings block: each is read separately, so a failed settings read
                must not hide them. */}
            <div className={styles["accounts"]}>
              <NotifyRow
                onOpen={() => {
                  setPage("notify");
                }}
              />
              <RemoteRow
                onOpen={() => {
                  setPage("remote");
                }}
              />
            </div>

            <AccountsSection
              accounts={accounts}
              removal={removal}
              onRemove={onRemove}
              onDismissRemoval={onDismissRemoval}
            />
          </>
        )}
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

/** Every account with a way to remove it; the one Codex is using is removed by signing out. */
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

/** The amber mark on the account Codex is using; its meaning is in the tooltip. */
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

/** A failed removal; a failed sign-out also reports the rollback, worded as the switch does. */
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

/** The sliders mark, shared with the panel's settings button. */
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

/** `30s`, `5m`, `1h`. */
function intervalLabel(seconds: number): string {
  if (seconds < 60) {
    return `${seconds.toString()}s`;
  }
  if (seconds < 3600) {
    return `${(seconds / 60).toString()}m`;
  }
  return `${(seconds / 3600).toString()}h`;
}
