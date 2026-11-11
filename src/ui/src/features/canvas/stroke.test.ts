import { describe, expect, it } from 'vitest';
import { hitsStroke, radiusAt, smoothPoints, type Stroke } from './stroke';

const point = (x: number, y: number, pressure = 1) => ({ x, y, pressure });

describe('radiusAt', () => {
  it('筆圧が大きいほど太くなる', () => {
    expect(radiusAt(1)).toBeGreaterThan(radiusAt(0.5));
  });

  it('筆圧 0 でも幅は 0 にならない', () => {
    expect(radiusAt(0)).toBeGreaterThan(0);
  });
});

describe('smoothPoints', () => {
  it('始点と終点を保ち、間の点を増やす', () => {
    const points = [point(0, 0), point(10, 0), point(10, 10)];

    const smoothed = smoothPoints(points);

    expect(smoothed[0]).toEqual(points[0]);
    expect(smoothed.at(-1)).toEqual(points[2]);
    expect(smoothed).toHaveLength(6);
  });

  it('2点以下はそのまま返す', () => {
    const points = [point(0, 0), point(10, 0)];
    expect(smoothPoints(points)).toEqual(points);
  });
});

describe('hitsStroke', () => {
  const stroke: Stroke = {
    id: 'a',
    author: 'human',
    points: [point(0, 0), point(100, 0)],
  };

  it('線分の近くは当たる', () => {
    expect(hitsStroke(stroke, point(50, 10), 10)).toBe(true);
  });

  it('線分から離れていれば当たらない', () => {
    expect(hitsStroke(stroke, point(50, 30), 10)).toBe(false);
  });

  it('1点だけの筆跡にも当たる', () => {
    const dot: Stroke = { id: 'b', author: 'human', points: [point(0, 0)] };
    expect(hitsStroke(dot, point(5, 0), 5)).toBe(true);
  });
});
