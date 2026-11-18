import { describe, expect, it } from 'vitest';
import {
  CommittedStrokes,
  FLOATS_PER_VERTEX,
  type CommittedVertices,
  tessellateStroke,
} from './strokeMesh';
import type { Stroke } from './stroke';

const point = (x: number, y: number) => ({ x, y, pressure: 1 });

function stroke(id: string, x: number): Stroke {
  return { id, author: 'human', points: [point(x, 0), point(x + 10, 0)] };
}

/** Vertices of a stroke (number of floats) */
function meshLength(s: Stroke): number {
  return tessellateStroke(s.points, '#000000').length;
}

/** Checks kind and narrows to that shape */
function asAppend(
  v: CommittedVertices,
): Extract<CommittedVertices, { kind: 'append' }> {
  expect(v.kind).toBe('append');
  return v as Extract<CommittedVertices, { kind: 'append' }>;
}

function asRebuild(
  v: CommittedVertices,
): Extract<CommittedVertices, { kind: 'rebuild' }> {
  expect(v.kind).toBe('rebuild');
  return v as Extract<CommittedVertices, { kind: 'rebuild' }>;
}

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

describe('CommittedStrokes', () => {
  it('初めは rebuild になる(空なら長さ 0 の data)', () => {
    const committed = new CommittedStrokes();

    const first = asRebuild(committed.update([]));

    expect(first.data.length).toBe(0);
    expect(committed.length).toBe(0);
  });

  it('同じ配列・同じ要素の別の配列を渡すと unchanged になる', () => {
    const a = stroke('a', 0);
    const b = stroke('b', 100);
    const committed = new CommittedStrokes();

    expect(committed.update([a, b]).kind).toBe('rebuild');
    expect(committed.update([a, b]).kind).toBe('unchanged');
    expect(committed.update([a, b]).kind).toBe('unchanged');
  });

  it('後ろに足すと append になる(offset は前の長さ、data は足した筆跡の頂点)', () => {
    const a = stroke('a', 0);
    const b = stroke('b', 100);
    const c = stroke('c', 200);
    const committed = new CommittedStrokes();
    committed.update([a]);

    const first = asAppend(committed.update([a, b]));

    expect(first.offset).toBe(meshLength(a));
    expect(Array.from(first.data)).toEqual(
      Array.from(tessellateStroke(b.points, '#000000')),
    );

    // Adding again also extends the whole previous list
    const second = asAppend(committed.update([a, b, c]));
    expect(second.offset).toBe(meshLength(a) + meshLength(b));
  });

  it('途中の筆跡が消える・入れ替わる・並べ替わる・前に足されると rebuild になる', () => {
    const a = stroke('a', 0);
    const b = stroke('b', 100);

    // A stroke in the middle is removed
    let committed = new CommittedStrokes();
    committed.update([a, b]);
    let rebuilt = asRebuild(committed.update([a]));
    expect(rebuilt.data.length).toBe(meshLength(a));

    // A stroke is replaced
    committed = new CommittedStrokes();
    committed.update([a]);
    rebuilt = asRebuild(committed.update([b]));
    expect(rebuilt.data.length).toBe(meshLength(b));

    // The order changes
    committed = new CommittedStrokes();
    committed.update([a, b]);
    rebuilt = asRebuild(committed.update([b, a]));
    expect(rebuilt.data.length).toBe(meshLength(a) + meshLength(b));

    // Added at the front
    committed = new CommittedStrokes();
    committed.update([b]);
    rebuilt = asRebuild(committed.update([a, b]));
    expect(rebuilt.data.length).toBe(meshLength(a) + meshLength(b));
  });

  it('rebuild の data は全部の筆跡の頂点をつなげたものになる', () => {
    const a = stroke('a', 0);
    const b = stroke('b', 100);
    const committed = new CommittedStrokes();

    const rebuilt = asRebuild(committed.update([a, b]));

    expect(Array.from(rebuilt.data)).toEqual(
      Array.from([
        ...tessellateStroke(a.points, '#000000'),
        ...tessellateStroke(b.points, '#000000'),
      ]),
    );
  });

  it('length がいつも全部の筆跡の頂点の float の数と合う', () => {
    const a = stroke('a', 0);
    const b = stroke('b', 100);
    const committed = new CommittedStrokes();

    committed.update([]);
    expect(committed.length).toBe(0);

    committed.update([a]);
    expect(committed.length).toBe(meshLength(a));

    committed.update([a, b]);
    expect(committed.length).toBe(meshLength(a) + meshLength(b));

    committed.update([b]);
    expect(committed.length).toBe(meshLength(b));
  });
});
