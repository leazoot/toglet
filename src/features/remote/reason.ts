/**
 * Maps remote outcome, failure and action codes to copy. Unknown codes return `null` and are
 * shown verbatim rather than as "unknown error".
 */

import type { MessageKey } from "../../i18n";

export function outcomeKey(code: string): MessageKey | null {
  switch (code) {
    case "applied":
      return "remote.outcome.applied";
    case "remote_bad_mac":
      return "remote.outcome.badMac";
    case "remote_replayed":
    case "remote_nonce_reused":
      return "remote.outcome.replayed";
    case "remote_expired":
      return "remote.outcome.expired";
    case "remote_session_mismatch":
      return "remote.outcome.otherTask";
    case "remote_state_changed":
      return "remote.outcome.stateChanged";
    case "remote_rate_limited":
      return "remote.outcome.tooMany";
    case "remote_unavailable":
      return "remote.outcome.unavailable";
    case "remote_version":
    case "remote_malformed":
    case "remote_unknown_action":
      return "remote.outcome.unreadable";
    default:
      return null;
  }
}

export function failureKey(code: string): MessageKey | null {
  switch (code) {
    case "network_unavailable":
      return "remote.reason.network";
    case "remote_bridge_rejected":
      return "remote.reason.rejected";
    case "credential_store_unavailable":
      return "remote.reason.credentials";
    case "internal":
      return "remote.reason.internal";
    default:
      return null;
  }
}

export function actionKey(action: string): MessageKey | null {
  switch (action) {
    case "resume":
      return "remote.action.resume";
    case "pause":
      return "remote.action.pause";
    case "cancel":
      return "remote.action.cancel";
    case "status":
      return "remote.action.status";
    default:
      return null;
  }
}
