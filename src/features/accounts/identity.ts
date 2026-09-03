/**
 * The two presentation facts derived from an account: its initial and its identity colour.
 *
 * Derived rather than stored: both are presentation, the initial has to follow a rename, and the
 * design uses the colour only for the avatar's hairline ring and the initial - never as a fill.
 * Shared because the bar and the account row must agree; two copies would drift and the same
 * account would appear in two colours.
 */

import type { AccountView } from "../../types/ipc";

/** How many identity colours the design defines. */
const ACCENTS = 5;

/**
 * Segments by grapheme rather than by code unit or code point.
 *
 * `charAt` would cut a character outside the basic plane in half, and spreading the string would
 * still split a flag or a family emoji into its parts. A name is user data and the first thing of
 * it the user sees, so it has to survive being shortened.
 */
const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** The first character of the account's own name. User data, so it is never translated. */
export function initialOf(account: AccountView | null): string {
  if (account === null) {
    return "";
  }
  const first = GRAPHEMES.segment(account.displayName.trim())[Symbol.iterator]().next();
  return first.done === true ? "" : first.value.segment.toUpperCase();
}

/**
 * Picks one of the five identity colours, stably, from the account's own id.
 *
 * The value selects a rule in the stylesheet, so no colour is named in TypeScript. `0` means
 * "no account", which no rule matches.
 */
export function accentOf(account: AccountView | null): number {
  if (account === null) {
    return 0;
  }
  let sum = 0;
  for (const character of account.id) {
    sum = (sum + (character.codePointAt(0) ?? 0)) % ACCENTS;
  }
  return sum + 1;
}
