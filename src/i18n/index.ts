/**
 * Copy lookup, and which language it resolves against.
 *
 * The key type is closed, so a key that is not in the dictionary is a type error rather than a
 * string that renders as itself at run time.
 *
 * **The active language is module state, deliberately.** `t` is called from every component that
 * renders copy, and threading a language through all of them would put the same argument in a
 * hundred call sites to say the same thing each time. What makes a change take effect is that
 * the language is stored with the settings: the settings store writes it here as it arrives, and
 * the shell re-renders from that same store write, so the whole tree re-labels in one pass with
 * no restart. `src/App.test.tsx` holds that behaviour down.
 */

import { en } from "./en";
import type { MessageKey } from "./en";
import { zh } from "./zh";

export type { MessageKey };

/** The languages a dictionary exists for. */
export type Language = "en" | "zh";

/**
 * What is stored. `system` means the user has never chosen, which is not the same as having
 * chosen English - it is why an upgrade cannot quietly pin someone to the wrong language.
 */
export type LanguagePreference = Language | "system";

const DICTIONARIES: Record<Language, Record<MessageKey, string>> = { en, zh };

/** Values substituted into a message's `{name}` slots. */
export type MessageParams = Readonly<Record<string, string | number>>;

/**
 * Started from the operating system rather than from English.
 *
 * The stored preference arrives a moment later, over IPC. Resolving here first is what keeps a
 * Chinese desktop from being shown a frame of English on the way - and if the settings cannot be
 * read at all, following the system is still a better answer than defaulting.
 */
let active: Language = resolveLanguage("system");

/**
 * Which language a stored preference means here and now.
 *
 * `system` is answered from what the operating system told the webview. Only this side can
 * answer it: Rust stores the preference, and asking a second layer to resolve the same tag is
 * how two answers to one question start to disagree.
 */
export function resolveLanguage(preference: LanguagePreference): Language {
  if (preference !== "system") {
    return preference;
  }
  if (typeof navigator === "undefined") {
    return "en";
  }
  // Matched on the primary subtag, so `zh`, `zh-CN`, `zh-Hans` and `zh-TW` all land on the one
  // Chinese dictionary. Anything else falls to English, which is the only other one there is.
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

/** The language copy is currently being resolved against. */
export function activeLanguage(): Language {
  return active;
}

/** Points the lookup at another dictionary. Called by the settings store and nowhere else. */
export function setLanguage(language: Language): void {
  active = language;
}

/**
 * Looks up a message in the active language and fills its slots.
 *
 * A slot with no value is left in place rather than blanked. A message reading `resets in {when}`
 * is a visible bug; one reading `resets in ` looks like real copy and hides it.
 */
export function t(key: MessageKey, params?: MessageParams): string {
  return translate(active, key, params);
}

/**
 * The same lookup against a named language.
 *
 * For copy that is built for somewhere other than the screen being rendered - the tray menu is
 * relabelled from an effect that already knows which language it is reacting to, and taking it
 * as an argument is what makes that effect honest about what it depends on.
 */
export function translate(language: Language, key: MessageKey, params?: MessageParams): string {
  const message: string = DICTIONARIES[language][key];
  if (params === undefined) {
    return message;
  }
  return message.replace(/\{(\w+)\}/g, (slot, name: string) =>
    name in params ? String(params[name]) : slot,
  );
}
