// Segmented choice and switch rows, shared by the settings sheet and the auto-continue group.

import type { JSX } from "react";

import { cx } from "../../styles/classes";
import styles from "./SettingsSheet.module.css";

export interface SegmentedOption<T extends string | number> {
  value: T;
  label: string;
  /** An option that cannot be picked right now - a deadline that has already passed. */
  disabled?: boolean;
}

export interface SegmentedProps<T extends string | number> {
  label: string;
  value: T;
  options: readonly SegmentedOption<T>[];
  disabled: boolean;
  /** Label above the control, for choices too wide to share a row; the control may wrap. */
  stacked?: boolean;
  /** The label in the heading weight, where it names a section rather than one setting. */
  strong?: boolean;
  onPick: (value: T) => void;
}

export function Segmented<T extends string | number>({
  label,
  value,
  options,
  disabled,
  stacked = false,
  strong = false,
  onPick,
}: SegmentedProps<T>): JSX.Element {
  return (
    <div className={cx(styles["row"], stacked && styles["stacked"])}>
      <span className={cx(styles["label"], strong && styles["strong"])} id={fieldId(label)}>
        {label}
      </span>
      <div
        // Three or more items give up a pixel of padding each so the group still fits the row.
        className={cx(styles["segmented"], options.length > 2 && styles["tight"])}
        role="radiogroup"
        aria-labelledby={fieldId(label)}
      >
        {options.map((option) => (
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={option.value === value}
            className={cx(styles["segment"], option.value === value && styles["picked"])}
            disabled={disabled || option.disabled === true}
            onClick={() => {
              onPick(option.value);
            }}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export interface ToggleProps {
  label: string;
  value: boolean;
  disabled: boolean;
  /** Why the switch cannot be turned on, shown beside it. */
  note?: string | undefined;
  /** See `SegmentedProps::strong`. */
  strong?: boolean;
  onPick: (value: boolean) => void;
}

export function Toggle({
  label,
  value,
  disabled,
  note,
  strong = false,
  onPick,
}: ToggleProps): JSX.Element {
  return (
    <div className={styles["row"]}>
      <span className={cx(styles["label"], strong && styles["strong"])}>{label}</span>
      {note !== undefined && <span className={styles["note"]}>{note}</span>}
      <button
        type="button"
        role="switch"
        aria-checked={value}
        aria-label={label}
        className={cx(styles["switch"], value && styles["on"])}
        disabled={disabled}
        onClick={() => {
          onPick(!value);
        }}
      >
        <span className={styles["knob"]} aria-hidden="true" />
      </button>
    </div>
  );
}

function fieldId(label: string): string {
  return `setting-${label.replace(/\W+/g, "-").toLowerCase()}`;
}
