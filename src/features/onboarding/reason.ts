/**
 * Maps a sign-in failure code to a specific sentence. A code without its own sentence is still
 * shown, so the user can quote it.
 */

import type { MessageKey } from "../../i18n";
import type { IpcFailure } from "../../types/ipc";

const SENTENCES: Readonly<Record<string, MessageKey>> = {
  login_canceled: "add.whyCanceled",
  login_timeout: "add.whyTimeout",
  runtime_not_installed: "add.whyRuntimeMissing",
  runtime_incompatible: "add.whyRuntimeIncompatible",
  network_unavailable: "add.whyNetwork",
  app_server_crashed: "add.whyServerStopped",
  app_server_unresponsive: "add.whyServerSilent",
  credential_store_unavailable: "add.whyCredentialStore",
  codex_home_unwritable: "add.whyHomeUnwritable",
  auth_file_conflict: "add.whyAuthFileConflict",
};

export interface SignInReason {
  readonly key: MessageKey;
  /** `null` when the IPC call itself failed rather than the command returning an error. */
  readonly code: string | null;
}

export function signInReason(failure: IpcFailure | null): SignInReason {
  const code = failure?.error?.code ?? null;
  if (code === null) {
    return { key: "add.whyUnreported", code: null };
  }
  return { key: SENTENCES[code] ?? "add.whyOther", code };
}
