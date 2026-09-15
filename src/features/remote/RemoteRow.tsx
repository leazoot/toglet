// Settings row that opens the phone remote page. It reads the same store as the page, and never
// shows "Off" while the state is still unknown.

import { useEffect } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import sheet from "../settings/SettingsSheet.module.css";
import notify from "../notify/NotifySection.module.css";
import { cx } from "../../styles/classes";
import { useRemote } from "./store";

export function RemoteRow({ onOpen }: { onOpen: () => void }): JSX.Element {
  const remote = useRemote((state) => state.remote);
  const load = useRemote((state) => state.load);

  useEffect(() => {
    void load();
  }, [load]);

  const off = remote.state === "ready" && !remote.value.enabled;

  return (
    <div className={sheet["row"]} data-testid="remote-row">
      <span className={cx(sheet["label"], sheet["strong"])}>{t("remote.section")}</span>
      <button
        type="button"
        className={cx(notify["entry"], off && notify["entryEmpty"])}
        onClick={onOpen}
      >
        {summary(remote)}
        <Chevron />
      </button>
    </div>
  );
}

function summary(remote: ReturnType<typeof useRemote.getState>["remote"]): string {
  if (remote.state === "loading") {
    return t("remote.checking");
  }
  if (remote.state === "failed") {
    return t("remote.unknown");
  }
  if (!remote.value.enabled) {
    return t("remote.off");
  }
  return remote.value.bridgeHost === "" ? t("remote.on") : remote.value.bridgeHost;
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
