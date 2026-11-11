import {
  distanceToSegment,
  radiusAt,
  type Point,
  type Stroke,
  type StrokePoint,
} from './stroke';
import { newStrokeId } from './strokeId';

/** Diameter of the tracing eraser (CSS pixels) */
export const ERASER_SIZE = { min: 4, max: 100, initial: 24 } as const;

/** Point spacing when looking for cut points (CSS pixels). Smaller values make cuts match the eraser's edge more closely */
const CUT_STEP = 1;

function cross(o: Point, a: Point, b: Point): number {
  return (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
}

/** Distance between segments a–b and c–d (0 if they intersect) */
function distanceBetweenSegments(
  a: Point,
  b: Point,
  c: Point,
  d: Point,
): number {
  const crosses =
    cross(a, b, c) * cross(a, b, d) < 0 && cross(c, d, a) * cross(c, d, b) < 0;
  if (crosses) return 0;
  return Math.min(
    distanceToSegment(a, c, d),
    distanceToSegment(b, c, d),
    distanceToSegment(c, a, b),
    distanceToSegment(d, a, b),
  );
}

interface Bounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

// Finished strokes never change, so bounds can be reused per object
const boundsCache = new WeakMap<Stroke, Bounds>();

/** Area covered by a stroke's ink (including line width) */
function strokeBounds(stroke: Stroke): Bounds {
  let bounds = boundsCache.get(stroke);
  if (!bounds) {
    bounds = {
      left: Infinity,
      top: Infinity,
      right: -Infinity,
      bottom: -Infinity,
    };
    for (const { x, y, pressure } of stroke.points) {
      const r = radiusAt(pressure);
      bounds.left = Math.min(bounds.left, x - r);
      bounds.top = Math.min(bounds.top, y - r);
      bounds.right = Math.max(bounds.right, x + r);
      bounds.bottom = Math.max(bounds.bottom, y + r);
    }
    boundsCache.set(stroke, bounds);
  }
  return bounds;
}

/** A point marked with whether it was in the original stroke (not one added to look for cut points) */
interface MarkedPoint {
  point: StrokePoint;
  original: boolean;
}

function lerp(a: StrokePoint, b: StrokePoint, t: number): StrokePoint {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
    pressure: a.pressure + (b.pressure - a.pressure) * t,
  };
}

/**
 * Adds points every CUT_STEP only in segments near the eraser,
 * so strokes with widely spaced points (such as ones written by the AI) are still cut at the eraser's edge
 */
function densifyNear(
  points: readonly StrokePoint[],
  isNear: (a: StrokePoint, b: StrokePoint) => boolean,
): MarkedPoint[] {
  const marked: MarkedPoint[] = [];
  points.forEach((b, i) => {
    const a = points[i - 1];
    if (a && isNear(a, b)) {
      const steps = Math.ceil(Math.hypot(b.x - a.x, b.y - a.y) / CUT_STEP);
      for (let s = 1; s < steps; s++) {
        marked.push({ point: lerp(a, b, s / steps), original: false });
      }
    }
    marked.push({ point: b, original: true });
  });
  return marked;
}

/**
 * Turns a remaining run of points into a stroke. Added points lie on straight lines, so all but the cut ends are dropped
 */
function toPiece(run: readonly MarkedPoint[]): StrokePoint[] {
  return run
    .filter(({ original }, i) => original || i === 0 || i === run.length - 1)
    .map(({ point }) => point);
}

/**
 * Removes what an eraser of radius `radius` erases while moving from `from` to `to`,
 * and returns the remaining parts as strokes with new IDs. null if the eraser did not touch it (the stroke stays as is)
 */
export function eraseAlong(
  stroke: Stroke,
  from: Point,
  to: Point,
  radius: number,
  newId: () => string = newStrokeId,
): Stroke[] | null {
  const bounds = strokeBounds(stroke);
  if (
    bounds.right < Math.min(from.x, to.x) - radius ||
    bounds.left > Math.max(from.x, to.x) + radius ||
    bounds.bottom < Math.min(from.y, to.y) - radius ||
    bounds.top > Math.max(from.y, to.y) + radius
  ) {
    return null;
  }

  // Remove points whose ink (line width) overlaps the eraser. The ends of the remaining parts fall exactly on the eraser's edge
  const isErased = (p: StrokePoint) =>
    distanceToSegment(p, from, to) <= radius + radiusAt(p.pressure);
  const isNear = (a: StrokePoint, b: StrokePoint) =>
    distanceBetweenSegments(a, b, from, to) <=
    radius + radiusAt(Math.max(a.pressure, b.pressure));

  const marked = densifyNear(stroke.points, isNear);
  if (!marked.some(({ point }) => isErased(point))) return null;

  // Split into runs of points that are not erased. A run with only one point is a fragment at a cut, so drop it
  const runs: MarkedPoint[][] = [];
  let run: MarkedPoint[] = [];
  for (const m of marked) {
    if (!isErased(m.point)) {
      run.push(m);
      continue;
    }
    if (run.length >= 2) runs.push(run);
    run = [];
  }
  if (run.length >= 2) runs.push(run);

  // What remains keeps the author and color of the original stroke
  return runs.map((points) => ({
    ...stroke,
    id: newId(),
    points: toPiece(points),
  }));
}
