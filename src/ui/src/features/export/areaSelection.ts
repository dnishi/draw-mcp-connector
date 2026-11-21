import type { Point } from '../canvas/stroke';
import type { Area } from './exportFile';

/**
 * The size that counts as hitting the corner handles (screen CSS pixels, one side of the square).
 * It is 24 so that a finger can grab it (the bridge's area_select.rs is 10, for the mouse)
 */
export const HANDLE_SIZE = 24;

/**
 * The margin around the sheet while choosing the area. The same as the handle size `HANDLE_SIZE`,
 * so the handles on the edges fit in the margin
 */
export const EXPORT_MARGIN = 24;

/** A corner of the area */
export type Corner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';

/** The mouse shape over a corner handle */
export const CORNER_CURSOR: Record<Corner, string> = {
  'top-left': 'nwse-resize',
  'top-right': 'nesw-resize',
  'bottom-left': 'nesw-resize',
  'bottom-right': 'nwse-resize',
};

/** The sheet size (CSS pixels) */
export type Page = { width: number; height: number };

/**
 * The scale and position of the sheet while choosing the area. It shrinks the sheet without
 * changing its aspect ratio and centers it in the canvas area, leaving a margin on all four
 * sides. The margin is `EXPORT_MARGIN` in the direction that shrinks more, and at least that in
 * the other (display only; the area coordinates and the exported content do not change)
 */
export function areaSelectionView(page: Page): {
  scale: number;
  x: number;
  y: number;
} {
  // When the sheet's width or height is no more than two margins (including while it is 0×0),
  // there is no room for a margin, so do not shrink
  if (page.width <= 2 * EXPORT_MARGIN || page.height <= 2 * EXPORT_MARGIN) {
    return { scale: 1, x: 0, y: 0 };
  }
  const scale = Math.min(
    (page.width - 2 * EXPORT_MARGIN) / page.width,
    (page.height - 2 * EXPORT_MARGIN) / page.height,
  );
  return {
    scale,
    x: (page.width - page.width * scale) / 2,
    y: (page.height - page.height * scale) / 2,
  };
}

/** What is under the pressed position (canvas coordinates) */
export type AreaHit =
  { kind: 'resize'; corner: Corner } | { kind: 'move' } | { kind: 'new' };

/** A drag in progress (the position and area when pressed) */
export type AreaDrag =
  | { kind: 'resize'; corner: Corner; start: Area; startPoint: Point }
  | { kind: 'move'; start: Area; startPoint: Point }
  | { kind: 'new'; start: Point };

/**
 * What is under the pressed position (canvas coordinates). Given the view scale, the size that
 * hits a handle matches `HANDLE_SIZE` in screen CSS pixels, which is `HANDLE_SIZE / scale` in
 * canvas coordinates. It is checked in the same order as hit in the bridge's area_select.rs
 * (corners → inside the area → otherwise). When the area is small the handles overlap; then the
 * corner nearest to the pressed point is returned (if only the first corner, top-left, could be
 * grabbed, the area could not be extended toward the bottom right)
 */
export function hitArea(point: Point, area: Area, scale = 1): AreaHit {
  const right = area.x + area.width;
  const bottom = area.y + area.height;
  const reach = HANDLE_SIZE / 2 / scale;
  const corners: Array<[Corner, number, number]> = [
    ['top-left', area.x, area.y],
    ['top-right', right, area.y],
    ['bottom-left', area.x, bottom],
    ['bottom-right', right, bottom],
  ];
  let closest: Corner | null = null;
  let closestDistance = 0;
  for (const [corner, cx, cy] of corners) {
    if (Math.abs(point.x - cx) <= reach && Math.abs(point.y - cy) <= reach) {
      const distance = (point.x - cx) ** 2 + (point.y - cy) ** 2;
      if (closest === null || distance < closestDistance) {
        closest = corner;
        closestDistance = distance;
      }
    }
  }
  if (closest !== null) {
    return { kind: 'resize', corner: closest };
  }
  if (
    area.x <= point.x &&
    point.x <= right &&
    area.y <= point.y &&
    point.y <= bottom
  ) {
    return { kind: 'move' };
  }
  return { kind: 'new' };
}

/** The drag to start from the pressed position (canvas coordinates; the view scale is as in hitArea) */
export function beginDrag(
  point: Point,
  area: Area,
  page: Page,
  scale = 1,
): AreaDrag {
  const hit = hitArea(point, area, scale);
  switch (hit.kind) {
    case 'resize':
      return {
        kind: 'resize',
        corner: hit.corner,
        start: area,
        startPoint: point,
      };
    case 'move':
      return { kind: 'move', start: area, startPoint: point };
    case 'new':
      // A press outside the sheet is clamped to the sheet edge (same as the bridge)
      return {
        kind: 'new',
        start: {
          x: Math.min(Math.max(point.x, 0), page.width),
          y: Math.min(Math.max(point.y, 0), page.height),
        },
      };
  }
}

/**
 * The new area from the position while dragging. It is kept inside the sheet, and the width and height are at least 1.
 * Moving a corner does not move the opposite corner, and moving does not change the size (same as the bridge)
 */
export function draggedArea(drag: AreaDrag, point: Point, page: Page): Area {
  switch (drag.kind) {
    case 'new': {
      const left = Math.min(drag.start.x, point.x);
      const right = Math.max(drag.start.x, point.x);
      const top = Math.min(drag.start.y, point.y);
      const bottom = Math.max(drag.start.y, point.y);
      // Keep it inside the sheet
      const clampedLeft = Math.min(Math.max(left, 0), page.width);
      const clampedTop = Math.min(Math.max(top, 0), page.height);
      // The width and height are at least 1
      const clampedRight = Math.min(
        Math.max(right, clampedLeft + 1),
        page.width,
      );
      const clampedBottom = Math.min(
        Math.max(bottom, clampedTop + 1),
        page.height,
      );
      return {
        x: clampedLeft,
        y: clampedTop,
        width: clampedRight - clampedLeft,
        height: clampedBottom - clampedTop,
      };
    }
    case 'move': {
      // Without changing the size, keep it inside the sheet
      return {
        x: Math.min(
          Math.max(drag.start.x + (point.x - drag.startPoint.x), 0),
          page.width - drag.start.width,
        ),
        y: Math.min(
          Math.max(drag.start.y + (point.y - drag.startPoint.y), 0),
          page.height - drag.start.height,
        ),
        width: drag.start.width,
        height: drag.start.height,
      };
    }
    case 'resize': {
      const left = drag.start.x;
      const top = drag.start.y;
      const right = drag.start.x + drag.start.width;
      const bottom = drag.start.y + drag.start.height;
      // Do not move the opposite corner. If it goes past the edge, clamp it to the sheet edge, and if it would be less than 1, make it 1
      const nextLeft =
        drag.corner === 'top-left' || drag.corner === 'bottom-left'
          ? Math.min(
              Math.max(left + (point.x - drag.startPoint.x), 0),
              right - 1,
            )
          : left;
      const nextRight =
        drag.corner === 'top-right' || drag.corner === 'bottom-right'
          ? Math.min(
              Math.max(right + (point.x - drag.startPoint.x), left + 1),
              page.width,
            )
          : right;
      const nextTop =
        drag.corner === 'top-left' || drag.corner === 'top-right'
          ? Math.min(
              Math.max(top + (point.y - drag.startPoint.y), 0),
              bottom - 1,
            )
          : top;
      const nextBottom =
        drag.corner === 'bottom-left' || drag.corner === 'bottom-right'
          ? Math.min(
              Math.max(bottom + (point.y - drag.startPoint.y), top + 1),
              page.height,
            )
          : bottom;
      return {
        x: nextLeft,
        y: nextTop,
        width: nextRight - nextLeft,
        height: nextBottom - nextTop,
      };
    }
  }
}
