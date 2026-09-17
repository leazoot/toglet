// Reset-alerts page of the settings sheet: one switch, the credit the feed's terms ask for right
// under it, then the channels to send to as well. Each channel is a switch of its own, the same
// control the notifications page uses. One line of intro, no explanations (user 2026-09-17).

import { useEffect } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import { useNotify } from "../notify/store";
import { Toggle } from "../settings/controls";
import { useResets } from "./store";
import styles from "./ResetsSection.module.css";

export function ResetsSection(): JSX.Element {
  const resets = useResets((state) => state.resets);
  const busy = useResets((state) => state.busy);
  const failure = useResets((state) => state.failure);
  const load = useResets((state) => state.load);
  const save = useResets((state) => state.save);
  const openSite = useResets((state) => state.openSite);
  const channels = useNotify((state) => state.channels);
  const loadChannels = useNotify((state) => state.load);

  useEffect(() => {
    void load();
    void loadChannels();
  }, [load, loadChannels]);

  if (resets.state === "loading") {
    return <div className={styles["page"]}>{t("resets.checking")}</div>;
  }
  if (resets.state === "failed") {
    return <div className={styles["page"]}>{t("resets.unreachable")}</div>;
  }

  const view = resets.value;
  const chosen = new Set(view.channelIds);
  const pick = (id: string, on: boolean): void => {
    void save({
      enabled: view.enabled,
      channelIds: on ? [...view.channelIds, id] : view.channelIds.filter((one) => one !== id),
    });
  };

  return (
    <div className={styles["page"]} data-testid="resets-section">
      <p className={styles["hint"]}>{t("resets.intro")}</p>

      <Toggle
        label={t("resets.enable")}
        value={view.enabled}
        disabled={busy}
        strong
        onPick={(enabled) => {
          void save({ enabled, channelIds: view.channelIds });
        }}
      />
      <p className={styles["credit"]}>
        <button
          type="button"
          className={styles["tool"]}
          onClick={() => {
            void openSite();
          }}
        >
          {t("resets.credit")}
        </button>
      </p>

      {view.enabled && (
        <>
          <p className={styles["sectionTitle"]}>{t("resets.channelsTitle")}</p>
          {channels.state === "loading" && <p className={styles["hint"]}>{t("notify.counting")}</p>}
          {channels.state === "failed" && (
            <p className={styles["alert"]}>{t("notify.unreadable")}</p>
          )}
          {channels.state === "ready" && channels.value.channels.length === 0 && (
            <p className={styles["hint"]}>{t("resets.noChannels")}</p>
          )}
          {channels.state === "ready" &&
            channels.value.channels.map((channel) => (
              <Toggle
                key={channel.id}
                label={channel.label}
                note={channel.hint}
                value={chosen.has(channel.id)}
                disabled={busy}
                onPick={(on) => {
                  pick(channel.id, on);
                }}
              />
            ))}
        </>
      )}

      {failure !== null && (
        <p className={styles["alert"]} role="alert">
          {failure.error === null
            ? t("resets.unreachable")
            : t("resets.commandFailed", { code: failure.error.code })}
        </p>
      )}
    </div>
  );
}
