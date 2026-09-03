/**
 * What the tray menu's entries say.
 *
 * Built from the copy dictionary for the same reason the summary line is: the tray is the one
 * part of Toglet that is drawn by the operating system, and it is the easiest one to forget when
 * a language changes. Reading it from the dictionary means it cannot be forgotten.
 *
 * The language is a parameter rather than the ambient one, because the caller is an effect that
 * already knows which language it is reacting to - passing it is what keeps that effect honest
 * about what it depends on.
 *
 * There is deliberately nothing here that switches an account: a switch needs the
 * confirmation, and the confirmation needs the panel.
 */

import { translate } from "../../i18n";
import type { Language } from "../../i18n";
import type { TrayLabels } from "../../types/ipc";

export function trayLabels(language: Language): TrayLabels {
  return {
    show: translate(language, "tray.show"),
    // The panel's own wording: the menu entry runs the panel's refresh, and one sentence in two
    // keys is one sentence two translations can disagree about.
    refresh: translate(language, "panel.refresh"),
    primary: translate(language, "tray.primary"),
    settings: translate(language, "tray.settings"),
    quit: translate(language, "tray.quit"),
  };
}
