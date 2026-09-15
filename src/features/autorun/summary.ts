// `Default instruction · No deadline · Up to 8`, shared by the sheet and the confirmation.

import { t } from "../../i18n";
import { clockTime } from "../quotas/format";
import { deadlineAt } from "./draft";
import type { BindDraft } from "./draft";

export function optionsSummary(draft: BindDraft, nowSeconds: number): string {
  const deadline = deadlineAt(draft, nowSeconds);
  return [
    draft.instruction.trim() === t("autorun.defaultInstruction")
      ? t("autorun.defaultInstructionSummary")
      : t("autorun.customInstructionSummary"),
    deadline === null
      ? t("autorun.summaryNoDeadline")
      : t("autorun.summaryDeadline", { when: clockTime(deadline, nowSeconds) }),
    draft.maxResumes === null
      ? t("autorun.unlimited")
      : t("autorun.summaryResumes", { count: draft.maxResumes }),
  ].join(" · ");
}
