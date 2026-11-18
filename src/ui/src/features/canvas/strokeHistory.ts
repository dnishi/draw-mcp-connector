import {
  applyCanvasOp,
  applyCanvasOps,
  applySheetOp,
  type SheetOp,
  type StrokeOp,
} from '../sync/canvasOp';
import {
  type CanvasOp,
  type ImagePlacement,
  INITIAL_SHEET_ID,
  MAX_SHEETS,
  type Sheet,
} from '../sync/protocol';
import type { Stroke } from './stroke';

/** A stroke and its position in strokes (stacking order) */
interface PlacedStroke {
  stroke: Stroke;
  index: number;
}

/** An action taken on this page (the unit of undo and redo) */
type Operation =
  | { type: 'add'; stroke: Stroke }
  | {
      type: 'erase';
      /** Erased strokes and their positions before erasing. Undo puts them back in the same stacking order */
      erased: PlacedStroke[];
      /** Remaining parts (when erasing by tracing) and their positions after erasing */
      remaining: PlacedStroke[];
    }
  /** Adding, moving, resizing or deleting the image. before / after of null means "no image" */
  | {
      type: 'image';
      before: ImagePlacement | null;
      after: ImagePlacement | null;
    };

/**
 * IDs of strokes erased with the eraser, and what remains of them.
 * When erasing whole strokes, nothing remains; when erasing by tracing, the remaining parts as strokes
 */
export type ErasedPieces = ReadonlyMap<string, readonly Stroke[]>;

/** State of one sheet. Undo and redo history is per sheet */
export interface SheetState {
  id: string;
  strokes: readonly Stroke[];
  image: ImagePlacement | null;
  undoStack: readonly Operation[];
  redoStack: readonly Operation[];
}

/** An operation made on this page that the PC has not acked yet */
export interface PendingOp {
  seq: number;
  op: CanvasOp;
}

export interface StrokeHistoryState {
  /** In order */
  sheets: readonly SheetState[];
  /** ID of the current sheet */
  current: string;
  /** Operations made on this page that the PC has not acked yet, oldest first */
  pending: readonly PendingOp[];
  /** The next number to give. Starts at 1 */
  nextSeq: number;
}

/** The current sheet. `current` is always present in `sheets` */
export function currentSheet(state: StrokeHistoryState): SheetState {
  return state.sheets.find(({ id }) => id === state.current) ?? state.sheets[0];
}

/** Makes an empty sheet with the ID */
const emptySheet = (id: string): SheetState => ({
  id,
  strokes: [],
  image: null,
  undoStack: [],
  redoStack: [],
});

/** Whether the sheet is empty (no strokes and no image) */
const isEmptySheet = (sheet: SheetState) =>
  sheet.strokes.length === 0 && sheet.image === null;

export type StrokeHistoryAction =
  | { type: 'add'; stroke: Stroke }
  | { type: 'erase'; pieces: ErasedPieces }
  /** Adds, moves, resizes or deletes the image (null to delete) */
  | { type: 'image'; image: ImagePlacement | null }
  | { type: 'undo' }
  | { type: 'redo' }
  /** Wipes only the current sheet (removes its strokes and image, and empties its history) */
  | { type: 'clear' }
  /** Inserts an empty sheet right after the current one and switches to it */
  | { type: 'newSheet'; id: string }
  /** Changes the current sheet */
  | { type: 'selectSheet'; id: string }
  /** Removes the current sheet with its contents */
  | { type: 'removeSheet' }
  /** Removes all sheets and makes one empty INITIAL_SHEET_ID sheet the current sheet */
  | { type: 'resetSheets' }
  /** A change from the PC (such as the AI's writing). Not added to the undo history */
  | { type: 'remote'; op: CanvasOp }
  /** The whole canvas the PC sends when the connection is approved (the sheets and the current sheet's ID) */
  | {
      type: 'snapshot';
      sheets: readonly Sheet[];
      current: string;
    }
  /** The PC reports it handled (applied or ignored) the operation with this number.
      Removes the operations with this number or lower from pending */
  | { type: 'ack'; seq: number };

export type RemoteAction = Extract<
  StrokeHistoryAction,
  { type: 'remote' | 'snapshot' | 'ack' }
>;

export const initialStrokeHistory: StrokeHistoryState = {
  sheets: [emptySheet(INITIAL_SHEET_ID)],
  current: INITIAL_SHEET_ID,
  pending: [],
  nextSeq: 1,
};

/**
 * A change that removes `removed` and then inserts `inserted`.
 * inserted is sorted by index and holds positions after insertion, so inserting from the front lands each at its position
 */
function swap(
  sheet: string,
  removed: readonly PlacedStroke[],
  inserted: readonly PlacedStroke[],
): CanvasOp[] {
  const ops: CanvasOp[] = inserted.map(({ stroke, index }) => ({
    type: 'insert',
    sheet,
    stroke,
    index,
  }));
  if (removed.length > 0) {
    ops.unshift({
      type: 'remove',
      sheet,
      ids: removed.map(({ stroke }) => stroke.id),
    });
  }
  return ops;
}

const imageOps = (sheet: string, image: ImagePlacement | null): CanvasOp[] =>
  image
    ? [{ type: 'setImage', sheet, image }]
    : [{ type: 'removeImage', sheet }];

const samePlacement = (a: ImagePlacement | null, b: ImagePlacement | null) =>
  a === b ||
  (!!a &&
    !!b &&
    a.id === b.id &&
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height);

/** History without image operations (returns the same array if there is nothing to remove) */
const withoutImageOps = (stack: readonly Operation[]): readonly Operation[] =>
  stack.some(({ type }) => type === 'image')
    ? stack.filter(({ type }) => type !== 'image')
    : stack;

/**
 * When the image changed through another page's operation (via the PC), removes image operations from the history.
 * Writing back the placement from before an operation would overwrite the other page's operation and even restore an image it deleted,
 * so this page can no longer undo/redo it (stroke operations are handled by ID, so they remain).
 * Images of the removed operations drop out of retain and are discarded from the page's store
 */
const dropImageHistory = (sheet: SheetState): SheetState => ({
  ...sheet,
  undoStack: withoutImageOps(sheet.undoStack),
  redoStack: withoutImageOps(sheet.redoStack),
});

/** Actions other than image actions (they only cause stroke changes) */
type StrokeOperation = Exclude<Operation, { type: 'image' }>;

/** Changes made when doing an action */
function forward(
  strokes: readonly Stroke[],
  op: StrokeOperation,
  sheet: string,
): StrokeOp[];
function forward(
  strokes: readonly Stroke[],
  op: Operation,
  sheet: string,
): CanvasOp[];
function forward(
  strokes: readonly Stroke[],
  op: Operation,
  sheet: string,
): CanvasOp[] {
  if (op.type === 'add') {
    return [
      { type: 'insert', sheet, stroke: op.stroke, index: strokes.length },
    ];
  }
  if (op.type === 'image') {
    return imageOps(sheet, op.after);
  }
  return swap(sheet, op.erased, op.remaining);
}

/** Changes made when undoing an action */
function backward(op: StrokeOperation, sheet: string): StrokeOp[];
function backward(op: Operation, sheet: string): CanvasOp[];
function backward(op: Operation, sheet: string): CanvasOp[] {
  if (op.type === 'add') {
    return [{ type: 'remove', sheet, ids: [op.stroke.id] }];
  }
  if (op.type === 'image') {
    return imageOps(sheet, op.before);
  }
  return swap(sheet, op.remaining, op.erased);
}

function toOperation(
  strokes: readonly Stroke[],
  action: Extract<StrokeHistoryAction, { type: 'add' | 'erase' }>,
): StrokeOperation | null {
  if (action.type === 'add') {
    return action;
  }

  // Build the list with the remaining parts at the erased strokes' positions, and record each position
  const erased: PlacedStroke[] = [];
  const remaining: PlacedStroke[] = [];
  let length = 0;
  strokes.forEach((stroke, index) => {
    const pieces = action.pieces.get(stroke.id);
    if (!pieces) {
      length++;
      return;
    }
    erased.push({ stroke, index });
    for (const piece of pieces) {
      remaining.push({ stroke: piece, index: length++ });
    }
  });
  return erased.length > 0 ? { type: 'erase', erased, remaining } : null;
}

/** Sheet changes (addSheet / removeSheet / selectSheet / resetSheets) */
const isSheetOp = (op: CanvasOp): op is SheetOp =>
  op.type === 'addSheet' ||
  op.type === 'removeSheet' ||
  op.type === 'selectSheet' ||
  op.type === 'resetSheets';

/** Numbers ops in order, starting from `from` */
const numbered = (from: number, ops: readonly CanvasOp[]): PendingOp[] =>
  ops.map((op, i) => ({ seq: from + i, op }));

/** Replaces one sheet (pending is not changed) */
function replaceSheet(
  state: StrokeHistoryState,
  sheet: SheetState,
): StrokeHistoryState {
  return {
    ...state,
    sheets: state.sheets.map((s) => (s.id === sheet.id ? sheet : s)),
  };
}

/** Appends numbered ops to pending (operations this page sends to the PC) */
function appendOps(
  state: StrokeHistoryState,
  ops: readonly CanvasOp[],
): StrokeHistoryState {
  return {
    ...state,
    pending: [...state.pending, ...numbered(state.nextSeq, ops)],
    nextSeq: state.nextSeq + ops.length,
  };
}

/** Replaces the current sheet and appends ops, numbered, to pending */
function commit(
  state: StrokeHistoryState,
  sheet: SheetState,
  ops: readonly CanvasOp[],
): StrokeHistoryState {
  return appendOps(replaceSheet(state, sheet), ops);
}

/** Operations that change a sheet's contents (strokes and image) */
type ContentOp =
  StrokeOp | Extract<CanvasOp, { type: 'setImage' | 'removeImage' }>;

/**
 * Applies an operation to one sheet's contents (strokes and image). null if nothing changes.
 * The caller manages the undo/redo history
 */
function applyContentOp(sheet: SheetState, op: ContentOp): SheetState | null {
  if (op.type === 'setImage' || op.type === 'removeImage') {
    const image = op.type === 'setImage' ? op.image : null;
    // Nothing to do if the image did not change
    if (samePlacement(sheet.image, image)) {
      return null;
    }
    return { ...sheet, image };
  }
  const strokes = applyCanvasOp(sheet.strokes, op);
  // Nothing to do for the same stroke change (such as our own change coming back through the bridge)
  return strokes === sheet.strokes ? null : { ...sheet, strokes };
}

export function strokeHistoryReducer(
  state: StrokeHistoryState,
  action: StrokeHistoryAction,
): StrokeHistoryState {
  switch (action.type) {
    case 'add':
    case 'erase': {
      const sheet = currentSheet(state);
      const op = toOperation(sheet.strokes, action);
      if (!op) {
        return state;
      }
      const ops = forward(sheet.strokes, op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, ops),
          undoStack: [...sheet.undoStack, op],
          redoStack: [],
        },
        ops,
      );
    }
    case 'image': {
      const sheet = currentSheet(state);
      if (samePlacement(sheet.image, action.image)) {
        return state;
      }
      const op: Operation = {
        type: 'image',
        before: sheet.image,
        after: action.image,
      };
      return commit(
        state,
        {
          ...sheet,
          image: action.image,
          undoStack: [...sheet.undoStack, op],
          redoStack: [],
        },
        imageOps(sheet.id, action.image),
      );
    }
    case 'undo': {
      const sheet = currentSheet(state);
      const op = sheet.undoStack.at(-1);
      if (!op) {
        return state;
      }
      if (op.type === 'image') {
        return commit(
          state,
          {
            ...sheet,
            image: op.before,
            undoStack: sheet.undoStack.slice(0, -1),
            redoStack: [...sheet.redoStack, op],
          },
          imageOps(sheet.id, op.before),
        );
      }
      const ops = backward(op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, ops),
          undoStack: sheet.undoStack.slice(0, -1),
          redoStack: [...sheet.redoStack, op],
        },
        ops,
      );
    }
    case 'redo': {
      const sheet = currentSheet(state);
      const op = sheet.redoStack.at(-1);
      if (!op) {
        return state;
      }
      if (op.type === 'image') {
        return commit(
          state,
          {
            ...sheet,
            image: op.after,
            undoStack: [...sheet.undoStack, op],
            redoStack: sheet.redoStack.slice(0, -1),
          },
          imageOps(sheet.id, op.after),
        );
      }
      const ops = forward(sheet.strokes, op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, ops),
          undoStack: [...sheet.undoStack, op],
          redoStack: sheet.redoStack.slice(0, -1),
        },
        ops,
      );
    }
    case 'clear': {
      const sheet = currentSheet(state);
      const empty =
        sheet.strokes.length === 0 &&
        sheet.image === null &&
        sheet.undoStack.length === 0 &&
        sheet.redoStack.length === 0;
      if (empty) {
        return state;
      }
      const ops: CanvasOp[] =
        sheet.strokes.length > 0
          ? [
              {
                type: 'remove',
                sheet: sheet.id,
                ids: sheet.strokes.map(({ id }) => id),
              },
            ]
          : [];
      if (sheet.image) {
        ops.push({ type: 'removeImage', sheet: sheet.id });
      }
      return commit(
        state,
        {
          ...sheet,
          strokes: [],
          image: null,
          undoStack: [],
          redoStack: [],
        },
        ops,
      );
    }
    case 'newSheet': {
      if (state.sheets.length >= MAX_SHEETS) {
        return state;
      }
      // Insert an empty sheet right after the current one and switch to it
      const index =
        state.sheets.findIndex(({ id }) => id === state.current) + 1;
      const added = applySheetOp(
        state.sheets,
        state.current,
        { type: 'addSheet', id: action.id, index },
        emptySheet,
        isEmptySheet,
      );
      if (!added) {
        return state;
      }
      const selected = applySheetOp(
        added.sheets,
        state.current,
        { type: 'selectSheet', id: action.id },
        emptySheet,
        isEmptySheet,
      );
      if (!selected) {
        return state;
      }
      return appendOps(
        { ...state, sheets: selected.sheets, current: selected.current },
        [
          { type: 'addSheet', id: action.id, index },
          { type: 'selectSheet', id: action.id },
        ],
      );
    }
    case 'selectSheet': {
      const applied = applySheetOp(
        state.sheets,
        state.current,
        { type: 'selectSheet', id: action.id },
        emptySheet,
        isEmptySheet,
      );
      return applied
        ? appendOps({ ...state, current: applied.current }, [
            { type: 'selectSheet', id: action.id },
          ])
        : state;
    }
    case 'removeSheet': {
      const applied = applySheetOp(
        state.sheets,
        state.current,
        { type: 'removeSheet', id: state.current },
        emptySheet,
        isEmptySheet,
      );
      return applied
        ? appendOps(
            { ...state, sheets: applied.sheets, current: applied.current },
            [{ type: 'removeSheet', id: state.current }],
          )
        : state;
    }
    case 'resetSheets': {
      const applied = applySheetOp(
        state.sheets,
        state.current,
        { type: 'resetSheets' },
        emptySheet,
        isEmptySheet,
      );
      return applied
        ? appendOps(
            { ...state, sheets: applied.sheets, current: applied.current },
            [{ type: 'resetSheets' }],
          )
        : state;
    }
    case 'remote':
      return applyRemoteOp(state, action.op);
    case 'snapshot': {
      // Match the list, order and contents (strokes and images) of sheets and the current sheet to the PC.
      // If this page has a sheet with the same ID, keep its undo/redo history.
      // Sheets the PC does not have are dropped
      let sheets: readonly SheetState[] = action.sheets.map((remote) => ({
        ...(state.sheets.find(({ id }) => id === remote.id) ??
          emptySheet(remote.id)),
        strokes: remote.strokes,
        image: remote.image ?? null,
      }));
      let current = action.current;
      // On top of that, reapply the operations without an ack, oldest first.
      // Only the contents change (nothing is added to the history), and operations that cannot be applied are ignored
      for (const { op } of state.pending) {
        if (isSheetOp(op)) {
          const applied = applySheetOp(
            sheets,
            current,
            op,
            emptySheet,
            isEmptySheet,
          );
          if (applied) {
            sheets = applied.sheets;
            current = applied.current;
          }
          continue;
        }
        const index = sheets.findIndex(({ id }) => id === op.sheet);
        if (index < 0) {
          continue;
        }
        const next = applyContentOp(sheets[index], op);
        if (next) {
          sheets = sheets.with(index, next);
        }
      }
      // Drop the image history only on sheets whose image after reapplying differs from this page's original image
      return {
        sheets: sheets.map((sheet) => {
          const original = state.sheets.find(({ id }) => id === sheet.id);
          return original && !samePlacement(original.image, sheet.image)
            ? dropImageHistory(sheet)
            : sheet;
        }),
        current,
        // Keep pending and nextSeq as they are (the sync side resends them)
        pending: state.pending,
        nextSeq: state.nextSeq,
      };
    }
    case 'ack': {
      const pending = state.pending.filter(({ seq }) => seq > action.seq);
      // Nothing changes when there is nothing to remove
      return pending.length === state.pending.length
        ? state
        : { ...state, pending };
    }
  }
}

/** Applies a change from the PC (not added to the undo history).
    Changes to an unknown sheet are ignored. pending is not changed */
function applyRemoteOp(
  state: StrokeHistoryState,
  op: CanvasOp,
): StrokeHistoryState {
  if (isSheetOp(op)) {
    const applied = applySheetOp(
      state.sheets,
      state.current,
      op,
      emptySheet,
      isEmptySheet,
    );
    return applied
      ? { ...state, sheets: applied.sheets, current: applied.current }
      : state;
  }
  const sheet = state.sheets.find(({ id }) => id === op.sheet);
  if (!sheet) {
    return state;
  }
  const next = applyContentOp(sheet, op);
  if (!next) {
    return state;
  }
  // Only when the image changed, remove the image history of that sheet
  const imageChanged = op.type === 'setImage' || op.type === 'removeImage';
  return replaceSheet(state, imageChanged ? dropImageHistory(next) : next);
}
