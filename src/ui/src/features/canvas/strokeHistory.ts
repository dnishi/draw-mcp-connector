import { applyCanvasOps } from '../sync/canvasOp';
import type { CanvasOp, ImagePlacement } from '../sync/protocol';
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

export interface StrokeHistoryState {
  strokes: readonly Stroke[];
  /** The image laid under the strokes (one at most for now) */
  image: ImagePlacement | null;
  undoStack: readonly Operation[];
  redoStack: readonly Operation[];
  /** Changes this canvas page made in the last action. Sent to the PC (bridge) */
  outgoing: readonly CanvasOp[];
}

export type StrokeHistoryAction =
  | { type: 'add'; stroke: Stroke }
  | { type: 'erase'; pieces: ErasedPieces }
  /** Adds, moves, resizes or deletes the image (null to delete) */
  | { type: 'image'; image: ImagePlacement | null }
  | { type: 'undo' }
  | { type: 'redo' }
  /** Erases all strokes and the image and empties the undo/redo history too (a clean initial state) */
  | { type: 'clear' }
  /** Changes from the PC (such as AI writes). Not added to the undo history */
  | { type: 'remote'; op: CanvasOp }
  /** The whole canvas that arrives from the PC when the connection is approved */
  | {
      type: 'snapshot';
      strokes: readonly Stroke[];
      image?: ImagePlacement | null;
    };

export type RemoteAction = Extract<
  StrokeHistoryAction,
  { type: 'remote' | 'snapshot' }
>;

const NO_OPS: readonly CanvasOp[] = [];

export const initialStrokeHistory: StrokeHistoryState = {
  strokes: [],
  image: null,
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

const imageOps = (image: ImagePlacement | null): CanvasOp[] =>
  image ? [{ type: 'setImage', image }] : [{ type: 'removeImage' }];

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
function dropImageHistory(state: StrokeHistoryState): StrokeHistoryState {
  return {
    ...state,
    undoStack: withoutImageOps(state.undoStack),
    redoStack: withoutImageOps(state.redoStack),
  };
}

/** Changes when performing an operation */
function forward(strokes: readonly Stroke[], op: Operation): CanvasOp[] {
  if (op.type === 'add') {
    return [{ type: 'insert', stroke: op.stroke, index: strokes.length }];
  }
  if (op.type === 'image') {
    return imageOps(op.after);
  }
  return swap(op.erased, op.remaining);
}

/** Changes for undoing an action */
function backward(op: Operation): CanvasOp[] {
  if (op.type === 'add') {
    return [{ type: 'remove', ids: [op.stroke.id] }];
  }
  if (op.type === 'image') {
    return imageOps(op.before);
  }
  return swap(op.remaining, op.erased);
}

function toOperation(
  strokes: readonly Stroke[],
  action: Extract<StrokeHistoryAction, { type: 'add' | 'erase' }>,
): Operation | null {
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

/** Applies the changes and keeps them in outgoing as changes to send to the PC. Image changes go into the image slot */
function applyLocal(
  state: StrokeHistoryState,
  ops: readonly CanvasOp[],
  stacks: Pick<StrokeHistoryState, 'undoStack' | 'redoStack'>,
  image: ImagePlacement | null = state.image,
): StrokeHistoryState {
  return {
    ...stacks,
    strokes: applyCanvasOps(state.strokes, ops),
    image,
    outgoing: ops,
  };
}

/**
 * Matches the whole canvas on the PC. Strokes only this iPad has (such as ones written before the PC restarted) are kept,
 * and sent back to the PC. The bridge's image wins, but one the page has and the bridge does not
 * is sent back (since the bridge has restarted)
 */
function mergeSnapshot(
  state: StrokeHistoryState,
  remote: readonly Stroke[],
  remoteImage: ImagePlacement | null | undefined,
): StrokeHistoryState {
  const remoteIds = new Set(remote.map(({ id }) => id));
  const localOnly = state.strokes.filter(({ id }) => !remoteIds.has(id));
  const outgoing: CanvasOp[] = localOnly.map((stroke, i): CanvasOp => ({
    type: 'insert',
    stroke,
    index: remote.length + i,
  }));
  const image = remoteImage ?? state.image;
  if (!remoteImage && state.image) {
    outgoing.push({ type: 'setImage', image: state.image });
  }
  const next: StrokeHistoryState = {
    ...state,
    strokes: [...remote, ...localOnly],
    image,
    outgoing: outgoing.length > 0 ? outgoing : NO_OPS,
  };
  // If the PC's image is adopted and differs from the current one, remove image operations from the history.
  // If the PC has no image and its own is kept as is, the image does not change, so the history is kept too
  return remoteImage && !samePlacement(state.image, remoteImage)
    ? dropImageHistory(next)
    : next;
}

export function strokeHistoryReducer(
  state: StrokeHistoryState,
  action: StrokeHistoryAction,
): StrokeHistoryState {
  switch (action.type) {
    case 'add':
    case 'erase': {
      const op = toOperation(state.strokes, action);
      if (!op) {
        return state;
      }
      return applyLocal(state, forward(state.strokes, op), {
        undoStack: [...state.undoStack, op],
        redoStack: [],
      });
    }
    case 'image': {
      if (samePlacement(state.image, action.image)) {
        return state;
      }
      const op: Operation = {
        type: 'image',
        before: state.image,
        after: action.image,
      };
      return applyLocal(
        state,
        forward(state.strokes, op),
        {
          undoStack: [...state.undoStack, op],
          redoStack: [],
        },
        action.image,
      );
    }
    case 'undo': {
      const op = state.undoStack.at(-1);
      if (!op) {
        return state;
      }
      return applyLocal(
        state,
        backward(op),
        {
          undoStack: state.undoStack.slice(0, -1),
          redoStack: [...state.redoStack, op],
        },
        op.type === 'image' ? op.before : state.image,
      );
    }
    case 'redo': {
      const op = state.redoStack.at(-1);
      if (!op) {
        return state;
      }
      return applyLocal(
        state,
        forward(state.strokes, op),
        {
          undoStack: [...state.undoStack, op],
          redoStack: state.redoStack.slice(0, -1),
        },
        op.type === 'image' ? op.after : state.image,
      );
    }
    case 'clear': {
      const empty =
        state.strokes.length === 0 &&
        state.image === null &&
        state.undoStack.length === 0 &&
        state.redoStack.length === 0;
      if (empty) {
        return state;
      }
      const ids = state.strokes.map(({ id }) => id);
      const ops: CanvasOp[] = ids.length > 0 ? [{ type: 'remove', ids }] : [];
      if (state.image) {
        ops.push({ type: 'removeImage' });
      }
      return {
        strokes: [],
        image: null,
        undoStack: [],
        redoStack: [],
        outgoing: ops.length > 0 ? ops : NO_OPS,
      };
    }
    case 'remote': {
      const { op } = action;
      if (op.type === 'setImage' || op.type === 'removeImage') {
        const image = op.type === 'setImage' ? op.image : null;
        const next: StrokeHistoryState = {
          ...state,
          image,
          outgoing: NO_OPS,
        };
        // When the image changed through another page's operation, remove image operations from the history
        return samePlacement(state.image, image)
          ? next
          : dropImageHistory(next);
      }
      return {
        ...state,
        strokes: applyCanvasOps(state.strokes, [op]),
        outgoing: NO_OPS,
      };
    }
    case 'snapshot':
      return mergeSnapshot(state, action.strokes, action.image);
  }
}
