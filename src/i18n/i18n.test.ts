import { afterEach, describe, expect, it } from "vitest";

import { en } from "./en";
import type { MessageKey } from "./en";
import { activeLanguage, resolveLanguage, setLanguage, t, translate } from "./index";
import { zh } from "./zh";

afterEach(() => {
  setLanguage("en");
});

describe("the copy dictionary", () => {
  it("has no blank entry", () => {
    // A blank entry renders as nothing at all, which reads to the user as a missing state
    // rather than a missing translation.
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), key).not.toBe("");
    }
  });

  it("resolves a key to its entry", () => {
    expect(t("app.name")).toBe("Toglet");
  });

  it("says what is still safe when a read fails", () => {
    // Every error the user sees has to answer whether their current account is still usable.
    // For a read that failed, the answer is that nothing moved.
    expect(t("bar.notice.unreadable")).toContain("Nothing was changed");
    expect(t("bar.notice.environment")).toContain("Nothing was changed");
  });

  it("fills a slot with the value it is given", () => {
    expect(t("quota.resets", { when: "51m" })).toBe("Resets in 51m.");
  });

  it("leaves a slot with no value visible rather than blanking it", () => {
    // A sentence reading "Resets in ." looks like real copy and hides the bug; one that still
    // shows `{when}` does not.
    expect(t("quota.resets", {})).toContain("{when}");
  });

  it("leaves every slot filled in the copy the bar actually renders", () => {
    const filled = [
      t("quota.remaining", { window: "5-hour quota", percent: "68%" }),
      t("quota.resets", { when: "51m" }),
      t("quota.reading", { window: "5-hour quota" }),
      t("quota.notReturned", { window: "Weekly quota" }),
      t("quota.unreadable", { window: "Weekly quota" }),
    ];

    for (const sentence of filled) {
      expect(sentence, sentence).not.toMatch(/[{}]/);
    }
  });
});

/**
 * The entries that read the same in both languages on purpose.
 *
 * `5H` and `W` are the ring marks the design fixes for a 60-pixel bar. The two language names are
 * endonyms, as the design draws them: somebody who reads only Chinese still has to be able to
 * find 中文 in a sheet currently labelled in English.
 */
const SAME_IN_BOTH: readonly MessageKey[] = [
  "app.name",
  "bar.fiveHour",
  "bar.weekly",
  "settings.languageEnglish",
  "settings.languageChinese",
];

describe("the Chinese dictionary", () => {
  it("covers every key, with nothing left over", () => {
    // The types already say so. Asserted at run time as well because the check that matters is
    // that the file is complete, and a `satisfies` clause is easy to widen in a hurry.
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
  });

  it("has no blank entry", () => {
    for (const [key, value] of Object.entries(zh)) {
      expect(value.trim(), key).not.toBe("");
    }
  });

  it("actually translates everything that is not deliberately shared", () => {
    // The failure this catches is a key copied across during translation and never revisited.
    // It reads as finished work and shows English to somebody who asked for Chinese.
    for (const key of Object.keys(en) as MessageKey[]) {
      if (SAME_IN_BOTH.includes(key)) {
        continue;
      }
      expect(zh[key], key).not.toBe(en[key]);
    }
  });

  it("keeps every slot a sentence was carrying", () => {
    // A translation that drops `{when}` still reads as ordinary copy while silently losing the
    // number it existed to carry - the quietest way to fail this whole feature.
    for (const key of Object.keys(en) as MessageKey[]) {
      expect(slots(zh[key]), key).toEqual(slots(en[key]));
    }
  });

  it("says what is still safe when a read fails, in Chinese too", () => {
    // Every failure line has to answer whether the account Codex uses is still the expected one.
    // A translation that loses that sentence loses the point of it.
    expect(zh["bar.notice.unreadable"]).toContain("什么都没有被改动");
    expect(zh["bar.notice.environment"]).toContain("什么都没有被改动");
    expect(zh["switch.failedUntouched"]).toContain("仍在原来的账户");
  });
});

function slots(message: string): string[] {
  return [...message.matchAll(/\{(\w+)\}/g)].map((found) => found[1] ?? "").sort();
}

describe("choosing a language", () => {
  it("follows the operating system until somebody chooses", () => {
    // jsdom reports en-US, which is what the assertions in every other suite rely on.
    expect(resolveLanguage("system")).toBe("en");
  });

  it("takes an explicit choice over the operating system", () => {
    expect(resolveLanguage("zh")).toBe("zh");
    expect(resolveLanguage("en")).toBe("en");
  });

  it("resolves every regional Chinese tag onto the one dictionary there is", () => {
    for (const tag of ["zh", "zh-CN", "zh-Hans", "zh-TW", "ZH-hans-CN"]) {
      Object.defineProperty(navigator, "language", { value: tag, configurable: true });
      expect(resolveLanguage("system"), tag).toBe("zh");
    }
    Object.defineProperty(navigator, "language", { value: "en-US", configurable: true });
  });

  it("answers in whichever language is in force, without a restart", () => {
    expect(t("settings.title")).toBe("Settings");

    setLanguage("zh");

    expect(activeLanguage()).toBe("zh");
    expect(t("settings.title")).toBe("设置");
    expect(t("switch.confirmTitle", { name: "Team" })).toBe("切换到 Team？");
  });

  it("looks a message up in a named language without disturbing the one in force", () => {
    // What the tray menu is built from: copy for somewhere other than the screen being drawn.
    expect(translate("zh", "tray.quit")).toBe("退出 Toglet");
    expect(activeLanguage()).toBe("en");
  });
});
