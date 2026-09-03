// The application shell.
//
// It holds no interface of its own: it reads the stores, works out the two things the docked
// surface cannot derive - which account is active, and what the status bar should say - and hands
// the window's size back to Rust when the panel opens.

import { useCallback, useEffect, useState } from "react";
import type { JSX } from "react";

import { useAccounts } from "./features/accounts/store";
import { Dock } from "./features/dock/Dock";
import type { BarNotice } from "./features/dock/EdgeBar";
import type { PanelStatus } from "./features/dock/Panel";
import { compactReset, isStale } from "./features/quotas/format";
import { dueForRefresh, quotaOf, useQuota } from "./features/quotas/store";
import { AddAccountSheet } from "./features/onboarding/AddAccountSheet";
import { useAdding } from "./features/onboarding/store";
import { SettingsSheet } from "./features/settings/SettingsSheet";
import { useSettings } from "./features/settings/store";
import { useStartup } from "./features/startup/store";
import { SwitchOverlay } from "./features/switching/SwitchOverlay";
import { useSwitching } from "./features/switching/store";
import { traySummary } from "./features/dock/traySummary";
import { trayLabels } from "./features/dock/trayMenu";
import { resolveLanguage } from "./i18n";
import {
  onTrayRefresh,
  onTraySettings,
  onTrayShow,
  setDockExpansion,
  setTrayLabels,
  setTraySummary,
} from "./ipc";
import type {
  AccountView,
  CheckId,
  EnvironmentReport,
  QuotaView,
  RecoveryOutcome,
} from "./types/ipc";
import type { Loadable } from "./types/load";

/** The side to draw against while the stored one is still on its way. */
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

  // Selected field by field rather than as a whole: taking the object would re-render the shell
  // on every store write and give the effects below a new identity each time.
  const phase = useSwitching((state) => state.phase);
  const target = useSwitching((state) => state.target);
  const step = useSwitching((state) => state.step);
  const result = useSwitching((state) => state.result);
  const switchFailure = useSwitching((state) => state.failure);
  const detailsOpen = useSwitching((state) => state.detailsOpen);
  const beginSwitching = useSwitching((state) => state.begin);
  const confirmSwitching = useSwitching((state) => state.confirm);
  const cancelSwitching = useSwitching((state) => state.cancel);
  const toggleDetails = useSwitching((state) => state.toggleDetails);

  const settings = useSettings((state) => state.settings);
  const saving = useSettings((state) => state.saving);
  const loadSettings = useSettings((state) => state.load);
  const replaceSettings = useSettings((state) => state.replace);
  const updateSettings = useSettings((state) => state.update);

  const addPhase = useAdding((state) => state.phase);
  const addedAccount = useAdding((state) => state.account);
  const openAdd = useAdding((state) => state.open);
  const beginAdd = useAdding((state) => state.begin);
  const cancelAdd = useAdding((state) => state.cancel);
  const dismissAdd = useAdding((state) => state.dismiss);

  const [expanded, setExpanded] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    void loadAccounts();
    void loadStartup();
    void loadSettings();
  }, [loadAccounts, loadStartup, loadSettings]);

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

  // Opening the panel re-reads what is missing, failed, or more than two minutes old - not
  // everything: a reading a few seconds old is still the answer, and asking again on every hover
  // started an app server for each account each time the pointer rested on the bar. The readings
  // are taken from the store rather than from render state so the effect runs when the panel
  // opens, not again for every reading that then arrives.
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

  // The scheduled poll. Two intervals: the account in use is read more often than the rest,
  // which is the whole point of having two settings. Nothing is scheduled until the settings
  // have arrived - guessing an interval would make the first poll happen at a rate nobody chose.
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
    // Split back out of `ids` rather than reusing the array: a fresh array every render would
    // restart the timer every render, while the joined string only changes when the accounts do.
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

  // The window never changes size; what changes is what the pointer can reach. While the panel
  // is open the whole window is surface, and while it is not the transparent strip lets clicks
  // through to the desktop. Rust makes that decision and needs to know which state this is.
  useEffect(() => {
    void setDockExpansion(expanded);
  }, [expanded]);

  const beginSwitch = useCallback(
    (account: AccountView) => {
      void beginSwitching(account);
    },
    [beginSwitching],
  );

  // "Check again" and "Try again" both start the flow over rather than continuing it: the first
  // has to re-probe the clients, and the second has to re-check everything the failure may have
  // changed. Only `confirm` runs the switch itself.
  const confirmSwitch = useCallback(() => {
    if (phase === "confirm") {
      void confirmSwitching(nowSeconds());
    } else if (target !== null) {
      void beginSwitching(target);
    }
  }, [phase, target, beginSwitching, confirmSwitching]);

  // The account list is what says who is active, so it is re-read once a switch has finished.
  // Nothing about the active account is inferred from the switch having succeeded.
  // A sign-in that produced a new account changes the list, so the list is re-read rather than
  // having the new account spliced in here. One that produced an account already held can have
  // changed the list too: Rust claims the account as the current one when the default home turns
  // out to be signed in as it. Found on the real machine - the duplicate was reported,
  // Rust had recorded it as current, and the bar went on showing no current account.
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

  // Esc dismisses whatever is waiting for an answer. A switch already under way is
  // not dismissible, and `cancel` is what enforces that rather than this listener.
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        cancelSwitching();
        cancelAdd();
        setSettingsOpen(false);
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [cancelSwitching, cancelAdd]);

  const now = nowSeconds();

  // The tray shows the current account and its two windows, so the two facts checked
  // most often are readable without opening anything.
  const summary = traySummary(account, quotaOf(quotas, activeId), now, hasAccounts);
  useEffect(() => {
    void setTraySummary(summary);
  }, [summary]);

  // The menu is drawn by the operating system, so it is the one surface a re-render cannot
  // reach. It is pushed across whenever the language settles on a different one - including the
  // first time the settings arrive, which is what replaces the English the tray was built with.
  const language = settings.state === "ready" ? resolveLanguage(settings.value.language) : null;
  useEffect(() => {
    if (language !== null) {
      void setTrayLabels(trayLabels(language));
    }
  }, [language]);

  // The tray can only ask; it holds no quota and no settings sheet of its own. Both "show" and
  // "settings" open the panel: the bar is always on screen, so showing has to mean the panel, and
  // a settings sheet inside a closed panel is a menu entry that appears to do nothing.
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
      // Straight from the settings, which are re-read from Rust on every save. Asking a second
      // command for the same stored value is what let the window move to the other edge while the
      // bar went on mirroring against the one it had left.
      side={settings.state === "ready" ? settings.value.dockEdge : DEFAULT_DOCK_EDGE}
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
      onOpenSettings={() => {
        setSettingsOpen(true);
      }}
      onAddAccount={openAdd}
      sheet={
        addPhase !== "idle" ? (
          <AddAccountSheet
            phase={addPhase}
            account={addedAccount}
            noCurrentAccount={account.state === "ready" && account.value === null}
            onBegin={() => {
              void beginAdd(nowSeconds());
            }}
            onCancel={cancelAdd}
            onDone={dismissAdd}
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
            onChange={(patch) => {
              void updateSettings(patch);
            }}
            onClose={() => {
              setSettingsOpen(false);
            }}
          />
        ) : null
      }
      overlay={
        // `null` while idle, and only then. Anything else pins the panel open - including the
        // moment between the click and the client probe answering.
        phase === "idle" ? null : (
          <SwitchOverlay
            phase={phase}
            target={target}
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

/** How long the success overlay stays before the list comes back (the design's ≈1.1s). */
const SUCCESS_DWELL_MS = 1100;

/** Below this, a reading is "just now" rather than an age. */
const SECONDS_PER_MINUTE = 60;

function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

/**
 * The account Rust marked active.
 *
 * Never inferred from anything that happened in the interface: `isActive` is written only after
 * a switch has been verified, and copying it is the whole of the rule.
 */
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
 * The checks whose failure means Codex cannot be managed on this machine (the first five). The
 * last two describe the sign-in Codex currently has, and the report keeps them as checks because
 * the start-up report lists every item - but "no importable account" is what a signed-out Codex
 * looks like, which after a sign-out through Toglet is the ordinary case, not a fault. Reporting
 * it as "Codex cannot be managed" was false.
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
 * What the amber dot on the bar means, most serious first.
 *
 * The order is the order of consequence. An interrupted switch that could not be repaired comes
 * first: it is the one case where the user cannot be sure which account Codex would actually
 * use. A Codex that cannot be managed comes next, because nothing else in Toglet will work
 * either. Only then the reads, and last the one case the design drew the dot for.
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
  // A read that did not come back at all. `notApplicable` checks are deliberately not counted:
  // a check that could not run has not found anything wrong, and reporting it as a fault would
  // be the mirror image of reporting it as a pass.
  if (accounts.state === "failed" || environment.state === "failed") {
    return "unreadable";
  }
  if (account.state === "ready" && account.value?.status === "reauth_required") {
    return "reauth_required";
  }
  return null;
}

/**
 * The status bar, in the same order of seriousness.
 *
 * This is where "the numbers you are looking at are cached" belongs. On the bar it could only be
 * a tooltip; here it is a sentence with a coloured dot beside it, and the dot is never the only
 * carrier.
 */
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
  // Accounts, but none verified as current: there is no quota to be reading, so "reading quota"
  // would be a claim about work that is not happening.
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
  // Whether a reading counts as "just now" is decided on the seconds, not on the words. The
  // compact form is copy - it reads `0分` in Chinese - so testing it against `0m` would have
  // quietly turned every fresh reading into "read 0 minutes ago" for half the users.
  if (nowSeconds - quota.value.fetchedAt < SECONDS_PER_MINUTE) {
    return { tone: "ok", key: "status.justNow" };
  }
  // `compactReset` counts forward to a moment; the age of a reading is the same arithmetic with
  // the arguments the other way round.
  return {
    tone: "ok",
    key: "status.ready",
    params: { when: compactReset(nowSeconds, quota.value.fetchedAt) },
  };
}
