import {
  type CanvasOp,
  type Stroke,
  INITIAL_SHEET_ID,
  MAX_SHEETS,
} from './protocol';

/** Stroke changes only (image and sheet changes are dispatched by the caller) */
export type StrokeOp = Extract<
  CanvasOp,
  { type: 'insert' | 'remove' | 'replace' }
>;

/** Sheet changes only (stroke and image changes are dispatched by the caller) */
export type SheetOp = Extract<
  CanvasOp,
  { type: 'addSheet' | 'removeSheet' | 'selectSheet' | 'resetSheets' }
>;

/**
 * Applies one change to a list of strokes, with the same rules as the bridge (src/bridge/src/canvas.rs):
 * - insert: inserts at position index (at the end if out of range). Does nothing if the same ID exists
 * - remove: missing IDs are ignored
 * - replace: replaces the stroke with the same ID, keeping the stacking order. Does nothing if it is missing
 * Returns the given array as is if nothing changes
 */
export function applyCanvasOp(
  strokes: readonly Stroke[],
  op: StrokeOp,
): readonly Stroke[] {
  switch (op.type) {
    case 'insert': {
      if (strokes.some(({ id }) => id === op.stroke.id)) {
        return strokes;
      }
      const next = [...strokes];
      next.splice(Math.min(op.index, strokes.length), 0, op.stroke);
      return next;
    }
    case 'remove': {
      const ids = new Set(op.ids);
      const next = strokes.filter(({ id }) => !ids.has(id));
      return next.length === strokes.length ? strokes : next;
    }
    case 'replace': {
      const index = strokes.findIndex(({ id }) => id === op.stroke.id);
      if (index < 0) {
        return strokes;
      }
      return strokes.with(index, op.stroke);
    }
  }
}

export function applyCanvasOps(
  strokes: readonly Stroke[],
  ops: readonly StrokeOp[],
): readonly Stroke[] {
  return ops.reduce(applyCanvasOp, strokes);
}

/**
 * Applies one sheet change to a list of sheets, with the same rules as the bridge (src/bridge/src/canvas.rs):
 * - addSheet: inserts an empty sheet at position index (at the end if out of range). The current sheet does not change.
 *   Ignored if a sheet with the same ID exists or there are already MAX_SHEETS sheets
 * - removeSheet: removes a sheet with its contents. When the current sheet is removed, the sheet at that position
 *   (or the new last sheet) becomes current. Ignored if the sheet is missing or only one sheet is left
 * - selectSheet: changes the current sheet. Ignored if the sheet is missing or already current
 * - resetSheets: removes all sheets and makes one empty INITIAL_SHEET_ID sheet the current sheet.
 *   Ignored if there is already just one empty INITIAL_SHEET_ID sheet
 * null if nothing changes
 */
export function applySheetOp<T extends { id: string }>(
  sheets: readonly T[],
  current: string,
  op: SheetOp,
  /** Makes an empty sheet (the page also keeps history per sheet, so the caller decides how) */
  emptySheet: (id: string) => T,
  /** Whether a sheet is empty (used to decide whether resetSheets is ignored) */
  isEmpty: (sheet: T) => boolean,
): { sheets: readonly T[]; current: string } | null {
  switch (op.type) {
    case 'addSheet': {
      if (
        sheets.length >= MAX_SHEETS ||
        sheets.some(({ id }) => id === op.id)
      ) {
        return null;
      }
      const index = Math.min(op.index, sheets.length);
      return {
        sheets: [
          ...sheets.slice(0, index),
          emptySheet(op.id),
          ...sheets.slice(index),
        ],
        current,
      };
    }
    case 'removeSheet': {
      const position = sheets.findIndex(({ id }) => id === op.id);
      if (position < 0 || sheets.length <= 1) {
        return null;
      }
      const next = [
        ...sheets.slice(0, position),
        ...sheets.slice(position + 1),
      ];
      return {
        sheets: next,
        // When the current sheet is removed, the sheet at that position (or the new last sheet) becomes current
        current:
          current === op.id
            ? next[Math.min(position, next.length - 1)].id
            : current,
      };
    }
    case 'selectSheet': {
      if (current === op.id || sheets.every(({ id }) => id !== op.id)) {
        return null;
      }
      return { sheets, current: op.id };
    }
    case 'resetSheets': {
      if (
        sheets.length === 1 &&
        sheets[0].id === INITIAL_SHEET_ID &&
        isEmpty(sheets[0])
      ) {
        return null;
      }
      return {
        sheets: [emptySheet(INITIAL_SHEET_ID)],
        current: INITIAL_SHEET_ID,
      };
    }
  }
}
