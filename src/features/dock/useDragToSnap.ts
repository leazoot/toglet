/**
 * Dragging the bar to another edge or monitor. Rust decides where the window lands; this only
 * reports screen-coordinate increments, since the window moves out from under the pointer.
 *
 * Only sideways travel moves the window: macOS will not move a window's top above the menu bar,
 * so vertical travel lifts the bar inside the strip (`onLift`) and goes to Rust once at the end.
 * At most one move is in flight; moves arriving meanwhile are summed so commands never back up.
 */

import { useCallback, useRef } from "react";
import type { PointerEvent } from "react";

import { endDrag, moveDock } from "../../ipc";
import type { SettingsView } from "../../types/ipc";

/** Logical pixels a press must travel to become a drag, so clicks do not nudge the window. */
const THRESHOLD = 4;

export interface DragHandlers {
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  onPointerMove: (event: PointerEvent<HTMLElement>) => void;
  onPointerUp: (event: PointerEvent<HTMLElement>) => void;
  onPointerCancel: (event: PointerEvent<HTMLElement>) => void;
}

interface Grab {
  pointerId: number;
  lastX: number;
  lastY: number;
  travelled: number;
  dragging: boolean;
  /** The pointer's vertical travel since the drag began, positive downward. */
  lift: number;
}

/** Sideways travel the pointer has reported that the window has not yet been asked to make. */
interface Pending {
  dx: number;
  /** Resolves once the in-flight move and everything summed behind it has been sent. */
  flight: Promise<void> | null;
}

function drain(queue: Pending): Promise<void> {
  const { dx } = queue;
  queue.dx = 0;
  if (dx === 0) {
    queue.flight = null;
    return Promise.resolve();
  }
  // A failed move leaves the window in place; the next one carries on, and settling reports errors.
  queue.flight = moveDock(dx).then(() => drain(queue));
  return queue.flight;
}

/**
 * `onStart` fires once when a press becomes a drag; `onLift` gets the vertical travel so far on
 * every move; `onEnd` gets the settings Rust stored (or `null` on failure) and must always fire,
 * because the bar and Rust's hover target are both placed from the stored offset.
 */
export function useDragToSnap(
  onStart: () => void,
  onLift: (lift: number) => void,
  onEnd: (settings: SettingsView | null) => void,
): DragHandlers {
  // A ref, not state: the release is a discrete event and can arrive before React renders the
  // low-priority move that turned the press into a drag.
  const grab = useRef<Grab | null>(null);
  const pending = useRef<Pending>({ dx: 0, flight: null });

  const onPointerDown = useCallback((event: PointerEvent<HTMLElement>) => {
    // Primary button only.
    if (event.button !== 0) {
      return;
    }
    grab.current = {
      pointerId: event.pointerId,
      lastX: event.screenX,
      lastY: event.screenY,
      travelled: 0,
      dragging: false,
      lift: 0,
    };
  }, []);

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      const held = grab.current;
      if (held?.pointerId !== event.pointerId) {
        return;
      }

      const dx = event.screenX - held.lastX;
      const dy = event.screenY - held.lastY;
      held.lastX = event.screenX;
      held.lastY = event.screenY;
      held.travelled += Math.abs(dx) + Math.abs(dy);

      if (held.travelled < THRESHOLD) {
        return;
      }
      if (!held.dragging) {
        held.dragging = true;
        // Captured only now: capturing on press would steal clicks from the bar's own buttons.
        event.currentTarget.setPointerCapture(event.pointerId);
        onStart();
      }
      held.lift += dy;
      onLift(held.lift);
      const queue = pending.current;
      queue.dx += dx;
      if (queue.flight === null) {
        void drain(queue);
      }
    },
    [onStart, onLift],
  );

  const release = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      const held = grab.current;
      if (held?.pointerId !== event.pointerId) {
        return;
      }
      grab.current = null;

      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }

      // A plain click must not ask Rust to re-dock.
      if (!held.dragging) {
        return;
      }
      // Settle only after every move is sent, or Rust would dock from a stale position.
      // A failed settle keeps the previous (still stored) settings.
      const { lift } = held;
      void (pending.current.flight ?? drain(pending.current))
        .then(() => endDrag(lift))
        .then((result) => {
          onEnd(result.ok ? result.value : null);
        });
    },
    [onEnd],
  );

  return {
    onPointerDown,
    onPointerMove,
    onPointerUp: release,
    onPointerCancel: release,
  };
}
