/**
 * Joins CSS-module class names.
 *
 * `noUncheckedIndexedAccess` types a module lookup as possibly `undefined`, and a missing class
 * interpolated into a template literal becomes the literal word `undefined` in the DOM - a class
 * that silently matches nothing. Dropping it here keeps the check on without a cast.
 */
export function cx(...names: readonly (string | false | null | undefined)[]): string {
  return names.filter((name): name is string => typeof name === "string" && name !== "").join(" ");
}
