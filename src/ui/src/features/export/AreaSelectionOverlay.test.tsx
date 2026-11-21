import { fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AreaSelectionOverlay } from './AreaSelectionOverlay';

const page = { width: 400, height: 300 };

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
  // The canvas area (this div) is 400×300 at (0,0). The coordinates use the top left of this div as the origin
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

    fireEvent.pointerDown(overlay, {
      pointerId: 1,
      clientX: 200,
      clientY: 150,
    });
    // The minimal area appears on press and grows with the drag
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 200,
      y: 150,
      width: 1,
      height: 1,
    });
    fireEvent.pointerMove(overlay, {
      pointerId: 1,
      clientX: 300,
      clientY: 250,
    });
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
    fireEvent.pointerDown(overlay, {
      pointerId: 1,
      clientX: 300,
      clientY: 220,
    });
    fireEvent.pointerMove(overlay, {
      pointerId: 1,
      clientX: 350,
      clientY: 270,
    });
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
    fireEvent.pointerDown(overlay, {
      pointerId: 1,
      clientX: 200,
      clientY: 150,
    });
    expect(onAreaChange).not.toHaveBeenCalled();
    fireEvent.pointerMove(overlay, {
      pointerId: 1,
      clientX: 220,
      clientY: 160,
    });
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
    fireEvent.pointerDown(overlay, {
      pointerId: 1,
      clientX: 200,
      clientY: 150,
    });
    // The second one (another finger) is ignored whether pressed or moved
    fireEvent.pointerDown(overlay, { pointerId: 2, clientX: 50, clientY: 50 });
    fireEvent.pointerMove(overlay, { pointerId: 2, clientX: 60, clientY: 60 });
    expect(onAreaChange).not.toHaveBeenCalled();
    // Only the first pointer's movement takes effect
    fireEvent.pointerMove(overlay, {
      pointerId: 1,
      clientX: 220,
      clientY: 150,
    });
    expect(onAreaChange).toHaveBeenLastCalledWith({
      x: 120,
      y: 80,
      width: 200,
      height: 140,
    });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    // Releasing the first pointer ends the drag, and the second pointer's movement keeps being ignored
    fireEvent.pointerMove(overlay, { pointerId: 2, clientX: 70, clientY: 70 });
    expect(onAreaChange).toHaveBeenCalledTimes(1);
  });
});
