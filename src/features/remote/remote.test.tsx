import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { RemoteView } from "../../types/ipc";
import { RemoteSection } from "./RemoteSection";
import { failureKey, outcomeKey } from "./reason";
import { useRemote } from "./store";

function view(overrides: Partial<RemoteView> = {}): RemoteView {
  return {
    enabled: false,
    shareExcerpt: false,
    paired: false,
    bridgeHost: "",
    bridgeEndpoint: null,
    statusKey: null,
    lastCommand: null,
    ...overrides,
  };
}

/** Finds an input by type; throws if it is missing. */
function box(container: HTMLElement, kind: "url" | "password" | "text"): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[type="${kind}"]`);
  if (found === null) {
    throw new Error(`the page has no ${kind} box`);
  }
  return found;
}

function answerWith(current: RemoteView, then?: RemoteView): void {
  invoke.mockImplementation((command: string) =>
    Promise.resolve(command === "read_remote" ? current : (then ?? current)),
  );
}

beforeEach(() => {
  invoke.mockReset();
  useRemote.setState({ remote: { state: "loading" }, busy: false, failure: null });
});

afterEach(cleanup);

async function repair(): Promise<void> {
  fireEvent.click(await screen.findByRole("button", { name: /re-pair/i }));
}

describe("the remote continuation page", () => {
  // The key is a one-way derivation of the secret and is meant to be pasted onto the bridge, so
  // showing it is the point. The secret itself must still have no way onto the screen.
  it("shows the status key masked, never the whole value", async () => {
    const key = "184b2be9f1ed19cd39a53a95715a7ee4dca55a817e48aa4c20ac6d99279845ea";
    answerWith(
      view({
        enabled: true,
        paired: true,
        bridgeHost: "bridge.example.com",
        statusKey: key,
      }),
    );
    const { container } = render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText("184b********45ea")).toBeDefined();
    // The value leaves only through the copy button; it must not sit on the screen.
    expect(container.textContent).not.toContain(key);
  });

  it("offers no status key before anything is paired", async () => {
    answerWith(view());
    render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.queryByText(/status key/i)).toBeNull();
  });

  it("keeps the one limit the phone cannot work around, and nothing else", async () => {
    answerWith(view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText(/asleep/i)).toBeDefined();
  });

  it("leads with the bridge it has, rather than with boxes to fill in", async () => {
    answerWith(view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText("bridge.example.com")).toBeDefined();
    expect(screen.getByText("On")).toBeDefined();
    expect(container.querySelectorAll("input").length).toBe(0);
  });

  it("says a stored bridge is off rather than showing it as if it were working", async () => {
    answerWith(view({ enabled: false, paired: true, bridgeHost: "bridge.example.com" }));
    render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText("bridge.example.com")).toBeDefined();
    expect(screen.getByText("Off")).toBeDefined();
    // Reachability is unknown until the next poll.
    expect(screen.queryByText(/connected/i)).toBeNull();
  });

  it("leaves both boxes empty even when a bridge is already paired", async () => {
    answerWith(view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    await repair();

    const boxes = container.querySelectorAll("input");
    expect(boxes.length).toBe(2);
    for (const box of boxes) {
      expect(box.value).toBe("");
    }
  });

  it("asks a password manager not to fill the secret, since empty means 'keep what is stored'", async () => {
    answerWith(view({ paired: true }));
    const { container } = render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    await repair();
    expect(box(container, "password").getAttribute("autocomplete")).toBe("new-password");
  });

  it("makes a secret long enough to be one, and shows it while it still has to be typed into a phone", async () => {
    answerWith(view());
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    expect(box(container, "password").value).toBe("");
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));

    const made = box(container, "text");
    expect(made.value.length).toBeGreaterThanOrEqual(16);
    // The alphabet Rust accepts: printable ASCII, no spaces.
    expect(made.value).toMatch(/^[A-Za-z0-9_-]+$/);
    const first = made.value;
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    expect(box(container, "text").value).not.toBe(first);
  });

  it("keeps a generated secret until it is dismissed, since it still has to reach the phone", async () => {
    answerWith(view(), view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    fireEvent.change(box(container, "url"), { target: { value: "https://bridge.example.com/x" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    const made = box(container, "text").value;

    fireEvent.click(screen.getByRole("button", { name: /^pair$/i }));

    // The fields go, but the generated value stays: losing it means generating another one, which
    // silently invalidates the status key already deployed on the bridge.
    await waitFor(() => {
      expect(container.querySelectorAll("input").length).toBe(0);
    });
    expect(screen.getByText(made)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: /^done$/i }));
    expect(screen.queryByText(made)).toBeNull();
  });

  it("does not keep a typed secret after saving, since the user already has it", async () => {
    answerWith(view(), view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    fireEvent.change(box(container, "url"), { target: { value: "https://bridge.example.com/x" } });
    fireEvent.change(box(container, "password"), { target: { value: "typed-by-hand-1234" } });
    fireEvent.click(screen.getByRole("button", { name: /^pair$/i }));

    await waitFor(() => {
      expect(container.querySelectorAll("input").length).toBe(0);
    });
    expect(screen.queryByText("typed-by-hand-1234")).toBeNull();
    expect(screen.queryByRole("button", { name: /^done$/i })).toBeNull();
  });

  it("never offers a stored secret, however the pairing was made", async () => {
    answerWith(view(), view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    fireEvent.change(box(container, "url"), { target: { value: "https://bridge.example.com/x" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    fireEvent.click(screen.getByRole("button", { name: /^pair$/i }));
    await waitFor(() => {
      expect(container.querySelectorAll("input").length).toBe(0);
    });
    fireEvent.click(screen.getByRole("button", { name: /^done$/i }));

    // Re-opening shows an empty, masked field, not the stored secret.
    fireEvent.click(screen.getByRole("button", { name: /re-pair/i }));
    expect(box(container, "password").value).toBe("");
    expect(container.querySelector('input[type="text"]')).toBeNull();
  });

  it("offers the stored address for editing, and never the stored secret", async () => {
    answerWith(
      view({
        enabled: true,
        paired: true,
        bridgeHost: "bridge.example.com",
        bridgeEndpoint: "https://bridge.example.com/a8c3343e0f6bd7d1ca0d777a/toglet",
      }),
    );
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");
    await repair();

    expect(box(container, "url").value).toBe(
      "https://bridge.example.com/a8c3343e0f6bd7d1ca0d777a/toglet",
    );
    expect(box(container, "password").value).toBe("");
  });

  it("sends only the half that was changed, so the other keeps what is stored", async () => {
    answerWith(
      view({
        enabled: true,
        paired: true,
        bridgeHost: "bridge.example.com",
        bridgeEndpoint: "https://bridge.example.com/old/toglet",
      }),
    );
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");
    await repair();

    fireEvent.change(box(container, "url"), {
      target: { value: "https://bridge.example.com/new/toglet" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^pair$/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_remote", {
        draft: { enabled: true, endpoint: "https://bridge.example.com/new/toglet" },
      });
    });
  });

  it("refuses a half-filled pairing rather than sending one usable half", async () => {
    answerWith(view());
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    fireEvent.change(box(container, "url"), {
      target: { value: "https://bridge.example.com/x" },
    });

    await waitFor(() => {
      expect(screen.getByText(/both boxes, or neither/i)).toBeDefined();
    });
    expect(invoke).not.toHaveBeenCalledWith("save_remote", expect.anything());
  });

  it("sends the address and the secret together, once, and then forgets them", async () => {
    answerWith(view(), view({ enabled: true, paired: true, bridgeHost: "bridge.example.com" }));
    const { container } = render(<RemoteSection />);
    await screen.findByTestId("remote-section");

    fireEvent.change(box(container, "url"), {
      target: { value: "https://bridge.example.com/x" },
    });
    fireEvent.change(box(container, "password"), {
      target: { value: "a-secret-long-enough" },
    });
    fireEvent.click(screen.getByRole("button", { name: /pair/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_remote", {
        draft: {
          enabled: true,
          endpoint: "https://bridge.example.com/x",
          secret: "a-secret-long-enough",
        },
      });
    });
    await waitFor(() => {
      expect(container.querySelectorAll("input").length).toBe(0);
    });
    expect(screen.getByText("bridge.example.com")).toBeDefined();
  });

  it("says what the last command did, in words rather than in a code", async () => {
    answerWith(
      view({
        enabled: true,
        paired: true,
        bridgeHost: "bridge.example.com",
        lastCommand: { at: 1_757_664_000, action: "resume", result: "remote_state_changed" },
      }),
    );
    render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText(/already moved on/i)).toBeDefined();
  });

  it("shows a code it has never seen as itself, rather than as 'unknown error'", async () => {
    answerWith(
      view({
        enabled: true,
        paired: true,
        bridgeHost: "bridge.example.com",
        lastCommand: { at: 1, action: "teleport", result: "remote_something_new" },
      }),
    );
    render(<RemoteSection />);

    await screen.findByTestId("remote-section");
    expect(screen.getByText(/remote_something_new/)).toBeDefined();
  });
});

describe("translating the codes", () => {
  it("has copy for every refusal the envelope can produce", () => {
    const refusals = [
      "remote_version",
      "remote_malformed",
      "remote_unknown_action",
      "remote_bad_mac",
      "remote_expired",
      "remote_replayed",
      "remote_nonce_reused",
      "remote_session_mismatch",
      "remote_state_changed",
      "remote_rate_limited",
      "applied",
    ];
    for (const code of refusals) {
      expect(outcomeKey(code), code).not.toBeNull();
    }
  });

  it("has copy for the bridge refusing, so nobody is sent to check their network instead", () => {
    expect(failureKey("remote_bridge_rejected")).not.toBeNull();
    expect(failureKey("network_unavailable")).not.toBeNull();
    expect(failureKey("credential_store_unavailable")).not.toBeNull();
  });
});
