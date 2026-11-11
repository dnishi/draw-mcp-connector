import type { Stroke } from './stroke';

/** Color of strokes with no color specified, and of strokes from before colors existed (same as BLACK in the bridge's src/color.rs) */
export const BLACK = '#000000';

/** Pen color choices. Apart from black, Apple system colors (the same family as the connected green) */
export const PEN_COLORS = [
  { value: BLACK, label: 'Black' },
  { value: '#ff3b30', label: 'Red' },
  { value: '#007aff', label: 'Blue' },
  { value: '#34c759', label: 'Green' },
] as const;

/** Whether it is in "#rrggbb" form */
export function isHexColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value);
}

/** Color of a stroke. Strokes without a color (from before colors existed) are black */
export function strokeColor(stroke: Pick<Stroke, 'color'>): string {
  return stroke.color ?? BLACK;
}

/** Converts "#rrggbb" to [r, g, b] in 0 to 1. Black if the format is wrong */
export function toRgb(hex: string): [number, number, number] {
  if (!isHexColor(hex)) return [0, 0, 0];
  const channel = (i: number) => parseInt(hex.slice(i, i + 2), 16) / 255;
  return [channel(1), channel(3), channel(5)];
}
