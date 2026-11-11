import { describe, expect, it } from 'vitest';
import {
  buildSceneVertices,
  FLOATS_PER_VERTEX,
  tessellateStroke,
} from './strokeMesh';
import type { Stroke } from './stroke';

const point = (x: number, y: number) => ({ x, y, pressure: 1 });

describe('tessellateStroke', () => {
  it('三角形リストの頂点になる(頂点数が3の倍数)', () => {
    const vertices = tessellateStroke(
      [point(0, 0), point(10, 0), point(10, 10)],
      '#000000',
    );
    expect((vertices.length / FLOATS_PER_VERTEX) % 3).toBe(0);
    expect(vertices.length).toBeGreaterThan(0);
  });

  it('1点だけでも点として描ける', () => {
    expect(tessellateStroke([point(5, 5)], '#000000').length).toBeGreaterThan(
      0,
    );
  });

  it('頂点ごとに筆跡の色(0〜1 の r, g, b)を持つ', () => {
    const vertices = tessellateStroke([point(5, 5)], '#ff0000');
    expect([...vertices.slice(2, FLOATS_PER_VERTEX)]).toEqual([1, 0, 0]);
  });
});

describe('buildSceneVertices', () => {
  it('確定した筆跡と描いている途中の筆跡をつなげる', () => {
    const stroke: Stroke = {
      id: 'a',
      author: 'human',
      points: [point(0, 0), point(10, 0)],
    };
    const active = { points: [point(20, 20)], color: '#007aff' };

    const vertices = buildSceneVertices([stroke], active);

    expect(vertices.length).toBe(
      tessellateStroke(stroke.points, '#000000').length +
        tessellateStroke(active.points, active.color).length,
    );
  });

  it('何も無ければ空になる', () => {
    expect(buildSceneVertices([])).toHaveLength(0);
  });
});
