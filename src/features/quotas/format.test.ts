import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { setLanguage } from "../../i18n";
import type { QuotaView, QuotaWindowView } from "../../types/ipc";
import {
  RING_CIRCUMFERENCE,
  STALE_AFTER_SECONDS,
  compactReset,
  isStale,
  percentLabel,
  ringDash,
  tone,
  windowValue,
} from "./format";

const NOW = 1_800_000_000;

function window(overrides: Partial<QuotaWindowView> = {}): QuotaWindowView {
  return {
    kind: "five_hour",
    usedPercent: 32,
    remainingPercent: 68,
    resetsAt: null,
    ...overrides,
  };
}

function view(windows: readonly QuotaWindowView[], overrides: Partial<QuotaView> = {}): QuotaView {
  return {
    accountId: "acct-1",
    windows,
    fetchedAt: NOW,
    source: "codex_app_server",
    stale: false,
    lastErrorCode: null,
    ...overrides,
  };
}

describe("reading a window out of a quota", () => {
  it("reports a window the server did not return as not returned", () => {
    // The weekly window is routinely absent, and the absence is the answer.
    const value = windowValue(view([window()]), "weekly");

    expect(value).toStrictEqual({ kind: "not_returned" });
  });

  it("never turns a missing window into zero", () => {
    const value = windowValue(view([]), "five_hour");

    expect(value.kind).not.toBe("value");
    expect(percentLabel(value)).toBe("—");
    expect(percentLabel(value)).not.toBe("0%");
  });

  it("keeps a genuine zero as zero", () => {
    // The opposite mistake: an account really out of quota must not read as "unknown".
    const value = windowValue(
      view([window({ usedPercent: 100, remainingPercent: 0 })]),
      "five_hour",
    );

    expect(value).toStrictEqual({ kind: "value", remainingPercent: 0, resetsAt: null });
    expect(percentLabel(value)).toBe("0%");
    expect(tone(value)).toBe("empty");
  });

  it("treats a window whose number is not a number as unreadable", () => {
    const value = windowValue(view([window({ remainingPercent: Number.NaN })]), "five_hour");

    expect(value).toStrictEqual({ kind: "unreadable" });
  });

  it("carries the reset time through untouched", () => {
    const value = windowValue(view([window({ resetsAt: NOW + 3600 })]), "five_hour");

    expect(value).toStrictEqual({ kind: "value", remainingPercent: 68, resetsAt: NOW + 3600 });
  });
});

describe("staleness", () => {
  it("trusts a reading inside the freshness window", () => {
    expect(isStale(view([window()]), NOW + STALE_AFTER_SECONDS)).toBe(false);
  });

  it("marks a reading that has aged past the window even though Rust called it fresh", () => {
    // The flag was true when the answer was made. The clock has moved since.
    expect(isStale(view([window()]), NOW + STALE_AFTER_SECONDS + 1)).toBe(true);
  });

  it("keeps a reading Rust already marked stale marked", () => {
    expect(isStale(view([window()], { stale: true }), NOW)).toBe(true);
  });
});

describe("the percentage label", () => {
  it("rounds to whole percent", () => {
    expect(percentLabel({ kind: "value", remainingPercent: 67.6, resetsAt: null })).toBe("68%");
  });

  it("shows an em dash when the reading failed", () => {
    expect(percentLabel({ kind: "unreadable" })).toBe("—");
  });
});

describe("the ring arc", () => {
  it("matches the formula the design gives", () => {
    // (pct / 100 × 103.67).toFixed(1) + ' 103.67'.
    expect(ringDash({ kind: "value", remainingPercent: 68, resetsAt: null })).toBe("70.5 103.67");
  });

  it("draws the full circle at a hundred percent", () => {
    expect(ringDash({ kind: "value", remainingPercent: 100, resetsAt: null })).toBe(
      `${RING_CIRCUMFERENCE.toFixed(1)} ${RING_CIRCUMFERENCE.toFixed(2)}`,
    );
  });

  it("draws no arc at all when there is no reading", () => {
    // The dashed track underneath is what says "unknown"; the arc simply is not there.
    expect(ringDash({ kind: "not_returned" })).toBe(`0 ${RING_CIRCUMFERENCE.toFixed(2)}`);
    expect(ringDash({ kind: "unreadable" })).toBe(`0 ${RING_CIRCUMFERENCE.toFixed(2)}`);
  });

  it("never draws an arc longer than the circle", () => {
    expect(ringDash({ kind: "value", remainingPercent: 140, resetsAt: null })).toBe(
      `${RING_CIRCUMFERENCE.toFixed(1)} ${RING_CIRCUMFERENCE.toFixed(2)}`,
    );
  });
});

describe("the status band", () => {
  it.each([
    [100, "healthy"],
    [50, "healthy"],
    [49, "warn"],
    [20, "warn"],
    [19, "low"],
    [1, "low"],
    [0, "empty"],
  ])("reads %i%% as %s", (remaining, expected) => {
    expect(tone({ kind: "value", remainingPercent: remaining, resetsAt: null })).toBe(expected);
  });

  it("agrees with the label when a fraction rounds down to zero", () => {
    // The ring and the number must not disagree: 0.4% shows as "0%", so it is exhausted.
    const value = { kind: "value", remainingPercent: 0.4, resetsAt: null } as const;

    expect(percentLabel(value)).toBe("0%");
    expect(tone(value)).toBe("empty");
  });

  it("has its own band for a reading that failed", () => {
    expect(tone({ kind: "unreadable" })).toBe("unreadable");
    expect(tone({ kind: "not_returned" })).toBe("unreadable");
  });
});

describe("the compact reset time", () => {
  it("counts minutes under an hour", () => {
    expect(compactReset(NOW + 51 * 60, NOW)).toBe("51m");
  });

  it("counts hours and minutes under a day", () => {
    expect(compactReset(NOW + (2 * 60 + 14) * 60, NOW)).toBe("2h 14m");
  });

  it("names the weekday and local time within the week", () => {
    const at = new Date(NOW * 1000 + 3 * 24 * 3600 * 1000);

    const label = compactReset(Math.floor(at.getTime() / 1000), NOW);

    expect(label).toMatch(/^(Sun|Mon|Tue|Wed|Thu|Fri|Sat) \d{2}:\d{2}$/);
    expect(label.slice(0, 3)).toBe(["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][at.getDay()]);
  });

  it("falls back to a date beyond a week", () => {
    const at = new Date(NOW * 1000 + 30 * 24 * 3600 * 1000);

    expect(compactReset(Math.floor(at.getTime() / 1000), NOW)).toMatch(/^[A-Z][a-z]{2} \d{1,2}$/);
  });

  it("never counts down past zero", () => {
    // A reset that has already happened means the reading predates it. The countdown does not
    // go negative; the staleness marker is what says the reading is old.
    expect(compactReset(NOW - 3600, NOW)).toBe("0m");
  });
});

describe("the compact reset time in Chinese", () => {
  // Exactly the forms the design names.
  beforeEach(() => {
    setLanguage("zh");
  });
  afterEach(() => {
    setLanguage("en");
  });

  it("counts minutes under an hour", () => {
    expect(compactReset(NOW + 51 * 60, NOW)).toBe("51分");
  });

  it("counts hours and minutes under a day", () => {
    expect(compactReset(NOW + (2 * 60 + 14) * 60, NOW)).toBe("2小时14分");
  });

  it("names the weekday and local time within the week", () => {
    const at = new Date(NOW * 1000 + 3 * 24 * 3600 * 1000);

    const label = compactReset(Math.floor(at.getTime() / 1000), NOW);

    expect(label).toMatch(/^周[日一二三四五六] \d{2}:\d{2}$/);
    expect(label.slice(0, 2)).toBe(
      ["周日", "周一", "周二", "周三", "周四", "周五", "周六"][at.getDay()],
    );
  });

  it("falls back to a date beyond a week", () => {
    const at = new Date(NOW * 1000 + 30 * 24 * 3600 * 1000);

    const label = compactReset(Math.floor(at.getTime() / 1000), NOW);

    expect(label).toBe(`${(at.getMonth() + 1).toString()}月${at.getDate().toString()}日`);
  });

  it("still never counts down past zero", () => {
    // The reason `App` decides "just now" on the seconds rather than on this string: it reads
    // `0分` here, and a comparison against `0m` would have quietly failed for half the users.
    expect(compactReset(NOW - 3600, NOW)).toBe("0分");
  });
});
