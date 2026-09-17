// Notification channels page of the settings sheet. Connection details travel one way: Rust
// only returns a host hint and a label, so editing starts with empty fields and leaving them
// empty keeps what is stored.
//
// Two views, never both (user 2026-09-17): the list, one line per channel that opens on a
// click; and the form, which replaces the list rather than growing under it, so a mail form
// with its eight boxes fits the panel on its own.

import { useEffect, useState } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import type { MessageKey } from "../../i18n";
import type {
  MailSecurity,
  NotifyChannelKind,
  NotifyChannelView,
  NotifyOutcome,
} from "../../types/ipc";
import { compactReset } from "../quotas/format";
import { Segmented } from "../settings/controls";
import sheet from "../settings/SettingsSheet.module.css";
import { cx } from "../../styles/classes";
import { FIELDS, HELP, MAIL_PORTS, fill } from "./fields";
import type { FormValues } from "./fields";
import { reasonKey } from "./reason";
import { useNotify } from "./store";
import styles from "./NotifySection.module.css";

/** Which view is showing. Owned by the sheet, which names the page in its header. */
export type NotifyMode = "list" | "add" | "edit";

export interface NotifySectionProps {
  mode: NotifyMode;
  onMode: (mode: NotifyMode) => void;
}

const KINDS: readonly { value: NotifyChannelKind; key: MessageKey }[] = [
  { value: "bark", key: "notify.kind.bark" },
  { value: "wecom", key: "notify.kind.wecom" },
  { value: "telegram", key: "notify.kind.telegram" },
  { value: "webhook", key: "notify.kind.webhook" },
  { value: "email", key: "notify.kind.email" },
];

const KIND_NAMES: Readonly<Record<NotifyChannelKind, MessageKey>> = {
  bark: "notify.kind.bark",
  wecom: "notify.kind.wecom",
  telegram: "notify.kind.telegram",
  webhook: "notify.kind.webhook",
  email: "notify.kind.email",
};

interface Draft {
  /** `null` while adding. */
  readonly id: string | null;
  readonly kind: NotifyChannelKind;
  readonly label: string;
  readonly values: FormValues;
  readonly security: MailSecurity;
}

function blank(kind: NotifyChannelKind, id: string | null, label: string): Draft {
  return {
    id,
    kind,
    label,
    values: kind === "email" ? { port: MAIL_PORTS.tls } : {},
    security: "tls",
  };
}

export function NotifySection({ mode, onMode }: NotifySectionProps): JSX.Element {
  const channels = useNotify((state) => state.channels);
  const busy = useNotify((state) => state.busy);
  const testing = useNotify((state) => state.testing);
  const tested = useNotify((state) => state.tested);
  const failure = useNotify((state) => state.failure);
  const load = useNotify((state) => state.load);
  const save = useNotify((state) => state.save);
  const remove = useNotify((state) => state.remove);
  const test = useNotify((state) => state.test);

  const [draft, setDraft] = useState<Draft | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  /** The one row showing its details and actions; the rest are a line each. */
  const [expanded, setExpanded] = useState<string | null>(null);
  // Set when Save was pressed on a half-filled form; cleared by the next edit.
  const [incomplete, setIncomplete] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  // The header's Back returns to the list without asking this page: drop the draft with it.
  // Adjusted during render from the previous prop (react.dev, "storing information from
  // previous renders"), the same way the dock follows `expanded`.
  const [seenMode, setSeenMode] = useState(mode);
  if (mode !== seenMode) {
    setSeenMode(mode);
    if (mode === "list") {
      setDraft(null);
      setIncomplete(false);
    }
  }

  const list = channels.state === "ready" ? channels.value.channels : [];
  const full = channels.state === "ready" && list.length >= channels.value.maxChannels;

  const close = (): void => {
    setDraft(null);
    setIncomplete(false);
    onMode("list");
  };

  const submit = (): void => {
    if (draft === null) {
      return;
    }
    const filled = fill(draft.kind, draft.values, draft.security);
    if (filled.state === "incomplete" || (draft.id === null && filled.state === "empty")) {
      setIncomplete(true);
      return;
    }
    const enabled =
      draft.id === null ? true : (list.find((one) => one.id === draft.id)?.enabled ?? true);
    void save({
      ...(draft.id === null ? {} : { id: draft.id }),
      label: draft.label,
      enabled,
      ...(filled.state === "ready" ? { connection: filled.connection } : {}),
    }).then((saved) => {
      if (saved) {
        close();
      }
    });
  };

  const alert =
    failure === null ? null : (
      <p className={styles["alert"]} role="alert">
        {t("notify.commandFailed", { code: failure.error?.code ?? t("notify.reason.unreached") })}
      </p>
    );

  if (mode !== "list" && draft !== null) {
    return (
      <div className={styles["page"]} data-testid="notify-section">
        <ChannelForm
          draft={draft}
          busy={busy}
          incomplete={incomplete}
          onChange={(next) => {
            setIncomplete(false);
            setDraft(next);
          }}
          onCancel={close}
          onSubmit={submit}
        />
        {alert}
      </div>
    );
  }

  return (
    <div className={styles["page"]} data-testid="notify-section">
      {/* No heading of its own: the sheet's header carries the page's name. */}
      <p className={styles["hint"]}>{t("notify.hint")}</p>

      {channels.state === "loading" && <p className={sheet["message"]}>{t("panel.loading")}</p>}
      {channels.state === "failed" && <p className={sheet["message"]}>{t("notify.unreadable")}</p>}

      {channels.state === "ready" && list.length === 0 && (
        <p className={sheet["message"]}>{t("notify.empty")}</p>
      )}

      {list.length > 0 && (
        <ul className={styles["list"]}>
          {list.map((channel) => (
            <ChannelRow
              key={channel.id}
              channel={channel}
              busy={busy}
              testing={testing === channel.id}
              tested={tested[channel.id]}
              expanded={expanded === channel.id}
              pending={pending === channel.id}
              onExpand={() => {
                setPending(null);
                setExpanded(expanded === channel.id ? null : channel.id);
              }}
              onPending={setPending}
              onToggle={(enabled) => {
                void save({ id: channel.id, label: channel.label, enabled });
              }}
              onTest={() => {
                void test(channel.id, t("app.name"), t("notify.testBody"));
              }}
              onEdit={() => {
                setIncomplete(false);
                setDraft(blank(channel.kind, channel.id, channel.label));
                onMode("edit");
              }}
              onRemove={() => {
                setPending(null);
                void remove(channel.id);
              }}
            />
          ))}
        </ul>
      )}

      <button
        type="button"
        className={styles["add"]}
        disabled={busy || full || channels.state !== "ready"}
        onClick={() => {
          setIncomplete(false);
          setDraft(blank("bark", null, ""));
          onMode("add");
        }}
      >
        {t("notify.add")}
      </button>

      {full && <p className={styles["hint"]}>{t("notify.full")}</p>}
      {alert}
    </div>
  );
}

interface ChannelRowProps {
  channel: NotifyChannelView;
  busy: boolean;
  testing: boolean;
  tested: NotifyOutcome | undefined;
  expanded: boolean;
  pending: boolean;
  onExpand: () => void;
  onPending: (channelId: string | null) => void;
  onToggle: (enabled: boolean) => void;
  onTest: () => void;
  onEdit: () => void;
  onRemove: () => void;
}

/** One line: name, service and host, a dot for the last delivery, the switch. Opens on click. */
function ChannelRow({
  channel,
  busy,
  testing,
  tested,
  expanded,
  pending,
  onExpand,
  onPending,
  onToggle,
  onTest,
  onEdit,
  onRemove,
}: ChannelRowProps): JSX.Element {
  const failed = lastFailed(channel, tested);
  const never = tested === undefined && channel.lastDelivery === null;
  return (
    <li className={styles["row"]}>
      <div className={styles["head"]}>
        <button
          type="button"
          className={styles["summary"]}
          aria-expanded={expanded}
          onClick={onExpand}
        >
          <span className={styles["name"]}>{channel.label}</span>
          <span className={styles["kind"]}>
            {t(KIND_NAMES[channel.kind])}
            {channel.hint === "" ? "" : ` · ${channel.hint}`}
          </span>
          {/* Never the only carrier: the opened row says it in words. */}
          <span
            className={cx(
              styles["mark"],
              never ? styles["markNone"] : failed ? styles["markBad"] : styles["markOk"],
            )}
            aria-hidden="true"
          />
          <Chevron open={expanded} />
        </button>
        <button
          type="button"
          role="switch"
          aria-checked={channel.enabled}
          aria-label={t("notify.enable", { name: channel.label })}
          className={cx(sheet["switch"], channel.enabled && sheet["on"])}
          disabled={busy}
          onClick={() => {
            onToggle(!channel.enabled);
          }}
        >
          <span className={sheet["knob"]} aria-hidden="true" />
        </button>
      </div>

      {expanded && (
        <div className={styles["details"]}>
          <p className={cx(styles["meta"], failed && styles["bad"])}>
            {lastLine(channel, testing, tested)}
          </p>
          <div className={styles["rowActions"]}>
            <button
              type="button"
              className={styles["chipPrimary"]}
              disabled={busy || testing}
              onClick={onTest}
            >
              {t(testing ? "notify.testing" : "notify.test")}
            </button>
            <button type="button" className={styles["chip"]} disabled={busy} onClick={onEdit}>
              {t("notify.edit")}
            </button>
            {pending ? (
              <>
                <button
                  type="button"
                  className={styles["chipRemove"]}
                  disabled={busy}
                  onClick={onRemove}
                >
                  {t("notify.removeConfirm")}
                </button>
                <button
                  type="button"
                  className={styles["chip"]}
                  onClick={() => {
                    onPending(null);
                  }}
                >
                  {t("notify.cancel")}
                </button>
              </>
            ) : (
              <button
                type="button"
                className={styles["chipRemove"]}
                disabled={busy}
                onClick={() => {
                  onPending(channel.id);
                }}
              >
                {t("notify.remove")}
              </button>
            )}
          </div>
        </div>
      )}
    </li>
  );
}

function Chevron({ open }: { open: boolean }): JSX.Element {
  return (
    <svg
      viewBox="0 0 12 12"
      className={cx(styles["chevron"], open && styles["chevronOpen"])}
      aria-hidden="true"
    >
      <path
        d="M4.5 2.5 L8 6 L4.5 9.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function lastFailed(channel: NotifyChannelView, tested: NotifyOutcome | undefined): boolean {
  return tested === undefined ? channel.lastDelivery?.ok === false : !tested.ok;
}

/** A test just run outranks the stored record, which may not have been re-read yet. */
function lastLine(
  channel: NotifyChannelView,
  testing: boolean,
  tested: NotifyOutcome | undefined,
): string {
  if (testing) {
    return t("notify.testing");
  }
  if (tested !== undefined) {
    return tested.ok ? t("notify.testOk") : t("notify.testFailed", { reason: reason(tested.code) });
  }
  const last = channel.lastDelivery;
  if (last === null) {
    return t("notify.never");
  }
  const when = compactReset(Math.floor(Date.now() / 1000), last.at);
  return last.ok
    ? t("notify.lastOk", { when })
    : t("notify.lastFailed", { when, reason: reason(last.code) });
}

function reason(code: string | null): string {
  if (code === null) {
    return t("notify.reason.unreached");
  }
  const key = reasonKey(code);
  return key === null ? code : t(key);
}

interface ChannelFormProps {
  draft: Draft;
  busy: boolean;
  incomplete: boolean;
  onChange: (draft: Draft) => void;
  onCancel: () => void;
  onSubmit: () => void;
}

function ChannelForm({
  draft,
  busy,
  incomplete,
  onChange,
  onCancel,
  onSubmit,
}: ChannelFormProps): JSX.Element {
  const named = draft.label.trim() !== "";

  return (
    <div className={styles["form"]} data-testid="notify-form">
      {/* The service is fixed while editing: the stored details belong to it. */}
      {draft.id === null && (
        <Segmented
          label={t("notify.service")}
          value={draft.kind}
          options={KINDS.map((one) => ({ value: one.value, label: t(one.key) }))}
          disabled={busy}
          stacked
          onPick={(kind) => {
            onChange(blank(kind, null, draft.label));
          }}
        />
      )}

      <p className={styles["hint"]}>{t(HELP[draft.kind])}</p>

      <label className={styles["field"]}>
        <span className={styles["fieldLabel"]}>{t("notify.label")}</span>
        <input
          className={styles["input"]}
          value={draft.label}
          maxLength={32}
          disabled={busy}
          onChange={(event) => {
            onChange({ ...draft, label: event.target.value });
          }}
        />
      </label>

      {draft.kind === "email" && (
        <Segmented
          label={t("notify.field.security")}
          value={draft.security}
          options={[
            { value: "tls" as const, label: t("notify.security.tls") },
            { value: "startTls" as const, label: t("notify.security.startTls") },
          ]}
          disabled={busy}
          onPick={(security) => {
            // Swap the default port along with the encryption; keep a port the user typed.
            const port = draft.values["port"];
            const followed = port === undefined || port === MAIL_PORTS[other(security)];
            onChange({
              ...draft,
              security,
              values: followed ? { ...draft.values, port: MAIL_PORTS[security] } : draft.values,
            });
          }}
        />
      )}

      {/* Placed above the fields it explains. */}
      {draft.id !== null && <p className={styles["hint"]}>{t("notify.keepDetails")}</p>}

      {/* A grid so a pair of short fields (host, port) can share a line. */}
      <div className={styles["fields"]}>
        {FIELDS[draft.kind].map((field) => (
          <label
            key={field.name}
            className={cx(styles["field"], field.half === true && styles["half"])}
          >
            <span className={styles["fieldLabel"]}>{t(field.label)}</span>
            <input
              className={styles["input"]}
              type={field.secret ? "password" : "text"}
              inputMode={field.name === "port" ? "numeric" : undefined}
              value={draft.values[field.name] ?? ""}
              placeholder={field.placeholder}
              // Password managers ignore `off` but honour `new-password`. An empty field means
              // "keep what is stored", so an autofilled password would replace working details.
              autoComplete={field.secret ? "new-password" : "off"}
              name={`notify-${draft.kind}-${field.name}`}
              spellCheck={false}
              disabled={busy}
              onChange={(event) => {
                onChange({
                  ...draft,
                  values: { ...draft.values, [field.name]: event.target.value },
                });
              }}
            />
          </label>
        ))}
      </div>

      {incomplete && (
        <p className={styles["alert"]} role="alert">
          {t("notify.incomplete")}
        </p>
      )}

      <div className={styles["actions"]}>
        <button type="button" className={styles["action"]} disabled={busy} onClick={onCancel}>
          {t("notify.cancel")}
        </button>
        <button
          type="button"
          className={cx(styles["action"], styles["primary"])}
          disabled={busy || !named}
          onClick={onSubmit}
        >
          {t("notify.save")}
        </button>
      </div>
    </div>
  );
}

function other(security: MailSecurity): MailSecurity {
  return security === "tls" ? "startTls" : "tls";
}
