import { describe, expect, it } from 'vitest';
import type { Stroke } from './stroke';
import {
  initialStrokeHistory,
  strokeHistoryReducer,
  type StrokeHistoryAction,
  type StrokeHistoryState,
} from './strokeHistory';

const stroke = (id: string): Stroke => ({
  id,
  author: 'human',
  points: [{ x: 0, y: 0, pressure: 1 }],
});

function run(
  actions: StrokeHistoryAction[],
  from = initialStrokeHistory,
): StrokeHistoryState {
  return actions.reduce(strokeHistoryReducer, from);
}

const ids = (state: StrokeHistoryState) => state.strokes.map(({ id }) => id);
const add = (id: string): StrokeHistoryAction => ({
  type: 'add',
  stroke: stroke(id),
});
/** Erase whole strokes (nothing remains) */
const erase = (...erasedIds: string[]): StrokeHistoryAction => ({
  type: 'erase',
  pieces: new Map(erasedIds.map((id) => [id, []])),
});

describe('strokeHistoryReducer', () => {
  it('描いた筆跡を取り消し・やり直しできる', () => {
    const drawn = run([add('a'), add('b')]);
    expect(ids(drawn)).toEqual(['a', 'b']);

    const undone = strokeHistoryReducer(drawn, { type: 'undo' });
    expect(ids(undone)).toEqual(['a']);
    expect(undone.redoStack).toHaveLength(1);

    const redone = strokeHistoryReducer(undone, { type: 'redo' });
    expect(ids(redone)).toEqual(['a', 'b']);
  });

  it('消した筆跡は取り消すと元の重なり順に戻る', () => {
    const erased = run([add('a'), add('b'), add('c'), erase('a', 'c')]);
    expect(ids(erased)).toEqual(['b']);

    expect(ids(strokeHistoryReducer(erased, { type: 'undo' }))).toEqual([
      'a',
      'b',
      'c',
    ]);
  });

  it('なぞって消すと、消した筆跡の位置に消し残りが入り、取り消すと元に戻る', () => {
    const drawn = run([add('a'), add('b'), add('c')]);
    const erased = strokeHistoryReducer(drawn, {
      type: 'erase',
      pieces: new Map([['b', [stroke('b1'), stroke('b2')]]]),
    });
    expect(ids(erased)).toEqual(['a', 'b1', 'b2', 'c']);
    expect(erased.outgoing).toEqual([
      { type: 'remove', ids: ['b'] },
      { type: 'insert', stroke: stroke('b1'), index: 1 },
      { type: 'insert', stroke: stroke('b2'), index: 2 },
    ]);

    const undone = strokeHistoryReducer(erased, { type: 'undo' });
    expect(ids(undone)).toEqual(['a', 'b', 'c']);
    expect(undone.outgoing).toEqual([
      { type: 'remove', ids: ['b1', 'b2'] },
      { type: 'insert', stroke: stroke('b'), index: 1 },
    ]);

    const redone = strokeHistoryReducer(undone, { type: 'redo' });
    expect(ids(redone)).toEqual(['a', 'b1', 'b2', 'c']);
  });

  it('クリアすると、すべての筆跡と履歴が消え、PC にはまとめて消す変更を送る', () => {
    const drawn = run([add('a'), add('b'), add('c'), { type: 'undo' }]);

    const cleared = strokeHistoryReducer(drawn, { type: 'clear' });

    expect(cleared.strokes).toEqual([]);
    expect(cleared.undoStack).toEqual([]);
    expect(cleared.redoStack).toEqual([]);
    expect(cleared.outgoing).toEqual([{ type: 'remove', ids: ['a', 'b'] }]);
  });

  it('何も無いときのクリアは何もしない', () => {
    expect(strokeHistoryReducer(initialStrokeHistory, { type: 'clear' })).toBe(
      initialStrokeHistory,
    );
  });

  it('何も消さなかった消しゴムは履歴に残らない', () => {
    const before = run([add('a')]);
    const after = strokeHistoryReducer(before, erase('x'));
    expect(after).toBe(before);
  });

  it('新しく描くとやり直しの履歴は消える', () => {
    const state = run([add('a'), { type: 'undo' }, add('b')]);
    expect(state.redoStack).toHaveLength(0);
    expect(ids(state)).toEqual(['b']);
  });

  it('履歴が無いときの取り消し・やり直しは何もしない', () => {
    expect(strokeHistoryReducer(initialStrokeHistory, { type: 'undo' })).toBe(
      initialStrokeHistory,
    );
    expect(strokeHistoryReducer(initialStrokeHistory, { type: 'redo' })).toBe(
      initialStrokeHistory,
    );
  });
});

describe('PC に送る変更(outgoing)', () => {
  it('描く・消す・取り消すを、それぞれ PC の変更として出す', () => {
    const drawn = run([add('a'), add('b')]);
    expect(drawn.outgoing).toEqual([
      { type: 'insert', stroke: stroke('b'), index: 1 },
    ]);

    const erased = strokeHistoryReducer(drawn, erase('a'));
    expect(erased.outgoing).toEqual([{ type: 'remove', ids: ['a'] }]);

    const undone = strokeHistoryReducer(erased, { type: 'undo' });
    expect(undone.outgoing).toEqual([
      { type: 'insert', stroke: stroke('a'), index: 0 },
    ]);
  });

  it('PC から届いた変更は反映するが、送り返さず履歴にも入れない', () => {
    const ai: Stroke = { ...stroke('ai-1'), author: 'ai' };
    const state = run([
      add('a'),
      { type: 'remote', op: { type: 'insert', stroke: ai, index: 99 } },
    ]);
    expect(ids(state)).toEqual(['a', 'ai-1']);
    expect(state.outgoing).toEqual([]);
    expect(state.undoStack).toHaveLength(1);
  });

  it('接続時は PC の全体に合わせ、iPad だけが持つ筆跡は残して送る', () => {
    const local = run([add('a'), add('b')]);
    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [stroke('b'), stroke('pc')],
    });
    expect(ids(merged)).toEqual(['b', 'pc', 'a']);
    expect(merged.outgoing).toEqual([
      { type: 'insert', stroke: stroke('a'), index: 2 },
    ]);
  });
});
