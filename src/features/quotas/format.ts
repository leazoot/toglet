/**
 * Quota presentation: the one place a percentage, a status colour, a ring arc or a reset time is
 * derived. Anything that displays quota calls in here.
 *
 * The type below is the reason this file exists. A quota reading has three outcomes and they are
 * not interchangeable:
 *
 * * a **value**, which is a number the user can act on;
 * * **not returned**, meaning the server answered without that window - the weekly one is
 *   routinely absent;
 * * **unreadable**, meaning the reading itself failed.
 *
 * Only the first has a percentage. The other two render as an em dash on a dashed track, never
 * as `0%`, which would mean "you have run out" - a different and much more alarming claim.
 */

import type { QuotaTone } from "../../components/QuotaRing";
import { activeLanguage } from "../../i18n";
import type { Language } from "../../i18n";
import type { QuotaView, QuotaWindowKind } from "../../types/ipc";

export type QuotaValue =
  | {
      readonly kind: "value";
      readonly remainingPercent: number;
      readonly resetsAt: number | null;
    }
  | { readonly kind: "not_returned" }
  | { readonly kind: "unreadable" };

/**
 * The ring's circumference, from the design's own geometry (r=16.5, so 2πr rounds to 103.67).
 * It is duplicated in `src/styles/tokens.css`, where the dashed track needs it as a CSS value; a
 * test asserts the two agree so neither can drift.
 */
export const RING_CIRCUMFERENCE = 103.67;

/** The circumference as `stroke-dasharray` needs to see it. */
const CIRCUMFERENCE_TEXT = RING_CIRCUMFERENCE.toFixed(2);

/** What Rust considers fresh (`quota::cache::STALE_AFTER_SECONDS`). */
export const STALE_AFTER_SECONDS = 600;

/**
 * Pulls one window out of a reading.
 *
 * A window that is not in the list was not returned. That is a statement about the server's
 * answer, not about the user's allowance, so it must not be filled in.
 */
export function windowValue(view: QuotaView, kind: QuotaWindowKind): QuotaValue {
  const found = view.windows.find((window) => window.kind === kind);
  if (found === undefined) {
    return { kind: "not_returned" };
  }
  if (!Number.isFinite(found.remainingPercent)) {
    // A window that is present but carries no usable number is a reading that failed, not a
    // reading of zero. Collapsing it to 0 would tell the user they had run out.
    return { kind: "unreadable" };
  }
  return {
    kind: "value",
    remainingPercent: found.remainingPercent,
    resetsAt: found.resetsAt,
  };
}

/**
 * Whether a reading has aged out.
 *
 * Recomputed against the current clock rather than trusting the flag Rust set when it answered:
 * a reading that was fresh when it arrived is not still fresh ten minutes later, and the window
 * may have been open the whole time.
 */
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
 * The arc length for `stroke-dasharray`.
 *
 * A reading without a value draws no arc at all - the dashed track underneath is what says
 * "unknown". A zero-length arc on a solid track would be indistinguishable from 0%.
 */
export function ringDash(value: QuotaValue): string {
  if (value.kind !== "value") {
    return `0 ${CIRCUMFERENCE_TEXT}`;
  }
  const arc = (clampPercent(value.remainingPercent) / 100) * RING_CIRCUMFERENCE;
  return `${arc.toFixed(1)} ${CIRCUMFERENCE_TEXT}`;
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
  // Rounding decides the band, so that what the ring says and what the number says cannot
  // disagree: 0.4% reads as "0%" and must be the exhausted colour, not the low one.
  return Math.round(remaining) === 0 ? "empty" : "low";
}

/**
 * The compact reset form: `51m`, `2h 14m`, `Mon 09:00`, `Aug 31`, and in Chinese `51分`,
 * `2小时14分`, `周一 09:00`, `8月31日`.
 *
 * Localised here rather than through the copy dictionary: this is the one place a reset time is
 * derived, and splitting the numbers from the words that go round them across two files is how
 * they come to disagree about a boundary.
 *
 * A reset time that has already passed reads as zero rather than as a negative countdown. It
 * means the window has turned over and this reading predates it - which the staleness marker is
 * what says, not this.
 *
 * **The result is copy, not a value.** Nothing may compare it against a literal to find out how
 * old a reading is; the seconds are still there to be compared.
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
    // Local time throughout: an absolute timestamp is stored, and the user reads it where they
    // are.
    return `${weekday(at)} ${two(at.getHours())}:${two(at.getMinutes())}`;
  }
  return chinese
    ? `${(at.getMonth() + 1).toString()}月${at.getDate().toString()}日`
    : `${month(at)} ${at.getDate().toString()}`;
}

/**
 * Written out rather than taken from `Intl`.
 *
 * The platform's date names depend on which ICU data the runtime was built with, so the same
 * code can produce `周一` on one machine and `Mon` on another. A table cannot do that, and these
 * are seven and twelve short words, not a locale database.
 */
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

/**
 * Keeps a percentage inside the range a ring can draw.
 *
 * Clamping guards the arc, not the truth: a reading outside 0-100 would otherwise draw an arc
 * longer than the circle. Values that are not numbers at all are turned away earlier, in
 * [`windowValue`], because those are unreadable rather than out of range.
 */
function clampPercent(percent: number): number {
  return Math.min(100, Math.max(0, percent));
}
