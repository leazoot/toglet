/**
 * Joins CSS-module class names, dropping `undefined` lookups (typed so under
 * `noUncheckedIndexedAccess`) so a missing class never renders as the word "undefined".
 */
export function cx(...names: readonly (string | false | null | undefined)[]): string {
  return names.filter((name): name is string => typeof name === "string" && name !== "").join(" ");
}
