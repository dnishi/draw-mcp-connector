import { describe, expect, it } from 'vitest';
import type { ImagePlacement } from '../sync/protocol';
import { INITIAL_SHEET_ID, MAX_SHEETS } from '../sync/protocol';
import type { Stroke } from './stroke';
import {
  currentSheet,
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
const ids = (state: StrokeHistoryState) =>
  currentSheet(state).strokes.map(({ id }) => id);
const idsOf = (state: StrokeHistoryState, sheetId: string) =>
  state.sheets.find(({ id }) => id === sheetId)?.strokes.map(({ id }) => id) ??
  [];
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
    expect(currentSheet(undone).redoStack).toHaveLength(1);

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
      { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['b'] },
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('b1'),
        index: 1,
      },
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('b2'),
        index: 2,
      },
    ]);

    const undone = strokeHistoryReducer(erased, { type: 'undo' });
    expect(ids(undone)).toEqual(['a', 'b', 'c']);
    expect(undone.outgoing).toEqual([
      { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['b1', 'b2'] },
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('b'),
        index: 1,
      },
    ]);

    const redone = strokeHistoryReducer(undone, { type: 'redo' });
    expect(ids(redone)).toEqual(['a', 'b1', 'b2', 'c']);
  });

  it('クリアすると、今の紙の筆跡と履歴が消え、PC にはまとめて消す変更を送る', () => {
    const drawn = run([add('a'), add('b'), add('c'), { type: 'undo' }]);

    const cleared = strokeHistoryReducer(drawn, { type: 'clear' });

    expect(ids(cleared)).toEqual([]);
    expect(currentSheet(cleared).undoStack).toEqual([]);
    expect(currentSheet(cleared).redoStack).toEqual([]);
    expect(cleared.outgoing).toEqual([
      { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['a', 'b'] },
    ]);
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
    expect(currentSheet(state).redoStack).toHaveLength(0);
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

  describe('紙ごとの履歴', () => {
    it('紙 1 で書いて紙 2 に切り替えても undo しても紙 1 の筆跡は消えず、紙 1 に戻れば undo できる', () => {
      const state = run([add('a'), { type: 'newSheet', id: 's2' }, add('b')]);

      // Undo on sheet 2 affects only sheet 2; sheet 1's stroke stays
      const undone = strokeHistoryReducer(state, { type: 'undo' });
      expect(idsOf(undone, INITIAL_SHEET_ID)).toEqual(['a']);
      expect(idsOf(undone, 's2')).toEqual([]);

      // Back on sheet 1, undo works on sheet 1
      const back = strokeHistoryReducer(undone, {
        type: 'selectSheet',
        id: INITIAL_SHEET_ID,
      });
      expect(idsOf(back, INITIAL_SHEET_ID)).toEqual(['a']);
      expect(ids(strokeHistoryReducer(back, { type: 'undo' }))).toEqual([]);
      expect(idsOf(back, 's2')).toEqual([]);
    });
  });

  describe('紙の変更', () => {
    it('newSheet は今の紙のすぐ後ろに空の紙を差し込んでその紙にする', () => {
      const state = run([add('a'), { type: 'newSheet', id: 's2' }]);
      expect(state.sheets.map(({ id }) => id)).toEqual([
        INITIAL_SHEET_ID,
        's2',
      ]);
      expect(state.current).toBe('s2');
      expect(state.outgoing).toEqual([
        { type: 'addSheet', id: 's2', index: 1 },
        { type: 'selectSheet', id: 's2' },
      ]);
      expect(state.sheets[1].strokes).toEqual([]);
    });

    it('newSheet は MAX_SHEETS 枚あれば何もしない', () => {
      let state = initialStrokeHistory;
      for (let i = 2; i <= MAX_SHEETS; i++) {
        state = strokeHistoryReducer(state, {
          type: 'newSheet',
          id: `s${i}`,
        });
      }
      expect(state.sheets).toHaveLength(MAX_SHEETS);
      expect(strokeHistoryReducer(state, { type: 'newSheet', id: 's51' })).toBe(
        state,
      );
    });

    it('selectSheet は今の紙を替えて PC に送る。既に今の紙・無い紙は何もしない', () => {
      const state = run([{ type: 'newSheet', id: 's2' }]);
      const switched = strokeHistoryReducer(state, {
        type: 'selectSheet',
        id: INITIAL_SHEET_ID,
      });
      expect(switched.current).toBe(INITIAL_SHEET_ID);
      expect(switched.outgoing).toEqual([
        { type: 'selectSheet', id: INITIAL_SHEET_ID },
      ]);
      expect(
        strokeHistoryReducer(switched, {
          type: 'selectSheet',
          id: INITIAL_SHEET_ID,
        }),
      ).toBe(switched);
      expect(
        strokeHistoryReducer(switched, { type: 'selectSheet', id: 'nope' }),
      ).toBe(switched);
    });

    it('removeSheet は今の紙を中身ごと消し、消した位置の紙が今の紙になる', () => {
      const state = run([
        add('a'),
        { type: 'newSheet', id: 's2' },
        add('b'),
        { type: 'newSheet', id: 's3' },
        { type: 'removeSheet' },
      ]);
      expect(state.sheets.map(({ id }) => id)).toEqual([
        INITIAL_SHEET_ID,
        's2',
      ]);
      expect(state.current).toBe('s2');
      expect(state.outgoing).toEqual([{ type: 'removeSheet', id: 's3' }]);
      expect(idsOf(state, INITIAL_SHEET_ID)).toEqual(['a']);
      expect(idsOf(state, 's2')).toEqual(['b']);
    });

    it('removeSheet は 1 枚なら何もしない', () => {
      const state = run([add('a')]);
      expect(strokeHistoryReducer(state, { type: 'removeSheet' })).toBe(state);
    });

    it('resetSheets は白紙の INITIAL_SHEET_ID の紙 1 枚にして PC に送る', () => {
      const state = run([
        add('a'),
        { type: 'newSheet', id: 's2' },
        { type: 'resetSheets' },
      ]);
      expect(state.sheets.map(({ id }) => id)).toEqual([INITIAL_SHEET_ID]);
      expect(state.current).toBe(INITIAL_SHEET_ID);
      expect(ids(state)).toEqual([]);
      expect(state.outgoing).toEqual([{ type: 'resetSheets' }]);
    });

    it('resetSheets は既に白紙の INITIAL_SHEET_ID の紙 1 枚だけなら何もしない', () => {
      expect(
        strokeHistoryReducer(initialStrokeHistory, { type: 'resetSheets' }),
      ).toBe(initialStrokeHistory);
    });
  });
});

describe('PC に送る変更(outgoing)', () => {
  it('描く・消す・取り消すを、それぞれ PC の変更として出す(sheet 付き)', () => {
    const drawn = run([add('a'), add('b')]);
    expect(drawn.outgoing).toEqual([
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('b'),
        index: 1,
      },
    ]);

    const erased = strokeHistoryReducer(drawn, erase('a'));
    expect(erased.outgoing).toEqual([
      { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['a'] },
    ]);

    const undone = strokeHistoryReducer(erased, { type: 'undo' });
    expect(undone.outgoing).toEqual([
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('a'),
        index: 0,
      },
    ]);
  });

  it('PC から届いた変更は反映するが、送り返さず履歴にも入れない', () => {
    const ai: Stroke = { ...stroke('ai-1'), author: 'ai' };
    const state = run([
      add('a'),
      {
        type: 'remote',
        op: {
          type: 'insert',
          sheet: INITIAL_SHEET_ID,
          stroke: ai,
          index: 99,
        },
      },
    ]);
    expect(ids(state)).toEqual(['a', 'ai-1']);
    expect(state.outgoing).toEqual([]);
    expect(currentSheet(state).undoStack).toHaveLength(1);
  });

  it('接続時は PC の全体に合わせ、このページだけが持つ筆跡は残して送る', () => {
    const local = run([add('a'), add('b')]);
    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [
        {
          id: INITIAL_SHEET_ID,
          strokes: [stroke('b'), stroke('pc')],
          image: null,
        },
      ],
      current: INITIAL_SHEET_ID,
    });
    expect(ids(merged)).toEqual(['b', 'pc', 'a']);
    expect(merged.outgoing).toEqual([
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('a'),
        index: 2,
      },
    ]);
  });
});

describe('remote(紙ごと)', () => {
  it('別の紙への筆跡の変更は今の紙に入らず、無い紙への変更は無視する', () => {
    const ai: Stroke = { ...stroke('ai-1'), author: 'ai' };
    const state = run([
      add('a'),
      { type: 'newSheet', id: 's2' },
      {
        type: 'remote',
        op: { type: 'insert', sheet: INITIAL_SHEET_ID, stroke: ai, index: 0 },
      },
    ]);
    expect(state.current).toBe('s2');
    expect(idsOf(state, INITIAL_SHEET_ID)).toEqual(['ai-1', 'a']);
    expect(idsOf(state, 's2')).toEqual([]);
    expect(state.outgoing).toEqual([]);

    const missing = strokeHistoryReducer(state, {
      type: 'remote',
      op: { type: 'insert', sheet: 'nope', stroke: ai, index: 0 },
    });
    expect(missing).toBe(state);
  });

  it('selectSheet で今の紙が変わる', () => {
    const state = run([{ type: 'newSheet', id: 's2' }]);
    const switched = strokeHistoryReducer(state, {
      type: 'remote',
      op: { type: 'selectSheet', id: INITIAL_SHEET_ID },
    });
    expect(switched.current).toBe(INITIAL_SHEET_ID);
    expect(switched.outgoing).toEqual([]);
  });
});

describe('snapshot', () => {
  it('PC の紙の並びと current に合わさり、自分だけの紙は中身があれば末尾に残して送り返す', () => {
    const local = run([add('a'), { type: 'newSheet', id: 's2' }, add('b')]);
    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [
        { id: 'pc2', strokes: [stroke('pc')], image: null },
        { id: 's2', strokes: [], image: null },
      ],
      current: 'pc2',
    });
    expect(merged.sheets.map(({ id }) => id)).toEqual([
      'pc2',
      's2',
      INITIAL_SHEET_ID,
    ]);
    expect(merged.current).toBe('pc2');
    // A sheet this page did not have takes the PC's contents as is
    expect(idsOf(merged, 'pc2')).toEqual(['pc']);
    // Strokes only this page has are kept and sent back. A non-empty sheet only this page has is appended and sent back
    expect(merged.outgoing).toEqual([
      { type: 'insert', sheet: 's2', stroke: stroke('b'), index: 0 },
      { type: 'addSheet', id: INITIAL_SHEET_ID, index: 2 },
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('a'),
        index: 0,
      },
    ]);
  });

  it('自分だけの空の紙は捨てる', () => {
    const local = run([
      { type: 'newSheet', id: 's2' },
      { type: 'newSheet', id: 's3' },
      add('b'),
    ]);
    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: 'pc', strokes: [], image: null }],
      current: 'pc',
    });
    expect(merged.sheets.map(({ id }) => id)).toEqual(['pc', 's3']);
    expect(merged.outgoing).toEqual([
      { type: 'addSheet', id: 's3', index: 1 },
      { type: 'insert', sheet: 's3', stroke: stroke('b'), index: 0 },
    ]);
  });
});

describe('古い bridge(v0.3.0 まで)', () => {
  it('legacySnapshot は今の紙にだけ今の mergeSnapshot と同じ合わせ方をする', () => {
    const local = run([add('a'), { type: 'newSheet', id: 's2' }, add('b')]);
    const merged = strokeHistoryReducer(local, {
      type: 'legacySnapshot',
      strokes: [stroke('pc')],
    });
    // The current sheet (s2) follows the PC, and strokes only this page has are sent back. The other sheet stays
    expect(idsOf(merged, 's2')).toEqual(['pc', 'b']);
    expect(idsOf(merged, INITIAL_SHEET_ID)).toEqual(['a']);
    expect(merged.outgoing).toEqual([
      { type: 'insert', sheet: 's2', stroke: stroke('b'), index: 1 },
    ]);
  });

  it('legacyRemote は sheet の無い変更を今の紙への変更として扱う', () => {
    const ai: Stroke = { ...stroke('ai-1'), author: 'ai' };
    const state = run([
      add('a'),
      { type: 'newSheet', id: 's2' },
      { type: 'legacyRemote', op: { type: 'insert', stroke: ai, index: 0 } },
    ]);
    expect(idsOf(state, 's2')).toEqual(['ai-1']);
    expect(idsOf(state, INITIAL_SHEET_ID)).toEqual(['a']);
    expect(state.outgoing).toEqual([]);
  });
});

describe('画像', () => {
  const put = (placement: ImagePlacement | null): StrokeHistoryAction => ({
    type: 'image',
    image: placement,
  });

  it('追加・移動・削除を取り消し・やり直しでき、PC には setImage / removeImage で送る(sheet 付き)', () => {
    const placed = run([put(image('a'))]);
    expect(currentSheet(placed).image).toEqual(image('a'));
    expect(placed.outgoing).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const moved = strokeHistoryReducer(placed, put({ ...image('a'), x: 30 }));
    expect(moved.outgoing).toEqual([
      {
        type: 'setImage',
        sheet: INITIAL_SHEET_ID,
        image: { ...image('a'), x: 30 },
      },
    ]);

    const undone = strokeHistoryReducer(moved, { type: 'undo' });
    expect(currentSheet(undone).image).toEqual(image('a'));
    expect(undone.outgoing).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const removed = strokeHistoryReducer(undone, put(null));
    expect(currentSheet(removed).image).toBeNull();
    expect(removed.outgoing).toEqual([
      { type: 'removeImage', sheet: INITIAL_SHEET_ID },
    ]);

    const restored = strokeHistoryReducer(removed, { type: 'undo' });
    expect(currentSheet(restored).image).toEqual(image('a'));
    expect(restored.outgoing).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const redone = strokeHistoryReducer(restored, { type: 'redo' });
    expect(currentSheet(redone).image).toBeNull();
    expect(redone.outgoing).toEqual([
      { type: 'removeImage', sheet: INITIAL_SHEET_ID },
    ]);
  });

  it('配置が変わらなければ何もしない(履歴にも残らない)', () => {
    const before = run([put(image('a'))]);
    expect(strokeHistoryReducer(before, put(image('a')))).toBe(before);
  });

  it('クリアは今の紙の画像も消し、筆跡の削除とまとめて PC に送る。ほかの紙は消えない', () => {
    const state = run([
      add('a'),
      { type: 'newSheet', id: 's2' },
      add('b'),
      put(image('x')),
    ]);
    // Wipe the current sheet (s2): strokes, image, and history are cleared
    const cleared = strokeHistoryReducer(state, { type: 'clear' });
    expect(currentSheet(cleared).image).toBeNull();
    expect(cleared.outgoing).toEqual([
      { type: 'remove', sheet: 's2', ids: ['b'] },
      { type: 'removeImage', sheet: 's2' },
    ]);
    // The wiped sheet's history is empty too, but the other sheet stays
    expect(currentSheet(cleared).undoStack).toEqual([]);
    expect(idsOf(cleared, INITIAL_SHEET_ID)).toEqual(['a']);
    expect(strokeHistoryReducer(cleared, { type: 'clear' })).toBe(cleared);
  });

  it('PC から届いた画像の変更は反映するが、履歴にも出さない', () => {
    const state = run([
      {
        type: 'remote',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('r') },
      },
    ]);
    expect(currentSheet(state).image).toEqual(image('r'));
    expect(currentSheet(state).undoStack).toHaveLength(0);
    expect(state.outgoing).toEqual([]);

    const removed = strokeHistoryReducer(state, {
      type: 'remote',
      op: { type: 'removeImage', sheet: INITIAL_SHEET_ID },
    });
    expect(currentSheet(removed).image).toBeNull();
  });

  it('ほかのページで画像が変わると、取り消しは筆跡だけ消せて画像は戻らない', () => {
    const drawn = run([
      put(image('mine')),
      put({ ...image('mine'), x: 40 }),
      add('a'),
    ]);
    const changed = strokeHistoryReducer(drawn, {
      type: 'remote',
      op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('theirs') },
    });
    // Only the image actions are removed from the history; the stroke actions remain
    expect(currentSheet(changed).undoStack.map(({ type }) => type)).toEqual([
      'add',
    ]);

    const undone = strokeHistoryReducer(changed, { type: 'undo' });
    expect(ids(undone)).toEqual([]);
    // It does not go back to the image from before this page moved it
    expect(currentSheet(undone).image).toEqual(image('theirs'));
  });

  it('ほかのページで画像が変わると、画像を戻すやり直しも消える', () => {
    const undone = run([put(image('mine')), { type: 'undo' }]);
    expect(currentSheet(undone).redoStack).toHaveLength(1);

    const changed = strokeHistoryReducer(undone, {
      type: 'remote',
      op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('theirs') },
    });
    expect(currentSheet(changed).redoStack).toEqual([]);
    // Nothing to redo, so redoing does not bring mine back
    expect(strokeHistoryReducer(changed, { type: 'redo' })).toBe(changed);
  });

  it('画像が同じままの remote 操作では履歴は残る', () => {
    const moved = run([put(image('mine')), put({ ...image('mine'), x: 40 })]);
    // A setImage identical to the current placement (such as its own send coming back through the bridge)
    const same = strokeHistoryReducer(moved, {
      type: 'remote',
      op: {
        type: 'setImage',
        sheet: INITIAL_SHEET_ID,
        image: { ...image('mine'), x: 40 },
      },
    });
    expect(currentSheet(same).undoStack).toHaveLength(2);

    // removeImage when there is no image does not change the image
    const none = strokeHistoryReducer(run([add('a')]), {
      type: 'remote',
      op: { type: 'removeImage', sheet: INITIAL_SHEET_ID },
    });
    expect(currentSheet(none).undoStack).toHaveLength(1);
  });

  it('接続時は PC の画像を正とし、PC が持たないで自分が持つものは送り返す', () => {
    const local = run([put(image('mine'))]);

    // If the PC is empty (e.g. after a bridge restart), send its own image back
    const empty = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: null }],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(empty).image).toEqual(image('mine'));
    expect(empty.outgoing).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('mine') },
    ]);

    // If the PC also has one, adopt the PC's and do not send it back
    const both = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: image('pc') }],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(both).image).toEqual(image('pc'));
    expect(both.outgoing).toEqual([]);
  });

  it('スナップショットで PC の画像を採用すると、画像の操作は履歴から消える(筆跡は残る)', () => {
    const local = run([add('a'), put(image('mine')), { type: 'undo' }]);
    expect(currentSheet(local).undoStack.map(({ type }) => type)).toEqual([
      'add',
    ]);
    expect(currentSheet(local).redoStack).toHaveLength(1);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [
        { id: INITIAL_SHEET_ID, strokes: [stroke('a')], image: image('pc') },
      ],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(merged).image).toEqual(image('pc'));
    expect(currentSheet(merged).undoStack.map(({ type }) => type)).toEqual([
      'add',
    ]);
    expect(currentSheet(merged).redoStack).toEqual([]);
  });

  it('スナップショットで画像が変わらなければ履歴は残る(PC が持たず自分のを残すときも)', () => {
    const local = run([add('a'), put(image('mine'))]);

    // The PC is empty and its own image is sent back = the image does not change
    const empty = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [stroke('a')], image: null }],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(empty).image).toEqual(image('mine'));
    expect(currentSheet(empty).undoStack.map(({ type }) => type)).toEqual([
      'add',
      'image',
    ]);

    // The PC's image is exactly the same as the current one
    const same = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [
        { id: INITIAL_SHEET_ID, strokes: [stroke('a')], image: image('mine') },
      ],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(same).undoStack.map(({ type }) => type)).toEqual([
      'add',
      'image',
    ]);
  });
});
