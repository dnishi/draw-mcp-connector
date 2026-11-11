import { describe, expect, it } from 'vitest';
import { eraseAlong } from './erase';
import { radiusAt, type Stroke } from './stroke';

const point = (x: number, y: number, pressure = 1) => ({ x, y, pressure });

/** A straight horizontal stroke (widely spaced points, like a line written by the AI) */
const line: Stroke = {
  id: 'line',
  author: 'ai',
  points: [point(0, 0), point(100, 0)],
};

function sequentialIds() {
  let n = 0;
  return () => `piece-${++n}`;
}

const xs = (stroke: Stroke) => stroke.points.map(({ x }) => x);

describe('eraseAlong', () => {
  it('触れていなければ null を返す', () => {
    expect(eraseAlong(line, point(50, 40), point(60, 40), 10)).toBeNull();
  });

  it('なぞった所で切れ、両側が別の筆跡として残る', () => {
    const radius = 10;
    // Crosses the middle of the line vertically
    const pieces = eraseAlong(
      line,
      point(50, -30),
      point(50, 30),
      radius,
      sequentialIds(),
    );

    expect(pieces).toHaveLength(2);
    const [left, right] = pieces!;
    expect(left.id).toBe('piece-1');
    expect(right.id).toBe('piece-2');
    expect(left.author).toBe('ai');

    // The cut is where the ink's edge meets the eraser's edge (within the 1px step)
    const edge = radius + radiusAt(1);
    expect(xs(left)[0]).toBe(0);
    expect(xs(left).at(-1)).toBeGreaterThanOrEqual(50 - edge - 1);
    expect(xs(left).at(-1)).toBeLessThan(50 - edge);
    expect(xs(right)[0]).toBeGreaterThan(50 + edge);
    expect(xs(right)[0]).toBeLessThanOrEqual(50 + edge + 1);
    expect(xs(right).at(-1)).toBe(100);
  });

  it('消し残りは、元の筆跡の色を引き継ぐ', () => {
    const red: Stroke = { ...line, color: '#ff3b30' };
    const pieces = eraseAlong(red, point(50, -30), point(50, 30), 10)!;
    expect(pieces.map(({ color }) => color)).toEqual(['#ff3b30', '#ff3b30']);
  });

  it('切り口を探すために足した点は、切り口のほかは残さない', () => {
    const [left] = eraseAlong(line, point(50, -30), point(50, 30), 10)!;
    expect(left.points).toHaveLength(2);
  });

  it('端を消すと、残りは1本になる', () => {
    const pieces = eraseAlong(line, point(100, -30), point(100, 30), 10);
    expect(pieces).toHaveLength(1);
    expect(xs(pieces![0])[0]).toBe(0);
  });

  it('全体を覆えば何も残らない', () => {
    expect(eraseAlong(line, point(0, 0), point(100, 0), 10)).toEqual([]);
  });

  it('止まったまま(移動なし)でも、消しゴムの円の中は消える', () => {
    const pieces = eraseAlong(line, point(50, 0), point(50, 0), 5);
    expect(pieces).toHaveLength(2);
  });

  it('1点だけの筆跡は、触れれば消える', () => {
    const dot: Stroke = { id: 'dot', author: 'human', points: [point(0, 0)] };
    expect(eraseAlong(dot, point(5, 0), point(5, 0), 5)).toEqual([]);
    expect(eraseAlong(dot, point(50, 0), point(50, 0), 5)).toBeNull();
  });
});
