/**
 * The settings, mirrored from Rust: always what Rust says is stored, never what was asked for
 * (Rust may correct an out-of-range value). Theme, motion and language apply as they arrive.
 */

import { create } from "zustand";

import { resolveLanguage, setLanguage } from "../../i18n";
import { readSettings, updateSettings } from "../../ipc";
import type { SettingsPatch, SettingsView } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface SettingsState {
  readonly settings: Loadable<SettingsView>;
  /** True while a change is on its way to Rust. */
  readonly saving: boolean;
  readonly load: () => Promise<void>;
  readonly update: (patch: SettingsPatch) => Promise<void>;
  /** Takes settings another command reported as stored, e.g. the offset after a bar drag. */
  readonly replace: (settings: SettingsView) => void;
}

export const useSettings = create<SettingsState>()((set) => ({
  settings: { state: "loading" },
  saving: false,
  load: async () => {
    const result = await readSettings();
    set({
      settings: result.ok
        ? { state: "ready", value: apply(result.value) }
        : { state: "failed", failure: result.failure },
    });
  },
  update: async (patch) => {
    set({ saving: true });
    const result = await updateSettings(patch);
    // A failed change leaves the previous settings showing; they are still the ones in force.
    set(
      result.ok
        ? { settings: { state: "ready", value: apply(result.value) }, saving: false }
        : { saving: false },
    );
  },
  replace: (settings) => {
    set({ settings: { state: "ready", value: apply(settings) } });
  },
}));

/**
 * Puts the theme, motion and language choices into force. Removing an attribute hands control
 * back to the media query, so the motion toggle can only add reduced motion, never override the
 * system. The language is set before the store publishes, so the re-render already uses it.
 */
function apply(settings: SettingsView): SettingsView {
  setLanguage(resolveLanguage(settings.language));

  if (typeof document === "undefined") {
    return settings;
  }
  const root = document.documentElement;
  // The resolved language, not the preference: `lang="system"` is not a language.
  root.setAttribute("lang", resolveLanguage(settings.language));

  if (settings.theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", settings.theme);
  }

  if (settings.reduceMotion) {
    root.setAttribute("data-motion", "reduced");
  } else {
    root.removeAttribute("data-motion");
  }
  return settings;
}
