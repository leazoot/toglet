// Settings row that opens the reset-alerts page. It reads the same store as the page, and never
// shows "Off" while the state is still unknown.

import { useEffect } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import sheet from "../settings/SettingsSheet.module.css";
import notify from "../notify/NotifySection.module.css";
import { cx } from "../../styles/classes";
import { useResets } from "./store";

export function ResetsRow({ onOpen }: { onOpen: () => void }): JSX.Element {
  const resets = useResets((state) => state.resets);
  const load = useResets((state) => state.load);

  useEffect(() => {
    void load();
  }, [load]);

  const off = resets.state === "ready" && !resets.value.enabled;

  return (
    <div className={sheet["row"]} data-testid="resets-row">
      <span className={cx(sheet["label"], sheet["strong"])}>{t("resets.section")}</span>
      <button
        type="button"
        className={cx(notify["entry"], off && notify["entryEmpty"])}
        onClick={onOpen}
      >
        {summary(resets)}
        <Chevron />
      </button>
    </div>
  );
}

function summary(resets: ReturnType<typeof useResets.getState>["resets"]): string {
  if (resets.state === "loading") {
    return t("resets.checking");
  }
  if (resets.state === "failed") {
    return t("resets.unknown");
  }
  if (!resets.value.enabled) {
    return t("resets.off");
  }
  const count = resets.value.channelIds.length;
  if (count === 0) {
    return t("resets.on");
  }
  return t(count === 1 ? "resets.onCountOne" : "resets.onCount", { count });
}

function Chevron(): JSX.Element {
  return (
    <svg viewBox="0 0 12 12" className={notify["chevron"]} aria-hidden="true">
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
