import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { AccountView } from "../../types/ipc";
import { AddAccountSheet } from "./AddAccountSheet";
import type { AddAccountSheetProps } from "./AddAccountSheet";
import { useAdding } from "./store";

const NOW = 1_800_000_000;

const ACCOUNT: AccountView = {
  id: "acct-2",
  displayName: "Personal",
  maskedEmail: "ope***@gmail.com",
  planType: "pro",
  status: "ready",
  isActive: false,
};

describe("the add-account flow", () => {
  beforeEach(() => {
    invoke.mockReset();
    useAdding.setState({ phase: "idle", account: null, failure: null });
  });

  it("does nothing at all until the browser step has been opened", async () => {
    await useAdding.getState().begin(NOW);

    expect(invoke).not.toHaveBeenCalled();
    expect(useAdding.getState().phase).toBe("idle");
  });

  it("opens the browser through Rust and never handles the URL itself", async () => {
    invoke.mockResolvedValue(null);
    useAdding.getState().open();

    invoke.mockImplementation((command: string) =>
      command === "start_login"
        ? Promise.resolve(null)
        : Promise.resolve({ account: ACCOUNT, added: true }),
    );
    await useAdding.getState().begin(NOW);

    // The URL carries PKCE and the OAuth state, so `start_login` must return nothing.
    expect(invoke).toHaveBeenCalledWith("start_login", undefined);
    const results = invoke.mock.results.map((one) => JSON.stringify(one.value));
    expect(results.some((one) => one.includes("http"))).toBe(false);
  });

  it("sends no name: the account is named after itself", async () => {
    // Rust derives the account name from the credential the sign-in produced.
    invoke.mockImplementation((command: string) =>
      command === "start_login"
        ? Promise.resolve(null)
        : Promise.resolve({ account: ACCOUNT, added: true }),
    );
    useAdding.getState().open();

    await useAdding.getState().begin(NOW);

    expect(invoke).toHaveBeenCalledWith("finish_login", { displayName: null, now: NOW });
    expect(useAdding.getState().phase).toBe("added");
    expect(useAdding.getState().account?.displayName).toBe("Personal");
  });

  it("calls a sign-in that produced an account already held a duplicate, not a failure", async () => {
    // The sign-in succeeded but the browser reused a session; the protocol cannot force the
    // account chooser.
    invoke.mockImplementation((command: string) =>
      command === "start_login"
        ? Promise.resolve(null)
        : Promise.resolve({ account: ACCOUNT, added: false }),
    );
    useAdding.getState().open();

    await useAdding.getState().begin(NOW);

    expect(useAdding.getState().phase).toBe("duplicate");
    expect(useAdding.getState().account?.displayName).toBe("Personal");
  });

  it("reports a sign-in that did not complete as a failure", async () => {
    invoke.mockImplementation((command: string) =>
      command === "start_login" ? Promise.resolve(null) : Promise.reject(new Error("timed out")),
    );
    useAdding.getState().open();

    await useAdding.getState().begin(NOW);

    expect(useAdding.getState().phase).toBe("failed");
  });

  it("does not wait for a browser it never managed to open", async () => {
    invoke.mockImplementation((command: string) =>
      command === "start_login" ? Promise.reject(new Error("no browser")) : Promise.resolve(null),
    );
    useAdding.getState().open();

    await useAdding.getState().begin(NOW);

    expect(useAdding.getState().phase).toBe("failed");
    expect(invoke).not.toHaveBeenCalledWith("finish_login", expect.anything());
  });

  it("tears the sign-in down when it is abandoned", () => {
    invoke.mockResolvedValue(null);
    useAdding.setState({ phase: "waiting", account: null, failure: null });

    useAdding.getState().cancel();

    // Cancelling tears down the temporary home and its app server.
    expect(invoke).toHaveBeenCalledWith("cancel_login", undefined);
    expect(useAdding.getState().phase).toBe("idle");
  });

  it("does not ask Rust to cancel a sign-in that never started", () => {
    useAdding.getState().open();

    useAdding.getState().cancel();

    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("the add-account sheet", () => {
  afterEach(cleanup);

  function sheet(overrides: Partial<AddAccountSheetProps> = {}) {
    const props: AddAccountSheetProps = {
      phase: "confirming",
      account: null,
      noCurrentAccount: false,
      failure: null,
      onBegin: () => undefined,
      onCancel: () => undefined,
      onDone: () => undefined,
      onSwitch: () => undefined,
      ...overrides,
    };
    return render(<AddAccountSheet {...props} />);
  }

  it("says which step failed rather than one sentence for every cause", () => {
    sheet({
      phase: "failed",
      failure: {
        command: "start_login",
        error: { code: "runtime_not_installed", retryable: false },
      },
    });

    expect(screen.getByText(/找不到 Codex|could not be found/i)).toBeTruthy();
  });

  it("names a cause it has no sentence for, rather than admitting nothing", () => {
    sheet({
      phase: "failed",
      failure: { command: "finish_login", error: { code: "something_new", retryable: true } },
    });

    expect(screen.getByText(/something_new/)).toBeTruthy();
  });

  it("still says the accounts were left alone, whatever the cause", () => {
    // A failure must also say whether existing accounts are still safe.
    sheet({
      phase: "failed",
      failure: { command: "finish_login", error: { code: "login_canceled", retryable: false } },
    });

    expect(screen.getByText(/没有添加任何账户|Nothing was added/)).toBeTruthy();
  });

  it("warns about the browser before it is opened, not after", () => {
    sheet();

    expect(screen.getByText(/that account is used/)).toBeDefined();
  });

  it("asks for no name, and says where the name will come from", () => {
    // No name field: the account is named after itself, and the sheet says so.
    sheet();

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText(/ChatGPT name/)).toBeDefined();
    expect(screen.getByText("Open browser").hasAttribute("disabled")).toBe(false);
  });

  it("says nothing was changed while it waits", () => {
    sheet({ phase: "waiting" });

    expect(screen.getByText(/Nothing has been changed yet/)).toBeDefined();
  });

  it("looks alive while it waits for the browser", () => {
    // The heading carries the spinner and is announced as a busy status.
    sheet({ phase: "waiting" });

    expect(screen.getByTestId("spinner")).toBeDefined();
    const status = screen.getByRole("status");
    expect(status.getAttribute("aria-busy")).toBe("true");
    expect(status.textContent).toMatch(/Waiting for the browser/);
  });

  it("does not spin once the browser has answered", () => {
    sheet({ phase: "added", account: ACCOUNT });

    expect(screen.queryByTestId("spinner")).toBeNull();
  });

  it("explains a duplicate as what happened rather than as an error", () => {
    sheet({ phase: "duplicate", account: ACCOUNT });

    expect(screen.getByText(/reused a ChatGPT session/)).toBeDefined();
    expect(screen.getByText(/Nothing was added and Codex's sign-in was not touched/)).toBeDefined();
  });

  it("says a new account is not in use yet", () => {
    // Adding is not switching; Codex has not changed accounts.
    sheet({ phase: "added", account: ACCOUNT });

    expect(screen.getByText(/not in use yet/)).toBeDefined();
    expect(screen.queryByRole("button", { name: "Switch to it" })).toBeNull();
  });

  it("offers the switch when Codex is using none of the managed accounts", () => {
    // The offer hands over to the switch flow; nothing is switched here.
    const onSwitch = vi.fn();
    sheet({ phase: "added", account: ACCOUNT, noCurrentAccount: true, onSwitch });

    expect(screen.getByText(/using none of your accounts right now/)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Switch to it" }));

    expect(onSwitch).toHaveBeenCalledWith(ACCOUNT);
    expect(screen.getByRole("button", { name: "Close" })).toBeDefined();
  });

  it("says the account Codex uses is untouched when the sign-in failed", () => {
    sheet({ phase: "failed" });

    expect(screen.getByText(/has not been changed/)).toBeDefined();
  });

  it("shows nothing while idle", () => {
    sheet({ phase: "idle" });

    expect(screen.queryByTestId("add-sheet")).toBeNull();
  });
});
