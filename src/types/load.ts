/**
 * How the interface represents something it asked Rust for.
 *
 * Three states, and no fourth that quietly means "empty": a list that could not be read is
 * `failed`, never an empty array, and a value that has not arrived is `loading`, never a
 * default. Collapsing either into a plausible-looking value is exactly the dishonest state the
 * product forbids.
 */

import type { IpcFailure } from "./ipc";

export type Loadable<T> =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: T }
  | { readonly state: "failed"; readonly failure: IpcFailure };
