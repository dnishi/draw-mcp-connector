import { describe, expect, it } from 'vitest';
import {
  INITIAL_VIEW,
  PinchTracker,
  SCALE_RANGE,
  clampView,
  pinchView,
  toCanvasPoint,
  toScreenPoint,
  visibleRange,
  type View,
} from './view';

const p = (x: number, y: number) => ({ x, y });

describe('座標の変換', () => {
  const view: View = { originX: 100, originY: 50, scale: 2 };

  it('画面の点とキャンバスの点を行き来できる', () => {
    expect(toCanvasPoint(view, p(20, 10))).toEqual(p(110, 55));
    expect(toScreenPoint(view, p(110, 55))).toEqual(p(20, 10));
  });

  it('見えている範囲は、拡大するほど狭くなる', () => {
    expect(visibleRange(view, 800, 600)).toEqual({
      x: 100,
      y: 50,
      width: 400,
      height: 300,
    });
  });
});

describe('pinchView', () => {
  it('指を広げると、中点を中心に拡大する', () => {
    const next = pinchView(
      INITIAL_VIEW,
      [p(90, 100), p(110, 100)],
      [p(80, 100), p(120, 100)],
    );
    expect(next.scale).toBe(2);
    // The point under the midpoint (100, 100) stays under the midpoint after zooming
    expect(toCanvasPoint(next, p(100, 100))).toEqual(p(100, 100));
  });

  it('指をそろえて動かすと、表示位置が動く', () => {
    const zoomed: View = { originX: 100, originY: 100, scale: 2 };
    const next = pinchView(zoomed, [p(0, 0), p(10, 0)], [p(30, 20), p(40, 20)]);
    expect(next).toEqual({ originX: 85, originY: 90, scale: 2 });
  });

  it('拡大率は範囲に収める', () => {
    const next = pinchView(
      INITIAL_VIEW,
      [p(0, 0), p(1, 0)],
      [p(0, 0), p(1000, 0)],
    );
    expect(next.scale).toBe(SCALE_RANGE.max);
  });

  it('等倍より小さくはならない', () => {
    const next = pinchView(
      INITIAL_VIEW,
      [p(0, 0), p(100, 0)],
      [p(0, 0), p(10, 0)],
    );
    expect(next.scale).toBe(1);
  });
});

describe('clampView', () => {
  const page = { width: 800, height: 600 };

  it('見えている範囲を紙の中に収める', () => {
    expect(clampView({ originX: -50, originY: 900, scale: 2 }, page)).toEqual({
      originX: 0,
      originY: 300,
      scale: 2,
    });
  });

  it('等倍では、紙全体を見る位置に戻る', () => {
    expect(clampView({ originX: 30, originY: 40, scale: 1 }, page)).toEqual(
      INITIAL_VIEW,
    );
  });
});

describe('PinchTracker', () => {
  it('2本目の指を置くとピンチになり、1本離すと終わる', () => {
    const tracker = new PinchTracker();
    tracker.down(1, p(0, 0));
    expect(tracker.move(1, p(1, 0))).toBeNull();

    tracker.down(2, p(10, 0));
    expect(tracker.move(2, p(20, 0))).toEqual({
      before: [p(1, 0), p(10, 0)],
      after: [p(1, 0), p(20, 0)],
    });

    expect(tracker.up(2)).toBe(true);
    expect(tracker.up(1)).toBe(false);
  });
});
