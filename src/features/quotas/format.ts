/**
 * The single place quota percentages, tones, ring arcs and reset times are derived.
 * `not_returned` and `unreadable` render as an em dash on a dashed track, never as `0%`.
 */

import type { QuotaTone } from "../../components/QuotaRing";
import { activeLanguage } from "../../i18n";
import type { Language } from "../../i18n";
import type { QuotaView, QuotaWindowKind, ResetCreditsView } from "../../types/ipc";
import type { Loadable } from "../../types/load";

export type QuotaValue =
  | {
      readonly kind: "value";
      readonly remainingPercent: number;
      readonly resetsAt: number | null;
    }
  | { readonly kind: "not_returned" }
  | { readonly kind: "unreadable" };

/** Circumference for r=16.5. Duplicated in `src/styles/tokens.css`; a test keeps them equal. */
export const RING_CIRCUMFERENCE = 103.67;

/** The ring form's circumferences (r=16.5 and r=11), also duplicated in the tokens. */
export const RING_FORM_OUTER_CIRCUMFERENCE = RING_CIRCUMFERENCE;
export const RING_FORM_INNER_CIRCUMFERENCE = 69.12;

/** What Rust considers fresh (`quota::cache::STALE_AFTER_SECONDS`). */
export const STALE_AFTER_SECONDS = 600;

/** A window missing from the list was not returned by the server; it is never filled in. */
export function windowValue(view: QuotaView, kind: QuotaWindowKind): QuotaValue {
  const found = view.windows.find((window) => window.kind === kind);
  if (found === undefined) {
    return { kind: "not_returned" };
  }
  if (!Number.isFinite(found.remainingPercent)) {
    // Present but not a number: a failed reading, not zero.
    return { kind: "unreadable" };
  }
  return {
    kind: "value",
    remainingPercent: found.remainingPercent,
    resetsAt: found.resetsAt,
  };
}

/**
 * How many reset credits the account holds, or `null` when there is nothing to show: the reading
 * failed, the server never reported them, or it reported none.
 */
export function resetCreditsOf(quota: Loadable<QuotaView>): ResetCreditsView | null {
  if (quota.state !== "ready" || quota.value.resetCredits === null) {
    return null;
  }
  return quota.value.resetCredits.availableCount > 0 ? quota.value.resetCredits : null;
}

/** Checked against the current clock too: Rust's flag only reflects freshness when it answered. */
export function isStale(view: QuotaView, nowSeconds: number): boolean {
  return view.stale || nowSeconds - view.fetchedAt > STALE_AFTER_SECONDS;
}

/** `68%`, or an em dash when there is no number to show. */
export function percentLabel(value: QuotaValue): string {
  if (value.kind !== "value") {
    return "—";
  }
  return `${Math.round(clampPercent(value.remainingPercent)).toString()}%`;
}

/**
 * `stroke-dasharray` for the ø38 ring. No value draws no arc; the dashed track underneath is what
 * distinguishes "unknown" from 0%.
 */
export function ringDash(value: QuotaValue): string {
  return arcDash(value, RING_CIRCUMFERENCE);
}

/** The same arc on a ring of any circumference. */
export function arcDash(value: QuotaValue, circumference: number): string {
  const whole = circumference.toFixed(2);
  if (value.kind !== "value") {
    return `0 ${whole}`;
  }
  const arc = (clampPercent(value.remainingPercent) / 100) * circumference;
  return `${arc.toFixed(1)} ${whole}`;
}

/** Status band: ≥50 healthy, 20-49 warn, 1-19 low, 0 empty. */
export function tone(value: QuotaValue): QuotaTone {
  if (value.kind !== "value") {
    return "unreadable";
  }
  const remaining = clampPercent(value.remainingPercent);
  if (remaining >= 50) {
    return "healthy";
  }
  if (remaining >= 20) {
    return "warn";
  }
  // Round first so the tone matches the label: 0.4% reads "0%" and must look exhausted.
  return Math.round(remaining) === 0 ? "empty" : "low";
}

/**
 * Compact reset: `51m`, `2h 14m`, `Mon 09:00`, `Aug 31` (zh: `51分`, `2小时14分`, `周一 09:00`,
 * `8月31日`). A past reset reads as zero, never negative. The result is localised copy: never
 * compare it against a literal, compare the seconds.
 */
export function compactReset(resetsAt: number, nowSeconds: number): string {
  const seconds = Math.max(0, resetsAt - nowSeconds);
  const minutes = Math.floor(seconds / 60);
  const chinese = activeLanguage() === "zh";

  if (minutes < 60) {
    return chinese ? `${minutes.toString()}分` : `${minutes.toString()}m`;
  }
  if (minutes < 24 * 60) {
    const hours = Math.floor(minutes / 60).toString();
    const rest = (minutes % 60).toString();
    return chinese ? `${hours}小时${rest}分` : `${hours}h ${rest}m`;
  }

  const at = new Date(resetsAt * 1000);
  if (minutes < 7 * 24 * 60) {
    return `${weekday(at)} ${two(at.getHours())}:${two(at.getMinutes())}`;
  }
  return chinese
    ? `${(at.getMonth() + 1).toString()}月${at.getDate().toString()}日`
    : `${month(at)} ${at.getDate().toString()}`;
}

/**
 * A moment as local clock time: `Mon 09:00` within a week, `Aug 31 09:00` beyond (zh: `周一 09:00`
 * / `8月31日 09:00`). Used for deadlines, where a relative "in 6h" would go stale.
 */
export function clockTime(atSeconds: number, nowSeconds: number): string {
  const at = new Date(atSeconds * 1000);
  const time = `${two(at.getHours())}:${two(at.getMinutes())}`;
  if (Math.abs(atSeconds - nowSeconds) < 7 * 24 * 60 * 60) {
    return `${weekday(at)} ${time}`;
  }
  return activeLanguage() === "zh"
    ? `${(at.getMonth() + 1).toString()}月${at.getDate().toString()}日 ${time}`
    : `${month(at)} ${at.getDate().toString()} ${time}`;
}

/** Not from `Intl`: its date names depend on the runtime's ICU data and vary between machines. */
const WEEKDAYS: Record<Language, readonly string[]> = {
  en: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
  zh: ["周日", "周一", "周二", "周三", "周四", "周五", "周六"],
};

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

function weekday(at: Date): string {
  return WEEKDAYS[activeLanguage()][at.getDay()] ?? "";
}

function month(at: Date): string {
  return MONTHS[at.getMonth()] ?? "";
}

function two(value: number): string {
  return value.toString().padStart(2, "0");
}

/** Guards the arc against out-of-range values; non-numbers are rejected in `windowValue`. */
function clampPercent(percent: number): number {
  return Math.min(100, Math.max(0, percent));
}
