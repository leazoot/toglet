import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { t } from "../../i18n";
import type { NotifyView, ResetStatusView, ResetsView } from "../../types/ipc";
import { useNotify } from "../notify/store";
import { ago, bannerLine } from "./banner";
import { resetNotification } from "./notifications";
import { ResetBanner } from "./ResetBanner";
import { ResetsRow } from "./ResetsRow";
import { ResetsSection } from "./ResetsSection";
import { useResets } from "./store";

const NOW = 1_800_000_000;
const DAY = 86_400;

function status(overrides: Partial<ResetStatusView> = {}): ResetStatusView {
  return {
    latestReset: {
      id: "r1",
      kind: "regular",
      announcedAt: NOW - 5 * DAY,
      text: "Reset all propagated. Sweet dreams. https://t.co/x",
    },
    scheduledReset: null,
    activeWatch: null,
    stats: { total: 53, lastResetAt: NOW - 5 * DAY, daysSinceLast: 4.9, avgIntervalDays: 6.9 },
    generatedAt: NOW - 60,
    ...overrides,
  };
}

function view(overrides: Partial<ResetsView> = {}): ResetsView {
  return {
    enabled: true,
    channelIds: [],
    status: status(),
    fetchedAt: NOW - 60,
    stale: false,
    lastError: null,
    ...overrides,
  };
}

function channels(): NotifyView {
  return {
    maxChannels: 8,
    channels: [
      {
        id: "chan-a",
        kind: "bark",
        label: "Phone",
        enabled: true,
        hint: "api.day.app",
        lastDelivery: null,
      },
      {
        id: "chan-b",
        kind: "email",
        label: "Mail",
        enabled: false,
        hint: "l***@example.com",
        lastDelivery: null,
      },
    ],
  };
}

beforeEach(() => {
  invoke.mockReset();
  useResets.setState({ resets: { state: "loading" }, busy: false, failure: null });
  useNotify.setState({ channels: { state: "loading" } });
});

afterEach(cleanup);

describe("the banner's sentence", () => {
  it("says how long since the last reset and how often they come", () => {
    const line = bannerLine(view(), NOW, t);
    expect(line.text).toBe("Last reset 5d ago · every 6.9 days on average");
    expect(line.tone).toBe("mute");
    expect(line.fresh).toBe(false);
    expect(line.gauge).toBeCloseTo(4.9 / 6.9);
    expect(line.gaugeLabel).toBe("4.9 days since the last reset; 6.9 days apart on average");
    // The feed's own words go to the tooltip and nowhere else.
    expect(line.detail).toContain("Sweet dreams");
    expect(line.text).not.toContain("Sweet dreams");
  });

  it("leads with a reset that happened today", () => {
    const fresh = view({
      status: status({
        latestReset: { id: "r2", kind: "regular", announcedAt: NOW - 2 * 3600, text: null },
      }),
    });
    const line = bannerLine(fresh, NOW, t);
    expect(line.text).toBe("Codex usage was reset · 2h ago");
    expect(line.tone).toBe("ok");
    expect(line.fresh).toBe(true);
  });

  it("names a banked reset as one", () => {
    const banked = view({
      status: status({
        latestReset: { id: "r2", kind: "banked", announcedAt: NOW - 30, text: null },
      }),
    });
    expect(bannerLine(banked, NOW, t).text).toBe("A banked reset was granted · just now");
  });

  it("shows an announced reset with its time, or says the time is not set", () => {
    const timed = view({
      status: status({
        scheduledReset: {
          id: "s1",
          kind: "regular",
          announcedAt: NOW - 3600,
          scheduledFor: NOW + 3600,
          text: "Tomorrow.",
        },
      }),
    });
    const line = bannerLine(timed, NOW, t);
    expect(line.text.startsWith("Reset announced · ")).toBe(true);
    expect(line.text).not.toContain("time not set");
    expect(line.tone).toBe("warn");

    const untimed = view({
      status: status({
        scheduledReset: {
          id: "s1",
          kind: "regular",
          announcedAt: NOW - 3600,
          scheduledFor: null,
          text: null,
        },
      }),
    });
    expect(bannerLine(untimed, NOW, t).text).toBe("Reset announced · time not set");
  });

  it("labels a forecast as the AI's and shows an unknown chance as a dash, never 0%", () => {
    const watch = (chancePercent: number | null, expiresAt: number): ResetsView =>
      view({
        status: status({
          activeWatch: {
            level: "strong",
            chancePercent,
            forecastWindow: "next 48h",
            observedAt: NOW - 600,
            expiresAt,
            text: null,
          },
        }),
      });
    expect(bannerLine(watch(70, NOW + DAY), NOW, t).text).toBe("AI forecast: strong, 70% chance");
    expect(bannerLine(watch(null, NOW + DAY), NOW, t).text).toBe("AI forecast: strong, chance —");
    // An expired forecast is not shown; the ordinary sentence returns.
    expect(bannerLine(watch(70, NOW - 1), NOW, t).text).toContain("Last reset");
  });

  it("says it is reading, or why it could not, before there is any reading", () => {
    expect(bannerLine(view({ status: null, fetchedAt: null }), NOW, t)).toMatchObject({
      text: "Reading reset status…",
      tone: "mute",
      gauge: null,
      gaugeLabel: "Reset interval not known",
    });
    expect(
      bannerLine(view({ status: null, fetchedAt: null, lastError: "network_unavailable" }), NOW, t)
        .text,
    ).toBe("Cannot reach codex-resets.com");
    expect(
      bannerLine(
        view({ status: null, fetchedAt: null, lastError: "reset_feed_unreadable" }),
        NOW,
        t,
      ).text,
    ).toBe("The reply from codex-resets.com could not be read");
  });

  it("keeps the old reading and says how old it is when the feed stops answering", () => {
    const line = bannerLine(
      view({ stale: true, fetchedAt: NOW - 20 * 60, lastError: "network_unavailable" }),
      NOW,
      t,
    );
    expect(line.text).toContain("Last reset");
    expect(line.asOf).toBe("as of 20m ago");
  });

  it("draws no gauge when either figure is unknown", () => {
    const unknown = view({
      status: status({
        stats: { total: 53, lastResetAt: null, daysSinceLast: null, avgIntervalDays: 6.9 },
      }),
    });
    const line = bannerLine(unknown, NOW, t);
    expect(line.gauge).toBeNull();
    expect(line.gaugeLabel).toBe("Reset interval not known");
    expect(line.text).toBe("Last reset 5d ago · every 6.9 days on average");
  });

  it("floors elapsed time so nothing is claimed early", () => {
    expect(ago(NOW - 59, NOW, t)).toBe("just now");
    expect(ago(NOW - 61, NOW, t)).toBe("1m ago");
    expect(ago(NOW - 3 * 3600 - 59 * 60, NOW, t)).toBe("3h ago");
    expect(ago(NOW - 2 * DAY - 1, NOW, t)).toBe("2d ago");
  });
});

describe("the notification for a reset event", () => {
  it("is a dictionary sentence with no feed text in it", () => {
    expect(resetNotification({ kind: "reset", resetType: "regular", at: NOW }, NOW, t)).toEqual({
      title: "Toglet",
      body: "Codex announced a usage reset for all paid users.",
    });
    expect(resetNotification({ kind: "reset", resetType: "banked", at: NOW }, NOW, t).body).toBe(
      "Codex granted a banked reset, to use when you need it.",
    );
    expect(
      resetNotification(
        { kind: "scheduled", resetType: "regular", at: NOW, scheduledFor: null },
        NOW,
        t,
      ).body,
    ).toBe("Codex announced a reset; the time is not set yet.");
    expect(
      resetNotification({ kind: "watch", level: "elevated", chancePercent: 40, at: NOW }, NOW, t)
        .body,
    ).toBe("AI forecast: a Codex reset may be coming (elevated, 40% chance).");
    expect(
      resetNotification({ kind: "watch", level: "strong", chancePercent: null, at: NOW }, NOW, t)
        .body,
    ).toBe("AI forecast: a Codex reset may be coming (strong).");
  });
});

describe("the reset banner", () => {
  it("shows the sentence and the gauge's reading, and nothing that lengthens the line", () => {
    render(<ResetBanner view={view()} nowSeconds={NOW} />);
    expect(screen.getByRole("status").textContent).toBe(
      "Last reset 5d ago · every 6.9 days on average",
    );
    expect(screen.getByTestId("reset-gauge").getAttribute("aria-label")).toBe(
      "4.9 days since the last reset; 6.9 days apart on average",
    );
    // The credit lives on the settings page now; the banner holds no button at all.
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("the reset-alerts page", () => {
  function answer(current: ResetsView, saved?: ResetsView): void {
    invoke.mockImplementation((command: string) => {
      switch (command) {
        case "read_resets":
          return Promise.resolve(current);
        case "save_resets":
          return Promise.resolve(saved ?? current);
        case "read_notify_channels":
          return Promise.resolve(channels());
        case "open_resets_site":
          return Promise.resolve(null);
        default:
          return Promise.reject(new Error(`unexpected ${command}`));
      }
    });
  }

  it("turns the feature on with the channels already chosen kept", async () => {
    answer(view({ enabled: false, channelIds: ["chan-a"] }), view({ channelIds: ["chan-a"] }));
    render(<ResetsSection />);
    await screen.findByTestId("resets-section");

    fireEvent.click(screen.getByRole("switch", { name: "Show reset status and alert me" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_resets", {
        draft: { enabled: true, channelIds: ["chan-a"] },
      });
    });
  });

  it("offers every channel as its own switch and saves the picked ids", async () => {
    answer(view(), view({ channelIds: ["chan-b"] }));
    render(<ResetsSection />);
    await screen.findByRole("switch", { name: "Mail" });

    fireEvent.click(screen.getByRole("switch", { name: "Mail" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_resets", {
        draft: { enabled: true, channelIds: ["chan-b"] },
      });
    });
    expect(screen.getByRole("switch", { name: "Mail" }).getAttribute("aria-checked")).toBe("true");
  });

  it("says where to add a channel when there is none", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "read_notify_channels" ? { channels: [], maxChannels: 8 } : view(),
      ),
    );
    render(<ResetsSection />);
    expect(
      await screen.findByText("No channels yet. Add one under Notifications, then pick it here."),
    ).toBeDefined();
  });

  it("hides the channel list while the feature is off", async () => {
    answer(view({ enabled: false }));
    render(<ResetsSection />);
    await screen.findByTestId("resets-section");
    expect(screen.queryByRole("switch", { name: "Phone" })).toBeNull();
  });

  it("opens the feed's site from the credit under the switch, even while off", async () => {
    answer(view({ enabled: false }));
    render(<ResetsSection />);
    await screen.findByTestId("resets-section");
    fireEvent.click(screen.getByRole("button", { name: "Data from Codex Resets" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("open_resets_site", undefined);
    });
  });
});

describe("the reset-alerts row", () => {
  it("counts the chosen channels once the state is known, and never says Off before", async () => {
    let resolve: (value: ResetsView) => void = () => undefined;
    invoke.mockImplementation(
      () =>
        new Promise<ResetsView>((done) => {
          resolve = done;
        }),
    );
    render(<ResetsRow onOpen={() => undefined} />);
    expect(screen.getByRole("button").textContent).toBe("reading…");

    resolve(view({ channelIds: ["chan-a", "chan-b"] }));
    await screen.findByText("On · 2 channels");
  });
});
