import { describe, expect, it } from 'vitest';
import { applyCanvasOp, applySheetOp, type SheetOp } from './canvasOp';
import { type Stroke, INITIAL_SHEET_ID, MAX_SHEETS } from './protocol';

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
      sheet: 's',
      stroke: stroke('a'),
      index: 0,
    });
    const two = applyCanvasOp(one, {
      type: 'insert',
      sheet: 's',
      stroke: stroke('b'),
      index: 99,
    });
    expect(ids(two)).toEqual(['a', 'b']);
    expect(
      applyCanvasOp(two, {
        type: 'insert',
        sheet: 's',
        stroke: stroke('a'),
        index: 0,
      }),
    ).toBe(two);
  });

  it('remove は無い ID を無視し、何も消えなければ同じ配列を返す', () => {
    const strokes = [stroke('a'), stroke('b')];
    expect(
      ids(
        applyCanvasOp(strokes, { type: 'remove', sheet: 's', ids: ['a', 'x'] }),
      ),
    ).toEqual(['b']);
    expect(
      applyCanvasOp(strokes, { type: 'remove', sheet: 's', ids: ['x'] }),
    ).toBe(strokes);
  });

  it('replace は重なり順を保って置き換える', () => {
    const strokes = [stroke('a'), stroke('b')];
    const replaced = applyCanvasOp(strokes, {
      type: 'replace',
      sheet: 's',
      stroke: stroke('a', 5),
    });
    expect(ids(replaced)).toEqual(['a', 'b']);
    expect(replaced[0].points[0].x).toBe(5);
  });
});

describe('applySheetOp', () => {
  // A sheet here only has an ID and whether it is empty
  type TestSheet = { id: string; empty: boolean };
  const testSheet = (id: string, empty = false): TestSheet => ({ id, empty });
  const emptySheet = (id: string): TestSheet => testSheet(id, true);
  const isEmpty = (sheet: TestSheet) => sheet.empty;
  const ids = (sheets: readonly TestSheet[]) => sheets.map(({ id }) => id);

  const apply = (sheets: readonly TestSheet[], current: string, op: SheetOp) =>
    applySheetOp(sheets, current, op, emptySheet, isEmpty);

  it('addSheet は index の位置に差し込み、範囲外なら末尾に置く。今のシートは変えない', () => {
    const sheets = [testSheet('a'), testSheet('b')];
    expect(
      ids(apply(sheets, 'a', { type: 'addSheet', id: 'c', index: 1 })!.sheets),
    ).toEqual(['a', 'c', 'b']);
    expect(
      apply(sheets, 'a', { type: 'addSheet', id: 'c', index: 1 })!.current,
    ).toBe('a');
    expect(
      ids(apply(sheets, 'a', { type: 'addSheet', id: 'c', index: 99 })!.sheets),
    ).toEqual(['a', 'b', 'c']);
  });

  it('addSheet は同じ ID のシートがある・既に MAX_SHEETS 枚なら無視する', () => {
    const sheets = [testSheet('a')];
    expect(
      apply(sheets, 'a', { type: 'addSheet', id: 'a', index: 1 }),
    ).toBeNull();
    const full = Array.from({ length: MAX_SHEETS }, (_, i) =>
      testSheet(`s${i}`),
    );
    expect(
      apply(full, 's0', { type: 'addSheet', id: 'new', index: 0 }),
    ).toBeNull();
  });

  it('removeSheet はシートを消し、今のシートを消したときは消した位置のシート(末尾なら新しい末尾)が今のシートになる', () => {
    const sheets = [testSheet('a'), testSheet('b'), testSheet('c')];
    // Removing the current sheet (b): the sheet at that position (c) becomes current
    const removedCurrent = apply(sheets, 'b', {
      type: 'removeSheet',
      id: 'b',
    })!;
    expect(ids(removedCurrent.sheets)).toEqual(['a', 'c']);
    expect(removedCurrent.current).toBe('c');
    // Removing the last sheet: the new last sheet (b) becomes current
    const removedLast = apply(sheets, 'c', { type: 'removeSheet', id: 'c' })!;
    expect(removedLast.current).toBe('b');
    // Removing another sheet does not change the current sheet
    expect(apply(sheets, 'b', { type: 'removeSheet', id: 'a' })!.current).toBe(
      'b',
    );
  });

  it('removeSheet はシートが無い・残り 1 枚なら無視する', () => {
    const sheets = [testSheet('a'), testSheet('b')];
    expect(apply(sheets, 'a', { type: 'removeSheet', id: 'x' })).toBeNull();
    expect(
      apply([testSheet('a')], 'a', { type: 'removeSheet', id: 'a' }),
    ).toBeNull();
  });

  it('selectSheet は今のシートを替える', () => {
    const sheets = [testSheet('a'), testSheet('b')];
    const selected = apply(sheets, 'a', { type: 'selectSheet', id: 'b' })!;
    expect(selected.current).toBe('b');
    expect(selected.sheets).toBe(sheets);
  });

  it('selectSheet はシートが無い・既に今のシートなら無視する', () => {
    const sheets = [testSheet('a')];
    expect(apply(sheets, 'a', { type: 'selectSheet', id: 'x' })).toBeNull();
    expect(apply(sheets, 'a', { type: 'selectSheet', id: 'a' })).toBeNull();
  });

  it('resetSheets は全部を消し、空の INITIAL_SHEET_ID のシート 1 枚にして今のシートにする', () => {
    const sheets = [testSheet('a'), testSheet('b')];
    const reset = apply(sheets, 'b', { type: 'resetSheets' })!;
    expect(ids(reset.sheets)).toEqual([INITIAL_SHEET_ID]);
    expect(reset.current).toBe(INITIAL_SHEET_ID);
    expect(reset.sheets[0].empty).toBe(true);
    // Ignored when there is just one empty INITIAL_SHEET_ID sheet
    expect(
      apply([testSheet(INITIAL_SHEET_ID, true)], INITIAL_SHEET_ID, {
        type: 'resetSheets',
      }),
    ).toBeNull();
    // Not ignored when it has contents (not empty)
    expect(
      apply([testSheet(INITIAL_SHEET_ID, false)], INITIAL_SHEET_ID, {
        type: 'resetSheets',
      }),
    ).not.toBeNull();
  });
});
