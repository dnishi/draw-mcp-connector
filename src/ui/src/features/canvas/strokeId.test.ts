import { describe, expect, it } from 'vitest';
import { newStrokeId } from './strokeId';

describe('newStrokeId', () => {
  it('16 進の 12 文字で、呼ぶたびに変わる', () => {
    const ids = Array.from({ length: 100 }, newStrokeId);
    for (const id of ids) expect(id).toMatch(/^[0-9a-f]{12}$/);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
