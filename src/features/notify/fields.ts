/**
 * The fields each notification service needs, and how filled-in fields become a connection.
 * Only completeness is checked here; whether a value is usable is decided by Rust.
 */

import type { MessageKey } from "../../i18n";
import type { MailSecurity, NotifyChannelKind, NotifyConnectionInput } from "../../types/ipc";

export interface FieldSpec {
  readonly name: string;
  readonly label: MessageKey;
  /** An optional field is one Rust fills in a default for. */
  readonly required: boolean;
  /** Masked in the form. */
  readonly secret: boolean;
  readonly placeholder?: string;
}

/** Placeholders only; Rust applies the default when the field is left empty. */
export const BARK_DEFAULT_SERVER = "https://api.day.app";
export const TELEGRAM_DEFAULT_API = "https://api.telegram.org";

export const MAIL_PORTS: Readonly<Record<MailSecurity, string>> = { tls: "465", startTls: "587" };

const need = (name: string, label: MessageKey): FieldSpec => ({
  name,
  label,
  required: true,
  secret: false,
});

export const FIELDS: Readonly<Record<NotifyChannelKind, readonly FieldSpec[]>> = {
  bark: [
    { name: "deviceKey", label: "notify.field.deviceKey", required: true, secret: true },
    {
      name: "server",
      label: "notify.field.server",
      required: false,
      secret: false,
      placeholder: BARK_DEFAULT_SERVER,
    },
  ],
  wecom: [need("webhook", "notify.field.webhook")],
  telegram: [
    { name: "botToken", label: "notify.field.botToken", required: true, secret: true },
    need("chatId", "notify.field.chatId"),
    {
      name: "apiBase",
      label: "notify.field.apiBase",
      required: false,
      secret: false,
      placeholder: TELEGRAM_DEFAULT_API,
    },
  ],
  webhook: [need("url", "notify.field.url")],
  email: [
    need("host", "notify.field.host"),
    need("port", "notify.field.port"),
    need("username", "notify.field.username"),
    { name: "password", label: "notify.field.password", required: true, secret: true },
    need("from", "notify.field.from"),
    need("to", "notify.field.to"),
  ],
};

/** One line per service saying where its values are found. */
export const HELP: Readonly<Record<NotifyChannelKind, MessageKey>> = {
  bark: "notify.help.bark",
  wecom: "notify.help.wecom",
  telegram: "notify.help.telegram",
  webhook: "notify.help.webhook",
  email: "notify.help.email",
};

export type FormValues = Readonly<Record<string, string>>;

/**
 * `empty` is not a failure: blank fields while editing mean "keep the stored details", which is
 * how a channel is renamed without re-entering them.
 */
export type Filled =
  | { readonly state: "empty" }
  | { readonly state: "incomplete" }
  | { readonly state: "ready"; readonly connection: NotifyConnectionInput };

export function fill(kind: NotifyChannelKind, values: FormValues, security: MailSecurity): Filled {
  const required = FIELDS[kind].filter((field) => field.required);
  const given = required.filter((field) => (values[field.name] ?? "").trim() !== "");
  if (given.length === 0) {
    return { state: "empty" };
  }
  if (given.length < required.length) {
    return { state: "incomplete" };
  }

  const at = (name: string): string => (values[name] ?? "").trim();
  // An empty optional field must be absent, not `undefined`, so that Rust applies its default.
  const optional = (key: string, name: string): Record<string, string> => {
    const value = at(name);
    return value === "" ? {} : { [key]: value };
  };

  switch (kind) {
    case "bark":
      return ready({ kind, deviceKey: at("deviceKey"), ...optional("server", "server") });
    case "wecom":
      return ready({ kind, webhook: at("webhook") });
    case "telegram":
      return ready({
        kind,
        botToken: at("botToken"),
        chatId: at("chatId"),
        ...optional("apiBase", "apiBase"),
      });
    case "webhook":
      return ready({ kind, url: at("url") });
    case "email": {
      // An invalid port makes the form incomplete rather than being sent for Rust to refuse.
      const port = Number(at("port"));
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        return { state: "incomplete" };
      }
      return ready({
        kind,
        host: at("host"),
        port,
        security,
        username: at("username"),
        password: values["password"] ?? "",
        from: at("from"),
        to: at("to"),
      });
    }
  }
}

function ready(connection: NotifyConnectionInput): Filled {
  return { state: "ready", connection };
}
