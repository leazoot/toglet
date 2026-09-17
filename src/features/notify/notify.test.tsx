import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import type { JSX } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { NotifyChannelView, NotifyView } from "../../types/ipc";
import { BARK_DEFAULT_SERVER, MAIL_PORTS, fill } from "./fields";
import { SettingsSheet } from "../settings/SettingsSheet";
import { NotifySection } from "./NotifySection";
import type { NotifyMode } from "./NotifySection";
import { useNotify } from "./store";

/** The page as the sheet hosts it: the sheet owns which view is showing. */
function Page({ start = "list" }: { start?: NotifyMode }): JSX.Element {
  const [mode, setMode] = useState<NotifyMode>(start);
  return <NotifySection mode={mode} onMode={setMode} />;
}

/** Opens a channel's row, which is a line until clicked. */
async function open(name: RegExp): Promise<void> {
  fireEvent.click(await screen.findByRole("button", { name, expanded: false }));
}

function channel(overrides: Partial<NotifyChannelView> = {}): NotifyChannelView {
  return {
    id: "chan-1",
    kind: "wecom",
    label: "Team",
    enabled: true,
    hint: "qyapi.weixin.qq.com",
    lastDelivery: null,
    ...overrides,
  };
}

function view(channels: readonly NotifyChannelView[]): NotifyView {
  return { channels, maxChannels: 8 };
}

/** Answers `read_notify_channels` with `list` and every other command with `then`. */
function answerWith(list: readonly NotifyChannelView[], then?: unknown): void {
  invoke.mockImplementation((command: string) =>
    Promise.resolve(command === "read_notify_channels" ? view(list) : (then ?? view(list))),
  );
}

describe("filling in a channel's form", () => {
  it("treats every box left empty as 'keep what is stored' rather than as an error", () => {
    expect(fill("wecom", {}, "tls").state).toBe("empty");
  });

  it("refuses a half-filled form rather than sending part of it", () => {
    expect(fill("telegram", { botToken: "1:A" }, "tls").state).toBe("incomplete");
  });

  it("leaves an optional address out entirely, so Rust fills in the service's own", () => {
    const filled = fill("bark", { deviceKey: "AbCd", server: "  " }, "tls");

    expect(filled.state).toBe("ready");
    if (filled.state !== "ready") return;
    expect(filled.connection).toEqual({ kind: "bark", deviceKey: "AbCd" });
    // The default is Rust's to apply; this side only shows it as the box's placeholder.
    expect(BARK_DEFAULT_SERVER).toBe("https://api.day.app");
  });

  it("will not send a port that is not a port", () => {
    const values = {
      host: "smtp.example.com",
      username: "leanne",
      password: "hunter2",
      from: "a@example.com",
      to: "b@example.com",
    };
    expect(fill("email", { ...values, port: "smtp" }, "tls").state).toBe("incomplete");
    expect(fill("email", { ...values, port: "0" }, "tls").state).toBe("incomplete");
    expect(fill("email", { ...values, port: MAIL_PORTS.tls }, "tls").state).toBe("ready");
  });

  it("keeps the spaces inside a mailbox password", () => {
    // App passwords come in groups of four separated by spaces. Trimming the inside of one
    // would make a correct password fail with nothing on screen to explain it.
    const filled = fill(
      "email",
      {
        host: "smtp.example.com",
        port: "587",
        username: "leanne",
        password: "abcd efgh ijkl mnop",
        from: "a@example.com",
        to: "b@example.com",
      },
      "startTls",
    );

    expect(filled.state).toBe("ready");
    if (filled.state !== "ready" || filled.connection.kind !== "email") return;
    expect(filled.connection.password).toBe("abcd efgh ijkl mnop");
    expect(filled.connection.security).toBe("startTls");
  });

  it("sends from the account itself when the sender is left blank", () => {
    const filled = fill(
      "email",
      {
        host: "smtp.example.com",
        port: "465",
        username: "leanne@example.com",
        password: "hunter2",
        from: "  ",
        to: "b@example.com",
      },
      "tls",
    );

    expect(filled.state).toBe("ready");
    if (filled.state !== "ready" || filled.connection.kind !== "email") return;
    expect(filled.connection.from).toBe("leanne@example.com");
  });
});

describe("the notification store", () => {
  beforeEach(() => {
    invoke.mockReset();
    useNotify.setState({
      channels: { state: "loading" },
      busy: false,
      testing: null,
      tested: {},
      failure: null,
    });
  });

  it("holds the list Rust answered with, never the one that was asked for", async () => {
    answerWith([channel()], view([]));

    await useNotify.getState().load();
    // Rust refuses the save, so the list it answers with is the list that still stands.
    invoke.mockRejectedValueOnce({ code: "internal", retryable: false });
    const saved = await useNotify.getState().save({ label: "Phone", enabled: true });

    expect(saved).toBe(false);
    const held = useNotify.getState().channels;
    expect(held.state).toBe("ready");
    if (held.state !== "ready") return;
    expect(held.value.channels).toHaveLength(1);
    expect(useNotify.getState().failure?.error?.code).toBe("internal");
  });

  it("forgets a verdict about details that have just been replaced", async () => {
    answerWith([channel()]);
    await useNotify.getState().load();
    useNotify.setState({ tested: { "chan-1": { channelId: "chan-1", ok: true, code: null } } });

    await useNotify.getState().save({
      id: "chan-1",
      label: "Team",
      enabled: true,
      connection: { kind: "wecom", webhook: "https://qyapi.weixin.qq.com/hook/new" },
    });

    expect(useNotify.getState().tested["chan-1"]).toBeUndefined();
  });

  it("keeps a verdict when only the name changed", async () => {
    answerWith([channel()]);
    await useNotify.getState().load();
    useNotify.setState({ tested: { "chan-1": { channelId: "chan-1", ok: true, code: null } } });

    await useNotify.getState().save({ id: "chan-1", label: "Group", enabled: true });

    expect(useNotify.getState().tested["chan-1"]?.ok).toBe(true);
  });

  it("reports what a test send did, and reads the list again afterwards", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "send_notification"
          ? [{ channelId: "chan-1", ok: false, code: "notification_rejected" }]
          : view([channel()]),
      ),
    );
    await useNotify.getState().load();

    await useNotify.getState().test("chan-1", "Toglet", "test");

    expect(useNotify.getState().testing).toBeNull();
    expect(useNotify.getState().tested["chan-1"]?.code).toBe("notification_rejected");
  });
});

describe("the notification group", () => {
  beforeEach(() => {
    invoke.mockReset();
    useNotify.setState({
      channels: { state: "loading" },
      busy: false,
      testing: null,
      tested: {},
      failure: null,
    });
  });

  afterEach(cleanup);

  it("names a channel by its host, never by the address that reaches it", async () => {
    answerWith([channel({ hint: "qyapi.weixin.qq.com" })]);
    render(<Page />);

    await screen.findByText(/qyapi\.weixin\.qq\.com/);
    // The webhook address is a credential. Nothing on this side has ever been given it.
    expect(screen.queryByText(/https:/)).toBeNull();
  });

  it("says a channel's state to assistive technology, not only through the knob's position", async () => {
    answerWith([channel({ enabled: false })]);
    render(<Page />);

    const toggle = await screen.findByRole("switch", { name: /Team/ });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });

  it("says a failed delivery's reason rather than only that it failed", async () => {
    answerWith([
      channel({
        lastDelivery: {
          at: Math.floor(Date.now() / 1000) - 120,
          ok: false,
          code: "network_unavailable",
        },
      }),
    ]);
    render(<Page />);

    // Closed, the row shows the failure only as a dot; the words are one click away.
    expect(screen.queryByText(/could not be reached/)).toBeNull();
    await open(/Team/);
    expect(await screen.findByText(/could not be reached/)).toBeTruthy();
  });

  it("refuses a half-filled form instead of storing part of it", async () => {
    answerWith([]);
    render(<Page />);

    fireEvent.click(await screen.findByText("Add a channel"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Phone" } });
    fireEvent.change(screen.getByLabelText("Device key"), { target: { value: "" } });
    fireEvent.click(screen.getByText("Save"));

    expect(screen.getByRole("alert").textContent).toContain("leave every one of them empty");
    expect(invoke).not.toHaveBeenCalledWith("save_notify_channel", expect.anything());
  });

  it("sends a new channel's details once, and the service's default is left to Rust", async () => {
    answerWith([]);
    render(<Page />);

    fireEvent.click(await screen.findByText("Add a channel"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Phone" } });
    fireEvent.change(screen.getByLabelText("Device key"), { target: { value: "AbCdEf" } });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_notify_channel", {
        request: {
          label: "Phone",
          enabled: true,
          connection: { kind: "bark", deviceKey: "AbCdEf" },
        },
      });
    });
  });

  it("lets a channel be renamed without its details being typed out again", async () => {
    answerWith([channel()]);
    render(<Page />);

    await open(/Team/);
    fireEvent.click(screen.getByText("Edit"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Group" } });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_notify_channel", {
        request: { id: "chan-1", label: "Group", enabled: true },
      });
    });
  });

  it("tells the reader what an empty box means before showing them the boxes", async () => {
    answerWith([channel()]);
    render(<Page />);

    await open(/Team/);
    fireEvent.click(screen.getByText("Edit"));
    const form = screen.getByTestId("notify-form");
    const said = form.textContent.indexOf("Leave the boxes below empty");
    const firstBox = form.textContent.indexOf("Webhook address");

    expect(said).toBeGreaterThanOrEqual(0);
    expect(firstBox).toBeGreaterThanOrEqual(0);
    expect(said).toBeLessThan(firstBox);
  });

  it("keeps a webview's password manager out of the boxes that mean 'keep what is stored'", async () => {
    // Password managers ignore `off` but honour `new-password`; an autofilled field would
    // silently replace a working channel's details.
    answerWith([]);
    render(<Page />);

    fireEvent.click(await screen.findByText("Add a channel"));
    fireEvent.click(screen.getByText("E-mail"));

    expect(screen.getByLabelText("Password").getAttribute("autocomplete")).toBe("new-password");
    // Everything that is not a secret is left as it was.
    expect(screen.getByLabelText("Port").getAttribute("autocomplete")).toBe("off");
  });

  it("shows the form instead of the list, never under it", async () => {
    answerWith([channel()]);
    render(<Page />);

    await screen.findByRole("switch", { name: /Team/ });
    fireEvent.click(screen.getByText("Add a channel"));

    // The list is gone while the form shows: the two never stack into one tall page.
    expect(screen.getByTestId("notify-form")).toBeTruthy();
    expect(screen.queryByRole("switch", { name: /Team/ })).toBeNull();

    fireEvent.click(screen.getByText("Cancel"));
    await screen.findByRole("switch", { name: /Team/ });
    expect(screen.queryByTestId("notify-form")).toBeNull();
  });

  it("puts the mail host and port on one line and everything else on its own", async () => {
    answerWith([]);
    render(<Page />);

    fireEvent.click(await screen.findByText("Add a channel"));
    fireEvent.click(screen.getByText("E-mail"));

    const half = (label: string): boolean =>
      screen.getByLabelText(label).closest("label")?.className.includes("half") === true;
    expect(half("Server")).toBe(true);
    expect(half("Port")).toBe(true);
    expect(half("User name")).toBe(false);
  });

  it("removes a channel only after a second press", async () => {
    answerWith([channel()]);
    render(<Page />);

    await open(/Team/);
    fireEvent.click(screen.getByText("Remove"));
    expect(invoke).not.toHaveBeenCalledWith("remove_notify_channel", expect.anything());

    fireEvent.click(screen.getByText("Remove it"));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("remove_notify_channel", { channelId: "chan-1" });
    });
  });
});

/** The channels are a sub-page of the settings sheet, so the sheet stays one page tall. */
describe("reaching the channels from the settings sheet", () => {
  beforeEach(() => {
    invoke.mockReset();
    useNotify.setState({
      channels: { state: "loading" },
      busy: false,
      testing: null,
      tested: {},
      failure: null,
    });
  });

  afterEach(cleanup);

  function sheet(): void {
    render(
      <SettingsSheet
        settings={{ state: "loading" }}
        saving={false}
        onChange={() => undefined}
        onClose={() => undefined}
        accounts={{ state: "ready", value: [] }}
        removal={null}
        onRemove={() => undefined}
        onDismissRemoval={() => undefined}
        onPinned={() => undefined}
      />,
    );
  }

  it("shows one row with the count, not the channels themselves", async () => {
    answerWith([channel(), channel({ id: "chan-2", label: "Phone" })]);
    sheet();

    await screen.findByText("2 channels");
    // The list itself is behind the row; none of it is measured into this page's height.
    expect(screen.queryByTestId("notify-section")).toBeNull();
    expect(screen.queryByText("Team")).toBeNull();
  });

  it("does not call an empty list 'none' before it has been read", () => {
    // The read is still in flight here. "None" would be a claim about something unknown.
    invoke.mockImplementation(() => new Promise(() => undefined));
    sheet();

    expect(screen.getByTestId("notify-row").textContent).toContain("reading");
    expect(screen.queryByText("None")).toBeNull();
  });

  it("opens the channels as a page of their own, and comes back", async () => {
    answerWith([channel()]);
    sheet();

    fireEvent.click(await screen.findByText("1 channel"));
    await screen.findByTestId("notify-section");
    // The settings themselves are gone while the channels are showing, so the sheet is as tall
    // as one page, never as tall as both.
    expect(screen.queryByTestId("settings-accounts")).toBeNull();
    expect(screen.queryByTestId("notify-row")).toBeNull();

    fireEvent.click(screen.getByText("Back"));
    await screen.findByTestId("notify-row");
    expect(screen.queryByTestId("notify-section")).toBeNull();
  });
});
