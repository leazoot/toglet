import type { JSX } from "react";

/** The plus of the toolbar's add button and of the bar's empty state: one drawing, two sizes. */
export function AddIcon({ className }: { className: string | undefined }): JSX.Element {
  return (
    <svg viewBox="0 0 15 15" className={className} aria-hidden="true">
      <path
        d="M7.5 3 L7.5 12 M3 7.5 L12 7.5"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinecap="round"
      />
    </svg>
  );
}
