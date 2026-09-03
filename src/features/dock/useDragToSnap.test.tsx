import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { SettingsView } from "../../types/ipc";
import { useDragToSnap } from "./useDragToSnap";

function Surface({
  onStart = () => undefined,
  onSettled = () => undefined,
}: {
  onStart?: () => void;
  onSettled?: (settings: SettingsView) => void;
}) {
  const drag = useDragToSnap(onStart, onSettled);
  return <div data-testid="bar" {...drag} />;
}

const SETTLED: SettingsView = {
  dockEdge: "right",
  verticalOffset: 120,
  alwaysOnTop: true,
  activeRefreshSeconds: 60,
  inactiveRefreshSeconds: 300,
  reopenCodexAfterSwitch: true,
  theme: "system",
  reduceMotion: false,
  language: "system",
};

/** jsdom implements neither, and neither is what any of this is about. */
function capturable(element: HTMLElement): HTMLElement {
  element.setPointerCapture = vi.fn();
  element.releasePointerCapture = vi.fn();
  element.hasPointerCapture = vi.fn(() => true);
  return element;
}

function press(element: HTMLElement, x = 100, y = 100): void {
  fireEvent.pointerDown(element, { pointerId: 1, button: 0, screenX: x, screenY: y });
}

function moveTo(element: HTMLElement, x: number, y: number): void {
  fireEvent.pointerMove(element, { pointerId: 1, screenX: x, screenY: y });
}

function calls(command: string): unknown[][] {
  return invoke.mock.calls.filter(([name]) => name === command);
}

describe("dragging the bar", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });
  afterEach(cleanup);

  it("does not move the window for a press that never travels", () => {
    // The bar is what the pointer rests on to open the panel. If a press moved it, the panel
    // could not be opened without nudging the window.
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    fireEvent.pointerUp(bar, { pointerId: 1 });

    expect(invoke).not.toHaveBeenCalled();
  });

  it("takes the pointer only once the press has become a drag", () => {
    // A pointer captured on the press delivers its release to the bar rather than to whatever
    // was pressed, and a click on the bar's own add button would never arrive.
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));
    const capture = vi.fn();
    bar.setPointerCapture = capture;

    press(bar);
    expect(capture).not.toHaveBeenCalled();

    moveTo(bar, 140, 100);
    expect(capture).toHaveBeenCalledWith(1);
  });

  it("ignores a wobble below the threshold", () => {
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 101, 101);
    fireEvent.pointerUp(bar, { pointerId: 1 });

    expect(invoke).not.toHaveBeenCalled();
  });

  it("moves the window once the pointer has travelled far enough", () => {
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);

    expect(calls("move_dock")).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith("move_dock", { dx: 40, dy: 0 });
  });

  it("sends increments, not positions", async () => {
    // Rust adds each one to where the window is. Absolute coordinates would need the interface to
    // know about screens and scale factors, and the window moves out from under the pointer.
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    await waitFor(() => {
      expect(calls("move_dock")).toHaveLength(1);
    });
    moveTo(bar, 150, 110);

    await waitFor(() => {
      expect(invoke).toHaveBeenLastCalledWith("move_dock", { dx: 10, dy: 10 });
    });
  });

  it("sums the moves that arrive while one is still on its way", async () => {
    // A pointer reports hundreds of moves a second. Sent one by one they queued behind each
    // other, and the window was still working through them long after the pointer had stopped.
    let answer: (value: null) => void = () => undefined;
    invoke.mockImplementation((command: string) =>
      command === "move_dock"
        ? new Promise<null>((resolve) => {
            answer = resolve;
          })
        : Promise.resolve(null),
    );
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    moveTo(bar, 150, 110);
    moveTo(bar, 170, 105);
    moveTo(bar, 165, 120);

    expect(calls("move_dock")).toEqual([["move_dock", { dx: 40, dy: 0 }]]);

    answer(null);
    await waitFor(() => {
      expect(calls("move_dock")).toHaveLength(2);
    });
    expect(invoke).toHaveBeenLastCalledWith("move_dock", { dx: 25, dy: 20 });
  });

  it("makes the moves still pending before it settles", async () => {
    // A settle that overtook the last increment would dock the window from a place the pointer
    // had already left.
    let answer: (value: null) => void = () => undefined;
    invoke.mockImplementation((command: string) =>
      command === "move_dock"
        ? new Promise<null>((resolve) => {
            answer = resolve;
          })
        : Promise.resolve(null),
    );
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    moveTo(bar, 160, 100);
    fireEvent.pointerUp(bar, { pointerId: 1 });

    expect(calls("end_drag")).toHaveLength(0);

    answer(null);
    await waitFor(() => {
      expect(calls("move_dock")).toHaveLength(2);
    });
    expect(calls("end_drag")).toHaveLength(0);

    answer(null);
    await waitFor(() => {
      expect(calls("end_drag")).toHaveLength(1);
    });
    const order = invoke.mock.calls.map(([name]) => String(name));
    expect(order).toEqual(["move_dock", "move_dock", "end_drag"]);
    expect(invoke.mock.calls[1]).toEqual(["move_dock", { dx: 20, dy: 0 }]);
  });

  it("asks Rust to settle the window when the drag ends", async () => {
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    fireEvent.pointerUp(bar, { pointerId: 1 });

    await waitFor(() => {
      expect(calls("end_drag")).toHaveLength(1);
    });
  });

  it("hands back the settings Rust stored when the drag settled", async () => {
    // The bar is drawn from the stored offset. A drag that ended without the interface learning
    // the new offset drew the bar where the pointer no longer reached it.
    invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "end_drag" ? SETTLED : null),
    );
    const onSettled = vi.fn();
    render(<Surface onSettled={onSettled} />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    fireEvent.pointerUp(bar, { pointerId: 1 });
    await waitFor(() => {
      expect(onSettled).toHaveBeenCalledWith(SETTLED);
    });
  });

  it("keeps the previous settings when the settle failed", async () => {
    invoke.mockImplementation((command: string) =>
      command === "end_drag" ? Promise.reject(new Error("dock_failed")) : Promise.resolve(null),
    );
    const onSettled = vi.fn();
    render(<Surface onSettled={onSettled} />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    fireEvent.pointerUp(bar, { pointerId: 1 });
    await waitFor(() => {
      expect(calls("end_drag")).toHaveLength(1);
    });

    expect(onSettled).not.toHaveBeenCalled();
  });

  it("settles the window when the drag is cancelled rather than leaving it adrift", async () => {
    // A cancelled drag has still moved the window. Doing nothing would leave it off the edge with
    // nothing stored about where it is.
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    moveTo(bar, 140, 100);
    fireEvent.pointerCancel(bar, { pointerId: 1 });

    await waitFor(() => {
      expect(calls("end_drag")).toHaveLength(1);
    });
  });

  it("calls the hover gesture off exactly once, when the press becomes a drag", () => {
    const onStart = vi.fn();
    render(<Surface onStart={onStart} />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    expect(onStart).not.toHaveBeenCalled();

    moveTo(bar, 140, 100);
    moveTo(bar, 160, 100);

    expect(onStart).toHaveBeenCalledTimes(1);
  });

  it("ignores a button that is not the primary one", () => {
    // The right button belongs to whatever menu the platform offers; the middle one must not move
    // the window.
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    fireEvent.pointerDown(bar, { pointerId: 1, button: 2, screenX: 100, screenY: 100 });
    moveTo(bar, 140, 100);

    expect(invoke).not.toHaveBeenCalled();
  });

  it("ignores a second pointer that did not start the drag", () => {
    render(<Surface />);
    const bar = capturable(screen.getByTestId("bar"));

    press(bar);
    fireEvent.pointerMove(bar, { pointerId: 2, screenX: 400, screenY: 400 });

    expect(invoke).not.toHaveBeenCalled();
  });
});
