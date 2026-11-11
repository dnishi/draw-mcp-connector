import { applyCanvasOps } from '../sync/canvasOp';
import type { CanvasOp } from '../sync/protocol';
import type { Stroke } from './stroke';

/** A stroke and its position in strokes (stacking order) */
interface PlacedStroke {
  stroke: Stroke;
  index: number;
}

/** An action done on this canvas page (the unit of undo and redo) */
type Operation =
  | { type: 'add'; stroke: Stroke }
  | {
      type: 'erase';
      /** Erased strokes and their positions before erasing. Undo puts them back in the same stacking order */
      erased: PlacedStroke[];
      /** Remaining parts (when erasing by tracing) and their positions after erasing */
      remaining: PlacedStroke[];
    };

/**
 * IDs of strokes erased with the eraser, and what remains of them.
 * When erasing whole strokes, nothing remains; when erasing by tracing, the remaining parts as strokes
 */
export type ErasedPieces = ReadonlyMap<string, readonly Stroke[]>;

export interface StrokeHistoryState {
  strokes: readonly Stroke[];
  undoStack: readonly Operation[];
  redoStack: readonly Operation[];
  /** Changes this canvas page made in the last action. Sent to the PC (bridge) */
  outgoing: readonly CanvasOp[];
}

export type StrokeHistoryAction =
  | { type: 'add'; stroke: Stroke }
  | { type: 'erase'; pieces: ErasedPieces }
  | { type: 'undo' }
  | { type: 'redo' }
  /** Removes every stroke and empties the undo/redo history (a blank initial state) */
  | { type: 'clear' }
  /** Changes from the PC (such as AI writes). Not added to the undo history */
  | { type: 'remote'; op: CanvasOp }
  /** The whole canvas sent by the PC when the connection is approved */
  | { type: 'snapshot'; strokes: readonly Stroke[] };

export type RemoteAction = Extract<
  StrokeHistoryAction,
  { type: 'remote' | 'snapshot' }
>;

const NO_OPS: readonly CanvasOp[] = [];

export const initialStrokeHistory: StrokeHistoryState = {
  strokes: [],
  undoStack: [],
  redoStack: [],
  outgoing: NO_OPS,
};

/**
 * A change that removes `removed` and then inserts `inserted`.
 * inserted is sorted by index and holds positions after insertion, so inserting from the front lands each at its position
 */
function swap(
  removed: readonly PlacedStroke[],
  inserted: readonly PlacedStroke[],
): CanvasOp[] {
  const ops: CanvasOp[] = inserted.map(({ stroke, index }) => ({
    type: 'insert',
    stroke,
    index,
  }));
  if (removed.length > 0) {
    ops.unshift({
      type: 'remove',
      ids: removed.map(({ stroke }) => stroke.id),
    });
  }
  return ops;
}

/** Changes for performing an action */
function forward(strokes: readonly Stroke[], op: Operation): CanvasOp[] {
  if (op.type === 'add') {
    return [{ type: 'insert', stroke: op.stroke, index: strokes.length }];
  }
  return swap(op.erased, op.remaining);
}

/** Changes for undoing an action */
function backward(op: Operation): CanvasOp[] {
  if (op.type === 'add') return [{ type: 'remove', ids: [op.stroke.id] }];
  return swap(op.remaining, op.erased);
}

function toOperation(
  strokes: readonly Stroke[],
  action: Extract<StrokeHistoryAction, { type: 'add' | 'erase' }>,
): Operation | null {
  if (action.type === 'add') return action;

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
    for (const piece of pieces)
      remaining.push({ stroke: piece, index: length++ });
  });
  return erased.length > 0 ? { type: 'erase', erased, remaining } : null;
}

/** Applies the changes and keeps them in outgoing as changes to send to the PC */
function applyLocal(
  state: StrokeHistoryState,
  ops: readonly CanvasOp[],
  stacks: Pick<StrokeHistoryState, 'undoStack' | 'redoStack'>,
): StrokeHistoryState {
  return {
    ...stacks,
    strokes: applyCanvasOps(state.strokes, ops),
    outgoing: ops,
  };
}

/**
 * Adopts the PC's whole canvas. Strokes only this canvas page has (such as ones written before the PC restarted) are kept
 * and sent back to the PC
 */
function mergeSnapshot(
  state: StrokeHistoryState,
  remote: readonly Stroke[],
): StrokeHistoryState {
  const remoteIds = new Set(remote.map(({ id }) => id));
  const localOnly = state.strokes.filter(({ id }) => !remoteIds.has(id));
  const outgoing = localOnly.map((stroke, i): CanvasOp => ({
    type: 'insert',
    stroke,
    index: remote.length + i,
  }));
  return {
    ...state,
    strokes: [...remote, ...localOnly],
    outgoing: outgoing.length > 0 ? outgoing : NO_OPS,
  };
}

export function strokeHistoryReducer(
  state: StrokeHistoryState,
  action: StrokeHistoryAction,
): StrokeHistoryState {
  switch (action.type) {
    case 'add':
    case 'erase': {
      const op = toOperation(state.strokes, action);
      if (!op) return state;
      return applyLocal(state, forward(state.strokes, op), {
        undoStack: [...state.undoStack, op],
        redoStack: [],
      });
    }
    case 'undo': {
      const op = state.undoStack.at(-1);
      if (!op) return state;
      return applyLocal(state, backward(op), {
        undoStack: state.undoStack.slice(0, -1),
        redoStack: [...state.redoStack, op],
      });
    }
    case 'redo': {
      const op = state.redoStack.at(-1);
      if (!op) return state;
      return applyLocal(state, forward(state.strokes, op), {
        undoStack: [...state.undoStack, op],
        redoStack: state.redoStack.slice(0, -1),
      });
    }
    case 'clear': {
      const empty =
        state.strokes.length === 0 &&
        state.undoStack.length === 0 &&
        state.redoStack.length === 0;
      if (empty) return state;
      const ids = state.strokes.map(({ id }) => id);
      return {
        strokes: [],
        undoStack: [],
        redoStack: [],
        outgoing: ids.length > 0 ? [{ type: 'remove', ids }] : NO_OPS,
      };
    }
    case 'remote':
      return {
        ...state,
        strokes: applyCanvasOps(state.strokes, [action.op]),
        outgoing: NO_OPS,
      };
    case 'snapshot':
      return mergeSnapshot(state, action.strokes);
  }
}
