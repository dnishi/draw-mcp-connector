import { describe, expect, it } from 'vitest';
import {
  type ImagePlacement,
  INITIAL_SHEET_ID,
  MAX_SHEETS,
} from '../sync/protocol';
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
/** Operations in pending (in send order) */
const ops = (state: StrokeHistoryState) => state.pending.map(({ op }) => op);
/** The last n operations in pending */
const tail = (state: StrokeHistoryState, n: number) => ops(state).slice(-n);
/** Numbers in pending */
const seqs = (state: StrokeHistoryState) => state.pending.map(({ seq }) => seq);

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
    expect(tail(erased, 3)).toEqual([
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
    expect(tail(undone, 2)).toEqual([
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
    expect(tail(cleared, 1)).toEqual([
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
      expect(tail(state, 2)).toEqual([
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
      expect(tail(switched, 1)).toEqual([
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
      expect(tail(state, 1)).toEqual([{ type: 'removeSheet', id: 's3' }]);
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
      expect(tail(state, 1)).toEqual([{ type: 'resetSheets' }]);
    });

    it('resetSheets は既に白紙の INITIAL_SHEET_ID の紙 1 枚だけなら何もしない', () => {
      expect(
        strokeHistoryReducer(initialStrokeHistory, { type: 'resetSheets' }),
      ).toBe(initialStrokeHistory);
    });
  });
});

describe('pending(このページが起こした操作のうち、ack の来ていないもの)', () => {
  it('操作は pending に番号付きで積まれ、2 つ続けても両方残る', () => {
    const drawn = run([add('a'), add('b')]);
    expect(drawn.pending).toEqual([
      {
        seq: 1,
        op: {
          type: 'insert',
          sheet: INITIAL_SHEET_ID,
          stroke: stroke('a'),
          index: 0,
        },
      },
      {
        seq: 2,
        op: {
          type: 'insert',
          sheet: INITIAL_SHEET_ID,
          stroke: stroke('b'),
          index: 1,
        },
      },
    ]);
    expect(drawn.nextSeq).toBe(3);

    // Undo is queued after the earlier operations
    const undone = strokeHistoryReducer(drawn, { type: 'undo' });
    expect(seqs(undone)).toEqual([1, 2, 3]);
    expect(tail(undone, 1)).toEqual([
      { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['b'] },
    ]);
  });

  it('ack が来ると、その番号までが pending から除かれる', () => {
    const drawn = run([add('a'), add('b'), add('c')]);

    const acked = strokeHistoryReducer(drawn, { type: 'ack', seq: 2 });
    expect(acked.pending).toEqual([
      {
        seq: 3,
        op: {
          type: 'insert',
          sheet: INITIAL_SHEET_ID,
          stroke: stroke('c'),
          index: 2,
        },
      },
    ]);
    // Numbers are not reused
    expect(acked.nextSeq).toBe(4);

    // An ack above every number removes them all
    const all = strokeHistoryReducer(drawn, { type: 'ack', seq: 99 });
    expect(all.pending).toEqual([]);

    // Nothing changes when there is nothing to remove
    expect(strokeHistoryReducer(drawn, { type: 'ack', seq: 0 })).toBe(drawn);
  });

  it('PC から届いた変更は反映するが、pending と履歴には入れない', () => {
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
    expect(ops(state)).toEqual([
      {
        type: 'insert',
        sheet: INITIAL_SHEET_ID,
        stroke: stroke('a'),
        index: 0,
      },
    ]);
    expect(currentSheet(state).undoStack).toHaveLength(1);
  });
});

describe('remote(紙ごと)', () => {
  it('別の紙への筆跡の変更は今の紙に入らず、無い紙への変更は無視する', () => {
    const ai: Stroke = { ...stroke('ai-1'), author: 'ai' };
    const before = run([add('a'), { type: 'newSheet', id: 's2' }]);
    const state = strokeHistoryReducer(before, {
      type: 'remote',
      op: { type: 'insert', sheet: INITIAL_SHEET_ID, stroke: ai, index: 0 },
    });
    expect(state.current).toBe('s2');
    expect(idsOf(state, INITIAL_SHEET_ID)).toEqual(['ai-1', 'a']);
    expect(idsOf(state, 's2')).toEqual([]);
    // Received changes do not change pending
    expect(state.pending).toBe(before.pending);

    const missing = strokeHistoryReducer(state, {
      type: 'remote',
      op: { type: 'insert', sheet: 'nope', stroke: ai, index: 0 },
    });
    expect(missing).toBe(state);
  });

  it('selectSheet で今の紙が変わる', () => {
    const before = run([{ type: 'newSheet', id: 's2' }]);
    const switched = strokeHistoryReducer(before, {
      type: 'remote',
      op: { type: 'selectSheet', id: INITIAL_SHEET_ID },
    });
    expect(switched.current).toBe(INITIAL_SHEET_ID);
    expect(switched.pending).toBe(before.pending);
  });
});

describe('snapshot', () => {
  it('ほかの端末で消した筆跡・紙は戻らず、送る操作も増えない', () => {
    // The stroke and sheet made here reached the PC and were acked (then deleted while disconnected)
    const local = run([
      add('a'),
      { type: 'newSheet', id: 's2' },
      add('b'),
      { type: 'ack', seq: 4 },
    ]);
    expect(local.pending).toEqual([]);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [stroke('pc')], image: null }],
      current: INITIAL_SHEET_ID,
    });
    // s2 (b) was deleted elsewhere, and so was a. Neither comes back
    expect(merged.sheets.map(({ id }) => id)).toEqual([INITIAL_SHEET_ID]);
    expect(ids(merged)).toEqual(['pc']);
    expect(merged.pending).toEqual([]);
  });

  it('ack の来ていない操作は、PC の中身の上に当て直されて pending に残る', () => {
    // A stroke and sheet made while disconnected (none acked)
    const local = run([add('a'), { type: 'newSheet', id: 's2' }, add('b')]);
    expect(seqs(local)).toEqual([1, 2, 3, 4]);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [stroke('pc')], image: null }],
      current: INITIAL_SHEET_ID,
    });
    // The operations are reapplied, oldest first, on top of the PC's contents.
    // insert goes in at position 0, where it was drawn (an empty sheet on this page)
    expect(merged.sheets.map(({ id }) => id)).toEqual([INITIAL_SHEET_ID, 's2']);
    expect(idsOf(merged, INITIAL_SHEET_ID)).toEqual(['a', 'pc']);
    expect(idsOf(merged, 's2')).toEqual(['b']);
    // Sheet operations (selectSheet) are reapplied too
    expect(merged.current).toBe('s2');
    // pending and nextSeq stay as they are (the sync side resends them)
    expect(merged.pending).toBe(local.pending);
    expect(merged.nextSeq).toBe(5);
  });

  it('当て直しが当てられない操作は無視される', () => {
    // While disconnected, sheet s2 was made, then the page went back and drew on the original sheet.
    // The PC has neither (another device deleted everything)
    const local = run([
      { type: 'newSheet', id: 's2' },
      { type: 'selectSheet', id: INITIAL_SHEET_ID },
      add('a'),
    ]);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: 'pc', strokes: [stroke('pc')], image: null }],
      current: 'pc',
    });
    // addSheet s2 and selectSheet s2 are reapplied.
    // selectSheet and insert for the missing sheet (sheet-1) are ignored
    expect(merged.sheets.map(({ id }) => id)).toEqual(['pc', 's2']);
    expect(idsOf(merged, 's2')).toEqual([]);
    expect(idsOf(merged, 'pc')).toEqual(['pc']);
    expect(merged.pending).toBe(local.pending);
  });

  it('同じ ID の紙の取り消し・やり直しの履歴は残る', () => {
    // The undo (remove of a) has not reached the PC yet
    const local = run([add('a'), add('b'), { type: 'undo' }]);
    expect(ids(local)).toEqual(['a']);

    const merged = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [
        {
          id: INITIAL_SHEET_ID,
          strokes: [stroke('a'), stroke('b')],
          image: null,
        },
      ],
      current: INITIAL_SHEET_ID,
    });
    // The remove of b is reapplied on top of the PC's contents, and this page's history is kept
    expect(ids(merged)).toEqual(['a']);
    expect(currentSheet(merged).undoStack.map(({ type }) => type)).toEqual([
      'add',
    ]);
    expect(currentSheet(merged).redoStack.map(({ type }) => type)).toEqual([
      'add',
    ]);
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
    expect(tail(placed, 1)).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const moved = strokeHistoryReducer(placed, put({ ...image('a'), x: 30 }));
    expect(tail(moved, 1)).toEqual([
      {
        type: 'setImage',
        sheet: INITIAL_SHEET_ID,
        image: { ...image('a'), x: 30 },
      },
    ]);

    const undone = strokeHistoryReducer(moved, { type: 'undo' });
    expect(currentSheet(undone).image).toEqual(image('a'));
    expect(tail(undone, 1)).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const removed = strokeHistoryReducer(undone, put(null));
    expect(currentSheet(removed).image).toBeNull();
    expect(tail(removed, 1)).toEqual([
      { type: 'removeImage', sheet: INITIAL_SHEET_ID },
    ]);

    const restored = strokeHistoryReducer(removed, { type: 'undo' });
    expect(currentSheet(restored).image).toEqual(image('a'));
    expect(tail(restored, 1)).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('a') },
    ]);

    const redone = strokeHistoryReducer(restored, { type: 'redo' });
    expect(currentSheet(redone).image).toBeNull();
    expect(tail(redone, 1)).toEqual([
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
    expect(tail(cleared, 2)).toEqual([
      { type: 'remove', sheet: 's2', ids: ['b'] },
      { type: 'removeImage', sheet: 's2' },
    ]);
    // The wiped sheet's history is empty too, but the other sheet stays
    expect(currentSheet(cleared).undoStack).toEqual([]);
    expect(idsOf(cleared, INITIAL_SHEET_ID)).toEqual(['a']);
    expect(strokeHistoryReducer(cleared, { type: 'clear' })).toBe(cleared);
  });

  it('PC から届いた画像の変更は反映するが、pending と履歴には入れない', () => {
    const state = run([
      {
        type: 'remote',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('r') },
      },
    ]);
    expect(currentSheet(state).image).toEqual(image('r'));
    expect(currentSheet(state).undoStack).toHaveLength(0);
    expect(state.pending).toEqual([]);

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

  it('繋ぎ直しは、ack が来ていない画像は当て直し、ack が来ているときは PC の画像を正とする', () => {
    // No ack yet: even if the PC is empty, this page's image is reapplied (and resent)
    const local = run([put(image('mine'))]);
    const empty = strokeHistoryReducer(local, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: null }],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(empty).image).toEqual(image('mine'));
    expect(ops(empty)).toEqual([
      { type: 'setImage', sheet: INITIAL_SHEET_ID, image: image('mine') },
    ]);

    // Acked, and the PC has another image: the PC's image is used and pending does not grow
    const acked = run([put(image('mine')), { type: 'ack', seq: 1 }]);
    const both = strokeHistoryReducer(acked, {
      type: 'snapshot',
      sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: image('pc') }],
      current: INITIAL_SHEET_ID,
    });
    expect(currentSheet(both).image).toEqual(image('pc'));
    expect(both.pending).toEqual([]);
  });

  it('繋ぎ直しで画像が変わると、画像の操作は履歴から消える(筆跡は残る)', () => {
    const local = run([add('a'), put(image('mine')), { type: 'ack', seq: 2 }]);
    expect(currentSheet(local).undoStack.map(({ type }) => type)).toEqual([
      'add',
      'image',
    ]);

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

  it('繋ぎ直しで画像が変わらなければ履歴は残る', () => {
    const local = run([add('a'), put(image('mine'))]);

    // The PC is empty and the image without an ack is reapplied, so the image does not change
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
