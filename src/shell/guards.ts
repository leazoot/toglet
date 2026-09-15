/**
 * Suppresses the webview's browser affordances: the context menu and the shortcuts behind it.
 * "Reload" would restart the interface mid-switch and "save as" writes the page to disk. Only
 * the listed combinations' default actions are prevented.
 */

/** Function keys the engine binds on its own: reload, find next, caret browsing. */
const FUNCTION_KEYS = new Set(["F5", "F3", "F7"]);

/**
 * With Ctrl (or Cmd): reload, print, save, find, find next, view source, downloads, history,
 * open file, bookmark. Matched on `key` lower-cased so Shift variants (`Ctrl+Shift+R`) are
 * covered too.
 */
const MODIFIED_KEYS = new Set(["r", "p", "s", "f", "g", "u", "j", "h", "o", "d"]);

/** Mouse buttons the engine treats as back and forward. */
const NAVIGATION_BUTTONS = new Set([3, 4]);

function isBrowserShortcut(event: KeyboardEvent): boolean {
  if (FUNCTION_KEYS.has(event.key)) {
    return true;
  }
  if ((event.ctrlKey || event.metaKey) && MODIFIED_KEYS.has(event.key.toLowerCase())) {
    return true;
  }
  return event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight");
}

/** Installs the guards on `target` for the life of the document. */
export function installWebviewGuards(target: Document): void {
  target.addEventListener("contextmenu", (event) => {
    event.preventDefault();
  });
  target.addEventListener("keydown", (event) => {
    if (isBrowserShortcut(event)) {
      event.preventDefault();
    }
  });
  for (const type of ["mousedown", "mouseup", "auxclick"] as const) {
    target.addEventListener(type, (event) => {
      if (NAVIGATION_BUTTONS.has(event.button)) {
        event.preventDefault();
      }
    });
  }
}
