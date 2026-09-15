/**
 * Delays are read from duration tokens rather than hard-coded so that reduced motion, which
 * zeroes the tokens, removes the delays along with the animations.
 */

/** Milliseconds for `name`, or `fallback` when no stylesheet is attached (tests). */
export function durationToken(name: string, fallback: number): number {
  if (typeof window === "undefined") {
    return fallback;
  }
  const raw = window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return parseDuration(raw) ?? fallback;
}

function parseDuration(raw: string): number | null {
  const match = /^(-?[\d.]+)(ms|s)$/.exec(raw);
  if (match === null) {
    return null;
  }
  const value = Number.parseFloat(match[1] ?? "");
  if (!Number.isFinite(value) || value < 0) {
    return null;
  }
  return match[2] === "s" ? value * 1000 : value;
}
