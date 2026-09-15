/**
 * Copy lookup. The active language is module state: the settings store sets it, and the same
 * store write re-renders the tree, so a language change applies without a restart.
 */

import { en } from "./en";
import type { MessageKey } from "./en";
import { zh } from "./zh";

export type { MessageKey };

export type Language = "en" | "zh";

/** The stored preference. `system` means the user has never chosen. */
export type LanguagePreference = Language | "system";

const DICTIONARIES: Record<Language, Record<MessageKey, string>> = { en, zh };

/** Values substituted into a message's `{name}` slots. */
export type MessageParams = Readonly<Record<string, string | number>>;

/**
 * Starts from the OS language so a Chinese desktop does not flash English before the stored
 * preference arrives over IPC.
 */
let active: Language = resolveLanguage("system");

/** Resolves a preference; `system` is answered from the webview's reported language. */
export function resolveLanguage(preference: LanguagePreference): Language {
  if (preference !== "system") {
    return preference;
  }
  if (typeof navigator === "undefined") {
    return "en";
  }
  // Every `zh-*` tag maps to the one Chinese dictionary; anything else falls back to English.
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function activeLanguage(): Language {
  return active;
}

/** Points the lookup at another dictionary. Called by the settings store and nowhere else. */
export function setLanguage(language: Language): void {
  active = language;
}

/**
 * Looks up a message in the active language and fills its slots. A slot with no value is left
 * visible rather than blanked, so the bug shows.
 */
export function t(key: MessageKey, params?: MessageParams): string {
  return translate(active, key, params);
}

/** The same lookup against a named language, for copy outside the rendered tree (the tray). */
export function translate(language: Language, key: MessageKey, params?: MessageParams): string {
  const message: string = DICTIONARIES[language][key];
  if (params === undefined) {
    return message;
  }
  return message.replace(/\{(\w+)\}/g, (slot, name: string) =>
    name in params ? String(params[name]) : slot,
  );
}
