import type { CanvasOp, Stroke } from './protocol';

/**
 * Applies one change to the list of strokes. Follows the same rules as the bridge (src/bridge/src/canvas.rs):
 * - insert: inserts at index (at the end if out of range). Does nothing if the same ID already exists
 * - remove: ignores IDs that do not exist
 * - replace: replaces the stroke with the same ID, keeping its stacking order. Does nothing if there is none
 * - setImage / removeImage: change the image slot rather than strokes, so nothing happens here
 *   (the strokeHistory.ts reducer applies them)
 * If nothing changes, returns the received array as is
 */
export function applyCanvasOp(
  strokes: readonly Stroke[],
  op: CanvasOp,
): readonly Stroke[] {
  switch (op.type) {
    case 'setImage':
    case 'removeImage':
      return strokes;
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
  ops: readonly CanvasOp[],
): readonly Stroke[] {
  return ops.reduce(applyCanvasOp, strokes);
}
