import type { Stroke, StrokePoint } from '../sync/protocol';

// Stroke types use the definitions shared with the PC (bridge). Pressure is PointerEvent.pressure (0 to 1)
export type { Stroke, StrokePoint };

/** Line radius at pressure 1 (CSS pixels) */
const PEN_MAX_RADIUS = 3;
/** Lower bound so lines do not vanish when pressure is near 0 */
const MIN_PRESSURE = 0.25;

export function radiusAt(pressure: number): number {
  return PEN_MAX_RADIUS * Math.max(MIN_PRESSURE, pressure);
}

function lerp(a: StrokePoint, b: StrokePoint, t: number): StrokePoint {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
    pressure: a.pressure + (b.pressure - a.pressure) * t,
  };
}

/** Rounds corners once with Chaikin's algorithm. The first and last points do not move */
export function smoothPoints(points: readonly StrokePoint[]): StrokePoint[] {
  if (points.length < 3) return [...points];

  const smoothed = [points[0]];
  for (let i = 0; i < points.length - 1; i++) {
    smoothed.push(
      lerp(points[i], points[i + 1], 0.25),
      lerp(points[i], points[i + 1], 0.75),
    );
  }
  smoothed.push(points[points.length - 1]);
  return smoothed;
}

export type Point = Pick<StrokePoint, 'x' | 'y'>;

/** Distance from point p to segment a–b */
export function distanceToSegment(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSquared = dx * dx + dy * dy;
  const t =
    lengthSquared === 0
      ? 0
      : Math.min(
          1,
          Math.max(0, ((p.x - a.x) * dx + (p.y - a.y) * dy) / lengthSquared),
        );
  return Math.hypot(p.x - (a.x + dx * t), p.y - (a.y + dy * t));
}

/** Whether a circle of radius `radius` (the eraser) touches the stroke */
export function hitsStroke(
  stroke: Stroke,
  center: StrokePoint,
  radius: number,
): boolean {
  const { points } = stroke;
  return points.some((a, i) => {
    const b = points[i + 1] ?? a;
    const strokeRadius = radiusAt(Math.max(a.pressure, b.pressure));
    return distanceToSegment(center, a, b) <= radius + strokeRadius;
  });
}
