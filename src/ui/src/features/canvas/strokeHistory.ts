import {
  applyCanvasOp,
  applyCanvasOps,
  applySheetOp,
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

export interface StrokeHistoryState {
  /** In order */
  sheets: readonly SheetState[];
  /** ID of the current sheet */
  current: string;
  /** Changes this page made in the last action. Sent to the PC */
  outgoing: readonly CanvasOp[];
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
  /** The whole canvas sent by bridges up to v0.3.0 (only the current sheet's contents) */
  | {
      type: 'legacySnapshot';
      strokes: readonly Stroke[];
      image?: ImagePlacement | null;
    }
  /** A change from bridges up to v0.3.0 (without sheet). Treated as a change to the current sheet */
  | { type: 'legacyRemote'; op: LegacyCanvasOp };

/** Stroke and image changes without `sheet` (bridges up to v0.3.0) */
export type LegacyCanvasOp = LegacyOf<
  Extract<
    CanvasOp,
    {
      type: 'insert' | 'remove' | 'replace' | 'setImage' | 'removeImage';
    }
  >
>;
type LegacyOf<T> = T extends CanvasOp ? Omit<T, 'sheet'> : never;

export type RemoteAction = Extract<
  StrokeHistoryAction,
  { type: 'remote' | 'snapshot' | 'legacyRemote' | 'legacySnapshot' }
>;

const NO_OPS: readonly CanvasOp[] = [];

export const initialStrokeHistory: StrokeHistoryState = {
  sheets: [emptySheet(INITIAL_SHEET_ID)],
  current: INITIAL_SHEET_ID,
  outgoing: NO_OPS,
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

/** Replaces the current sheet and leaves the changes to send to the PC in outgoing */
function commit(
  state: StrokeHistoryState,
  sheet: SheetState,
  outgoing: readonly CanvasOp[],
): StrokeHistoryState {
  return {
    ...state,
    sheets: state.sheets.map((s) => (s.id === sheet.id ? sheet : s)),
    outgoing,
  };
}

/**
 * Makes one sheet follow the PC's contents. Strokes only this page has (such as ones written before the PC
 * restarted) are kept and sent back to the PC. The PC's image wins, but an image only this page has
 * (after the PC restarted, for example) is sent back. history is kept as is (only when the PC's image is
 * adopted and the image changes are the image actions removed)
 */
function mergeSheetContents(
  sheet: SheetState,
  remoteStrokes: readonly Stroke[],
  remoteImage: ImagePlacement | null,
): { sheet: SheetState; outgoing: CanvasOp[] } {
  const remoteIds = new Set(remoteStrokes.map(({ id }) => id));
  const localOnly = sheet.strokes.filter(({ id }) => !remoteIds.has(id));
  const outgoing: CanvasOp[] = localOnly.map((stroke, i): CanvasOp => ({
    type: 'insert',
    sheet: sheet.id,
    stroke,
    index: remoteStrokes.length + i,
  }));
  const image = remoteImage ?? sheet.image;
  if (!remoteImage && sheet.image) {
    outgoing.push({ type: 'setImage', sheet: sheet.id, image: sheet.image });
  }
  const next: SheetState = {
    ...sheet,
    strokes: [...remoteStrokes, ...localOnly],
    image,
  };
  return {
    sheet:
      remoteImage && !samePlacement(sheet.image, remoteImage)
        ? dropImageHistory(next)
        : next,
    outgoing,
  };
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
      const outgoing = forward(sheet.strokes, op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, outgoing),
          undoStack: [...sheet.undoStack, op],
          redoStack: [],
        },
        outgoing,
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
      const outgoing = backward(op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, outgoing),
          undoStack: sheet.undoStack.slice(0, -1),
          redoStack: [...sheet.redoStack, op],
        },
        outgoing,
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
      const outgoing = forward(sheet.strokes, op, sheet.id);
      return commit(
        state,
        {
          ...sheet,
          strokes: applyCanvasOps(sheet.strokes, outgoing),
          undoStack: [...sheet.undoStack, op],
          redoStack: sheet.redoStack.slice(0, -1),
        },
        outgoing,
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
      const outgoing: CanvasOp[] =
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
        outgoing.push({ type: 'removeImage', sheet: sheet.id });
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
        outgoing,
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
      return {
        ...state,
        sheets: selected.sheets,
        current: selected.current,
        outgoing: [
          { type: 'addSheet', id: action.id, index },
          { type: 'selectSheet', id: action.id },
        ],
      };
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
        ? {
            ...state,
            current: applied.current,
            outgoing: [{ type: 'selectSheet', id: action.id }],
          }
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
        ? {
            ...state,
            sheets: applied.sheets,
            current: applied.current,
            outgoing: [{ type: 'removeSheet', id: state.current }],
          }
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
        ? {
            ...state,
            sheets: applied.sheets,
            current: applied.current,
            outgoing: [{ type: 'resetSheets' }],
          }
        : state;
    }
    case 'remote':
      return applyRemoteOp(state, action.op);
    case 'snapshot': {
      const outgoing: CanvasOp[] = [];
      const sheets: SheetState[] = [];
      // Sheets the PC also has follow the PC's order and contents
      for (const remote of action.sheets) {
        const local = state.sheets.find(({ id }) => id === remote.id);
        if (!local) {
          // A sheet this page does not have (made on another page, for example) takes the PC's contents as is
          sheets.push({
            ...emptySheet(remote.id),
            strokes: remote.strokes,
            image: remote.image ?? null,
          });
          continue;
        }
        const merged = mergeSheetContents(
          local,
          remote.strokes,
          remote.image ?? null,
        );
        sheets.push(merged.sheet);
        outgoing.push(...merged.outgoing);
      }
      // Sheets only this page has: kept at the end and sent back if they have contents (strokes or an image); discarded if empty
      let room = MAX_SHEETS - action.sheets.length;
      for (const local of state.sheets) {
        if (action.sheets.some(({ id }) => id === local.id) || room <= 0) {
          continue;
        }
        if (isEmptySheet(local)) {
          continue;
        }
        const merged = mergeSheetContents(local, [], null);
        outgoing.push({ type: 'addSheet', id: local.id, index: sheets.length });
        outgoing.push(...merged.outgoing);
        sheets.push(merged.sheet);
        room--;
      }
      return {
        sheets,
        current: action.current,
        outgoing: outgoing.length > 0 ? outgoing : NO_OPS,
      };
    }
    case 'legacySnapshot': {
      const sheet = currentSheet(state);
      const merged = mergeSheetContents(
        sheet,
        action.strokes,
        action.image ?? null,
      );
      return commit(state, merged.sheet, merged.outgoing);
    }
    case 'legacyRemote': {
      // Changes from old bridges without sheet are treated as changes to the current sheet
      return applyRemoteOp(state, { ...action.op, sheet: state.current });
    }
  }
}

/** Applies a change from the PC (not added to the undo history). Changes to missing sheets are ignored */
function applyRemoteOp(
  state: StrokeHistoryState,
  op: CanvasOp,
): StrokeHistoryState {
  if (
    op.type === 'addSheet' ||
    op.type === 'removeSheet' ||
    op.type === 'selectSheet' ||
    op.type === 'resetSheets'
  ) {
    const applied = applySheetOp(
      state.sheets,
      state.current,
      op,
      emptySheet,
      isEmptySheet,
    );
    return applied
      ? {
          ...state,
          sheets: applied.sheets,
          current: applied.current,
          outgoing: NO_OPS,
        }
      : state;
  }
  const sheet = state.sheets.find(({ id }) => id === op.sheet);
  if (!sheet) {
    return state;
  }
  if (op.type === 'setImage' || op.type === 'removeImage') {
    const image = op.type === 'setImage' ? op.image : null;
    // Nothing to do if the image did not change
    if (samePlacement(sheet.image, image)) {
      return state;
    }
    // Only when the image changed, remove the image history of that sheet
    return replaceSheet(state, dropImageHistory({ ...sheet, image }));
  }
  const strokes = applyCanvasOp(sheet.strokes, op);
  // Nothing to do for the same stroke change (such as our own change coming back through the bridge)
  if (strokes === sheet.strokes) {
    return state;
  }
  return replaceSheet(state, { ...sheet, strokes });
}

/** Replaces one sheet (outgoing is not sent) */
function replaceSheet(
  state: StrokeHistoryState,
  sheet: SheetState,
): StrokeHistoryState {
  return {
    ...state,
    sheets: state.sheets.map((s) => (s.id === sheet.id ? sheet : s)),
    outgoing: NO_OPS,
  };
}
