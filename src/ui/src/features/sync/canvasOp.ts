import type { CanvasOp, Stroke } from './protocol';

/**
 * Applies one change to the list of strokes, following the same rules as the bridge (src/bridge/src/canvas.rs):
 * - insert: inserts at position index (appends if out of range). Does nothing if the same ID already exists
 * - remove: ignores unknown IDs
 * - replace: replaces the stroke with the same ID, keeping its stacking order. Does nothing if it is missing
 * Returns the given array as is if nothing changed
 */
export function applyCanvasOp(
  strokes: readonly Stroke[],
  op: CanvasOp,
): readonly Stroke[] {
  switch (op.type) {
    case 'insert': {
      if (strokes.some(({ id }) => id === op.stroke.id)) return strokes;
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
      if (index < 0) return strokes;
      return strokes.with(index, op.stroke);
    }
  }
}

export function applyCanvasOps(
  strokes: readonly Stroke[],
  ops: readonly CanvasOp[],
): readonly Stroke[] {
  return ops.reduce(applyCanvasOp, strokes);
}
