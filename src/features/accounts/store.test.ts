import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { useAccounts } from "./store";

describe("the account mirror", () => {
  beforeEach(() => {
    invoke.mockReset();
    useAccounts.setState({ accounts: { state: "loading" } });
  });

  it("takes the active account from Rust instead of deciding it here", async () => {
    invoke.mockResolvedValue([
      {
        id: "acct-1",
        displayName: "Work",
        maskedEmail: null,
        planType: null,
        status: "ready",
        isActive: false,
      },
      {
        id: "acct-2",
        displayName: "Personal",
        maskedEmail: null,
        planType: null,
        status: "active",
        isActive: true,
      },
    ]);

    await useAccounts.getState().load();
    const accounts = useAccounts.getState().accounts;

    expect(accounts.state).toBe("ready");
    if (accounts.state !== "ready") return;
    expect(accounts.value.filter((account) => account.isActive)).toHaveLength(1);
    expect(accounts.value[1]?.id).toBe("acct-2");
  });

  it("does not turn a read that failed into an empty list", async () => {
    // An empty list means "you have no accounts", which is a different thing to say than "the
    // list could not be read" - and only one of them is true here.
    invoke.mockRejectedValue(new Error("bridge unavailable"));

    await useAccounts.getState().load();

    expect(useAccounts.getState().accounts).toStrictEqual({
      state: "failed",
      failure: { command: "list_accounts" },
    });
  });

  it("reports an account list that really is empty as ready and empty", async () => {
    invoke.mockResolvedValue([]);

    await useAccounts.getState().load();

    expect(useAccounts.getState().accounts).toStrictEqual({ state: "ready", value: [] });
  });

  it("asks Rust to sign Codex out only for the account in use", async () => {
    // The explicit choice travels as a flag; a spare account never carries it, whatever the
    // sheet did.
    invoke.mockImplementation((command: string) =>
      command === "remove_account"
        ? Promise.resolve({
            removed: true,
            signedOut: true,
            credentialDeleted: true,
            rollback: null,
            error: null,
          })
        : Promise.resolve([]),
    );
    const inUse = {
      id: "acct-2",
      displayName: "Team",
      maskedEmail: null,
      planType: null,
      status: "active",
      isActive: true,
    } as const;

    const removed = await useAccounts.getState().remove(inUse, 1_800_000_000);

    expect(removed).toBe(true);
    expect(invoke).toHaveBeenCalledWith("remove_account", {
      accountId: "acct-2",
      signOut: true,
      now: 1_800_000_000,
    });
    expect(useAccounts.getState().removal).toBeNull();
  });

  it("keeps what happened to Codex's sign-in when a sign-out did not complete", async () => {
    // `Ok` with `removed: false` is Rust saying the account stayed and the rollback ran; the
    // rollback is what the user needs to hear, so it is kept rather than folded into "failed".
    invoke.mockImplementation((command: string) =>
      command === "remove_account"
        ? Promise.resolve({
            removed: false,
            signedOut: false,
            credentialDeleted: false,
            rollback: "restored",
            error: {
              code: "switch_verification_mismatch",
              phase: "verify",
              retryable: false,
              action: "restore_from_backup",
            },
          })
        : Promise.resolve([]),
    );
    const inUse = {
      id: "acct-2",
      displayName: "Team",
      maskedEmail: null,
      planType: null,
      status: "active",
      isActive: true,
    } as const;

    const removed = await useAccounts.getState().remove(inUse, 1_800_000_000);

    expect(removed).toBe(false);
    expect(useAccounts.getState().removal).toStrictEqual({
      phase: "failed",
      name: "Team",
      rollback: "restored",
    });
  });
});
