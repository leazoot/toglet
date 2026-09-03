/**
 * The settings, mirrored from Rust.
 *
 * What is held here is always what Rust said is stored - never what was asked for. An interval
 * outside the range is corrected on the way in, and showing the requested number instead of the
 * stored one would show a setting that is not in force.
 *
 * The theme, the reduced-motion preference and the language are applied as they arrive, which is
 * what makes all three take effect without a restart. The first two are attributes on `<html>`,
 * and the stylesheet is written so an explicit choice wins over the system preference in both
 * directions.
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
  /**
   * Takes settings some other command reported as stored - a drag of the bar ends with Rust
   * storing the new offset and answering with the settings. Same rule as the rest: what is held
   * is what Rust said is stored.
   */
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
    // A change that did not get through leaves the previous settings showing. They are still the
    // ones in force, so nothing about them has become untrue.
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
 * Puts the theme, motion and language choices into force.
 *
 * Theme and motion remove their attribute rather than setting an opposite value, because that is
 * what puts the media query back in charge - and "follow the system" is exactly what `system`
 * means, and what turning the motion toggle off means. The toggle can only ever *add* reduced
 * motion; it cannot take away a system preference somebody set on purpose.
 *
 * The language is pointed at its dictionary here, before the store publishes the new settings.
 * That ordering is what makes the change take effect in one pass: the write below re-renders the
 * shell, and by then `t` is already answering in the new language.
 */
function apply(settings: SettingsView): SettingsView {
  setLanguage(resolveLanguage(settings.language));

  if (typeof document === "undefined") {
    return settings;
  }
  const root = document.documentElement;
  // Assistive technology reads this to decide how to pronounce the page. The resolved language,
  // not the preference - `lang="system"` is not a language.
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
