/**
 * The open/close gesture. Both delays are motion tokens, so reduced motion turns them off: a short
 * open delay ignores pointers passing the screen edge, a close delay allows crossing the panel gap.
 */

import { useCallback, useEffect, useRef } from "react";

import { durationToken } from "../../styles/motion";

const OPEN_DELAY = "--tg-hover-intent-delay";
const CLOSE_DELAY = "--tg-collapse-delay";

export interface HoverIntent {
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  /** Calls off a scheduled transition, e.g. the open queued by a press that became a drag. */
  cancel: () => void;
}

/**
 * `onChange` is called at most once per transition. `held` pins the panel open regardless of the
 * pointer.
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
      // Already open: only call off the pending close.
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
