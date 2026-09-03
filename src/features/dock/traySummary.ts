/**
 * The one line the tray menu shows: the current account and its two quota windows.
 *
 * Built from the same values and the same formatters the panel uses, so the tray and the panel
 * cannot disagree about the same number. That is the whole reason this lives here rather than in
 * Rust: a second formatter is a second source of truth.
 *
 * The three-state rules carry over unchanged - a window with no reading is an em dash, never a
 * zero.
 */

import { t } from "../../i18n";
import type { AccountView, QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { isStale, percentLabel, windowValue } from "../quotas/format";

/**
 * `Team · 5H 68% · W 42%`, or an honest sentence when there is nothing to summarise.
 *
 * Kept to one line and free of the account's address: a tray menu is visible to anyone looking
 * over a shoulder, and the address adds nothing the name does not already say.
 */
export function traySummary(
  account: Loadable<AccountView | null>,
  quota: Loadable<QuotaView>,
  nowSeconds: number,
  hasAccounts: boolean,
): string {
  if (account.state === "failed") {
    return t("tray.unreadable");
  }
  if (account.state === "loading") {
    return t("tray.loading");
  }
  if (account.value === null) {
    // Two different facts. "No account" when nothing is managed; when accounts exist but none is
    // verified as current, saying "no account" beside a list of them is simply false.
    return t(hasAccounts ? "status.noCurrentAccount" : "bar.noAccount");
  }

  const name = account.value.displayName;
  if (quota.state !== "ready") {
    return t("tray.reading", { name });
  }

  const five = percentLabel(windowValue(quota.value, "five_hour"));
  const week = percentLabel(windowValue(quota.value, "weekly"));
  const line = `${name} · ${t("bar.fiveHour")} ${five} · ${t("bar.weekly")} ${week}`;
  // A cached reading says so here too. The tray is often the only thing on screen.
  return isStale(quota.value, nowSeconds) ? `${line} · ${t("tray.cached")}` : line;
}
