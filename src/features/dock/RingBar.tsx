// The collapsed surface as concentric rings (outer five-hour, inner weekly) with both numbers
// beneath. Same rules as the bar: no reading draws a dashed track and an em dash, never `0%`.
// A healthy inner ring has its own hue; warn / low / empty status colours outrank that identity.

import type { JSX } from "react";

import { RING_FORM_GEOMETRY } from "../../components/geometry";
import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import { accentOf, initialOf } from "../accounts/identity";
import {
  RING_FORM_INNER_CIRCUMFERENCE,
  RING_FORM_OUTER_CIRCUMFERENCE,
  arcDash,
  isStale,
  percentLabel,
  tone,
} from "../quotas/format";
import type { QuotaValue } from "../quotas/format";
import { AddIcon } from "./AddIcon";
import { describe, emptyKey, noticeKey, valueOf } from "./barText";
import { PickIcon } from "./EdgeBar";
import type { EdgeBarProps } from "./EdgeBar";
import styles from "./RingBar.module.css";

const CENTRE = RING_FORM_GEOMETRY.box / 2;
const VIEW_BOX = `0 0 ${RING_FORM_GEOMETRY.box.toString()} ${RING_FORM_GEOMETRY.box.toString()}`;

type Ring = "outer" | "inner";

/** The same props as the bar: this is the same surface in another shape. */
export function RingBar({
  side,
  account,
  hasAccounts,
  quota,
  notice,
  nowSeconds,
  drag,
  onAddAccount,
  onPickAccount,
}: EdgeBarProps): JSX.Element {
  const stale = quota.state === "ready" && isStale(quota.value, nowSeconds);
  const active = account.state === "ready" ? account.value : null;
  const empty = account.state === "ready" && !hasAccounts;
  const none = account.state === "ready" && hasAccounts && active === null;

  const five = valueOf(quota, "five_hour");
  const week = valueOf(quota, "weekly");
  // Both windows in one sentence, because the rings are one image.
  const sentence =
    active === null
      ? t(emptyKey(account, hasAccounts))
      : `${describe("five_hour", five, quota, nowSeconds, stale)} ${describe("weekly", week, quota, nowSeconds, stale)}`;

  return (
    <div
      className={cx(styles["bar"], side === "left" ? styles["left"] : styles["right"])}
      data-testid="ring-bar"
      {...drag}
    >
      {empty || none ? (
        <button
          type="button"
          className={cx(styles["rings"], styles["button"])}
          onClick={empty ? onAddAccount : onPickAccount}
          aria-label={t(empty ? "bar.addAccount" : "bar.pickAccount")}
          title={t(empty ? "bar.addAccount" : "bar.pickAccount")}
          data-testid={empty ? "bar-add" : "bar-pick"}
        >
          <Rings five={{ kind: "unreadable" }} week={{ kind: "unreadable" }} />
          {empty ? <AddIcon className={styles["icon"]} /> : <PickIcon />}
        </button>
      ) : (
        <div className={styles["rings"]} role="img" aria-label={sentence} title={sentence}>
          {/* No rings without an account: two empty rings would suggest a blank reading. */}
          {active !== null && <Rings five={five} week={week} />}
          <span className={styles["initial"]} data-accent={accentOf(active)}>
            {initialOf(active)}
          </span>
          {notice !== null && (
            <span
              className={styles["notice"]}
              role="img"
              aria-label={t(noticeKey(notice))}
              title={t(noticeKey(notice))}
            />
          )}
        </div>
      )}

      {active !== null && (
        <span className={styles["row"]} aria-hidden="true">
          <span className={cx(styles["value"], styles[valueTone(five, "outer")])}>
            {percentLabel(five)}
          </span>
          <span className={cx(styles["value"], styles[valueTone(week, "inner")])}>
            {percentLabel(week)}
          </span>
        </span>
      )}
    </div>
  );
}

/** The two rings: the five-hour window outside, the weekly inside. */
function Rings({ five, week }: { five: QuotaValue; week: QuotaValue }): JSX.Element {
  return (
    <svg className={styles["svg"]} viewBox={VIEW_BOX} aria-hidden="true">
      <Arc
        ring="outer"
        radius={RING_FORM_GEOMETRY.outerRadius}
        circumference={RING_FORM_OUTER_CIRCUMFERENCE}
        value={five}
      />
      <Arc
        ring="inner"
        radius={RING_FORM_GEOMETRY.innerRadius}
        circumference={RING_FORM_INNER_CIRCUMFERENCE}
        value={week}
      />
    </svg>
  );
}

/** One ring's track and arc, drawn the way QuotaRing draws its own. */
function Arc({
  ring,
  radius,
  circumference,
  value,
}: {
  ring: Ring;
  radius: number;
  circumference: number;
  value: QuotaValue;
}): JSX.Element {
  const hasReading = value.kind === "value";
  return (
    <>
      <circle
        className={hasReading ? styles["track"] : styles["trackUnread"]}
        cx={CENTRE}
        cy={CENTRE}
        r={radius}
        strokeWidth={RING_FORM_GEOMETRY.stroke}
      />
      <circle
        className={cx(styles["arc"], styles[arcTone(value, ring)])}
        cx={CENTRE}
        cy={CENTRE}
        r={radius}
        strokeWidth={RING_FORM_GEOMETRY.stroke}
        strokeDasharray={arcDash(value, circumference)}
      />
    </>
  );
}

/** The arc's colour class: the quota tone, except that a healthy inner ring has its own hue. */
function arcTone(value: QuotaValue, ring: Ring): string {
  const state = tone(value);
  return state === "healthy" && ring === "inner" ? "healthyInner" : state;
}

function valueTone(value: QuotaValue, ring: Ring): string {
  return `value-${arcTone(value, ring)}`;
}
