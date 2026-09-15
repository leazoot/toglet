/**
 * SVG geometry. `r`, `cx` and `viewBox` are attributes and cannot read CSS variables, so these
 * duplicate design tokens; src/styles/tokens.test.ts asserts the copies agree.
 */

/** The quota ring: ø38, r=16.5, stroke 3.5. */
export const RING_GEOMETRY = { box: 38, radius: 16.5, stroke: 3.5 } as const;

/**
 * The ring form: five-hour window on the outer ring (r=16.5), weekly inside it (r=11), stroke 3.
 * Circumferences: 2π × 16.5 = 103.67, 2π × 11 = 69.12.
 */
export const RING_FORM_GEOMETRY = {
  box: 40,
  outerRadius: RING_GEOMETRY.radius,
  innerRadius: 11,
  stroke: 3,
} as const;
