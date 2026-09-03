/**
 * Geometry that has to be a number rather than a CSS custom property.
 *
 * SVG's `r`, `cx` and `viewBox` are attributes, not styles, so they cannot read a variable. Each
 * value below is therefore a second copy of a design token, and src/styles/tokens.test.ts
 * asserts the two still agree - otherwise the arc would stop matching the circle it is drawn on.
 */

/** The quota ring: ø38, r=16.5, stroke 3.5. */
export const RING_GEOMETRY = { box: 38, radius: 16.5, stroke: 3.5 } as const;
