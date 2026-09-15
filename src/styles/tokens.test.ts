import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { RING_FORM_GEOMETRY, RING_GEOMETRY } from "../components/geometry";
import {
  RING_CIRCUMFERENCE,
  RING_FORM_INNER_CIRCUMFERENCE,
  RING_FORM_OUTER_CIRCUMFERENCE,
} from "../features/quotas/format";

/**
 * Tokens are the single source of colour: a literal elsewhere ignores the theme. The scan has no
 * allow-list beyond tokens.css, so a new file cannot opt out.
 */

// Vitest serves modules over an http URL, so `import.meta.url` is not a path; cwd is the root.
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
 * Everything from `marker` up to the next top-level block. Anchored to a line start so a mention
 * of the selector inside a comment does not match.
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
    // A light-only name would be a token that keeps its dark value in the light theme.
    const source = readFileSync(TOKENS, "utf8");
    const lightNames = declaredIn(block(source, "@media (prefers-color-scheme: light)"));
    const rootNames = declaredIn(source.split("@media (prefers-color-scheme: light)")[0] ?? "");

    expect(lightNames.size).toBeGreaterThan(0);
    for (const name of lightNames) {
      expect(rootNames, `${name} is only defined for the light theme`).toContain(name);
    }
  });

  it("let an explicit theme override the system in both directions", () => {
    // The light values exist twice (media query and `[data-theme="light"]`); the copies must match,
    // and the `:not()` lets dark be forced while the system is light.
    const source = readFileSync(TOKENS, "utf8");
    const bySystem = declaredIn(block(source, "@media (prefers-color-scheme: light)"));
    const byChoice = declaredIn(block(source, ':root[data-theme="light"]'));

    expect(byChoice).toStrictEqual(bySystem);
    expect(source).toContain(':root:not([data-theme="dark"])');
  });

  it("let the setting add reduced motion without ever taking the system's away", () => {
    // Turning the setting off means "follow the system", so both blocks zero the same tokens.
    const source = readFileSync(TOKENS, "utf8");
    const bySystem = declaredIn(block(source, "@media (prefers-reduced-motion: reduce)"));
    const byChoice = declaredIn(block(source, ':root[data-motion="reduced"]'));

    expect(byChoice).toStrictEqual(bySystem);
  });

  it("agree with the ring geometry the SVG is drawn with", () => {
    // SVG attributes cannot read custom properties, so the geometry is duplicated in TypeScript.
    const source = readFileSync(TOKENS, "utf8");
    const value = (name: string): string =>
      new RegExp(`${name}:\\s*([^;]+);`).exec(source)?.[1]?.trim() ?? "";

    expect(value("--tg-ring-circumference")).toBe(String(RING_CIRCUMFERENCE));
    expect(value("--tg-ring-radius")).toBe(`${RING_GEOMETRY.radius.toString()}px`);
    expect(value("--tg-ring-stroke")).toBe(`${RING_GEOMETRY.stroke.toString()}px`);
    expect(value("--tg-ring-size")).toBe(`${RING_GEOMETRY.box.toString()}px`);
    // 2π × 16.5 = 103.67 to two places.
    expect((2 * Math.PI * RING_GEOMETRY.radius).toFixed(2)).toBe(RING_CIRCUMFERENCE.toFixed(2));
  });

  it("agree with the ring form's two rings the same way", () => {
    const source = readFileSync(TOKENS, "utf8");
    const value = (name: string): string =>
      new RegExp(`${name}:\\s*([^;]+);`).exec(source)?.[1]?.trim() ?? "";

    expect(value("--tg-ring-form-box")).toBe(`${RING_FORM_GEOMETRY.box.toString()}px`);
    expect(value("--tg-ring-form-outer-radius")).toBe(
      `${RING_FORM_GEOMETRY.outerRadius.toString()}px`,
    );
    expect(value("--tg-ring-form-inner-radius")).toBe(
      `${RING_FORM_GEOMETRY.innerRadius.toString()}px`,
    );
    expect(value("--tg-ring-form-outer-circumference")).toBe(String(RING_FORM_OUTER_CIRCUMFERENCE));
    expect(value("--tg-ring-form-inner-circumference")).toBe(String(RING_FORM_INNER_CIRCUMFERENCE));
    expect((2 * Math.PI * RING_FORM_GEOMETRY.outerRadius).toFixed(2)).toBe(
      RING_FORM_OUTER_CIRCUMFERENCE.toFixed(2),
    );
    expect((2 * Math.PI * RING_FORM_GEOMETRY.innerRadius).toFixed(2)).toBe(
      RING_FORM_INNER_CIRCUMFERENCE.toFixed(2),
    );
    // The two rings must not touch: a stroke's half-width either side of each radius.
    expect(RING_FORM_GEOMETRY.outerRadius - RING_FORM_GEOMETRY.innerRadius).toBeGreaterThan(
      RING_FORM_GEOMETRY.stroke,
    );
  });

  it("inset the surface by exactly the room Rust sizes the window with", () => {
    // Rust sizes the window with room for shadows; the stylesheet insets by the same numbers.
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
      ["--tg-ring-form-height", "RING_HEIGHT"],
    ] as const) {
      expect(token(tokenName), tokenName).not.toBe("");
      expect(token(tokenName), `${tokenName} vs ${constantName}`).toBe(constant(constantName));
    }
  });

  it("turn motion off rather than merely speeding it up when reduced motion is asked for", () => {
    // Reduced motion means instant state changes, not shorter animations.
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
