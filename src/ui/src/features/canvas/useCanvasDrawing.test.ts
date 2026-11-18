import { act, renderHook } from '@testing-library/react';
import type { PointerEvent as ReactPointerEvent, RefObject } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type ImagePlacement, INITIAL_SHEET_ID } from '../sync/protocol';
import { ImageStore } from './imageStore';
import { useCanvasDrawing } from './useCanvasDrawing';
import type { ToolSettings } from './types';
import { INITIAL_VIEW } from './view';

// jsdom has no navigator.gpu, so no renderer is created (no drawing; only state is tracked)
const canvasStub = () => ({
  getBoundingClientRect: () => ({ left: 0, top: 0 }),
  setPointerCapture: vi.fn(),
});

interface FakeEventInit {
  type: string;
  pointerType: string;
  pointerId: number;
  x: number;
  y: number;
}

const pointerEvent = (
  canvas: ReturnType<typeof canvasStub>,
  { type, pointerType, pointerId, x, y }: FakeEventInit,
) =>
  ({
    type,
    pointerType,
    pointerId,
    pressure: 0.5,
    clientX: x,
    clientY: y,
    nativeEvent: { clientX: x, clientY: y, pressure: 0.5 },
    currentTarget: canvas,
    getCoalescedEvents: () => [],
  }) as unknown as ReactPointerEvent<HTMLCanvasElement>;

const baseSettings: ToolSettings = {
  tool: 'move',
  penColor: '#000000',
  eraserMode: 'stroke',
  eraserSize: 8,
  fingerDrawing: false,
};

const placement = (id: string, x = 10, y = 10): ImagePlacement => ({
  id,
  x,
  y,
  width: 100,
  height: 50,
});

function setup(settings = baseSettings) {
  const canvas = canvasStub();
  const images = new ImageStore();
  const ref = { current: null } as RefObject<HTMLCanvasElement | null>;
  const imageEvents = {
    onClick: vi.fn(),
    onClickOutside: vi.fn(),
    onGestureStart: vi.fn(),
  };
  // Passed as initialProps so that rerender can change the tool
  const hook = renderHook(
    (next: ToolSettings) =>
      useCanvasDrawing(ref, next, images, vi.fn(), imageEvents),
    { initialProps: settings },
  );
  const event = (init: FakeEventInit) => pointerEvent(canvas, init);
  /** Presses, moves and releases with one pointer (mouse) */
  const press = (
    from: { x: number; y: number },
    to: { x: number; y: number } = from,
  ) => {
    const at = (type: string, { x, y }: { x: number; y: number }) =>
      event({ type, pointerType: 'mouse', pointerId: 1, x, y });
    const handlers = () => hook.result.current.pointerHandlers;
    act(() => handlers().onPointerDown(at('pointerdown', from)));
    act(() => handlers().onPointerMove(at('pointermove', to)));
    act(() => handlers().onPointerUp(at('pointerup', to)));
  };
  return { ...hook, images, event, imageEvents, press };
}

describe('useCanvasDrawing', () => {
  // The stand-in div for the eraser circle is made here, so remove it after the test
  afterEach(() => {
    document.body.querySelector('[data-testid="eraser-cursor"]')?.remove();
  });

  it('「指で書く」が OFF でも、移動ツール(Move Image)なら 1 本指で画像が動く', () => {
    const { result, event } = setup();
    act(() => result.current.setImage(placement('img')));

    act(() =>
      result.current.pointerHandlers.onPointerDown(
        event({
          type: 'pointerdown',
          pointerType: 'touch',
          pointerId: 1,
          x: 50,
          y: 30,
        }),
      ),
    );
    act(() =>
      result.current.pointerHandlers.onPointerMove(
        event({
          type: 'pointermove',
          pointerType: 'touch',
          pointerId: 1,
          x: 90,
          y: 60,
        }),
      ),
    );
    act(() =>
      result.current.pointerHandlers.onPointerUp(
        event({
          type: 'pointerup',
          pointerType: 'touch',
          pointerId: 1,
          x: 90,
          y: 60,
        }),
      ),
    );

    const moved = placement('img', 50, 40);
    expect(result.current.image).toEqual(moved);
    // Both the place and move operations remain in pending
    expect(result.current.pending).toEqual([
      {
        seq: 1,
        op: {
          type: 'setImage',
          sheet: INITIAL_SHEET_ID,
          image: placement('img'),
        },
      },
      {
        seq: 2,
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: moved },
      },
    ]);
  });

  describe('移動ツール(Move Image)でのクリック(画像のそばの吹き出しを出す)', () => {
    it('画像の上を動かさずに押して離すとクリックとして知らせ、画像は動かさない', () => {
      const { result, imageEvents, press } = setup();
      act(() => result.current.setImage(placement('img')));
      const before = result.current.pending;

      press({ x: 50, y: 30 });

      // The popover appears near the released point (page coordinates)
      expect(imageEvents.onClick).toHaveBeenCalledExactlyOnceWith({
        x: 50,
        y: 30,
      });
      expect(imageEvents.onGestureStart).not.toHaveBeenCalled();
      expect(result.current.image).toEqual(placement('img'));
      expect(result.current.pending).toBe(before);
    });

    it('少しの揺れ(CLICK_SLOP 以内)なら、クリックのまま', () => {
      const { result, imageEvents, press } = setup();
      act(() => result.current.setImage(placement('img')));

      press({ x: 50, y: 30 }, { x: 53, y: 32 });

      expect(imageEvents.onClick).toHaveBeenCalledOnce();
      expect(result.current.image).toEqual(placement('img'));
    });

    it('画像の外のクリックは、外のクリックとして知らせる(吹き出しを閉じる)', () => {
      const { result, imageEvents, press } = setup();
      act(() => result.current.setImage(placement('img')));

      press({ x: 300, y: 300 });

      expect(imageEvents.onClick).not.toHaveBeenCalled();
      expect(imageEvents.onClickOutside).toHaveBeenCalledOnce();
    });

    it('ドラッグは動かし始めたときに知らせ、離してもクリックとは知らせない', () => {
      const { result, imageEvents, press } = setup();
      act(() => result.current.setImage(placement('img')));

      press({ x: 50, y: 30 }, { x: 90, y: 60 });

      expect(imageEvents.onGestureStart).toHaveBeenCalledOnce();
      expect(imageEvents.onClick).not.toHaveBeenCalled();
      expect(result.current.image).toEqual(placement('img', 50, 40));
    });

    it('2 本指のピンチは、動かし始めたとして知らせる', () => {
      const { result, imageEvents, event } = setup();
      act(() => result.current.setImage(placement('img')));
      const touch = (pointerId: number, x: number) =>
        event({
          type: 'pointerdown',
          pointerType: 'touch',
          pointerId,
          x,
          y: 30,
        });

      act(() => result.current.pointerHandlers.onPointerDown(touch(1, 40)));
      act(() => result.current.pointerHandlers.onPointerDown(touch(2, 80)));

      expect(imageEvents.onGestureStart).toHaveBeenCalled();
      expect(imageEvents.onClick).not.toHaveBeenCalled();
    });
  });

  it('「指で書く」が OFF で移動ツール(Move Image)でなければ、指では書かない', () => {
    const { result, event } = setup({ ...baseSettings, tool: 'pen' });

    act(() =>
      result.current.pointerHandlers.onPointerDown(
        event({
          type: 'pointerdown',
          pointerType: 'touch',
          pointerId: 1,
          x: 10,
          y: 10,
        }),
      ),
    );
    act(() =>
      result.current.pointerHandlers.onPointerUp(
        event({
          type: 'pointerup',
          pointerType: 'touch',
          pointerId: 1,
          x: 40,
          y: 40,
        }),
      ),
    );

    expect(result.current.pending).toEqual([]);
    expect(result.current.canClear).toBe(false);
  });

  it('tool: "move" で画像が無いとき、押して動かしても筆跡が増えない', () => {
    const { result, press } = setup();

    press({ x: 10, y: 10 }, { x: 40, y: 40 });

    expect(result.current.pending).toEqual([]);
    expect(result.current.canClear).toBe(false);
  });

  it('tool: "eraser" から tool: "move" にして、マウスで動かしても消しゴムの円が出ない', () => {
    const { result, rerender, event } = setup({
      ...baseSettings,
      tool: 'eraser',
    });
    // The eraser circle is a div in CanvasPage, so attach a stand-in here
    const cursor = document.createElement('div');
    cursor.setAttribute('data-testid', 'eraser-cursor');
    document.body.appendChild(cursor);
    result.current.eraserCursorRef.current = cursor;

    const hover = (x: number, y: number) =>
      act(() =>
        result.current.pointerHandlers.onPointerMove(
          event({
            type: 'pointermove',
            pointerType: 'mouse',
            pointerId: 1,
            x,
            y,
          }),
        ),
      );

    hover(50, 40);
    expect(cursor).toBeVisible();

    act(() => {
      rerender({ ...baseSettings, tool: 'move' });
    });
    hover(60, 50);

    expect(cursor).not.toBeVisible();
  });

  it('画像は、取り消しで戻れるものは残し、クリアすると全部捨てる', () => {
    const { result, images } = setup({ ...baseSettings, tool: 'pen' });
    images.put('a', new Uint8Array([1]));
    act(() => result.current.setImage(placement('a')));
    images.put('b', new Uint8Array([2]));
    act(() => result.current.setImage(placement('b')));

    // Even after replacing it with b, a stays because undo can bring it back
    expect(images.has('a')).toBe(true);
    expect(images.has('b')).toBe(true);

    act(() => result.current.clear());

    // The history is now empty, so both are discarded
    expect(images.has('a')).toBe(false);
    expect(images.has('b')).toBe(false);
  });

  it('紙を切り替えると表示が等倍に戻る', () => {
    const { result, event } = setup();
    // Zoom in with a two-finger pinch
    const touch = (type: string, pointerId: number, x: number) =>
      event({ type, pointerType: 'touch', pointerId, x, y: 30 });
    act(() =>
      result.current.pointerHandlers.onPointerDown(touch('pointerdown', 1, 40)),
    );
    act(() =>
      result.current.pointerHandlers.onPointerDown(touch('pointerdown', 2, 80)),
    );
    act(() =>
      result.current.pointerHandlers.onPointerMove(
        touch('pointermove', 2, 160),
      ),
    );
    act(() =>
      result.current.pointerHandlers.onPointerUp(touch('pointerup', 2, 160)),
    );
    expect(result.current.view.scale).toBeGreaterThan(1);

    act(() => result.current.sheets.add?.());

    expect(result.current.view).toEqual(INITIAL_VIEW);
    expect(result.current.sheets.number).toBe(2);
  });
});
