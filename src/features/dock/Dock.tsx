// The docked surface: the bar, and the panel when open, inside a fixed-size strip window.
// Owns the hover gesture and piece placement (closed → open → leaving → closed). The panel's
// height is measured because CSS cannot centre a box on a point without knowing its height.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { FocusEvent, JSX } from "react";

import { setDockExpansion } from "../../ipc";
import { cx } from "../../styles/classes";
import { durationToken } from "../../styles/motion";
import type {
  AccountView,
  AutoRunView,
  DockShape,
  QuotaView,
  SettingsView,
  ResetsView,
} from "../../types/ipc";
import type { Loadable } from "../../types/load";
import type { AutoRunControl, AutoRunFailure } from "../autorun/store";
import { AnchorNub, NUB_FILL_PATH } from "./AnchorNub";
import styles from "./Dock.module.css";
import { EdgeBar } from "./EdgeBar";
import type { BarNotice } from "./EdgeBar";
import { Panel } from "./Panel";
import type { PanelStatus } from "./Panel";
import { RingBar } from "./RingBar";
import { useDragToSnap } from "./useDragToSnap";
import { useHoverIntent } from "./useHoverIntent";

type Stage = "closed" | "open" | "leaving";

const COLLAPSE_DURATION = "--tg-duration-collapse";
/** The bar's centre, in logical pixels below the work area's centre. Read by Dock.module.css. */
const OFFSET_VARIABLE = "--tg-dock-offset";
/**
 * How far a drag in progress has carried the bar from its offset, in logical pixels. Read by
 * Dock.module.css, which moves the whole surface by it: the window itself is not moved
 * vertically during a drag (see useDragToSnap).
 */
const LIFT_VARIABLE = "--tg-drag-lift";
/** The panel's rendered height. Read by Dock.module.css to centre the panel on the bar. */
const PANEL_HEIGHT_VARIABLE = "--tg-panel-height";
/**
 * How far the panel's top is above the nub's top, so the nub's copy of the scrim can be drawn
 * with the panel's gradient in the panel's coordinates rather than its own.
 */
const NUB_SCRIM_SHIFT_VARIABLE = "--tg-nub-scrim-shift";

export interface DockProps {
  side: "left" | "right";
  /** Which collapsed surface to draw. Straight from the settings, like `side`. */
  shape: DockShape;
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
  onResetCredits: (account: AccountView, held: number) => void;
  onOpenSettings: () => void;
  onAddAccount: () => void;
  onOpenAutoRun: () => void;
  /**
   * The sheet in the panel, or `null`. Dims the nub but does not keep the panel open: settings
   * save as they change and the continuation draft survives in its store.
   */
  sheet: JSX.Element | null;
  /**
   * True while the sheet is waiting for something that would be lost with the panel - a browser
   * sign-in under way, a removal and its report. Then the panel stays regardless of the pointer.
   */
  held: boolean;
  /** The switch overlay, or `null`. While one is open the panel ignores the pointer leaving. */
  overlay: JSX.Element | null;
  /** Called once the exit animation ends, so the exit shows the panel as it was. */
  onCollapsed: () => void;
  /** The automatic-continuation plan for the panel's second status line and row marks. */
  autorun: AutoRunView | null;
  autorunBusy: boolean;
  autorunFailure: AutoRunFailure | null;
  onAutoRunControl: (action: AutoRunControl) => void;
  /** Reset alerts for the panel's banner; `null` while unknown. */
  resets: ResetsView | null;
}

export function Dock({
  side,
  shape,
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
  onResetCredits,
  onOpenSettings,
  onAddAccount,
  onOpenAutoRun,
  overlay,
  sheet,
  held,
  onCollapsed,
  autorun,
  autorunBusy,
  autorunFailure,
  onAutoRunControl,
  resets,
}: DockProps): JSX.Element {
  const box = useRef<HTMLDivElement | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  const nub = useRef<HTMLSpanElement | null>(null);
  const [stage, setStage] = useState<Stage>("closed");
  // Derives the stage from `expanded` during render (react.dev, "storing information from
  // previous renders"); the parent can also collapse it, e.g. on Esc.
  const [seen, setSeen] = useState(expanded);
  if (expanded !== seen) {
    setSeen(expanded);
    if (expanded) {
      setStage("open");
    } else if (stage !== "closed") {
      setStage("leaving");
    }
  }

  // A focused text field pins the panel so a drifting pointer does not lose half-typed input.
  const [typing, setTyping] = useState(false);
  const focusChanged = useCallback((event: FocusEvent<HTMLDivElement>) => {
    setTyping(event.type === "focus" && isTextField(event.target));
  }, []);
  // An overlay waiting for an answer pins the panel open. A sheet only dims the nub.
  const pinned = overlay !== null || held || typing;
  const dimmed = overlay !== null || sheet !== null;
  const intent = useHoverIntent(expanded, pinned, onExpandedChange);
  // Counts ended drags. The lift is cleared in the same layout effect that writes the offset, so
  // the bar never flashes at the old offset for a frame; a drag that leaves the offset unchanged
  // still bumps this count so the lift is cleared.
  const [drags, setDrags] = useState(0);
  const lift = useCallback((pixels: number) => {
    box.current?.style.setProperty(LIFT_VARIABLE, `${String(pixels)}px`);
  }, []);
  const dragEnded = useCallback(
    (settings: SettingsView | null) => {
      setDrags((count) => count + 1);
      if (settings !== null) {
        onDragSettled(settings);
      }
    },
    [onDragSettled],
  );
  // Once a press turns into a drag, the open that press scheduled is called off.
  const drag = useDragToSnap(intent.cancel, lift, dragEnded);

  // The sheet lives in the panel, so the bar's add control has to open both.
  const addFromBar = useCallback(() => {
    onExpandedChange(true);
    onAddAccount();
  }, [onExpandedChange, onAddAccount]);
  const pickFromBar = useCallback(() => {
    onExpandedChange(true);
  }, [onExpandedChange]);

  // Unmounts after the collapse duration; re-entering changes the stage and the cleanup cancels
  // it. The callback is read through a ref so a parent re-render does not restart the timer.
  const collapsed = useRef(onCollapsed);
  useEffect(() => {
    collapsed.current = onCollapsed;
  }, [onCollapsed]);
  useEffect(() => {
    if (stage !== "leaving") {
      return;
    }
    const timer = setTimeout(
      () => {
        setStage("closed");
        collapsed.current();
      },
      durationToken(COLLAPSE_DURATION, 160),
    );
    return () => {
      clearTimeout(timer);
    };
  }, [stage]);

  // Written to the element before paint so the bar is never drawn at a stale position.
  useLayoutEffect(() => {
    box.current?.style.setProperty(OFFSET_VARIABLE, `${String(offset)}px`);
    box.current?.style.setProperty(LIFT_VARIABLE, "0px");
  }, [offset, drags]);

  const measure = useCallback(() => {
    const height = panel.current?.offsetHeight ?? 0;
    box.current?.style.setProperty(PANEL_HEIGHT_VARIABLE, `${String(height)}px`);
    // Measured: both positions are CSS clamp() expressions, and the scrim needs a plain length.
    if (panel.current !== null && nub.current !== null) {
      const shift =
        panel.current.getBoundingClientRect().top - nub.current.getBoundingClientRect().top;
      box.current?.style.setProperty(NUB_SCRIM_SHIFT_VARIABLE, `${String(shift)}px`);
    }
    // The window is a full-height strip, so Rust needs the panel's rectangle, not just "open":
    // otherwise the whole column swallows clicks meant for the apps behind it.
    const rect = panel.current?.getBoundingClientRect();
    void setDockExpansion(
      true,
      rect === undefined ? null : { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    );
  }, []);

  // Before first paint so the first frame is centred; again when the bar (and so the nub) moves.
  useLayoutEffect(() => {
    if (stage !== "closed") {
      measure();
    }
  }, [stage, measure, offset]);

  // Closed: the bar is the only surface again, so the strip stops taking pointer events.
  useEffect(() => {
    if (stage === "closed") {
      void setDockExpansion(false, null);
    }
  }, [stage]);

  // The panel's height changes while open (loading, notices, sheets), which moves its centre.
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

  // The gesture is on the pieces, not the box: the rest of the strip is transparent desktop.
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
        shape === "ring" && styles["ring"],
        stage !== "closed" && styles["expanded"],
        styles[stage],
      )}
      data-testid="dock"
      data-stage={stage}
    >
      {stage !== "closed" && (
        <>
          <div
            className={styles["panelWrap"]}
            data-testid="dock-panel"
            {...hover}
            onFocus={focusChanged}
            onBlur={focusChanged}
          >
            <Panel
              ref={panel}
              accounts={accounts}
              quotas={quotas}
              refreshing={refreshing}
              status={status}
              nowSeconds={nowSeconds}
              onRefresh={onRefresh}
              onSelect={onSelect}
              onResetCredits={onResetCredits}
              onOpenSettings={onOpenSettings}
              onAddAccount={onAddAccount}
              onOpenAutoRun={onOpenAutoRun}
              overlay={overlay}
              sheet={sheet}
              autorun={autorun}
              autorunBusy={autorunBusy}
              autorunFailure={autorunFailure}
              onAutoRunControl={onAutoRunControl}
              resets={resets}
            />
          </div>
          <span ref={nub} className={styles["nub"]} {...hover}>
            <AnchorNub />
            {/* The panel's scrim continued over the nub, which sits outside the panel. */}
            {dimmed && (
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
        {shape === "ring" ? (
          <RingBar
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
        ) : (
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
        )}
      </div>
    </div>
  );
}

function isTextField(element: EventTarget): boolean {
  return (
    element instanceof HTMLTextAreaElement ||
    (element instanceof HTMLInputElement && element.type === "text")
  );
}
