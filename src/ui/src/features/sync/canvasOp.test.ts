import { describe, expect, it } from 'vitest';
import { applyCanvasOp } from './canvasOp';
import type { Stroke } from './protocol';

const stroke = (id: string, x = 0): Stroke => ({
  id,
  author: 'human',
  points: [{ x, y: 0, pressure: 1 }],
});
const ids = (strokes: readonly Stroke[]) => strokes.map(({ id }) => id);

// Checks the same rules as the bridge's tests (src/bridge/src/canvas.rs)
describe('applyCanvasOp', () => {
  it('insert は範囲外の位置なら末尾に置き、同じ ID は無視する', () => {
    const one = applyCanvasOp([], {
      type: 'insert',
      stroke: stroke('a'),
      index: 0,
    });
    const two = applyCanvasOp(one, {
      type: 'insert',
      stroke: stroke('b'),
      index: 99,
    });
    expect(ids(two)).toEqual(['a', 'b']);
    expect(
      applyCanvasOp(two, { type: 'insert', stroke: stroke('a'), index: 0 }),
    ).toBe(two);
  });

  it('remove は無い ID を無視し、何も消えなければ同じ配列を返す', () => {
    const strokes = [stroke('a'), stroke('b')];
    expect(
      ids(applyCanvasOp(strokes, { type: 'remove', ids: ['a', 'x'] })),
    ).toEqual(['b']);
    expect(applyCanvasOp(strokes, { type: 'remove', ids: ['x'] })).toBe(
      strokes,
    );
  });

  it('replace は重なり順を保って置き換える', () => {
    const strokes = [stroke('a'), stroke('b')];
    const replaced = applyCanvasOp(strokes, {
      type: 'replace',
      stroke: stroke('a', 5),
    });
    expect(ids(replaced)).toEqual(['a', 'b']);
    expect(replaced[0].points[0].x).toBe(5);
  });
});
