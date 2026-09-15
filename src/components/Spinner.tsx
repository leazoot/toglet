// The small brand-arc spinner. Hidden from assistive technology: callers supply the words.

import type { JSX } from "react";

import { cx } from "../styles/classes";
import styles from "./Spinner.module.css";

export interface SpinnerProps {
  className?: string;
}

export function Spinner({ className }: SpinnerProps): JSX.Element {
  return (
    <svg
      viewBox="0 0 15 15"
      className={cx(styles["spinner"], className)}
      aria-hidden="true"
      data-testid="spinner"
    >
      <circle
        cx="7.5"
        cy="7.5"
        r="5.9"
        fill="none"
        stroke="var(--tg-line-spinner-track)"
        strokeWidth="1.5"
      />
      <path
        d="M7.5 1.6 A5.9 5.9 0 0 1 13.4 7.5"
        fill="none"
        stroke="var(--tg-brand)"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}
