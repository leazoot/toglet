// Phone remote page of the settings sheet. When paired it leads with the saved bridge and shows
// the fields only on request. The secret never comes back from Rust, so its field starts empty
// and empty means "keep what is stored".
//
// The state reads "on"/"off", never "connected": reachability is unknown until the next poll.

import { useEffect, useState } from "react";
import type { JSX } from "react";

import { t } from "../../i18n";
import { cx } from "../../styles/classes";
import { actionKey, failureKey, outcomeKey } from "./reason";
import { useRemote } from "./store";
import styles from "./RemoteSection.module.css";

const SECRET_FIELD = "toglet-remote-secret";

export function RemoteSection(): JSX.Element {
  const remote = useRemote((state) => state.remote);
  const busy = useRemote((state) => state.busy);
  const failure = useRemote((state) => state.failure);
  const load = useRemote((state) => state.load);
  const save = useRemote((state) => state.save);
  const forget = useRemote((state) => state.forget);

  const [endpoint, setEndpoint] = useState("");
  const [secret, setSecret] = useState("");
  const [repairing, setRepairing] = useState(false);
  // True only for a freshly generated, unsaved secret; a stored secret is never shown.
  const [shown, setShown] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  if (remote.state === "loading") {
    return <div className={styles["page"]}>{t("remote.checking")}</div>;
  }
  if (remote.state === "failed") {
    return <div className={styles["page"]}>{t("remote.unreachable")}</div>;
  }

  const view = remote.value;
  const pairing = !view.paired || repairing;
  // A first pairing needs both fields; once paired, either one alone is an edit.
  const enough = view.paired
    ? endpoint.trim() !== "" || secret.trim() !== ""
    : endpoint.trim() !== "" && secret.trim() !== "";
  const half = !view.paired && (endpoint.trim() !== "") !== (secret.trim() !== "");

  const clear = (): void => {
    setEndpoint("");
    setSecret("");
    setRepairing(false);
    setShown(false);
    setCopied(false);
  };

  return (
    <div className={styles["page"]} data-testid="remote-section">
      {view.paired && <Connection view={view} />}

      {pairing ? (
        <>
          {!view.paired && <p className={styles["hint"]}>{t("remote.intro")}</p>}

          <label className={styles["field"]}>
            <span className={styles["fieldLabel"]}>{t("remote.bridge")}</span>
            <input
              className={styles["input"]}
              type="url"
              inputMode="url"
              placeholder="https://"
              autoComplete="off"
              name="toglet-remote-endpoint"
              value={endpoint}
              onChange={(event) => {
                setEndpoint(event.target.value);
              }}
            />
          </label>

          {/* Not a wrapping <label>: buttons are labelable, so wrapping them would take the
              label away from the input. */}
          <div className={styles["field"]}>
            <div className={styles["fieldHead"]}>
              <label className={styles["fieldLabel"]} htmlFor={SECRET_FIELD}>
                {t("remote.secret")}
              </label>
              <span className={styles["fieldTools"]}>
                <button
                  type="button"
                  className={styles["tool"]}
                  onClick={() => {
                    setSecret(freshSecret());
                    setShown(true);
                    setCopied(false);
                  }}
                >
                  {t("remote.generate")}
                </button>
                {shown && secret !== "" && (
                  <button
                    type="button"
                    className={styles["tool"]}
                    onClick={() => {
                      void copy(secret).then(setCopied);
                    }}
                  >
                    {t(copied ? "remote.copied" : "remote.copy")}
                  </button>
                )}
              </span>
            </div>
            <input
              id={SECRET_FIELD}
              className={styles["input"]}
              // Readable only while it is a freshly generated secret; typed values stay masked.
              type={shown ? "text" : "password"}
              // Password managers ignore `off` but honour `new-password`. An empty field means
              // "keep what is stored", so an autofilled password would replace a working pairing.
              autoComplete="new-password"
              name="toglet-remote-secret"
              value={secret}
              onChange={(event) => {
                setSecret(event.target.value);
                setShown(false);
                setCopied(false);
              }}
            />
          </div>

          <p className={styles["hint"]}>{t("remote.keepDetails")}</p>
          {half && <p className={styles["alert"]}>{t("remote.bothOrNeither")}</p>}
          {failure !== null && <Failure code={failure.error?.code ?? null} />}

          <div className={styles["actions"]}>
            {view.paired && (
              <button type="button" className={styles["action"]} disabled={busy} onClick={clear}>
                {t("remote.cancel")}
              </button>
            )}
            <button
              type="button"
              className={styles["primary"]}
              disabled={busy || !enough}
              onClick={() => {
                void save({
                  enabled: true,
                  // An empty field means "keep what is stored".
                  ...(endpoint.trim() === "" ? {} : { endpoint: endpoint.trim() }),
                  ...(secret.trim() === "" ? {} : { secret: secret.trim() }),
                }).then((ok) => {
                  if (ok) clear();
                });
              }}
            >
              {t("remote.pair")}
            </button>
          </div>
        </>
      ) : (
        <>
          {failure !== null && <Failure code={failure.error?.code ?? null} />}
          <div className={styles["actions"]}>
            <button
              type="button"
              className={styles["action"]}
              disabled={busy}
              onClick={() => {
                // The address is prefilled for editing; the secret never comes back.
                setEndpoint(view.bridgeEndpoint ?? "");
                setRepairing(true);
              }}
            >
              {t("remote.repair")}
            </button>
            <button
              type="button"
              className={styles["action"]}
              disabled={busy}
              onClick={() => {
                clear();
                void forget();
              }}
            >
              {t("remote.forget")}
            </button>
            <button
              type="button"
              className={styles["primary"]}
              disabled={busy}
              onClick={() => {
                void save({ enabled: !view.enabled });
              }}
            >
              {t(view.enabled ? "remote.turnOff" : "remote.turnOn")}
            </button>
          </div>
        </>
      )}

      <p className={styles["limit"]}>{t("remote.limitAsleep")}</p>
    </div>
  );
}

/** The saved bridge. The dot is paired with an on/off word so state never relies on colour. */
function Connection({
  view,
}: {
  view: {
    enabled: boolean;
    bridgeHost: string;
    lastCommand: { at: number; action: string; result: string } | null;
  };
}): JSX.Element {
  const last = view.lastCommand;
  return (
    <div className={styles["connection"]}>
      <div className={styles["head"]}>
        <span className={cx(styles["dot"], view.enabled && styles["dotLive"])} />
        <span className={styles["host"]}>
          {view.bridgeHost === "" ? t("remote.unknown") : view.bridgeHost}
        </span>
        <span className={styles["state"]}>{t(view.enabled ? "remote.on" : "remote.off")}</span>
      </div>
      {last !== null && <p className={styles["meta"]}>{said(last)}</p>}
    </div>
  );
}

/**
 * 18 random bytes as base64url: 24 characters, within the alphabet and minimum length Rust
 * accepts.
 */
function freshSecret(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(18));
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * Copies a just-generated, unsaved secret so it can be entered on the phone. A saved secret has
 * no path out of Rust.
 */
async function copy(secret: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(secret);
    return true;
  } catch {
    // No clipboard permission; the field is readable, so it can still be copied by hand.
    return false;
  }
}

/** Unknown codes are shown verbatim. */
function said(last: { action: string; result: string }): string {
  const action = actionKey(last.action);
  const outcome = outcomeKey(last.result);
  return t("remote.lastCommand", {
    action: action === null ? last.action : t(action),
    outcome: outcome === null ? last.result : t(outcome),
  });
}

// A null code means the IPC call itself failed, so there is nothing more specific to show.
function Failure({ code }: { code: string | null }): JSX.Element {
  if (code === null) {
    return <p className={styles["alert"]}>{t("remote.reason.unreached")}</p>;
  }
  const key = failureKey(code);
  return <p className={styles["alert"]}>{key === null ? code : t(key)}</p>;
}
