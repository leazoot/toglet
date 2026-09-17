/**
 * What the reset banner says. One sentence, chosen by what matters most right now, plus a gauge
 * that draws two figures the feed gives. Nothing here counts down or predicts: a forecast is
 * shown as the feed's forecast, an unknown figure as an unknown.
 */

import type { MessageKey, MessageParams } from "../../i18n";
import type { ResetsView, WatchLevel } from "../../types/ipc";
import { clockTime } from "../quotas/format";

export type Translate = (key: MessageKey, params?: MessageParams) => string;

/** The gauge's colour. Never the only carrier - the sentence says it too. */
export type BannerTone = "ok" | "warn" | "bad" | "mute";

export interface BannerLine {
  readonly tone: BannerTone;
  readonly text: string;
  /** The feed's own sentence for the item shown, for a tooltip; nowhere else. */
  readonly detail: string | null;
  /** Shown beside the text when the reading is old or the last read failed. */
  readonly asOf: string | null;
  /** Share of the average interval elapsed since the last reset, 0..1; `null` when unknown. */
  readonly gauge: number | null;
  readonly gaugeLabel: string;
  /** A reset within the last day: the one moment that earns a brighter mark. */
  readonly fresh: boolean;
}

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86_400;

/** `just now`, `12m ago`, `3h ago`, `5d ago`. Floors, so nothing is claimed early. */
export function ago(atSeconds: number, nowSeconds: number, t: Translate): string {
  const elapsed = Math.max(0, nowSeconds - atSeconds);
  if (elapsed < MINUTE) {
    return t("resets.ago.now");
  }
  if (elapsed < HOUR) {
    return t("resets.ago.minutes", { n: Math.floor(elapsed / MINUTE) });
  }
  if (elapsed < DAY) {
    return t("resets.ago.hours", { n: Math.floor(elapsed / HOUR) });
  }
  return t("resets.ago.days", { n: Math.floor(elapsed / DAY) });
}

/** `70%`. A missing figure never reaches here: those sentences have their own key. */
export function chance(percent: number): string {
  return `${percent.toString()}%`;
}

export function levelKey(level: WatchLevel): MessageKey {
  return level === "strong" ? "resets.level.strong" : "resets.level.elevated";
}

/** The sentence for a failed reading, by stable code. */
export function failureText(code: string, t: Translate): string {
  switch (code) {
    case "network_unavailable":
      return t("resets.banner.network");
    case "reset_feed_unreadable":
      return t("resets.banner.unreadable");
    default:
      return t("resets.banner.failed", { code });
  }
}

/** One decimal, without a trailing `.0`: `6.9`, `7`. */
function figure(value: number): string {
  return Number(value.toFixed(1)).toString();
}

export function bannerLine(view: ResetsView, nowSeconds: number, t: Translate): BannerLine {
  const status = view.status;
  const gauge = gaugeOf(view);
  const gaugeLabel =
    status?.stats.daysSinceLast != null && status.stats.avgIntervalDays != null
      ? t("resets.banner.gauge", {
          days: figure(status.stats.daysSinceLast),
          interval: figure(status.stats.avgIntervalDays),
        })
      : t("resets.banner.gaugeUnknown");
  const asOf =
    view.fetchedAt !== null && (view.stale || view.lastError !== null)
      ? t("resets.banner.asOf", { ago: ago(view.fetchedAt, nowSeconds, t) })
      : null;

  if (status === null) {
    return {
      tone: view.lastError === null ? "mute" : "bad",
      text: view.lastError === null ? t("resets.banner.loading") : failureText(view.lastError, t),
      detail: null,
      asOf: null,
      gauge: null,
      gaugeLabel,
      fresh: false,
    };
  }

  const base = { asOf, gauge, gaugeLabel };
  const latest = status.latestReset;

  // A reset that just happened outranks everything: it is the news.
  if (latest !== null && nowSeconds - latest.announcedAt < DAY) {
    return {
      ...base,
      tone: "ok",
      fresh: true,
      detail: latest.text,
      text: t(latest.kind === "banked" ? "resets.banner.banked" : "resets.banner.justReset", {
        ago: ago(latest.announcedAt, nowSeconds, t),
      }),
    };
  }
  // Then something concrete that is still to come.
  const scheduled = status.scheduledReset;
  if (scheduled !== null) {
    return {
      ...base,
      tone: "warn",
      fresh: false,
      detail: scheduled.text,
      text:
        scheduled.scheduledFor === null
          ? t("resets.banner.scheduledNoTime")
          : t("resets.banner.scheduled", { when: clockTime(scheduled.scheduledFor, nowSeconds) }),
    };
  }
  // Then a forecast, as long as the feed still stands by it.
  const watch = status.activeWatch;
  if (watch !== null && watch.expiresAt > nowSeconds) {
    const level = t(levelKey(watch.level));
    return {
      ...base,
      tone: "warn",
      fresh: false,
      detail: watch.text,
      text:
        watch.chancePercent === null
          ? t("resets.banner.watchNoChance", { level })
          : t("resets.banner.watch", { level, chance: chance(watch.chancePercent) }),
    };
  }
  // Otherwise the ordinary day: how long since the last one, and how often they come.
  if (latest !== null) {
    const when = ago(latest.announcedAt, nowSeconds, t);
    return {
      ...base,
      tone: "mute",
      fresh: false,
      detail: latest.text,
      text:
        status.stats.avgIntervalDays === null
          ? t("resets.banner.latestNoAverage", { ago: when })
          : t("resets.banner.latest", {
              ago: when,
              interval: figure(status.stats.avgIntervalDays),
            }),
    };
  }
  return { ...base, tone: "mute", fresh: false, detail: null, text: t("resets.banner.none") };
}

/** Elapsed share of the average interval, clamped to a full ring. Both figures are the feed's. */
function gaugeOf(view: ResetsView): number | null {
  const days = view.status?.stats.daysSinceLast ?? null;
  const interval = view.status?.stats.avgIntervalDays ?? null;
  if (days === null || interval === null || interval <= 0) {
    return null;
  }
  return Math.min(1, Math.max(0, days / interval));
}
