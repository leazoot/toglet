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

    // The URL carries PKCE and the OAuth state. `start_login` returns nothing, and there is no
    // call here that could have received one.
    expect(invoke).toHaveBeenCalledWith("start_login", undefined);
    const results = invoke.mock.results.map((one) => JSON.stringify(one.value));
    expect(results.some((one) => one.includes("http"))).toBe(false);
  });

  it("sends no name: the account is named after itself", async () => {
    // The name it carries at ChatGPT, or the local part of its address, is Rust's to work out
    // from the credential the sign-in produced.
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
    // The user did sign in successfully. The browser reused a session, and the protocol offers
    // no way to ask for the account chooser.
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

    // The throwaway home and the app server behind it go with it.
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
      onBegin: () => undefined,
      onCancel: () => undefined,
      onDone: () => undefined,
      onSwitch: () => undefined,
      ...overrides,
    };
    return render(<AddAccountSheet {...props} />);
  }

  it("warns about the browser before it is opened, not after", () => {
    // Afterwards it is too late to act on: the session has already been reused.
    sheet();

    expect(screen.getByText(/that account is used without asking/)).toBeDefined();
  });

  it("asks for no name, and says where the name will come from", () => {
    // The design's step 1 had a name field; the account is named after itself instead, and the
    // sheet says so rather than leaving the user to wonder.
    sheet();

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText(/name it has at ChatGPT/)).toBeDefined();
    expect(screen.getByText("Open browser").hasAttribute("disabled")).toBe(false);
  });

  it("says nothing was changed while it waits", () => {
    sheet({ phase: "waiting" });

    expect(screen.getByText(/Nothing has been changed yet/)).toBeDefined();
  });

  it("looks alive while it waits for the browser", () => {
    // The wait can be long and the work is in another window. The row's switching
    // spinner sits by the heading, and the heading is announced as a busy status.
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
    // Adding is not switching. Saying otherwise would claim Codex changed when it did not.
    sheet({ phase: "added", account: ACCOUNT });

    expect(screen.getByText(/not in use yet/)).toBeDefined();
    expect(screen.queryByRole("button", { name: "Switch to it" })).toBeNull();
  });

  it("offers the switch when Codex is using none of the managed accounts", () => {
    // The account just added is the obvious next thing for Codex to use, and the first account
    // added after a sign-out used to end at "Close" with the bar drawing a plain well.
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
