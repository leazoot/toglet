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

/**
 * The reset banner's gauge: ø14, r=5.5, stroke 1.5. Circumference 2π × 5.5 = 34.56. The arc is
 * the share of the average reset interval that has passed - two figures the feed gives, drawn
 * together; it is not a countdown.
 */
export const RESET_GAUGE_GEOMETRY = { box: 14, radius: 5.5, stroke: 1.5 } as const;
