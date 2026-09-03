import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import type { AccountView, SwitchView } from "../../types/ipc";
import { useSwitching } from "./store";

const NOW = 1_800_000_000;

const TARGET: AccountView = {
  id: "acct-2",
  displayName: "Personal",
  maskedEmail: "ope***@gmail.com",
  planType: "pro",
  status: "ready",
  isActive: false,
};

function view(overrides: Partial<SwitchView> = {}): SwitchView {
  return {
    switched: true,
    progress: 4,
    clientUpToDate: true,
    clients: "clear",
    rollback: null,
    error: null,
    manualRecoveryRequired: false,
    clientOutcome: "nothing_was_running",
    ...overrides,
  };
}

/** Captures the step handler so a test can deliver events the way Rust would. */
let emit: (step: number) => void;

describe("the switch flow", () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockImplementation((_event: string, handler: (e: { payload: number }) => void) => {
      emit = (step) => {
        handler({ payload: step });
      };
      return Promise.resolve(() => undefined);
    });
    useSwitching.setState({
      phase: "idle",
      target: null,
      verdict: null,
      step: 0,
      result: null,
      failure: null,
      detailsOpen: false,
    });
  });

  it("asks about running clients before offering to switch", async () => {
    invoke.mockResolvedValue("clear");

    await useSwitching.getState().begin(TARGET);

    expect(invoke).toHaveBeenCalledWith("inspect_clients", undefined);
    expect(useSwitching.getState().phase).toBe("confirm");
  });

  it("refuses rather than confirming when a Codex session is still running", async () => {
    invoke.mockResolvedValue("blocked");

    await useSwitching.getState().begin(TARGET);

    expect(useSwitching.getState().phase).toBe("blocked");
    expect(invoke).not.toHaveBeenCalledWith("switch_account", expect.anything());
  });

  it("treats an unknown verdict as blocking, not as clear", async () => {
    // Not knowing what is running is not the same as knowing nothing is.
    invoke.mockResolvedValue("unknown");

    await useSwitching.getState().begin(TARGET);

    expect(useSwitching.getState().phase).toBe("blocked");
  });

  it("lets a desktop client through, because the switch closes and reopens it", async () => {
    invoke.mockResolvedValue("desktop_only");

    await useSwitching.getState().begin(TARGET);

    expect(useSwitching.getState().phase).toBe("confirm");
  });

  it("does not offer a switch when the probe itself did not answer", async () => {
    invoke.mockRejectedValue(new Error("bridge unavailable"));

    await useSwitching.getState().begin(TARGET);

    expect(useSwitching.getState().phase).toBe("failed");
    expect(useSwitching.getState().failure).toStrictEqual({ command: "inspect_clients" });
  });

  it("moves the step count only when Rust says a step finished", async () => {
    invoke.mockImplementation((command: string) =>
      command === "inspect_clients" ? Promise.resolve("clear") : new Promise(() => undefined),
    );
    await useSwitching.getState().begin(TARGET);

    const running = useSwitching.getState().confirm(NOW);
    await Promise.resolve();
    expect(useSwitching.getState().step).toBe(0);

    emit(1);
    expect(useSwitching.getState().step).toBe(1);
    emit(3);
    expect(useSwitching.getState().step).toBe(3);
    void running;
  });

  it("calls a switch that did not switch a failure, however it resolved", async () => {
    // The command resolves with a failure view too. "The call came back" is not success.
    invoke.mockImplementation((command: string) =>
      command === "inspect_clients"
        ? Promise.resolve("clear")
        : Promise.resolve(view({ switched: false, progress: 2, rollback: "restored" })),
    );
    await useSwitching.getState().begin(TARGET);

    await useSwitching.getState().confirm(NOW);

    expect(useSwitching.getState().phase).toBe("failed");
    expect(useSwitching.getState().result?.rollback).toBe("restored");
  });

  it("reports a switch that really happened as done", async () => {
    invoke.mockImplementation((command: string) =>
      command === "inspect_clients" ? Promise.resolve("clear") : Promise.resolve(view()),
    );
    await useSwitching.getState().begin(TARGET);

    await useSwitching.getState().confirm(NOW);

    expect(useSwitching.getState().phase).toBe("done");
    expect(useSwitching.getState().step).toBe(4);
  });

  it("takes the final step count from the outcome, not from the events it happened to see", async () => {
    invoke.mockImplementation((command: string) =>
      command === "inspect_clients"
        ? Promise.resolve("clear")
        : Promise.resolve(view({ switched: false, progress: 2 })),
    );
    await useSwitching.getState().begin(TARGET);

    await useSwitching.getState().confirm(NOW);

    expect(useSwitching.getState().step).toBe(2);
  });

  it("cannot be dismissed while the credentials are being replaced", async () => {
    invoke.mockImplementation((command: string) =>
      command === "inspect_clients" ? Promise.resolve("clear") : new Promise(() => undefined),
    );
    await useSwitching.getState().begin(TARGET);
    void useSwitching.getState().confirm(NOW);
    await Promise.resolve();

    useSwitching.getState().cancel();

    expect(useSwitching.getState().phase).toBe("running");
  });

  it("can be dismissed before it starts", async () => {
    invoke.mockResolvedValue("clear");
    await useSwitching.getState().begin(TARGET);

    useSwitching.getState().cancel();

    expect(useSwitching.getState().phase).toBe("idle");
    expect(useSwitching.getState().target).toBeNull();
  });
});
