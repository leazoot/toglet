import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { QuotaView } from "../../types/ipc";
import { dueForRefresh, quotaOf, useQuota } from "./store";

const NOW = 1_800_000_000;

function reading(accountId = "acct-1"): QuotaView {
  return {
    accountId,
    windows: [{ kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null }],
    fetchedAt: NOW,
    source: "codex_app_server",
    stale: false,
    lastErrorCode: null,
  };
}

describe("the quota store", () => {
  beforeEach(() => {
    invoke.mockReset();
    useQuota.setState({ quotas: {}, refreshing: false });
  });

  it("holds each account's reading under its own id", async () => {
    invoke.mockImplementation((_command: string, args: { accountId: string }) =>
      Promise.resolve(reading(args.accountId)),
    );

    await useQuota.getState().load(["acct-1", "acct-2"], NOW);

    const { quotas } = useQuota.getState();
    expect(quotas["acct-1"]).toStrictEqual({ state: "ready", value: reading("acct-1") });
    expect(quotas["acct-2"]).toStrictEqual({ state: "ready", value: reading("acct-2") });
  });

  it("reads one account at a time", async () => {
    // Every reading starts a `codex app-server`, and Rust allows one at a time. Asking for all
    // of them at once would not make them arrive sooner.
    let running = 0;
    let peak = 0;
    invoke.mockImplementation(() => {
      running += 1;
      peak = Math.max(peak, running);
      return Promise.resolve(reading()).finally(() => {
        running -= 1;
      });
    });

    await useQuota.getState().load(["acct-1", "acct-2", "acct-3"], NOW);

    expect(peak).toBe(1);
  });

  it("keeps the previous values when a re-read fails, and stops calling them current", async () => {
    // An expired reading is marked, never cleared. Clearing would be indistinguishable from
    // "you have no quota".
    invoke.mockResolvedValueOnce(reading());
    await useQuota.getState().load(["acct-1"], NOW);

    invoke.mockRejectedValueOnce(new Error("app server did not answer"));
    await useQuota.getState().load(["acct-1"], NOW + 60);

    const held = useQuota.getState().quotas["acct-1"];
    expect(held?.state).toBe("ready");
    if (held?.state !== "ready") return;
    expect(held.value.windows).toStrictEqual(reading().windows);
    expect(held.value.stale).toBe(true);
    // The timestamp is untouched: it dates the reading, and no new reading was made.
    expect(held.value.fetchedAt).toBe(NOW);
  });

  it("reports a failure as a failure when there is nothing held to fall back on", async () => {
    invoke.mockRejectedValue(new Error("app server did not answer"));

    await useQuota.getState().load(["acct-1"], NOW);

    expect(useQuota.getState().quotas["acct-1"]).toStrictEqual({
      state: "failed",
      failure: { command: "refresh_quota" },
    });
  });

  it("leaves the current numbers up while the same account is re-read", async () => {
    invoke.mockResolvedValueOnce(reading());
    await useQuota.getState().load(["acct-1"], NOW);

    invoke.mockImplementation(() => new Promise(() => undefined));
    void useQuota.getState().load(["acct-1"], NOW + 60);

    expect(useQuota.getState().quotas["acct-1"]).toStrictEqual({
      state: "ready",
      value: reading(),
    });
  });

  it("marks itself refreshing only while a batch is running", async () => {
    invoke.mockResolvedValue(reading());

    const done = useQuota.getState().load(["acct-1"], NOW);
    expect(useQuota.getState().refreshing).toBe(true);

    await done;
    expect(useQuota.getState().refreshing).toBe(false);
  });

  it("treats an account never asked about as still loading, not as having no quota", () => {
    expect(quotaOf({}, "acct-1")).toStrictEqual({ state: "loading" });
    expect(quotaOf({}, null)).toStrictEqual({ state: "loading" });
  });
});

describe("what opening the panel re-reads", () => {
  // An account more than two minutes old joins the queue; a fresh one does not.
  it("leaves a reading younger than two minutes alone", () => {
    const quotas = { "acct-1": { state: "ready" as const, value: reading() } };

    expect(dueForRefresh(quotas, ["acct-1"], NOW + 119)).toEqual([]);
    expect(dueForRefresh(quotas, ["acct-1"], NOW + 121)).toEqual(["acct-1"]);
  });

  it("re-reads what is missing or failed, and not what is already on its way", () => {
    const quotas = {
      "acct-2": { state: "failed" as const, failure: { command: "refresh_quota" as const } },
      "acct-3": { state: "loading" as const },
    };

    expect(dueForRefresh(quotas, ["acct-1", "acct-2", "acct-3"], NOW)).toEqual([
      "acct-1",
      "acct-2",
    ]);
  });

  it("drops the reading of an account that was removed", () => {
    useQuota.setState({
      quotas: {
        "acct-1": { state: "ready", value: reading() },
        "acct-2": { state: "ready", value: reading("acct-2") },
      },
    });

    useQuota.getState().forget("acct-2");

    expect(Object.keys(useQuota.getState().quotas)).toEqual(["acct-1"]);
  });
});
