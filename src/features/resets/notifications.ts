/**
 * The sentence for one reset event. Content is a dictionary sentence plus a time or a level -
 * never the feed's own words, which are a third party's and may carry links.
 */

import type { ResetAnnouncementView } from "../../types/ipc";
import { clockTime } from "../quotas/format";
import { chance, levelKey } from "./banner";
import type { Translate } from "./banner";

export interface ResetNotification {
  readonly title: string;
  readonly body: string;
}

export function resetNotification(
  event: ResetAnnouncementView,
  nowSeconds: number,
  t: Translate,
): ResetNotification {
  const title = t("resets.notify.title");
  switch (event.kind) {
    case "reset":
      return {
        title,
        body: t(event.resetType === "banked" ? "resets.notify.banked" : "resets.notify.reset"),
      };
    case "scheduled":
      return {
        title,
        body:
          event.scheduledFor === null
            ? t("resets.notify.scheduledNoTime")
            : t("resets.notify.scheduled", { when: clockTime(event.scheduledFor, nowSeconds) }),
      };
    case "watch": {
      const level = t(levelKey(event.level));
      return {
        title,
        body:
          event.chancePercent === null
            ? t("resets.notify.watchNoChance", { level })
            : t("resets.notify.watch", { level, chance: chance(event.chancePercent) }),
      };
    }
  }
}
