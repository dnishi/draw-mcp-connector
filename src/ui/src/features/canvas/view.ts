import type { PageSize, Viewport } from '../sync/protocol';
import type { Point } from './stroke';

/**
 * Range of the zoom factor (1 is 100%). The canvas (paper) is the same size as the view, so it never goes below 100%
 */
export const SCALE_RANGE = { min: 1, max: 8 } as const;

/**
 * Mapping between the view (the canvas element) and the canvas. The view's top left is canvas (originX, originY),
 * and one canvas CSS pixel is `scale` CSS pixels on the view
 */
export interface View {
  originX: number;
  originY: number;
  scale: number;
}

export const INITIAL_VIEW: View = { originX: 0, originY: 0, scale: 1 };

export function isInitialView(view: View): boolean {
  return (
    view.originX === INITIAL_VIEW.originX &&
    view.originY === INITIAL_VIEW.originY &&
    view.scale === INITIAL_VIEW.scale
  );
}

/** Converts a point on the view (origin at the canvas element's top left) to canvas coordinates */
export function toCanvasPoint(view: View, screen: Point): Point {
  return {
    x: view.originX + screen.x / view.scale,
    y: view.originY + screen.y / view.scale,
  };
}

/** Converts a canvas point to a point on the view */
export function toScreenPoint(view: View, canvas: Point): Point {
  return {
    x: (canvas.x - view.originX) * view.scale,
    y: (canvas.y - view.originY) * view.scale,
  };
}

/** Area of the canvas visible in a view of width × height (CSS pixels) */
export function visibleRange(
  view: View,
  width: number,
  height: number,
): Required<Viewport> {
  return {
    x: view.originX,
    y: view.originY,
    width: width / view.scale,
    height: height / view.scale,
  };
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/** Clamps the zoom factor and keeps the visible area inside the paper (page, starting at (0, 0)) */
export function clampView(view: View, page: PageSize): View {
  const scale = clamp(view.scale, SCALE_RANGE.min, SCALE_RANGE.max);
  // The view is the same size as the paper, so the visible width is page.width / scale
  return {
    originX: clamp(view.originX, 0, page.width - page.width / scale),
    originY: clamp(view.originY, 0, page.height - page.height / scale),
    scale,
  };
}

const midpoint = (a: Point, b: Point): Point => ({
  x: (a.x + b.x) / 2,
  y: (a.y + b.y) / 2,
});
const distance = (a: Point, b: Point) => Math.hypot(b.x - a.x, b.y - a.y);

export type FingerPair = readonly [Point, Point];

/**
 * View after two fingers move from before to after.
 * Zooms by the ratio of the distance between the fingers, keeping the canvas point under the fingers' midpoint under the new midpoint
 */
export function pinchView(
  view: View,
  before: FingerPair,
  after: FingerPair,
): View {
  const spreadBefore = distance(...before);
  const ratio = spreadBefore > 0 ? distance(...after) / spreadBefore : 1;
  const scale = clamp(view.scale * ratio, SCALE_RANGE.min, SCALE_RANGE.max);
  const anchor = toCanvasPoint(view, midpoint(...before));
  const center = midpoint(...after);
  return {
    originX: anchor.x - center.x / scale,
    originY: anchor.y - center.y / scale,
    scale,
  };
}

/** Tracks fingers touching the screen and, once there are two or more, reports the movement of the first two as a pinch */
export class PinchTracker {
  readonly #fingers = new Map<number, Point>();

  /** Number of fingers touching the screen */
  get count(): number {
    return this.#fingers.size;
  }

  /** Positions of the first two fingers (null if fewer than two) */
  #pair(): FingerPair | null {
    const [a, b] = this.#fingers.values();
    return a && b ? [a, b] : null;
  }

  down(pointerId: number, screen: Point): void {
    this.#fingers.set(pointerId, screen);
  }

  /** A finger moved. During a pinch, returns the two positions before and after the move */
  move(
    pointerId: number,
    screen: Point,
  ): { before: FingerPair; after: FingerPair } | null {
    if (!this.#fingers.has(pointerId)) return null;
    const before = this.#pair();
    this.#fingers.set(pointerId, screen);
    const after = this.#pair();
    return before && after ? { before, after } : null;
  }

  /** A finger lifted. true if this ended the pinch */
  up(pointerId: number): boolean {
    const wasPinching = this.#fingers.size >= 2;
    this.#fingers.delete(pointerId);
    return wasPinching && this.#fingers.size < 2;
  }
}
