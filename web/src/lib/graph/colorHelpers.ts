/**
 * Pure color utility functions for graph node/edge rendering.
 *
 * These functions are called per-node per-frame inside Sigma reducers,
 * so they must be fast and allocation-minimal.
 *
 * Pattern: background-blend interpolation (GitNexus dimColor pattern).
 * NOT flat gray -- interpolates original color toward the dark background
 * to preserve hue hints while dimming.
 */

/** Dark background color used for dimming interpolation (#0d0b12 — matches --color-bg-void). */
const DARK_BG = { r: 13, g: 11, b: 18 };

/**
 * Parse a hex color string to RGB components.
 * Returns { r: 100, g: 100, b: 100 } for invalid input.
 */
export function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  return result
    ? {
        r: parseInt(result[1], 16),
        g: parseInt(result[2], 16),
        b: parseInt(result[3], 16),
      }
    : { r: 100, g: 100, b: 100 };
}

/**
 * Convert RGB components to a hex color string.
 * Each channel is clamped to [0, 255].
 */
export function rgbToHex(r: number, g: number, b: number): string {
  return (
    '#' +
    [r, g, b]
      .map((v) =>
        Math.round(Math.max(0, Math.min(255, v)))
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')
  );
}

/**
 * Interpolate a node color toward the dark background (#0d0b12),
 * preserving hue hints -- NOT flat gray.
 *
 * Formula: DARK_BG + (rgb - DARK_BG) * amount
 *
 * @param hex - Original node color as hex string.
 * @param amount - Blend amount. 1.0 = full original color; 0.25 = heavily dimmed.
 */
export function dimColor(hex: string, amount: number): string {
  const rgb = hexToRgb(hex);
  return rgbToHex(
    DARK_BG.r + (rgb.r - DARK_BG.r) * amount,
    DARK_BG.g + (rgb.g - DARK_BG.g) * amount,
    DARK_BG.b + (rgb.b - DARK_BG.b) * amount,
  );
}

/**
 * Increase a color's luminosity by blending toward white.
 *
 * Formula: rgb + (255 - rgb) * (factor - 1) / factor
 *
 * @param hex - Original color as hex string.
 * @param factor - Brightness factor. 1.5 brightens moderately; 2.0 brightens strongly.
 */
export function brightenColor(hex: string, factor: number): string {
  const rgb = hexToRgb(hex);
  return rgbToHex(
    rgb.r + ((255 - rgb.r) * (factor - 1)) / factor,
    rgb.g + ((255 - rgb.g) * (factor - 1)) / factor,
    rgb.b + ((255 - rgb.b) * (factor - 1)) / factor,
  );
}
