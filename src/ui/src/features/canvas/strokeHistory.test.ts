import { describe, expect, it } from 'vitest';
import type { ImagePlacement } from '../sync/protocol';
import type { Stroke } from './stroke';
import {
  initialStrokeHistory,
  strokeHistoryReducer,
  type StrokeHistoryAction,
  type StrokeHistoryState,
} from './strokeHistory';

const image = (id: string): ImagePlacement => ({
  id,
  x: 10,
  y: 20,
  width: 100,
  height: 50,
});

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

describe('画像', () => {
  const put = (placement: ImagePlacement | null): StrokeHistoryAction => ({
    type: 'image',
    image: placement,
  });

  it('追加・移動・削除を取り消し・やり直しでき、PC には setImage / removeImage で送る', () => {
    const placed = run([put(image('a'))]);
    expect(placed.image).toEqual(image('a'));
    expect(placed.outgoing).toEqual([{ type: 'setImage', image: image('a') }]);

    const moved = strokeHistoryReducer(placed, put({ ...image('a'), x: 30 }));
    expect(moved.outgoing).toEqual([
      { type: 'setImage', image: { ...image('a'), x: 30 } },
    ]);

    const undone = strokeHistoryReducer(moved, { type: 'undo' });
    expect(undone.image).toEqual(image('a'));
    expect(undone.outgoing).toEqual([{ type: 'setImage', image: image('a') }]);

    const removed = strokeHistoryReducer(undone, put(null));
    expect(removed.image).toBeNull();
    expect(removed.outgoing).toEqual([{ type: 'removeImage' }]);

    const restored = strokeHistoryReducer(removed, { type: 'undo' });
    expect(restored.image).toEqual(image('a'));
    expect(restored.outgoing).toEqual([
      { type: 'setImage', image: image('a') },
    ]);

    const redone = strokeHistoryReducer(restored, { type: 'redo' });
    expect(redone.image).toBeNull();
    expect(redone.outgoing).toEqual([{ type: 'removeImage' }]);
  });

  it('配置が変わらなければ何もしない(履歴にも残らない)', () => {
    const before = run([put(image('a'))]);
    expect(strokeHistoryReducer(before, put(image('a')))).toBe(before);
  });

  it('クリアは画像も消し、筆跡の削除とまとめて PC に送る', () => {
    const state = run([add('a'), put(image('x'))]);
    const cleared = strokeHistoryReducer(state, { type: 'clear' });
    expect(cleared.image).toBeNull();
    expect(cleared.outgoing).toEqual([
      { type: 'remove', ids: ['a'] },
      { type: 'removeImage' },
    ]);
    expect(strokeHistoryReducer(cleared, { type: 'clear' })).toBe(cleared);
  });

  it('PC から届いた画像の変更は反映するが、履歴にも出さない', () => {
    const state = run([
      { type: 'remote', op: { type: 'setImage', image: image('r') } },
    ]);
    expect(state.image).toEqual(image('r'));
    expect(state.undoStack).toHaveLength(0);
    expect(state.outgoing).toEqual([]);

    const removed = strokeHistoryReducer(state, {
      type: 'remote',
      op: { type: 'removeImage' },
    });
    expect(removed.image).toBeNull();
  });

  it('ほかのページで画像が変わると、取り消しは筆跡だけ消せて画像は戻らない', () => {
    const drawn = run([
      put(image('mine')),
      put({ ...image('mine'), x: 40 }),
      add('a'),
    ]);
    const changed = strokeHistoryReducer(drawn, {
      type: 'remote',
      op: { type: 'setImage', image: image('theirs') },
    });
    // Only the image operations are removed from the history; the stroke operations remain
    expect(changed.undoStack.map(({ type }) => type)).toEqual(['add']);

    const undone = strokeHistoryReducer(changed, { type: 'undo' });
    expect(ids(undone)).toEqual([]);
    // It does not go back to the image from before this page moved it
    expect(undone.image).toEqual(image('theirs'));
  });

  it('ほかのページで画像が変わると、画像を戻すやり直しも消える', () => {
    const undone = run([put(image('mine')), { type: 'undo' }]);
    expect(undone.redoStack).toHaveLength(1);

    const changed = strokeHistoryReducer(undone, {
      type: 'remote',
      op: { type: 'setImage', image: image('theirs') },
    });
    expect(changed.redoStack).toEqual([]);
    // Nothing can be redone, so mine does not come back even when redoing
    expect(strokeHistoryReducer(changed, { type: 'redo' })).toBe(changed);
  });

  it('画像が同じままの remote 操作では履歴は残る', () => {
    const moved = run([put(image('mine')), put({ ...image('mine'), x: 40 })]);
    // A setImage identical to the current placement (such as its own send coming back through the bridge)
    const same = strokeHistoryReducer(moved, {
      type: 'remote',
      op: { type: 'setImage', image: { ...image('mine'), x: 40 } },
    });
    expect(same.undoStack).toHaveLength(2);

    // removeImage when there is no image does not change the image
    const none = strokeHistoryReducer(run([add('a')]), {
      type: 'remote',
      op: { type: 'removeImage' },
    });
    expect(none.undoStack).toHaveLength(1);
  });

  it('接続時は PC の画像を正とし、PC が持たないで自分が持つものは送り返す', () => {
    const local = run([put(image('mine'))]);

    // If the PC is empty (e.g. after a bridge restart), send its own image back
    const empty = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [],
    });
    expect(empty.image).toEqual(image('mine'));
    expect(empty.outgoing).toEqual([
      { type: 'setImage', image: image('mine') },
    ]);

    // If the PC also has one, adopt the PC's and do not send it back
    const both = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [],
      image: image('pc'),
    });
    expect(both.image).toEqual(image('pc'));
    expect(both.outgoing).toEqual([]);
  });

  it('スナップショットで PC の画像を採用すると、画像の操作は履歴から消える(筆跡は残る)', () => {
    const local = run([add('a'), put(image('mine')), { type: 'undo' }]);
    expect(local.undoStack.map(({ type }) => type)).toEqual(['add']);
    expect(local.redoStack).toHaveLength(1);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [stroke('a')],
      image: image('pc'),
    });
    expect(merged.image).toEqual(image('pc'));
    expect(merged.undoStack.map(({ type }) => type)).toEqual(['add']);
    expect(merged.redoStack).toEqual([]);
  });

  it('スナップショットで画像が変わらなければ履歴は残る(PC が持たず自分のを残すときも)', () => {
    const local = run([add('a'), put(image('mine'))]);

    // The PC is empty and its own image is sent back = the image does not change
    const empty = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [stroke('a')],
    });
    expect(empty.image).toEqual(image('mine'));
    expect(empty.undoStack.map(({ type }) => type)).toEqual(['add', 'image']);

    // The PC's image is exactly the same as the current one
    const same = strokeHistoryReducer(local, {
      type: 'snapshot',
      strokes: [stroke('a')],
      image: image('mine'),
    });
    expect(same.undoStack.map(({ type }) => type)).toEqual(['add', 'image']);
  });
});
