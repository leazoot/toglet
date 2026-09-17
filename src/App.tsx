// The application shell: wires the stores to the dock and derives the active account, the bar
// notice and the status line.

import { useCallback, useEffect, useRef, useState } from "react";
import type { JSX } from "react";

import { useAccounts } from "./features/accounts/store";
import { AutoRunConfirm } from "./features/autorun/AutoRunConfirm";
import { notificationFor } from "./features/autorun/notifications";
import { useAutoRun } from "./features/autorun/store";
import { Dock } from "./features/dock/Dock";
import type { BarNotice } from "./features/dock/EdgeBar";
import type { PanelStatus } from "./features/dock/Panel";
import { compactReset, isStale } from "./features/quotas/format";
import { dueForRefresh, quotaOf, useQuota } from "./features/quotas/store";
import { AddAccountSheet } from "./features/onboarding/AddAccountSheet";
import { useAdding } from "./features/onboarding/store";
import { AutoRunSheet } from "./features/settings/AutoRunSheet";
import { SettingsSheet } from "./features/settings/SettingsSheet";
import { useSettings } from "./features/settings/store";
import { useStartup } from "./features/startup/store";
import { ResetOverlay } from "./features/accounts/ResetOverlay";
import { useReset } from "./features/accounts/resetStore";
import { resetNotification } from "./features/resets/notifications";
import { useResets } from "./features/resets/store";
import { SwitchOverlay } from "./features/switching/SwitchOverlay";
import { useSwitching } from "./features/switching/store";
import { traySummary } from "./features/dock/traySummary";
import { trayLabels } from "./features/dock/trayMenu";
import { resolveLanguage, t } from "./i18n";
import {
  deliverNotification,
  notify,
  onAccountsChanged,
  onAutoRunState,
  onResetAnnounced,
  onResetsState,
  onTrayRefresh,
  onTraySettings,
  onTrayShow,
  setTrayLabels,
  setTraySummary,
} from "./ipc";
import type {
  AccountView,
  AutoRunView,
  CheckId,
  EnvironmentReport,
  QuotaView,
  RecoveryOutcome,
} from "./types/ipc";
import type { Loadable } from "./types/load";

/** Used until the stored settings arrive. */
const DEFAULT_DOCK_EDGE = "right";

export function App(): JSX.Element {
  const accounts = useAccounts((state) => state.accounts);
  const loadAccounts = useAccounts((state) => state.load);
  const removal = useAccounts((state) => state.removal);
  const removeOne = useAccounts((state) => state.remove);
  const dismissRemoval = useAccounts((state) => state.dismissRemoval);
  const forgetQuota = useQuota((state) => state.forget);
  const environment = useStartup((state) => state.environment);
  const recovery = useStartup((state) => state.recovery);
  const loadStartup = useStartup((state) => state.load);
  const quotas = useQuota((state) => state.quotas);
  const refreshing = useQuota((state) => state.refreshing);
  const loadQuota = useQuota((state) => state.load);

  // Selected field by field so unrelated store writes do not re-render or re-run effects.
  const phase = useSwitching((state) => state.phase);
  const target = useSwitching((state) => state.target);
  const switchVerdict = useSwitching((state) => state.verdict);
  const step = useSwitching((state) => state.step);
  const result = useSwitching((state) => state.result);
  const switchFailure = useSwitching((state) => state.failure);
  const detailsOpen = useSwitching((state) => state.detailsOpen);
  const beginSwitching = useSwitching((state) => state.begin);
  const confirmSwitching = useSwitching((state) => state.confirm);
  const cancelSwitching = useSwitching((state) => state.cancel);
  const toggleDetails = useSwitching((state) => state.toggleDetails);
  const resetPhase = useReset((state) => state.phase);
  const resetTarget = useReset((state) => state.target);
  const resetHeld = useReset((state) => state.held);
  const resetResult = useReset((state) => state.result);
  const resetFailure = useReset((state) => state.failure);
  const beginReset = useReset((state) => state.begin);
  const confirmReset = useReset((state) => state.confirm);
  const dismissReset = useReset((state) => state.dismiss);

  const settings = useSettings((state) => state.settings);
  const saving = useSettings((state) => state.saving);
  const loadSettings = useSettings((state) => state.load);
  const replaceSettings = useSettings((state) => state.replace);
  const updateSettings = useSettings((state) => state.update);

  const addPhase = useAdding((state) => state.phase);
  const addedAccount = useAdding((state) => state.account);
  const addFailure = useAdding((state) => state.failure);
  const openAdd = useAdding((state) => state.open);
  const beginAdd = useAdding((state) => state.begin);
  const cancelAdd = useAdding((state) => state.cancel);
  const dismissAdd = useAdding((state) => state.dismiss);

  const plan = useAutoRun((state) => state.plan);
  const threads = useAutoRun((state) => state.threads);
  const draft = useAutoRun((state) => state.draft);
  const proposal = useAutoRun((state) => state.proposal);
  const committing = useAutoRun((state) => state.committing);
  const autoRunFailure = useAutoRun((state) => state.failure);
  const loadAutoRun = useAutoRun((state) => state.load);
  const replaceAutoRun = useAutoRun((state) => state.replace);
  const listThreads = useAutoRun((state) => state.listThreads);
  const editDraft = useAutoRun((state) => state.edit);
  const propose = useAutoRun((state) => state.propose);
  const dismissProposal = useAutoRun((state) => state.dismiss);
  const commitAutoRun = useAutoRun((state) => state.commit);
  const disableAutoRun = useAutoRun((state) => state.disable);
  const controlling = useAutoRun((state) => state.controlling);
  const controlAutoRun = useAutoRun((state) => state.control);

  const [expanded, setExpanded] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [autoRunOpen, setAutoRunOpen] = useState(false);
  // Set while the settings sheet is on a sub-page holding a credential being typed, which cannot
  // be shown again if lost; those pages keep the panel open.
  const [sheetPinned, setSheetPinned] = useState(false);

  // After the panel collapses the next opening starts at the list. A sign-in waiting on the
  // browser holds the panel open, so this never interrupts one.
  const closeSheets = useCallback(() => {
    setSettingsOpen(false);
    setAutoRunOpen(false);
    setSheetPinned(false);
    cancelAdd();
  }, [cancelAdd]);

  useEffect(() => {
    void loadAccounts();
    void loadStartup();
    void loadSettings();
    void loadAutoRun();
  }, [loadAccounts, loadStartup, loadSettings, loadAutoRun]);

  // Subscribed for the shell's lifetime: the scheduler changes the plan on its own.
  useEffect(() => {
    const stop = onAutoRunState(replaceAutoRun);
    return () => {
      void stop.then((off) => {
        off();
      });
    };
  }, [replaceAutoRun]);

  // Reset alerts: read once, then follow what the poll thread pushes. An announcement becomes
  // one sentence, sent to the desktop and to the channels chosen for it - by id, so a channel
  // switched off for continuation alerts still gets this one if it was picked here.
  const resets = useResets((state) => state.resets);
  const loadResets = useResets((state) => state.load);
  const replaceResets = useResets((state) => state.replace);
  useEffect(() => {
    void loadResets();
  }, [loadResets]);
  useEffect(() => {
    const stop = onResetsState(replaceResets);
    return () => {
      void stop.then((off) => {
        off();
      });
    };
  }, [replaceResets]);
  useEffect(() => {
    const stop = onResetAnnounced((event) => {
      const sentence = resetNotification(event, nowSeconds(), t);
      void notify(sentence.title, sentence.body);
      const current = useResets.getState().resets;
      const chosen = current.state === "ready" ? current.value.channelIds : [];
      for (const channelId of chosen) {
        void deliverNotification(sentence.title, sentence.body, channelId);
      }
    });
    return () => {
      void stop.then((off) => {
        off();
      });
    };
  }, []);

  // The scheduler switched accounts. Re-read the list; the active account is never inferred
  // from the plan.
  useEffect(() => {
    const stop = onAccountsChanged(() => {
      void loadAccounts();
    });
    return () => {
      void stop.then((off) => {
        off();
      });
    };
  }, [loadAccounts]);

  // Notifications come from comparing each plan with the previous one; the first plan is
  // compared with nothing, so a state present at start-up is not announced. The same text goes
  // to the desktop and to the configured channels, neither awaited.
  const previousPlan = useRef<AutoRunView | null>(null);
  useEffect(() => {
    if (plan.state !== "ready") {
      return;
    }
    const before = previousPlan.current;
    previousPlan.current = plan.value;
    const notification = notificationFor(
      before,
      plan.value,
      (id) => {
        const list = useAccounts.getState().accounts;
        return list.state === "ready"
          ? (list.value.find((account) => account.id === id)?.displayName ?? null)
          : null;
      },
      t,
    );
    if (notification !== null) {
      void notify(notification.title, notification.body);
      void deliverNotification(notification.title, notification.body);
    }
  }, [plan]);

  const account = activeAccount(accounts);
  const activeId = account.state === "ready" ? (account.value?.id ?? null) : null;
  const allIds = accounts.state === "ready" ? accounts.value.map((one) => one.id) : [];
  const ids = allIds.join(" ");
  const hasAccounts = allIds.length > 0;

  // The bar only ever shows the active account, so that is the only reading start-up needs.
  useEffect(() => {
    if (activeId !== null) {
      void loadQuota([activeId], nowSeconds());
    }
  }, [activeId, loadQuota]);

  // Opening the panel re-reads only readings that are missing, failed or older than two minutes,
  // since each read starts an app server. Readings come from the store, not render state, so the
  // effect runs on opening rather than on every arriving reading.
  useEffect(() => {
    if (!expanded || ids === "") {
      return;
    }
    const due = dueForRefresh(useQuota.getState().quotas, ids.split(" "), nowSeconds());
    if (due.length > 0) {
      void loadQuota(due, nowSeconds());
    }
  }, [expanded, ids, loadQuota]);

  const refresh = useCallback(() => {
    if (ids !== "") {
      void loadQuota(ids.split(" "), nowSeconds());
    }
  }, [ids, loadQuota]);

  // Scheduled polls: separate intervals for the active and other accounts. Nothing is scheduled
  // until the settings arrive.
  const activeSeconds = settings.state === "ready" ? settings.value.activeRefreshSeconds : null;
  const inactiveSeconds = settings.state === "ready" ? settings.value.inactiveRefreshSeconds : null;

  useEffect(() => {
    if (activeSeconds === null || activeId === null) {
      return undefined;
    }
    const timer = setInterval(() => {
      void loadQuota([activeId], nowSeconds());
    }, activeSeconds * 1000);
    return () => {
      clearInterval(timer);
    };
  }, [activeSeconds, activeId, loadQuota]);

  useEffect(() => {
    // Derived from the joined `ids` string so the timer restarts only when the accounts change.
    const others = ids.split(" ").filter((one) => one !== "" && one !== activeId);
    if (inactiveSeconds === null || others.length === 0) {
      return undefined;
    }
    const timer = setInterval(() => {
      void loadQuota(others, nowSeconds());
    }, inactiveSeconds * 1000);
    return () => {
      clearInterval(timer);
    };
  }, [inactiveSeconds, activeId, ids, loadQuota]);

  const beginSwitch = useCallback(
    (account: AccountView) => {
      void beginSwitching(account);
    },
    [beginSwitching],
  );

  // "Check again" and "Try again" restart the flow so clients are re-probed; only `confirm` runs
  // the switch itself.
  const confirmSwitch = useCallback(() => {
    if (phase === "confirm") {
      void confirmSwitching(nowSeconds());
    } else if (target !== null) {
      void beginSwitching(target);
    }
  }, [phase, target, beginSwitching, confirmSwitching]);

  // Re-read the list after a sign-in or a switch; the active account is never inferred here. A
  // duplicate sign-in can change it too: Rust marks the account current when Codex is signed in
  // as it.
  useEffect(() => {
    if (addPhase === "added" || addPhase === "duplicate") {
      void loadAccounts();
    }
  }, [addPhase, loadAccounts]);

  useEffect(() => {
    if (phase !== "done") {
      return undefined;
    }
    void loadAccounts();
    const settle = setTimeout(cancelSwitching, SUCCESS_DWELL_MS);
    return () => {
      clearTimeout(settle);
    };
  }, [phase, loadAccounts, cancelSwitching]);

  // Esc dismisses whatever awaits an answer. `cancel` refuses to stop a switch under way.
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        cancelSwitching();
        cancelAdd();
        dismissProposal();
        setSettingsOpen(false);
        setAutoRunOpen(false);
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [cancelSwitching, cancelAdd, dismissProposal]);

  const now = nowSeconds();

  const summary = traySummary(account, quotaOf(quotas, activeId), now, hasAccounts);
  useEffect(() => {
    void setTraySummary(summary);
  }, [summary]);

  // The OS draws the tray menu, so labels are pushed whenever the resolved language changes,
  // including when settings first arrive.
  const language = settings.state === "ready" ? resolveLanguage(settings.value.language) : null;
  useEffect(() => {
    if (language !== null) {
      void setTrayLabels(trayLabels(language));
    }
  }, [language]);

  // "Show" and "settings" both open the panel: the bar is always visible, and a sheet inside a
  // closed panel would be invisible.
  useEffect(() => {
    const stops = [
      onTrayShow(() => {
        setExpanded(true);
      }),
      onTrayRefresh(refresh),
      onTraySettings(() => {
        setSettingsOpen(true);
        setExpanded(true);
      }),
    ];
    return () => {
      for (const stop of stops) {
        void stop.then((off) => {
          off();
        });
      }
    };
  }, [refresh]);

  return (
    <Dock
      // From the settings Rust returns on every save, so the bar follows the window's edge.
      side={settings.state === "ready" ? settings.value.dockEdge : DEFAULT_DOCK_EDGE}
      shape={settings.state === "ready" ? settings.value.dockShape : "bar"}
      expanded={expanded}
      onExpandedChange={setExpanded}
      offset={settings.state === "ready" ? settings.value.verticalOffset : 0}
      onDragSettled={replaceSettings}
      account={account}
      accounts={accounts}
      quotas={quotas}
      activeQuota={quotaOf(quotas, activeId)}
      refreshing={refreshing}
      notice={noticeFor(accounts, environment, recovery, account)}
      status={statusFor(
        accounts,
        environment,
        recovery,
        activeId,
        quotaOf(quotas, activeId),
        refreshing,
        now,
      )}
      nowSeconds={now}
      onRefresh={refresh}
      onSelect={beginSwitch}
      onResetCredits={beginReset}
      onOpenSettings={() => {
        setAutoRunOpen(false);
        setSettingsOpen(true);
      }}
      onOpenAutoRun={() => {
        setSettingsOpen(false);
        setAutoRunOpen(true);
      }}
      onAddAccount={() => {
        openAdd();
      }}
      // Kept open for a sign-in waiting on the browser, a removal or its report, and a pinned
      // settings sub-page.
      held={addPhase === "waiting" || (settingsOpen && (removal !== null || sheetPinned))}
      onCollapsed={closeSheets}
      autorun={plan.state === "ready" ? plan.value : null}
      resets={resets.state === "ready" ? resets.value : null}
      autorunBusy={controlling}
      autorunFailure={autoRunFailure}
      onAutoRunControl={(action) => {
        void controlAutoRun(action);
      }}
      sheet={
        addPhase !== "idle" ? (
          <AddAccountSheet
            phase={addPhase}
            account={addedAccount}
            noCurrentAccount={account.state === "ready" && account.value === null}
            failure={addFailure}
            onBegin={() => {
              void beginAdd(nowSeconds());
            }}
            onCancel={() => {
              cancelAdd();
            }}
            onDone={() => {
              dismissAdd();
            }}
            onSwitch={(added) => {
              dismissAdd();
              beginSwitch(added);
            }}
          />
        ) : settingsOpen ? (
          <SettingsSheet
            settings={settings}
            saving={saving}
            accounts={accounts}
            removal={removal}
            onRemove={(account) => {
              void removeOne(account, nowSeconds()).then((removed) => {
                if (removed) {
                  forgetQuota(account.id);
                }
              });
            }}
            onDismissRemoval={dismissRemoval}
            onPinned={setSheetPinned}
            onChange={(patch) => {
              void updateSettings(patch);
            }}
            onClose={() => {
              setSettingsOpen(false);
            }}
          />
        ) : autoRunOpen ? (
          <AutoRunSheet
            plan={plan}
            accounts={accounts}
            threads={threads}
            draft={draft}
            busy={committing}
            failure={autoRunFailure}
            nowSeconds={now}
            onEdit={editDraft}
            onListThreads={() => {
              void listThreads();
            }}
            onEnable={propose}
            onDisable={() => {
              void disableAutoRun();
            }}
            onClose={() => {
              setAutoRunOpen(false);
            }}
          />
        ) : null
      }
      overlay={
        // Any non-null overlay pins the panel open. A begun switch outranks spending a credit,
        // which in turn outranks the continuation confirmation: both of those were asked for by
        // a click, while the proposal appears on its own.
        phase === "idle" ? (
          resetPhase !== "idle" ? (
            <ResetOverlay
              phase={resetPhase}
              target={resetTarget}
              held={resetHeld}
              result={resetResult}
              failure={resetFailure}
              onConfirm={() => {
                void confirmReset(nowSeconds());
              }}
              onDismiss={dismissReset}
            />
          ) : proposal === null ? null : (
            <AutoRunConfirm
              draft={proposal}
              accounts={accounts}
              nowSeconds={now}
              committing={committing}
              failure={autoRunFailure}
              onConfirm={() => {
                void commitAutoRun(nowSeconds());
              }}
              onCancel={dismissProposal}
            />
          )
        ) : (
          <SwitchOverlay
            phase={phase}
            target={target}
            verdict={switchVerdict}
            step={step}
            result={result}
            unreachable={switchFailure !== null}
            detailsOpen={detailsOpen}
            onConfirm={confirmSwitch}
            onCancel={cancelSwitching}
            onToggleDetails={toggleDetails}
          />
        )
      }
    />
  );
}

/** How long the success overlay stays before the list comes back. */
const SUCCESS_DWELL_MS = 1100;

/** Below this, a reading is "just now" rather than an age. */
const SECONDS_PER_MINUTE = 60;

function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

/** The account Rust marked active; `isActive` is only written after a verified switch. */
function activeAccount(accounts: Loadable<readonly AccountView[]>): Loadable<AccountView | null> {
  if (accounts.state !== "ready") {
    return accounts;
  }
  return {
    state: "ready",
    value: accounts.value.find((candidate) => candidate.isActive) ?? null,
  };
}

/**
 * Checks whose failure means Codex cannot be managed. The sign-in checks are excluded: a
 * signed-out Codex is an ordinary state, not a fault.
 */
const UNMANAGEABLE_WHEN_FAILED: ReadonlySet<CheckId> = new Set<CheckId>([
  "operatingSystem",
  "codexCommand",
  "appServerMethods",
  "defaultCodexHome",
  "configFile",
]);

function codexUnmanageable(environment: Loadable<EnvironmentReport>): boolean {
  return (
    environment.state === "ready" &&
    environment.value.checks.some(
      (check) => check.status === "failed" && UNMANAGEABLE_WHEN_FAILED.has(check.id),
    )
  );
}

/**
 * The bar's notice, most serious first: an unrepaired switch (the account in use is uncertain),
 * an unmanageable Codex, failed reads, then an account needing sign-in.
 */
function noticeFor(
  accounts: Loadable<readonly AccountView[]>,
  environment: Loadable<EnvironmentReport>,
  recovery: Loadable<RecoveryOutcome | null>,
  account: Loadable<AccountView | null>,
): BarNotice | null {
  if (recovery.state === "ready" && recovery.value === "failed") {
    return "recovery_failed";
  }
  if (codexUnmanageable(environment)) {
    return "environment_failed";
  }
  // A read that failed outright. `notApplicable` checks are not faults.
  if (accounts.state === "failed" || environment.state === "failed") {
    return "unreadable";
  }
  if (account.state === "ready" && account.value?.status === "reauth_required") {
    return "reauth_required";
  }
  return null;
}

/** The panel's status line, in the same order of seriousness. */
function statusFor(
  accounts: Loadable<readonly AccountView[]>,
  environment: Loadable<EnvironmentReport>,
  recovery: Loadable<RecoveryOutcome | null>,
  activeId: string | null,
  quota: Loadable<QuotaView>,
  refreshing: boolean,
  nowSeconds: number,
): PanelStatus {
  if (recovery.state === "ready" && recovery.value === "failed") {
    return { tone: "bad", key: "status.recoveryFailed" };
  }
  if (codexUnmanageable(environment)) {
    return { tone: "bad", key: "status.environment" };
  }
  if (accounts.state === "failed" || environment.state === "failed") {
    return { tone: "bad", key: "status.unreadable" };
  }
  if (refreshing) {
    return { tone: "mute", key: "status.refreshing" };
  }
  if (accounts.state === "ready" && accounts.value.length === 0) {
    return { tone: "mute", key: "status.noAccounts" };
  }
  // No verified current account means no quota is being read.
  if (accounts.state === "ready" && activeId === null) {
    return { tone: "mute", key: "status.noCurrentAccount" };
  }
  if (quota.state === "failed") {
    return { tone: "warn", key: "status.unreadable" };
  }
  if (quota.state !== "ready") {
    return { tone: "mute", key: "status.refreshing" };
  }
  if (isStale(quota.value, nowSeconds)) {
    return { tone: "warn", key: "status.cached" };
  }
  // Compare seconds, not the localised compact string (`0m` reads `0分` in Chinese).
  if (nowSeconds - quota.value.fetchedAt < SECONDS_PER_MINUTE) {
    return { tone: "ok", key: "status.justNow" };
  }
  // `compactReset` with the arguments swapped yields the reading's age.
  return {
    tone: "ok",
    key: "status.ready",
    params: { when: compactReset(nowSeconds, quota.value.fetchedAt) },
  };
}
