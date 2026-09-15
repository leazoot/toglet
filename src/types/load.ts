/**
 * Something the interface asked Rust for. A failed read is `failed`, never an empty array, and a
 * value not yet arrived is `loading`, never a default.
 */

import type { IpcFailure } from "./ipc";

export type Loadable<T> =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: T }
  | { readonly state: "failed"; readonly failure: IpcFailure };
