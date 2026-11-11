import { describe, expect, it } from 'vitest';
import { BLACK, isHexColor, strokeColor, toRgb } from './color';

describe('色', () => {
  it('"#rrggbb" の形だけを色として扱う', () => {
    expect(isHexColor('#ff3b30')).toBe(true);
    expect(isHexColor('#FF3B30')).toBe(true);
    for (const bad of ['ff3b30', '#fff', '#ff3b3g', 'red']) {
      expect(isHexColor(bad)).toBe(false);
    }
  });

  it('0〜1 の r, g, b にする。形が違えば黒', () => {
    expect(toRgb('#ff0080')).toEqual([1, 0, 128 / 255]);
    expect(toRgb('red')).toEqual([0, 0, 0]);
  });

  it('色を持たない筆跡は黒', () => {
    expect(strokeColor({})).toBe(BLACK);
    expect(strokeColor({ color: '#007aff' })).toBe('#007aff');
  });
});
