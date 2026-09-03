// The whole docked surface: the bar, and the panel when it is open.
//
// The window is a fixed strip against the screen edge, as tall as the work area, and it never
// changes size. This component owns the two things the pieces below it cannot know:
//
// * **the gesture** - 120ms of pointer intent before opening, 260ms of grace before closing;
// * **where the pieces sit inside the strip** - the bar at the offset the user dragged it to, and
//   the panel centred on the bar but kept inside the room the window has for it. Both are
//   variables the stylesheet places from; the panel's height is measured because the stylesheet
//   cannot centre a box on a point without knowing how tall the box is.
//
// Opening is three stages:
//
//   closed → open → leaving → closed
//
// `open` starts the entrance animation in the frame the panel renders, which is right because
// the window is already its final size - it always is. `leaving` holds the panel on screen for
// the exit animation. Rust is told when the panel opens and closes so that the strip
// lets clicks through to the desktop while only the bar is showing.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { JSX } from "react";

import { cx } from "../../styles/classes";
import { durationToken } from "../../styles/motion";
import type { AccountView, QuotaView, SettingsView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { AnchorNub, NUB_FILL_PATH } from "./AnchorNub";
import styles from "./Dock.module.css";
import { EdgeBar } from "./EdgeBar";
import type { BarNotice } from "./EdgeBar";
import { Panel } from "./Panel";
import type { PanelStatus } from "./Panel";
import { useDragToSnap } from "./useDragToSnap";
import { useHoverIntent } from "./useHoverIntent";

type Stage = "closed" | "open" | "leaving";

const COLLAPSE_DURATION = "--tg-duration-collapse";
/** The bar's centre, in logical pixels below the work area's centre. Read by Dock.module.css. */
const OFFSET_VARIABLE = "--tg-dock-offset";
/** The panel's rendered height. Read by Dock.module.css to centre the panel on the bar. */
const PANEL_HEIGHT_VARIABLE = "--tg-panel-height";
/**
 * How far the panel's top is above the nub's top, so the nub's copy of the scrim can be drawn
 * with the panel's gradient in the panel's coordinates rather than its own.
 */
const NUB_SCRIM_SHIFT_VARIABLE = "--tg-nub-scrim-shift";

export interface DockProps {
  side: "left" | "right";
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  /**
   * Where the bar is: logical pixels from the work area's vertical centre to the bar's centre,
   * positive downward. Straight from the stored settings, which Rust clamps to the monitor
   * before storing, so the stylesheet and Rust's hover target agree about where the bar is.
   */
  offset: number;
  /** The settings Rust stored when a drag of the bar ended. They carry the new `offset`. */
  onDragSettled: (settings: SettingsView) => void;
  account: Loadable<AccountView | null>;
  accounts: Loadable<readonly AccountView[]>;
  quotas: Readonly<Record<string, Loadable<QuotaView>>>;
  activeQuota: Loadable<QuotaView>;
  refreshing: boolean;
  notice: BarNotice | null;
  status: PanelStatus;
  nowSeconds: number;
  onRefresh: () => void;
  onSelect: (account: AccountView) => void;
  onOpenSettings: () => void;
  onAddAccount: () => void;
  /** The settings sheet, or `null`. Like the overlay it pins the panel open. */
  sheet: JSX.Element | null;
  /** The switch overlay, or `null`. While one is open the panel stays open regardless of the
   *  pointer. */
  overlay: JSX.Element | null;
}

export function Dock({
  side,
  expanded,
  onExpandedChange,
  offset,
  onDragSettled,
  account,
  accounts,
  quotas,
  activeQuota,
  refreshing,
  notice,
  status,
  nowSeconds,
  onRefresh,
  onSelect,
  onOpenSettings,
  onAddAccount,
  overlay,
  sheet,
}: DockProps): JSX.Element {
  const box = useRef<HTMLDivElement | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  const nub = useRef<HTMLSpanElement | null>(null);
  const [stage, setStage] = useState<Stage>("closed");
  // The `expanded` the stage was last derived from. Comparing against it during render is how
  // a prop change becomes a stage change without an effect setting state (react.dev, "storing
  // information from previous renders"); `expanded` can also be pulled down by the parent - Esc
  // does - so the stage has to follow the prop, not only the gesture below.
  const [seen, setSeen] = useState(expanded);
  if (expanded !== seen) {
    setSeen(expanded);
    if (expanded) {
      setStage("open");
    } else if (stage !== "closed") {
      setStage("leaving");
    }
  }

  // An overlay waiting for an answer pins the panel open: a dialog that vanishes because the
  // pointer moved away would take its question with it.
  const pinned = overlay !== null || sheet !== null;
  const intent = useHoverIntent(expanded, pinned, onExpandedChange);
  // A press on the bar is ambiguous until the pointer moves: it could be the start of a drag, or
  // it could just be the pointer resting there on its way to opening the panel. Once it turns out
  // to be a drag, the open that press scheduled has to be called off.
  const drag = useDragToSnap(intent.cancel, onDragSettled);

  // The bar's empty state has one control. Opening the sheet alone would put it in a panel that
  // is not showing; opening the panel alone would leave the user to find the toolbar's plus.
  const addFromBar = useCallback(() => {
    onExpandedChange(true);
    onAddAccount();
  }, [onExpandedChange, onAddAccount]);
  // The rows are the chooser; opening the panel is all the bar has to do.
  const pickFromBar = useCallback(() => {
    onExpandedChange(true);
  }, [onExpandedChange]);

  // Leaving: the exit animation plays for the collapse duration, then the panel is unmounted.
  // The pointer coming back changes the stage, and the cleanup is what calls the unmount off.
  // Zero under reduced motion, which makes it a tick rather than instant; the animation is off
  // too, so nothing is seen of that tick.
  useEffect(() => {
    if (stage !== "leaving") {
      return;
    }
    const timer = setTimeout(
      () => {
        setStage("closed");
      },
      durationToken(COLLAPSE_DURATION, 160),
    );
    return () => {
      clearTimeout(timer);
    };
  }, [stage]);

  // Written to the element rather than held as state: it is a number the stylesheet needs, not
  // one the render depends on, and setting it before paint is what keeps the bar from ever being
  // drawn anywhere but where it belongs.
  useLayoutEffect(() => {
    box.current?.style.setProperty(OFFSET_VARIABLE, `${String(offset)}px`);
  }, [offset]);

  const measure = useCallback(() => {
    const height = panel.current?.offsetHeight ?? 0;
    box.current?.style.setProperty(PANEL_HEIGHT_VARIABLE, `${String(height)}px`);
    // Measured rather than derived: both positions are clamp() expressions in the stylesheet,
    // and the difference between them is what the nub's scrim needs as a plain length.
    if (panel.current !== null && nub.current !== null) {
      const shift =
        panel.current.getBoundingClientRect().top - nub.current.getBoundingClientRect().top;
      box.current?.style.setProperty(NUB_SCRIM_SHIFT_VARIABLE, `${String(shift)}px`);
    }
  }, []);

  // Before the panel's first paint, so its first frame is already centred on the bar. Again
  // whenever the bar moves: the nub moves with it, and the scrim's alignment with the panel
  // depends on where the nub is.
  useLayoutEffect(() => {
    if (stage !== "closed") {
      measure();
    }
  }, [stage, measure, offset]);

  // The panel can change height while it is open - an account finishes loading, a row swaps its
  // quota lines for a notice, the settings sheet replaces the list. Each of those moves where
  // "centred on the bar" is.
  useEffect(() => {
    const element = panel.current;
    if (stage === "closed" || element === null || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [stage, measure]);

  // The hover target is the surface - the bar with its buffer, the panel with its gap, the nub in
  // between - and nothing else. The strip around them is transparent desktop, and a pointer over
  // it is not over Toglet, so the gesture is on the pieces rather than on the box that holds them.
  const hover = {
    onPointerEnter: intent.onPointerEnter,
    onPointerLeave: intent.onPointerLeave,
  };

  return (
    <div
      ref={box}
      className={cx(
        styles["dock"],
        side === "left" ? styles["left"] : styles["right"],
        stage !== "closed" && styles["expanded"],
        styles[stage],
      )}
      data-testid="dock"
      data-stage={stage}
    >
      {stage !== "closed" && (
        <>
          <div className={styles["panelWrap"]} data-testid="dock-panel" {...hover}>
            <Panel
              ref={panel}
              accounts={accounts}
              quotas={quotas}
              refreshing={refreshing}
              status={status}
              nowSeconds={nowSeconds}
              onRefresh={onRefresh}
              onSelect={onSelect}
              onOpenSettings={onOpenSettings}
              onAddAccount={onAddAccount}
              overlay={overlay}
              sheet={sheet}
            />
          </div>
          <span ref={nub} className={styles["nub"]} {...hover}>
            <AnchorNub />
            {/* The panel's scrim, continued over the nub. The panel dims itself behind an
                overlay, but the nub is outside the panel: without this it stayed at full
                brightness against the dimmed edge and read as a piece stuck on. */}
            {pinned && (
              <span
                className={styles["nubScrim"]}
                data-testid="nub-scrim"
                style={{ clipPath: `path("${NUB_FILL_PATH}")` }}
              />
            )}
          </span>
        </>
      )}
      <div className={styles["barWrap"]} data-testid="dock-bar" {...hover}>
        <EdgeBar
          side={side}
          account={account}
          hasAccounts={accounts.state === "ready" && accounts.value.length > 0}
          quota={activeQuota}
          notice={notice}
          nowSeconds={nowSeconds}
          drag={drag}
          onAddAccount={addFromBar}
          onPickAccount={pickFromBar}
        />
      </div>
    </div>
  );
}
