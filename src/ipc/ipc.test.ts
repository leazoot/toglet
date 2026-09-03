import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { detectEnvironment, listAccounts, refreshQuota, startupRecovery } from "./index";

describe("the IPC boundary", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("passes a value through unchanged", async () => {
    invoke.mockResolvedValue([{ id: "acct-1", displayName: "Work", isActive: true }]);

    const result = await listAccounts();

    expect(result.ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("list_accounts", undefined);
  });

  it("names its arguments the way the Rust side declares them", async () => {
    // A misspelt argument name is not a type error on either side; it arrives as a missing
    // parameter and the command fails at run time.
    invoke.mockResolvedValue({ accountId: "acct-1", windows: [] });

    await refreshQuota("acct-1", 1_800_000_000);

    expect(invoke).toHaveBeenCalledWith("refresh_quota", {
      accountId: "acct-1",
      now: 1_800_000_000,
    });
  });

  it("reports a command that returned an error as a failure with nothing else attached", async () => {
    // `refresh_quota` can return `Err`, and its detail is redacted but still a message. The
    // boundary keeps the command name and nothing more.
    invoke.mockRejectedValue({ code: "app_server_crashed", detail: "C:/Users/somebody/.codex" });

    const result = await refreshQuota("acct-1", 0);

    expect(result).toStrictEqual({ ok: false, failure: { command: "refresh_quota" } });
  });

  it("does not carry the rejection's text, which can hold a path", async () => {
    // An operating-system message is the usual rejection, and those name files. Anything the
    // frontend keeps ends up in its console and its crash reports.
    invoke.mockRejectedValue(new Error("failed to open C:/Users/somebody/.codex/auth.json"));

    const result = await detectEnvironment();

    expect(result.ok).toBe(false);
    const serialised = JSON.stringify(result);
    expect(serialised).not.toContain("Users");
    expect(serialised).not.toContain(".codex");
    expect(serialised).toContain("detect_environment_command");
  });

  it("reports a failed read as failed rather than as an absent value", async () => {
    // `startup_recovery` answers `null` when there was nothing to recover. A call that never
    // arrived must not be mistaken for that answer.
    invoke.mockRejectedValue(new Error("bridge unavailable"));

    const result = await startupRecovery();

    expect(result).toStrictEqual({ ok: false, failure: { command: "startup_recovery" } });
  });

  it("keeps a genuine null distinct from a failure", async () => {
    invoke.mockResolvedValue(null);

    const result = await startupRecovery();

    expect(result).toStrictEqual({ ok: true, value: null });
  });
});
