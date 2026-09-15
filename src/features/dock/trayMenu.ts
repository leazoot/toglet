/**
 * The tray menu's labels, from the copy dictionary so the OS-drawn menu follows language changes.
 * No switch entry on purpose: a switch needs the confirmation in the panel.
 */

import { translate } from "../../i18n";
import type { Language } from "../../i18n";
import type { TrayLabels } from "../../types/ipc";

export function trayLabels(language: Language): TrayLabels {
  return {
    show: translate(language, "tray.show"),
    hide: translate(language, "tray.hide"),
    // Reuses the panel's key: the entry runs the same refresh.
    refresh: translate(language, "panel.refresh"),
    primary: translate(language, "tray.primary"),
    settings: translate(language, "tray.settings"),
    quit: translate(language, "tray.quit"),
  };
}
