import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { RING_GEOMETRY } from "../components/geometry";
import { RING_CIRCUMFERENCE } from "../features/quotas/format";

/**
 * The design system holds only if the tokens are the single source of colour. A hex, or a
 * functional colour notation written straight into a component, is how a second undocumented
 * palette starts - and how the light theme quietly stops working, because a literal does not
 * change when the theme does.
 *
 * The scanner deliberately has nothing to exclude but the token file itself: no allow-list, so
 * a new file cannot opt out of it.
 *
 * So this scans the tree rather than trusting review.
 */

// Vitest serves modules over an http-scheme URL, so `import.meta.url` cannot be turned into a
// path here. The runner's working directory is the project root, which is what Vite resolves
// its own config against.
const SOURCE_ROOT = join(process.cwd(), "src");
const TOKENS = join(SOURCE_ROOT, "styles", "tokens.css");
const WINDOW_GEOMETRY = join(process.cwd(), "src-tauri", "src", "window", "geometry.rs");

const HEX = /#[0-9a-fA-F]{3,8}\b/g;
const FUNCTIONAL = /\b(?:rgba?|hsla?|color-mix|oklch)\s*\(/g;

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return sourceFiles(path);
    }
    return /\.(?:css|ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

/**
 * Everything from `marker` up to the next top-level block.
 *
 * Anchored to the start of a line: the file's own doc comment names these selectors, and a
 * search that matched there would return the whole stylesheet.
 */
function block(source: string, marker: string): string {
  const at = source.indexOf(`\n${marker}`);
  expect(at, `${marker} is not a top-level rule`).toBeGreaterThan(-1);
  const rest = source.slice(at + marker.length + 1);
  const next = rest.search(/\n@media|\n:root\[/);
  return next === -1 ? rest : rest.slice(0, next);
}

function declaredIn(source: string): Set<string> {
  return new Set([...source.matchAll(/(--tg-[a-z0-9-]+):/g)].map((match) => match[1] ?? ""));
}

describe("design tokens", () => {
  it("are the only place a colour is written down", () => {
    const offenders = sourceFiles(SOURCE_ROOT)
      .filter((path) => path !== TOKENS)
      .flatMap((path) => {
        const source = readFileSync(path, "utf8");
        const found = [...source.matchAll(HEX), ...source.matchAll(FUNCTIONAL)];
        return found.map((match) => `${path}: ${match[0]}`);
      });

    expect(offenders).toStrictEqual([]);
  });

  it("define both themes with the same names", () => {
    // The light theme swaps values, never structure. A name that exists in one
    // block and not the other is a token that silently keeps its dark value in daylight.
    const source = readFileSync(TOKENS, "utf8");
    const lightNames = declaredIn(block(source, "@media (prefers-color-scheme: light)"));
    const rootNames = declaredIn(source.split("@media (prefers-color-scheme: light)")[0] ?? "");

    expect(lightNames.size).toBeGreaterThan(0);
    for (const name of lightNames) {
      expect(rootNames, `${name} is only defined for the light theme`).toContain(name);
    }
  });

  it("let an explicit theme override the system in both directions", () => {
    // The setting has to win over the media query, which means the light values exist twice: once
    // behind `prefers-color-scheme` and once behind `[data-theme="light"]`. Two copies drift, so
    // this asserts they declare the same names - and that dark can be forced while the system is
    // light, which is what the `:not()` in the media query is for.
    const source = readFileSync(TOKENS, "utf8");
    const bySystem = declaredIn(block(source, "@media (prefers-color-scheme: light)"));
    const byChoice = declaredIn(block(source, ':root[data-theme="light"]'));

    expect(byChoice).toStrictEqual(bySystem);
    expect(source).toContain(':root:not([data-theme="dark"])');
  });

  it("let the setting add reduced motion without ever taking the system's away", () => {
    // The setting is a toggle, not a three-way choice: it can only add. Turning it off means
    // "follow the system", so the two blocks have to zero exactly the same tokens.
    const source = readFileSync(TOKENS, "utf8");
    const bySystem = declaredIn(block(source, "@media (prefers-reduced-motion: reduce)"));
    const byChoice = declaredIn(block(source, ':root[data-motion="reduced"]'));

    expect(byChoice).toStrictEqual(bySystem);
  });

  it("agree with the ring geometry the SVG is drawn with", () => {
    // The ring's radius and circumference exist twice over: as CSS variables that size the
    // container and dash its track, and as numbers in TypeScript, because `r` and
    // `stroke-dasharray` lengths are SVG attributes and cannot read a custom property. The two
    // copies have to stay equal or the arc stops matching the circle it is drawn on.
    const source = readFileSync(TOKENS, "utf8");
    const value = (name: string): string =>
      new RegExp(`${name}:\\s*([^;]+);`).exec(source)?.[1]?.trim() ?? "";

    expect(value("--tg-ring-circumference")).toBe(String(RING_CIRCUMFERENCE));
    expect(value("--tg-ring-radius")).toBe(`${RING_GEOMETRY.radius.toString()}px`);
    expect(value("--tg-ring-stroke")).toBe(`${RING_GEOMETRY.stroke.toString()}px`);
    expect(value("--tg-ring-size")).toBe(`${RING_GEOMETRY.box.toString()}px`);
    // 2π × 16.5 = 103.67 to two places, which is where the design's number comes from.
    expect((2 * Math.PI * RING_GEOMETRY.radius).toFixed(2)).toBe(RING_CIRCUMFERENCE.toFixed(2));
  });

  it("inset the surface by exactly the room Rust sizes the window with", () => {
    // The window is taller than the surface by the room its shadows need (geometry.rs), and the
    // stylesheet insets the surface by the same two numbers so the bar lands centred. Two copies
    // of each number, in two languages; if they drift the bar sits off-centre by the difference.
    const css = readFileSync(TOKENS, "utf8");
    const rust = readFileSync(WINDOW_GEOMETRY, "utf8");
    const token = (name: string): string =>
      new RegExp(`${name}:\\s*(\\d+)px;`).exec(css)?.[1] ?? "";
    const constant = (name: string): string =>
      new RegExp(`pub const ${name}: f64 = (\\d+)\\.0;`).exec(rust)?.[1] ?? "";

    for (const [tokenName, constantName] of [
      ["--tg-window-room-above", "ROOM_ABOVE"],
      ["--tg-window-room-below", "ROOM_BELOW"],
      ["--tg-bar-hit-buffer", "HIT_BUFFER"],
      ["--tg-bar-width", "BAR_WIDTH"],
      ["--tg-bar-height", "BAR_HEIGHT"],
    ] as const) {
      expect(token(tokenName), tokenName).not.toBe("");
      expect(token(tokenName), `${tokenName} vs ${constantName}`).toBe(constant(constantName));
    }
  });

  it("turn motion off rather than merely speeding it up when reduced motion is asked for", () => {
    // Reduced motion becomes an instant state change. A shortened animation is still an
    // animation, and the setting exists for people who cannot tolerate one.
    const source = readFileSync(TOKENS, "utf8");
    for (const marker of [
      "@media (prefers-reduced-motion: reduce)",
      ':root[data-motion="reduced"]',
    ]) {
      const reduced = block(source, marker);
      expect(reduced, marker).not.toBe("");
      for (const [, value] of reduced.matchAll(/--tg-duration-[a-z-]+:\s*([^;]+);/g)) {
        expect(value?.trim(), marker).toBe("0ms");
      }
    }
  });
});
