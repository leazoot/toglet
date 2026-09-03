/**
 * Reads a duration token at run time.
 *
 * Timing that the user can feel - the hover intent delay, the delay before the panel closes -
 * belongs to the same token set as the animations, and reading it here rather than repeating the
 * number in TypeScript has a concrete consequence: `prefers-reduced-motion` zeroes those tokens,
 * so the delays disappear along with the animations and the panel opens instantly. A hard-coded
 * `120` would keep waiting.
 */

/**
 * Milliseconds for `name`, or `fallback` when the stylesheet is not loaded.
 *
 * The fallback exists for the test environment, where no stylesheet is attached to the document.
 * It is never the source of truth: `src/styles/tokens.test.ts` owns the values.
 */
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
