/**
 * An account's initial and identity colour, shared so the bar and the row always agree.
 * The colour is only for the avatar ring and initial, never a fill.
 */

import type { AccountView } from "../../types/ipc";

const ACCENTS = 5;

/** By grapheme: `charAt` splits astral characters and spreading splits flags and ZWJ emoji. */
const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: "grapheme" });

export function initialOf(account: AccountView | null): string {
  if (account === null) {
    return "";
  }
  const first = GRAPHEMES.segment(account.displayName.trim())[Symbol.iterator]().next();
  return first.done === true ? "" : first.value.segment.toUpperCase();
}

/** A stable 1-5 from the id, matched by a stylesheet rule. `0` means no account. */
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
