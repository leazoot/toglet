import { beforeEach, describe, expect, it } from "vitest";

import { installWebviewGuards } from "./guards";

function fire(event: Event): boolean {
  document.body.dispatchEvent(event);
  return event.defaultPrevented;
}

describe("the webview guards", () => {
  beforeEach(() => {
    installWebviewGuards(document);
  });

  it("refuses the engine's context menu", () => {
    // Back, reload, save as, print, inspect: none of them belong on the bar, and the menu
    // itself reads as a bug.
    expect(fire(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }))).toBe(true);
  });

  it("swallows the shortcuts that reach the same places", () => {
    for (const init of [
      { key: "F5" },
      { key: "r", ctrlKey: true },
      { key: "R", ctrlKey: true, shiftKey: true },
      { key: "p", ctrlKey: true },
      { key: "s", metaKey: true },
      { key: "ArrowLeft", altKey: true },
    ]) {
      expect(
        fire(new KeyboardEvent("keydown", { ...init, bubbles: true, cancelable: true })),
        JSON.stringify(init),
      ).toBe(true);
    }
  });

  it("leaves every other key to the interface", () => {
    for (const init of [{ key: "Escape" }, { key: "Enter" }, { key: "r" }, { key: "ArrowLeft" }]) {
      expect(
        fire(new KeyboardEvent("keydown", { ...init, bubbles: true, cancelable: true })),
        JSON.stringify(init),
      ).toBe(false);
    }
  });

  it("stops the mouse buttons the engine treats as back and forward", () => {
    expect(fire(new MouseEvent("mouseup", { button: 3, bubbles: true, cancelable: true }))).toBe(
      true,
    );
    expect(fire(new MouseEvent("mouseup", { button: 0, bubbles: true, cancelable: true }))).toBe(
      false,
    );
  });
});
