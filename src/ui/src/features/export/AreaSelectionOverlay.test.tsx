import { fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { areaSelectionView } from './areaSelection';
import { AreaSelectionOverlay } from './AreaSelectionOverlay';

const page = { width: 400, height: 300 };
// The overlay covers the whole canvas area (400×300). The sheet is shrunk and centered
const view = areaSelectionView(page);
/** A point in sheet coordinates to a screen point (with the top left of the overlay div as the origin) */
const at = (x: number, y: number) => ({
  clientX: view.x + x * view.scale,
  clientY: view.y + y * view.scale,
});

/** Shows the overlay on the canvas (400×300 with the origin at (0,0)) */
function renderOverlay(area: {
  x: number;
  y: number;
  width: number;
  height: number;
}) {
  const onAreaChange = vi.fn();
  const { container } = render(
    <AreaSelectionOverlay
      page={page}
      area={area}
      onAreaChange={onAreaChange}
    />,
  );
  const overlay = container.firstElementChild as HTMLElement;
  return { overlay, onAreaChange };
}

describe('AreaSelectionOverlay', () => {
  // The canvas area (this div) is 400×300 at (0,0). The sheet is shown shrunk and centered,
  // so at() turns sheet coordinates into screen points
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(
      new DOMRect(0, 0, 400, 300),
    );
    // jsdom has no Pointer Capture
    Element.prototype.setPointerCapture = vi.fn();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('範囲の外をドラッグすると、押した位置から新しく範囲を描く', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 0,
      y: 0,
      width: 100,
      height: 100,
    });

    fireEvent.pointerDown(overlay, { pointerId: 1, ...at(200, 150) });
    // The minimal area appears on press and grows with the drag
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 200,
      y: 150,
      width: 1,
      height: 1,
    });
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(300, 250) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 200,
      y: 150,
      width: 100,
      height: 100,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
  });

  it('隅のハンドルを引くと、向かいの隅を動かさずに大きさを変える', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 100,
      y: 80,
      width: 200,
      height: 140,
    });

    // Drag the bottom-right corner (300,220)
    fireEvent.pointerDown(overlay, { pointerId: 1, ...at(300, 220) });
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(350, 270) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 100,
      y: 80,
      width: 250,
      height: 190,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
  });

  it('範囲の中をドラッグすると、大きさを変えずに移動する', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 100,
      y: 80,
      width: 200,
      height: 140,
    });

    // Drag inside the area (200,150)
    fireEvent.pointerDown(overlay, { pointerId: 1, ...at(200, 150) });
    expect(onAreaChange).not.toHaveBeenCalled();
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(220, 160) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 120,
      y: 90,
      width: 200,
      height: 140,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
  });

  it('ドラッグ中に押した 2 本目のポインタは無視する', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 100,
      y: 80,
      width: 200,
      height: 140,
    });

    // The first pointer is dragging inside the area (200,150)
    fireEvent.pointerDown(overlay, { pointerId: 1, ...at(200, 150) });
    // The second one (another finger) is ignored whether pressed or moved
    fireEvent.pointerDown(overlay, { pointerId: 2, ...at(50, 50) });
    fireEvent.pointerMove(overlay, { pointerId: 2, ...at(60, 60) });
    expect(onAreaChange).not.toHaveBeenCalled();
    // Only the first pointer's movement takes effect
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(220, 150) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 120,
      y: 80,
      width: 200,
      height: 140,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    // Releasing the first pointer ends the drag, and the second pointer's movement keeps being ignored
    fireEvent.pointerMove(overlay, { pointerId: 2, ...at(70, 70) });
    expect(onAreaChange).toHaveBeenCalledTimes(1);
  });

  it('余白の上(シートの外)で押して動かすと、シートの縁に収まった範囲が描かれる', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 0,
      y: 0,
      width: 100,
      height: 100,
    });

    // The top-left corner of the canvas area (2,2) is on the margin, outside the sheet
    fireEvent.pointerDown(overlay, { pointerId: 1, clientX: 2, clientY: 2 });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 0,
      y: 0,
      width: 1,
      height: 1,
    });
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(100, 80) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 0,
      y: 0,
      width: 100,
      height: 80,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
  });

  it('範囲がシート全体のとき、余白にはみ出したハンドルは掴める', () => {
    const { overlay, onAreaChange } = renderOverlay({
      x: 0,
      y: 0,
      width: 400,
      height: 300,
    });

    // Pressing a few px outside the sheet's top-left corner ((-2,-2) in sheet coordinates) resizes from the top left
    fireEvent.pointerDown(overlay, { pointerId: 1, ...at(-2, -2) });
    fireEvent.pointerMove(overlay, { pointerId: 1, ...at(50, 50) });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 52,
      y: 52,
      width: 348,
      height: 248,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
  });
});
