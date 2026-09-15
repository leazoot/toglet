/**
 * Maps a delivery error code from Rust to copy. Unknown codes return `null` and are shown
 * verbatim rather than as "unknown error".
 */

import type { MessageKey } from "../../i18n";

export function reasonKey(code: string): MessageKey | null {
  switch (code) {
    case "network_unavailable":
      return "notify.reason.network";
    case "notification_rejected":
      return "notify.reason.rejected";
    case "credential_store_unavailable":
      return "notify.reason.credentials";
    case "internal":
      return "notify.reason.internal";
    default:
      return null;
  }
}
