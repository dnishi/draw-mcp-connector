import { useRef, useState, type PointerEvent } from 'react';
import {
  CORNER_CURSOR,
  areaSelectionView,
  beginDrag,
  draggedArea,
  hitArea,
  type AreaDrag,
  type AreaHit,
  type Page,
} from './areaSelection';
import type { Area } from './exportFile';

interface AreaSelectionOverlayProps {
  /** The sheet size */
  page: Page;
  /** The chosen area */
  area: Area;
  onAreaChange: (area: Area) => void;
}

/** The mouse shape. While dragging, it is by the drag kind; otherwise, it is by what is under the pointer */
function dragCursor(drag: AreaDrag): string {
  switch (drag.kind) {
    case 'resize':
      return CORNER_CURSOR[drag.corner];
    case 'move':
      return 'move';
    case 'new':
      return 'crosshair';
  }
}

function hitCursor(hit: AreaHit): string {
  switch (hit.kind) {
    case 'resize':
      return CORNER_CURSOR[hit.corner];
    case 'move':
      return 'move';
    case 'new':
      return 'crosshair';
  }
}

/**
 * The area selection overlay. Placed over the canvas with `absolute inset-0` (the whole canvas
 * area, including the margin) and shown only while the export panel is open. It takes the canvas
 * input itself, so writing, erasing, and pinching stop here. While it is shown, the sheet is
 * shrunk and centered by `areaSelectionView` (the margin is only dimmed), so a screen point is
 * turned into canvas coordinates from the top left of this div with that scale and position
 */
export function AreaSelectionOverlay({
  page,
  area,
  onAreaChange,
}: AreaSelectionOverlayProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<AreaDrag | null>(null);
  /** The pointer in progress (a second one at the same time is ignored) */
  const activePointerRef = useRef<number | null>(null);
  const [cursor, setCursor] = useState('crosshair');
  /** The scale and position (from the top left of the canvas area) of the shrunk sheet while this is shown */
  const view = areaSelectionView(page);

  /** A screen point to canvas coordinates */
  const toPoint = (event: PointerEvent<HTMLDivElement>) => {
    const bounds = rootRef.current?.getBoundingClientRect();
    if (!bounds) {
      return null;
    }
    return {
      x: (event.clientX - bounds.left - view.x) / view.scale,
      y: (event.clientY - bounds.top - view.y) / view.scale,
    };
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (activePointerRef.current !== null) {
      return;
    }
    const point = toPoint(event);
    if (!point) {
      return;
    }
    activePointerRef.current = event.pointerId;
    const drag = beginDrag(point, area, page, view.scale);
    dragRef.current = drag;
    setCursor(dragCursor(drag));
    event.currentTarget.setPointerCapture(event.pointerId);
    // On press, show the (minimal) shape of the new area
    if (drag.kind === 'new') {
      onAreaChange(draggedArea(drag, point, page));
    }
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const point = toPoint(event);
    if (!point) {
      return;
    }
    const drag = dragRef.current;
    if (drag && event.pointerId === activePointerRef.current) {
      onAreaChange(draggedArea(drag, point, page));
    } else if (!drag) {
      setCursor(hitCursor(hitArea(point, area, view.scale)));
    }
  };

  const endPointer = (event: PointerEvent<HTMLDivElement>) => {
    if (activePointerRef.current !== event.pointerId) {
      return;
    }
    activePointerRef.current = null;
    dragRef.current = null;
    setCursor('crosshair');
  };

  return (
    <div
      ref={rootRef}
      className="absolute inset-0 touch-none"
      style={{ cursor }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endPointer}
      onPointerCancel={endPointer}
    >
      {/* Darken outside the area with 40% black (the shadow of the area frame; the margin is darkened too). The edge is 2px blue */}
      <div
        className="absolute border-2 border-sky-500"
        style={{
          left: view.x + area.x * view.scale,
          top: view.y + area.y * view.scale,
          width: area.width * view.scale,
          height: area.height * view.scale,
          boxShadow: '0 0 0 100vmax rgba(0, 0, 0, 0.4)',
        }}
      >
        {/* The corner handles (12px, white with a blue edge). Placed at the center of the corner */}
        <div className="absolute -top-1.5 -left-1.5 size-3 border border-sky-500 bg-white" />
        <div className="absolute -top-1.5 -right-1.5 size-3 border border-sky-500 bg-white" />
        <div className="absolute -bottom-1.5 -left-1.5 size-3 border border-sky-500 bg-white" />
        <div className="absolute -bottom-1.5 -right-1.5 size-3 border border-sky-500 bg-white" />
      </div>
    </div>
  );
}
