/**
 * The plan being put together, not sent to Rust until confirmed. It holds what `bind_autorun`
 * takes, plus the picked deadline preset, which becomes a timestamp only on confirmation.
 */

import type { AutoRunView, BindRequest, ThreadView } from "../../types/ipc";

/** The trimmed instruction's limit, in code points (as Rust counts). */
export const INSTRUCTION_MAX_CHARS = 2000;

export const DEFAULT_MAX_RESUMES = 8;

export const MAX_RESUME_CHOICES = [4, 8, 16] as const;

/**
 * Deadline presets. `kept` is not offered: it shows a plan's stored deadline so it is neither lost
 * nor misrepresented as a preset.
 */
export type DeadlineChoice = "none" | "today_23" | "tomorrow_09" | "in_24h" | "kept";

export interface DraftThread {
  readonly threadId: string;
  readonly title: string | null;
  /** From the listing the session was picked from; `null` for one taken from the plan. */
  readonly preview: string | null;
  readonly projectLabel: string | null;
}

export interface BindDraft {
  readonly thread: DraftThread | null;
  /** Account ids, in priority order. */
  readonly participants: readonly string[];
  readonly instruction: string;
  readonly deadline: DeadlineChoice;
  /** The stored deadline, when `deadline` is `kept`. Unix seconds. */
  readonly keptDeadline: number | null;
  /** `null` means no limit. */
  readonly maxResumes: number | null;
}

/** What still stops the draft from being confirmed. */
export type DraftProblem = "no_thread" | "no_participants" | "no_instruction" | "instruction_long";

/** The draft a plan implies. A stored deadline already past is dropped, since Rust refuses it. */
export function draftFrom(
  plan: AutoRunView | null,
  defaultInstruction: string,
  nowSeconds: number,
): BindDraft {
  if (plan === null) {
    return {
      thread: null,
      participants: [],
      instruction: defaultInstruction,
      deadline: "none",
      keptDeadline: null,
      maxResumes: DEFAULT_MAX_RESUMES,
    };
  }
  const deadline = plan.deadline !== null && plan.deadline > nowSeconds ? plan.deadline : null;
  return {
    thread:
      plan.binding === null
        ? null
        : {
            threadId: plan.binding.threadId,
            title: plan.binding.threadTitle,
            preview: null,
            projectLabel: plan.binding.projectLabel,
          },
    participants: [...plan.participants]
      .sort((a, b) => a.order - b.order)
      .map((participant) => participant.accountId),
    instruction: plan.binding?.resumeInstruction ?? defaultInstruction,
    deadline: deadline === null ? "none" : "kept",
    keptDeadline: deadline,
    // A never-bound plan holds no cap; offer the default, so "no limit" is always a deliberate pick.
    maxResumes: plan.binding === null ? DEFAULT_MAX_RESUMES : plan.maxResumes,
  };
}

export function threadOf(view: ThreadView): DraftThread {
  return {
    threadId: view.threadId,
    title: view.title,
    preview: view.preview,
    projectLabel: view.projectLabel,
  };
}

/** The trimmed instruction's length, counted the way Rust counts. */
export function instructionLength(instruction: string): number {
  return codePoints(instruction.trim()).length;
}

/** Code points, not grapheme clusters: matches Rust's `chars().count()`, which enforces the limit. */
function codePoints(text: string): string[] {
  return Array.from(text);
}

export function problemOf(draft: BindDraft): DraftProblem | null {
  if (draft.thread === null) {
    return "no_thread";
  }
  if (draft.participants.length === 0) {
    return "no_participants";
  }
  const length = instructionLength(draft.instruction);
  if (length === 0) {
    return "no_instruction";
  }
  if (length > INSTRUCTION_MAX_CHARS) {
    return "instruction_long";
  }
  return null;
}

/**
 * When a deadline choice falls, in Unix seconds and local time, or `null` for none. A preset
 * already past is also `null`; `choiceAvailable` keeps it from being picked.
 */
export function deadlineAt(draft: BindDraft, nowSeconds: number): number | null {
  switch (draft.deadline) {
    case "none":
      return null;
    case "kept":
      return draft.keptDeadline;
    case "in_24h":
      return nowSeconds + 24 * 60 * 60;
    case "today_23": {
      const at = localMoment(nowSeconds, 0, 23);
      return at > nowSeconds ? at : null;
    }
    case "tomorrow_09":
      return localMoment(nowSeconds, 1, 9);
  }
}

/** Whether a preset still lies ahead. Only "today 23:00" can fall behind. */
export function choiceAvailable(choice: DeadlineChoice, nowSeconds: number): boolean {
  return choice !== "today_23" || localMoment(nowSeconds, 0, 23) > nowSeconds;
}

/** `hour`:00 local time, `days` days from the day `nowSeconds` falls on. */
function localMoment(nowSeconds: number, days: number, hour: number): number {
  const at = new Date(nowSeconds * 1000);
  at.setDate(at.getDate() + days);
  at.setHours(hour, 0, 0, 0);
  return Math.floor(at.getTime() / 1000);
}

export function toRequest(draft: BindDraft, threadId: string, nowSeconds: number): BindRequest {
  return {
    threadId,
    resumeInstruction: draft.instruction.trim(),
    participants: draft.participants,
    maxResumes: draft.maxResumes,
    deadline: deadlineAt(draft, nowSeconds),
  };
}

/**
 * Whether confirming the draft would change nothing. Re-enabling must not require re-binding:
 * binding needs the session in the last listing, which a plan bound in an earlier run lacks.
 */
export function sameAsPlan(draft: BindDraft, plan: AutoRunView): boolean {
  if (plan.binding === null || draft.thread === null) {
    return false;
  }
  const stored = [...plan.participants]
    .sort((a, b) => a.order - b.order)
    .map((participant) => participant.accountId);
  return (
    draft.thread.threadId === plan.binding.threadId &&
    draft.instruction.trim() === plan.binding.resumeInstruction &&
    draft.participants.length === stored.length &&
    draft.participants.every((id, index) => id === stored[index]) &&
    draft.maxResumes === plan.maxResumes &&
    (draft.deadline === "none" ? plan.deadline === null : draft.deadline === "kept") &&
    (draft.deadline !== "kept" || draft.keptDeadline === plan.deadline)
  );
}
