// Settings row that shows the channel count and opens the channels page. It reads the same
// store as the page, and never shows "None" while the count is still unknown.

import { useEffect } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import sheet from "../settings/SettingsSheet.module.css";
import { cx } from "../../styles/classes";
import { useNotify } from "./store";
import styles from "./NotifySection.module.css";

export function NotifyRow({ onOpen }: { onOpen: () => void }): JSX.Element {
  const channels = useNotify((state) => state.channels);
  const load = useNotify((state) => state.load);

  useEffect(() => {
    void load();
  }, [load]);

  const count = channels.state === "ready" ? channels.value.channels.length : null;

  return (
    <div className={sheet["row"]} data-testid="notify-row">
      <span className={cx(sheet["label"], sheet["strong"])}>{t("notify.section")}</span>
      <button
        type="button"
        className={cx(styles["entry"], count === 0 && styles["entryEmpty"])}
        onClick={onOpen}
      >
        {summary(channels.state, count)}
        <Chevron />
      </button>
    </div>
  );
}

function summary(state: "loading" | "ready" | "failed", count: number | null): string {
  if (state === "loading") {
    return t("notify.counting");
  }
  if (count === null) {
    return t("notify.countUnknown");
  }
  if (count === 0) {
    return t("notify.none");
  }
  return t(count === 1 ? "notify.countOne" : "notify.count", { count });
}

function Chevron(): JSX.Element {
  return (
    <svg viewBox="0 0 12 12" className={styles["chevron"]} aria-hidden="true">
      <path
        d="M4.5 2.5 L8 6 L4.5 9.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
