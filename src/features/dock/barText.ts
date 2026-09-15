// The sentences both collapsed shapes (EdgeBar, RingBar) share, so they cannot disagree.

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import type { AccountView, QuotaView, QuotaWindowKind } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { compactReset, percentLabel, windowValue } from "../quotas/format";
import type { QuotaValue } from "../quotas/format";

/** Why the amber dot on the avatar is lit. */
export type BarNotice = "reauth_required" | "unreadable" | "environment_failed" | "recovery_failed";

export function emptyKey(account: Loadable<AccountView | null>, hasAccounts: boolean): MessageKey {
  switch (account.state) {
    case "loading":
      return "bar.loadingAccount";
    case "failed":
      return "bar.notice.unreadable";
    case "ready":
      return hasAccounts ? "status.noCurrentAccount" : "bar.noAccount";
  }
}

/** A reading in flight draws the same dashed ring; the description says it is not a failure. */
export function valueOf(quota: Loadable<QuotaView>, window: QuotaWindowKind): QuotaValue {
  switch (quota.state) {
    case "ready":
      return windowValue(quota.value, window);
    case "failed":
    case "loading":
      return { kind: "unreadable" };
  }
}

export function describe(
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

export function noticeKey(notice: BarNotice): MessageKey {
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
