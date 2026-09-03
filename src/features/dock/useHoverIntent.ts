/**
 * The open/close gesture.
 *
 * Two delays, both from the token set so that `prefers-reduced-motion` turns them off along with
 * the animations:
 *
 * * **120ms before opening.** The bar sits against the screen edge, which is exactly where a
 *   pointer travels on its way somewhere else. Without the delay the panel would open every time
 *   somebody reached for a scrollbar.
 * * **260ms before closing.** Long enough to cross the 9 pixel gap between panel and bar, or to
 *   come back after overshooting. Re-entering cancels it.
 */

import { useCallback, useEffect, useRef } from "react";

import { durationToken } from "../../styles/motion";

const OPEN_DELAY = "--tg-hover-intent-delay";
const CLOSE_DELAY = "--tg-collapse-delay";

export interface HoverIntent {
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  /**
   * Calls off a transition that has been scheduled but has not happened yet.
   *
   * A drag takes the gesture over: without this, the open scheduled by the press that started the
   * drag would still fire, and the panel would appear under a pointer that is busy moving the
   * window.
   */
  cancel: () => void;
}

/**
 * `onChange` is called with the intended state, never more than once per transition.
 *
 * `held` pins the panel open regardless of the pointer - a dialog waiting for an answer must not
 * vanish because the pointer moved away.
 */
export function useHoverIntent(
  open: boolean,
  held: boolean,
  onChange: (open: boolean) => void,
): HoverIntent {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancel = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  // A component that unmounts mid-gesture must not leave a timer that fires into nothing.
  useEffect(() => cancel, [cancel]);

  const schedule = useCallback(
    (next: boolean, delay: number) => {
      cancel();
      if (delay <= 0) {
        onChange(next);
        return;
      }
      timer.current = setTimeout(() => {
        timer.current = null;
        onChange(next);
      }, delay);
    },
    [cancel, onChange],
  );

  const onPointerEnter = useCallback(() => {
    if (open) {
      // Already open and the pointer came back: the only thing to do is call off the close.
      cancel();
      return;
    }
    schedule(true, durationToken(OPEN_DELAY, 120));
  }, [cancel, open, schedule]);

  const onPointerLeave = useCallback(() => {
    if (held) {
      cancel();
      return;
    }
    schedule(false, durationToken(CLOSE_DELAY, 260));
  }, [cancel, held, schedule]);

  return { onPointerEnter, onPointerLeave, cancel };
}
