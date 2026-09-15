/**
 * Which plan change deserves a system notification: needs a person, stopped, round completed, or
 * switched. Content is a dictionary sentence plus at most a display name - never paths, session
 * titles or instructions, since notifications show on lock screens and other devices.
 */

import type { MessageKey, MessageParams } from "../../i18n";
import type { AutoRunView } from "../../types/ipc";
import { reasonKey } from "./status";

export interface Notification {
  readonly title: string;
  readonly body: string;
}

type Translate = (key: MessageKey, params?: MessageParams) => string;

/** Turns an account id into its display name, or `null` when the list does not hold it. */
export type NameOf = (accountId: string) => string | null;

export function notificationFor(
  previous: AutoRunView | null,
  next: AutoRunView,
  nameOf: NameOf,
  t: Translate,
): Notification | null {
  // The first plan of a run was read from disk; announcing it would repeat an old state on
  // every restart.
  if (previous === null) {
    return null;
  }

  const title = t("autorun.notify.title");
  const reason = next.waitReason === null ? null : sentence(next.waitReason, t);

  if (next.state === "needs_human" && previous.state !== "needs_human") {
    return {
      title,
      body: t("autorun.notify.needsHuman", { reason: reason ?? t("autorun.reason.unknown") }),
    };
  }
  if (next.state === "stopped" && previous.state !== "stopped") {
    return {
      title,
      body:
        reason === null
          ? t("autorun.notify.stoppedNoReason")
          : t("autorun.notify.stopped", { reason }),
    };
  }
  if (next.state === "round_completed" && previous.state !== "round_completed") {
    return { title, body: t("autorun.notify.roundCompleted") };
  }
  // A switch is a result rather than a state: it is announced once, when the result is new.
  const switched = next.lastResult;
  if (
    switched?.kind === "switched" &&
    (previous.lastResult?.kind !== "switched" || previous.lastResult.at !== switched.at)
  ) {
    const name = switched.accountId === null ? null : nameOf(switched.accountId);
    return {
      title,
      body:
        name === null
          ? t("autorun.notify.switchedUnnamed")
          : t("autorun.notify.switched", { name }),
    };
  }
  return null;
}

function sentence(code: string, t: Translate): string {
  const key = reasonKey(code);
  return key === null ? code : t(key);
}
