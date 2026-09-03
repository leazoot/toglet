/**
 * Dragging the bar to the other edge or another monitor.
 *
 * The gesture is here; the decision is not. Where the window lands - which monitor, which edge,
 * how far down - is worked out in Rust from the window's own rectangle, because that is the side
 * that knows what monitors are attached and what a work area is. The interface only says "the
 * pointer moved this far" and "the pointer was let go".
 *
 * Increments rather than positions, and screen coordinates rather than client ones: the window
 * moves out from under the pointer during the drag, so anything measured against the window would
 * feed back into itself.
 *
 * One move on its way at a time. A pointer reports several hundred moves a second, and each one
 * sent as its own command queued behind the last: the window worked through the backlog at the
 * pace the platform could move it, and by the time the pointer had stopped the window was still
 * setting off. Moves that arrive while one is unanswered are summed and sent as a single
 * increment when the answer comes, so the backlog is never more than one command deep and the
 * window is at most one move behind the pointer.
 */

import { useCallback, useRef } from "react";
import type { PointerEvent } from "react";

import { endDrag, moveDock } from "../../ipc";
import type { SettingsView } from "../../types/ipc";

/**
 * How far the pointer must travel before a press becomes a drag, in logical pixels.
 *
 * Without it every click on the bar would nudge the window. Small enough that a deliberate drag
 * feels immediate, large enough to survive the wobble of a click.
 */
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
  /** Whether the press has crossed the threshold and become a drag. */
  dragging: boolean;
}

/** Travel the pointer has reported that the window has not yet been asked to make. */
interface Pending {
  dx: number;
  dy: number;
  /** The move on its way, resolving once it and everything summed behind it has been sent. */
  flight: Promise<void> | null;
}

/**
 * Sends what is pending as one move, then whatever gathered while it was on its way, until
 * nothing is left. Resolves when the window has been asked for all of it.
 */
function drain(queue: Pending): Promise<void> {
  const { dx, dy } = queue;
  queue.dx = 0;
  queue.dy = 0;
  if (dx === 0 && dy === 0) {
    queue.flight = null;
    return Promise.resolve();
  }
  // A move that failed has left the window where it was; the next one carries on from there,
  // and a window that ended up adrift is reported when the drag settles.
  queue.flight = moveDock(dx, dy).then(() => drain(queue));
  return queue.flight;
}

/**
 * `onStart` is called once, when a press turns into a drag - the moment the hover gesture has to
 * be called off so the panel does not open under the pointer mid-drag.
 *
 * `onSettled` receives the settings Rust stored when the drag ended. The bar is drawn from the
 * stored offset, and Rust's hover target is placed from the same number: a drag whose new offset
 * never reached the stylesheet left the bar drawn where the pointer no longer got through, and
 * it could be neither hovered nor dragged again until the next start.
 */
export function useDragToSnap(
  onStart: () => void,
  onSettled: (settings: SettingsView) => void,
): DragHandlers {
  // A ref, not state, and deliberately so. Whether a press became a drag is read at release,
  // and a release can arrive before React has rendered the move that made it one: pointer moves
  // are continuous events, batched at low priority, while the release is discrete. Held in state
  // it was stale at exactly that moment, the release saw "not dragging", and the window was left
  // wherever the drag had moved it with nothing ever settling it.
  const grab = useRef<Grab | null>(null);
  const pending = useRef<Pending>({ dx: 0, dy: 0, flight: null });

  const onPointerDown = useCallback((event: PointerEvent<HTMLElement>) => {
    // Primary button only. A right click belongs to whatever context menu the platform offers,
    // and a middle click must not move the window.
    if (event.button !== 0) {
      return;
    }
    grab.current = {
      pointerId: event.pointerId,
      lastX: event.screenX,
      lastY: event.screenY,
      travelled: 0,
      dragging: false,
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
        // Taken only now. A pointer captured on the press would deliver its release to the bar
        // rather than to whatever was pressed, and a click on the bar's own add button would
        // never arrive; a press that stays put takes nothing.
        event.currentTarget.setPointerCapture(event.pointerId);
        onStart();
      }
      const queue = pending.current;
      queue.dx += dx;
      queue.dy += dy;
      if (queue.flight === null) {
        void drain(queue);
      }
    },
    [onStart],
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

      // A press that never became a drag moved nothing, so there is nothing to settle - asking
      // Rust to re-dock would move the window on a plain click.
      if (!held.dragging) {
        return;
      }
      // Settled only once every move has been made: a settle that overtook the last increment
      // would dock the window from a place the pointer had already left. A settle that failed
      // leaves the previous settings showing. They are still the ones stored, so nothing about
      // them has become untrue; the window itself may be adrift, and that is reported the way
      // every failed command is.
      void (pending.current.flight ?? drain(pending.current)).then(endDrag).then((result) => {
        if (result.ok) {
          onSettled(result.value);
        }
      });
    },
    [onSettled],
  );

  return {
    onPointerDown,
    onPointerMove,
    onPointerUp: release,
    onPointerCancel: release,
  };
}
