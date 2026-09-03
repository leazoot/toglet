import { afterEach, describe, expect, it } from "vitest";

import { durationToken } from "./motion";

const NAME = "--tg-test-duration";

afterEach(() => {
  document.documentElement.style.removeProperty(NAME);
});

function set(value: string): void {
  document.documentElement.style.setProperty(NAME, value);
}

describe("reading a duration token", () => {
  it("reads milliseconds", () => {
    set("120ms");

    expect(durationToken(NAME, 999)).toBe(120);
  });

  it("reads seconds", () => {
    set("1.6s");

    expect(durationToken(NAME, 999)).toBe(1600);
  });

  it("reads a token that reduced motion has zeroed", () => {
    // This is the reason the delays are read rather than written in TypeScript: the same media
    // query that turns the animations off turns the waiting off with them. A hard-coded 120
    // would keep the panel from opening for an eighth of a second after the setting is on.
    set("0ms");

    expect(durationToken(NAME, 120)).toBe(0);
  });

  it("falls back when the stylesheet is not there", () => {
    expect(durationToken("--tg-not-defined", 260)).toBe(260);
  });

  it("falls back rather than trusting something that is not a duration", () => {
    set("fast");

    expect(durationToken(NAME, 260)).toBe(260);
  });

  it("refuses a negative duration", () => {
    set("-120ms");

    expect(durationToken(NAME, 260)).toBe(260);
  });
});
