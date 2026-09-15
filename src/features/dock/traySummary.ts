/**
 * The tray menu's summary line, built with the panel's formatters so the two cannot disagree.
 * A window with no reading is an em dash, never a zero.
 */

import { t } from "../../i18n";
import type { AccountView, QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { isStale, percentLabel, windowValue } from "../quotas/format";

/** `Team · 5H 68% · W 42%`. Never includes the email: the tray is visible over a shoulder. */
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
    // "No account" only when nothing is managed, not when accounts exist but none is current.
    return t(hasAccounts ? "status.noCurrentAccount" : "bar.noAccount");
  }

  const name = account.value.displayName;
  if (quota.state !== "ready") {
    return t("tray.reading", { name });
  }

  const five = percentLabel(windowValue(quota.value, "five_hour"));
  const week = percentLabel(windowValue(quota.value, "weekly"));
  const line = `${name} · ${t("bar.fiveHour")} ${five} · ${t("bar.weekly")} ${week}`;
  // A cached reading says so here too.
  return isStale(quota.value, nowSeconds) ? `${line} · ${t("tray.cached")}` : line;
}
