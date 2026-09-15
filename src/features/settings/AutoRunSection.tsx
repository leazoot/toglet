// The body of the automatic-continuation sheet. Edits are a draft held in the store until the
// plan is enabled and confirmed; while the plan is on the fields are locked, so a change means
// turning it off and on again, which also starts a fresh resume count.

import { useState } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { cx } from "../../styles/classes";
import type { AccountView, AutoRunView, ThreadListView, ThreadView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import {
  INSTRUCTION_MAX_CHARS,
  MAX_RESUME_CHOICES,
  choiceAvailable,
  draftFrom,
  instructionLength,
  problemOf,
  threadOf,
} from "../autorun/draft";
import type { BindDraft, DeadlineChoice, DraftProblem, DraftThread } from "../autorun/draft";
import type { AutoRunFailure } from "../autorun/store";
import { optionsSummary } from "../autorun/summary";
import { clockTime, compactReset } from "../quotas/format";
import styles from "./AutoRunSection.module.css";
import { Segmented, Toggle } from "./controls";
import sheet from "./SettingsSheet.module.css";

/** The segmented control's word for "no cap". Never sent: it becomes `null` in the draft. */
const UNLIMITED = "unlimited";

/** Below a day, a session's age reads as a distance; beyond it, as the moment it was touched. */
const DAY_SECONDS = 24 * 60 * 60;

const PROBLEM_KEYS: Record<DraftProblem, MessageKey> = {
  no_thread: "autorun.needThread",
  no_participants: "autorun.needAccounts",
  no_instruction: "autorun.needInstruction",
  instruction_long: "autorun.instructionLong",
};

const DEADLINE_KEYS: readonly { value: DeadlineChoice; key: MessageKey }[] = [
  { value: "none", key: "autorun.deadlineNone" },
  { value: "today_23", key: "autorun.deadlineToday" },
  { value: "tomorrow_09", key: "autorun.deadlineTomorrow" },
  { value: "in_24h", key: "autorun.deadlineDay" },
];

export interface AutoRunSectionProps {
  plan: Loadable<AutoRunView>;
  accounts: Loadable<readonly AccountView[]>;
  /** `null` until the chooser has been opened once. */
  threads: Loadable<ThreadListView> | null;
  /** The draft in progress, or `null` when nothing has been edited since the plan changed. */
  draft: BindDraft | null;
  /** True while a confirmation is being carried out. */
  busy: boolean;
  failure: AutoRunFailure | null;
  nowSeconds: number;
  onEdit: (draft: BindDraft) => void;
  /** True while the sheet is on its chooser page, which replaces the group. */
  choosing: boolean;
  /** Asks the sheet for the chooser page. */
  onChooseSession: () => void;
  /** Tells the sheet a session was picked, so it can leave the chooser page. */
  onChosen: () => void;
  /** Asks to turn the plan on with `draft`. The confirmation is the dialog's job. */
  onEnable: (draft: BindDraft) => void;
  onDisable: () => void;
}

export function AutoRunSection({
  plan,
  accounts,
  threads,
  draft,
  busy,
  failure,
  nowSeconds,
  onEdit,
  choosing,
  onChooseSession,
  onChosen,
  onEnable,
  onDisable,
}: AutoRunSectionProps): JSX.Element {
  const current =
    plan.state === "ready"
      ? (draft ?? draftFrom(plan.value, t("autorun.defaultInstruction"), nowSeconds))
      : null;

  return (
    <div className={styles["section"]} data-testid="autorun-section">
      {plan.state === "loading" && <p className={sheet["message"]}>{t("autorun.loading")}</p>}
      {plan.state === "failed" && <p className={sheet["message"]}>{t("autorun.unreadable")}</p>}

      {plan.state === "ready" && current !== null && choosing && (
        <Chooser
          threads={threads}
          chosen={current.thread?.threadId ?? null}
          nowSeconds={nowSeconds}
          onPick={(thread) => {
            onEdit({ ...current, thread: threadOf(thread) });
            onChosen();
          }}
        />
      )}

      {plan.state === "ready" && current !== null && !choosing && (
        <Group
          plan={plan.value}
          accounts={accounts}
          threads={threads}
          draft={current}
          busy={busy}
          failure={failure}
          nowSeconds={nowSeconds}
          onEdit={onEdit}
          onChooseSession={onChooseSession}
          onEnable={onEnable}
          onDisable={onDisable}
        />
      )}
    </div>
  );
}

interface GroupProps extends Omit<AutoRunSectionProps, "plan" | "draft" | "choosing" | "onChosen"> {
  plan: AutoRunView;
  draft: BindDraft;
}

function Group({
  plan,
  accounts,
  threads,
  draft,
  busy,
  failure,
  nowSeconds,
  onEdit,
  onChooseSession,
  onEnable,
  onDisable,
}: GroupProps): JSX.Element {
  const [more, setMore] = useState(false);
  const locked = plan.enabled || busy;
  const problem = problemOf(draft);
  const length = instructionLength(draft.instruction);
  // Instruction, deadline and cap are folded by default; an instruction problem unfolds them.
  const showMore = more || problem === "no_instruction" || problem === "instruction_long";

  return (
    <>
      <Toggle
        label={t("autorun.toggle")}
        strong
        value={plan.enabled}
        disabled={busy || (!plan.enabled && problem !== null)}
        note={!plan.enabled && problem !== null ? t(PROBLEM_KEYS[problem]) : undefined}
        onPick={(on) => {
          if (on) {
            onEnable(draft);
          } else {
            onDisable();
          }
        }}
      />
      {plan.enabled && <p className={styles["hint"]}>{t("autorun.locked")}</p>}
      {failure?.step === "disable" && (
        <p className={styles["alert"]} role="alert">
          {failure.failure.error === null
            ? t("autorun.disableUnreported")
            : t("autorun.disableFailed", { code: failure.failure.error.code })}
        </p>
      )}

      <div className={sheet["row"]}>
        <span className={cx(sheet["label"], sheet["strong"])}>{t("autorun.session")}</span>
        <button
          type="button"
          className={cx(styles["session"], draft.thread === null && styles["unchosen"])}
          aria-label={t("autorun.pickSession")}
          disabled={locked}
          onClick={onChooseSession}
        >
          {draft.thread === null ? t("autorun.noSession") : threadLabel(draft.thread, threads)}
        </button>
      </div>

      <p className={cx(sheet["label"], sheet["strong"], styles["heading"])}>
        {t("autorun.participants")}
      </p>
      <Participants accounts={accounts} draft={draft} locked={locked} onEdit={onEdit} />

      <button
        type="button"
        className={styles["more"]}
        aria-expanded={showMore}
        aria-controls="autorun-more"
        onClick={() => {
          setMore(!showMore);
        }}
      >
        <span className={styles["moreLabel"]}>{t("autorun.moreOptions")}</span>
        <span className={styles["moreSummary"]}>{optionsSummary(draft, nowSeconds)}</span>
        <Chevron up={showMore} />
      </button>

      {showMore && (
        <div id="autorun-more">
          <label className={cx(sheet["label"], sheet["strong"])} htmlFor="autorun-instruction">
            {t("autorun.instruction")}
          </label>
          <textarea
            id="autorun-instruction"
            className={styles["instruction"]}
            rows={3}
            value={draft.instruction}
            disabled={locked}
            onChange={(event) => {
              onEdit({ ...draft, instruction: event.target.value });
            }}
          />
          <p className={cx(styles["counter"], length > INSTRUCTION_MAX_CHARS && styles["over"])}>
            {t("autorun.instructionCount", { count: length, max: INSTRUCTION_MAX_CHARS })}
          </p>

          <Segmented
            label={t("autorun.deadline")}
            strong
            value={draft.deadline}
            options={[
              ...DEADLINE_KEYS.map((one) => ({
                value: one.value,
                label: t(one.key),
                disabled: !choiceAvailable(one.value, nowSeconds),
              })),
              // A deadline the plan already holds, shown as its time rather than a preset.
              ...(draft.keptDeadline === null
                ? []
                : [{ value: "kept" as const, label: clockTime(draft.keptDeadline, nowSeconds) }]),
            ]}
            disabled={locked}
            stacked
            onPick={(deadline) => {
              onEdit({ ...draft, deadline });
            }}
          />

          <Segmented
            label={t("autorun.maxResumes")}
            strong
            value={draft.maxResumes ?? UNLIMITED}
            options={[
              ...MAX_RESUME_CHOICES.map((count) => ({ value: count, label: count.toString() })),
              { value: UNLIMITED, label: t("autorun.unlimited") },
            ]}
            disabled={locked}
            stacked
            onPick={(choice) => {
              onEdit({ ...draft, maxResumes: choice === UNLIMITED ? null : choice });
            }}
          />
        </div>
      )}
    </>
  );
}

/** `folder · title`. A session taken from the plan has no excerpt; the last listing may. */
function threadLabel(thread: DraftThread, threads: Loadable<ThreadListView> | null): string {
  const listed =
    threads?.state === "ready"
      ? threads.value.threads.find((one) => one.threadId === thread.threadId)
      : undefined;
  return `${thread.projectLabel ?? t("autorun.unknownProject")} · ${
    thread.title ?? thread.preview ?? listed?.preview ?? t("autorun.untitledThread")
  }`;
}

interface ChooserProps {
  threads: Loadable<ThreadListView> | null;
  chosen: string | null;
  nowSeconds: number;
  onPick: (thread: ThreadView) => void;
}

/** The listed sessions, grouped by folder; named by title, else first-message excerpt. */
function Chooser({ threads, chosen, nowSeconds, onPick }: ChooserProps): JSX.Element {
  if (threads === null || threads.state === "loading") {
    return <p className={sheet["message"]}>{t("autorun.threadsLoading")}</p>;
  }
  if (threads.state === "failed") {
    const error = threads.failure.error;
    return (
      <p className={sheet["message"]} role="alert">
        {error === null
          ? t("autorun.threadsUnreported")
          : t("autorun.threadsFailed", { code: error.code })}
      </p>
    );
  }
  if (threads.value.threads.length === 0) {
    const version = threads.value.runtimeVersion;
    return (
      <p className={sheet["message"]}>
        {version === null
          ? t("autorun.threadsEmptyUnversioned")
          : t("autorun.threadsEmpty", { version })}
      </p>
    );
  }

  return (
    <>
      <div className={styles["threads"]} role="listbox" aria-label={t("autorun.pickSession")}>
        {grouped(threads.value.threads).map((group) => (
          <div key={group.label ?? ""} className={styles["group"]}>
            <p className={styles["groupTitle"]}>
              <FolderMark />
              {group.label ?? t("autorun.unknownProject")}
            </p>
            {group.threads.map((thread) => (
              <button
                key={thread.threadId}
                type="button"
                role="option"
                aria-selected={thread.threadId === chosen}
                className={styles["thread"]}
                onClick={() => {
                  onPick(thread);
                }}
              >
                <span className={styles["threadTitle"]}>
                  {thread.title ?? thread.preview ?? t("autorun.untitledThread")}
                </span>
                <span className={styles["threadAge"]}>{age(thread.updatedAt, nowSeconds)}</span>
              </button>
            ))}
          </div>
        ))}
      </div>
      {threads.value.truncated && <p className={styles["hint"]}>{t("autorun.threadsTruncated")}</p>}
      {/* Shown even for a non-empty list: sessions written by a newer Codex may be missing. */}
      {threads.value.runtimeVersion !== null && (
        <p className={styles["version"]}>
          {t("autorun.threadsVersion", { version: threads.value.runtimeVersion })}
        </p>
      )}
    </>
  );
}

function FolderMark(): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={styles["folder"]} aria-hidden="true">
      <path
        d="M1.8 4.2 A1 1 0 0 1 2.8 3.2 H5.6 L7 4.6 H12.2 A1 1 0 0 1 13.2 5.6 V11 A1 1 0 0 1 12.2 12 H2.8 A1 1 0 0 1 1.8 11 Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

interface ThreadGroup {
  label: string | null;
  threads: ThreadView[];
}

/** By folder, sessions newest first, folders ordered by their newest session. */
function grouped(threads: readonly ThreadView[]): ThreadGroup[] {
  const groups = new Map<string | null, ThreadView[]>();
  for (const thread of threads) {
    const bucket = groups.get(thread.projectLabel);
    if (bucket === undefined) {
      groups.set(thread.projectLabel, [thread]);
    } else {
      bucket.push(thread);
    }
  }
  return [...groups.entries()]
    .map(([label, members]) => ({
      label,
      threads: [...members].sort((a, b) => b.updatedAt - a.updatedAt),
    }))
    .sort((a, b) => (b.threads[0]?.updatedAt ?? 0) - (a.threads[0]?.updatedAt ?? 0));
}

/**
 * Within a day the distance (`2h 14m ago`); beyond it the moment (`Mon 09:00`), because
 * `compactReset` would name the day of its first argument, which here is now.
 */
function age(updatedAt: number, nowSeconds: number): string {
  if (nowSeconds - updatedAt < DAY_SECONDS) {
    return t("autorun.threadAge", { when: compactReset(nowSeconds, updatedAt) });
  }
  return clockTime(updatedAt, nowSeconds);
}

interface ParticipantsProps {
  accounts: Loadable<readonly AccountView[]>;
  draft: BindDraft;
  locked: boolean;
  onEdit: (draft: BindDraft) => void;
}

/** Every account with a tick box; ticked ones first, in try order. Position is priority. */
function Participants({ accounts, draft, locked, onEdit }: ParticipantsProps): JSX.Element {
  if (accounts.state === "loading") {
    return <p className={sheet["message"]}>{t("panel.loading")}</p>;
  }
  if (accounts.state === "failed") {
    return <p className={sheet["message"]}>{t("bar.notice.unreadable")}</p>;
  }
  if (accounts.value.length === 0) {
    return <p className={sheet["message"]}>{t("panel.emptyTitle")}</p>;
  }

  const included = draft.participants
    .map((id) => accounts.value.find((account) => account.id === id))
    .filter((account): account is AccountView => account !== undefined);
  const excluded = accounts.value.filter((account) => !draft.participants.includes(account.id));

  const move = (id: string, by: -1 | 1): void => {
    const order = [...draft.participants];
    const at = order.indexOf(id);
    const to = at + by;
    if (at === -1 || to < 0 || to >= order.length) {
      return;
    }
    [order[at], order[to]] = [order[to] ?? id, order[at] ?? id];
    onEdit({ ...draft, participants: order });
  };

  return (
    <ul className={styles["participants"]}>
      {[...included, ...excluded].map((account) => {
        const at = draft.participants.indexOf(account.id);
        const on = at !== -1;
        return (
          <li key={account.id} className={styles["participant"]}>
            <input
              type="checkbox"
              className={styles["tick"]}
              checked={on}
              disabled={locked}
              aria-label={t("autorun.participate", { name: account.displayName })}
              onChange={() => {
                onEdit({
                  ...draft,
                  participants: on
                    ? draft.participants.filter((id) => id !== account.id)
                    : [...draft.participants, account.id],
                });
              }}
            />
            <span className={cx(styles["name"], !on && styles["excluded"])}>
              {account.displayName}
            </span>
            <button
              type="button"
              className={styles["iconButton"]}
              aria-label={t("autorun.moveUp", { name: account.displayName })}
              disabled={locked || !on || at === 0}
              onClick={() => {
                move(account.id, -1);
              }}
            >
              <Chevron up />
            </button>
            <button
              type="button"
              className={styles["iconButton"]}
              aria-label={t("autorun.moveDown", { name: account.displayName })}
              disabled={locked || !on || at === draft.participants.length - 1}
              onClick={() => {
                move(account.id, 1);
              }}
            >
              <Chevron up={false} />
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function Chevron({ up }: { up: boolean }): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={styles["icon"]} aria-hidden="true">
      <path
        d={up ? "M4 9.2 L7.5 5.8 L11 9.2" : "M4 5.8 L7.5 9.2 L11 5.8"}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
