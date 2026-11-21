import { describe, expect, it } from 'vitest';
import {
  areaSelectionView,
  beginDrag,
  draggedArea,
  hitArea,
} from './areaSelection';

describe('hitArea', () => {
  const area = { x: 100, y: 80, width: 200, height: 140 };
  // Corners: (100,80) (300,80) (100,220) (300,220), half of the hit size is 12

  it('隅のハンドル(24px 四方)の内側ならその隅', () => {
    expect(hitArea({ x: 100, y: 80 }, area)).toEqual({
      kind: 'resize',
      corner: 'top-left',
    });
    expect(hitArea({ x: 300, y: 80 }, area)).toEqual({
      kind: 'resize',
      corner: 'top-right',
    });
    expect(hitArea({ x: 100, y: 220 }, area)).toEqual({
      kind: 'resize',
      corner: 'bottom-left',
    });
    expect(hitArea({ x: 300, y: 220 }, area)).toEqual({
      kind: 'resize',
      corner: 'bottom-right',
    });
    // The far corner of the handle (12 from the center) still hits
    expect(hitArea({ x: 100 + 12, y: 80 + 12 }, area)).toEqual({
      kind: 'resize',
      corner: 'top-left',
    });
    expect(hitArea({ x: 300 - 12, y: 220 - 12 }, area)).toEqual({
      kind: 'resize',
      corner: 'bottom-right',
    });
  });

  it('ハンドルより外なら隅では無い', () => {
    expect(hitArea({ x: 100 - 12.1, y: 80 - 12.1 }, area)).toEqual({
      kind: 'new',
    });
  });

  it('範囲の中を動かす', () => {
    expect(hitArea({ x: 200, y: 150 }, area)).toEqual({ kind: 'move' });
    // On the edge also counts as inside (same as the bridge)
    expect(hitArea({ x: 100, y: 150 }, area)).toEqual({ kind: 'move' });
  });

  it('範囲の外なら新しく描く', () => {
    expect(hitArea({ x: 5, y: 5 }, area)).toEqual({ kind: 'new' });
    expect(hitArea({ x: 390, y: 290 }, area)).toEqual({ kind: 'new' });
  });

  it('scale を渡すと、ハンドルに当たる大きさはシート座標で HANDLE_SIZE/scale になる', () => {
    const scale = 0.8;
    const reach = 12 / scale; // HANDLE_SIZE / 2 / scale
    // Inside the boundary (reach from the corner) hits
    expect(hitArea({ x: 100 + reach, y: 80 + reach }, area, scale)).toEqual({
      kind: 'resize',
      corner: 'top-left',
    });
    expect(hitArea({ x: 100 - reach, y: 80 - reach }, area, scale)).toEqual({
      kind: 'resize',
      corner: 'top-left',
    });
    // Outside the boundary is not a corner
    expect(
      hitArea({ x: 100 - reach - 0.1, y: 80 - reach - 0.1 }, area, scale),
    ).toEqual({ kind: 'new' });
  });
});

describe('areaSelectionView', () => {
  it('横長のシートは上下の余白が 24px(左右はそれ以上)になるよう縮めて、中央に置く', () => {
    // scale = (300 - 48) / 300
    expect(areaSelectionView({ width: 400, height: 300 })).toEqual({
      scale: (300 - 48) / 300,
      x: 32,
      y: 24,
    });
  });

  it('縦長のシートは左右の余白が 24px になる', () => {
    expect(areaSelectionView({ width: 300, height: 400 })).toEqual({
      scale: (300 - 48) / 300,
      x: 24,
      y: 32,
    });
  });

  it('幅か高さが 48 以下(0×0 を含む)なら縮めない', () => {
    expect(areaSelectionView({ width: 48, height: 300 })).toEqual({
      scale: 1,
      x: 0,
      y: 0,
    });
    expect(areaSelectionView({ width: 0, height: 0 })).toEqual({
      scale: 1,
      x: 0,
      y: 0,
    });
  });
});

describe('beginDrag', () => {
  const page = { width: 400, height: 300 };
  const area = { x: 100, y: 80, width: 200, height: 140 };

  it('隅を押したらその隅のドラッグ、中を押したら移動', () => {
    expect(beginDrag({ x: 300, y: 220 }, area, page)).toMatchObject({
      kind: 'resize',
      corner: 'bottom-right',
      start: area,
      startPoint: { x: 300, y: 220 },
    });
    expect(beginDrag({ x: 200, y: 150 }, area, page)).toMatchObject({
      kind: 'move',
      start: area,
      startPoint: { x: 200, y: 150 },
    });
  });

  it('新しく描くときの開始位置はシートの中に収める', () => {
    expect(beginDrag({ x: -5, y: 350 }, area, page)).toMatchObject({
      kind: 'new',
      start: { x: 0, y: 300 },
    });
    expect(beginDrag({ x: 50, y: 270 }, area, page)).toMatchObject({
      kind: 'new',
      start: { x: 50, y: 270 },
    });
  });
});

describe('draggedArea', () => {
  const page = { width: 400, height: 300 };
  const area = { x: 100, y: 80, width: 200, height: 140 };

  describe('新しく描く', () => {
    const drag = { kind: 'new' as const, start: { x: 100, y: 100 } };

    it('押した位置からドラッグした位置までを範囲にする', () => {
      expect(draggedArea(drag, { x: 200, y: 150 }, page)).toEqual({
        x: 100,
        y: 100,
        width: 100,
        height: 50,
      });
      // The same when dragging up and left
      expect(draggedArea(drag, { x: 80, y: 90 }, page)).toEqual({
        x: 80,
        y: 90,
        width: 20,
        height: 10,
      });
    });

    it('シートからはみ出した部分はシートの縁で切る', () => {
      expect(draggedArea(drag, { x: 999, y: 999 }, page)).toEqual({
        x: 100,
        y: 100,
        width: 300,
        height: 200,
      });
    });

    it('縮みすぎたときは幅・高さが 1 になる', () => {
      expect(draggedArea(drag, { x: 100, y: 100 }, page)).toEqual({
        x: 100,
        y: 100,
        width: 1,
        height: 1,
      });
    });
  });

  describe('移動', () => {
    const drag = {
      kind: 'move' as const,
      start: area,
      startPoint: { x: 200, y: 150 },
    };

    it('押した位置からの移動分だけ動かす', () => {
      expect(draggedArea(drag, { x: 220, y: 140 }, page)).toEqual({
        x: 120,
        y: 70,
        width: 200,
        height: 140,
      });
    });

    it('シートからはみ出すとシートの縁に収まり、大きさは変わらない', () => {
      expect(draggedArea(drag, { x: 9999, y: 9999 }, page)).toEqual({
        x: 200,
        y: 160,
        width: 200,
        height: 140,
      });
      expect(draggedArea(drag, { x: -9999, y: -9999 }, page)).toEqual({
        x: 0,
        y: 0,
        width: 200,
        height: 140,
      });
    });
  });

  describe('隅を動かす', () => {
    const bottomRight = {
      kind: 'resize' as const,
      corner: 'bottom-right' as const,
      start: area,
      startPoint: { x: 300, y: 220 },
    };
    const topLeft = {
      kind: 'resize' as const,
      corner: 'top-left' as const,
      start: area,
      startPoint: { x: 100, y: 80 },
    };

    it('向かいの隅を動かさずに、押している隅だけ動かす', () => {
      expect(draggedArea(bottomRight, { x: 310, y: 230 }, page)).toEqual({
        x: 100,
        y: 80,
        width: 210,
        height: 150,
      });
      expect(draggedArea(topLeft, { x: 50, y: 40 }, page)).toEqual({
        x: 50,
        y: 40,
        width: 250,
        height: 180,
      });
    });

    it('シートからはみ出すとシートの縁に収まる', () => {
      expect(draggedArea(bottomRight, { x: 999, y: 999 }, page)).toEqual({
        x: 100,
        y: 80,
        width: 300,
        height: 220,
      });
      expect(draggedArea(topLeft, { x: -999, y: -999 }, page)).toEqual({
        x: 0,
        y: 0,
        width: 300,
        height: 220,
      });
    });

    it('縮みすぎたときは向かいの隅との間隔が 1 になる', () => {
      expect(draggedArea(bottomRight, { x: 90, y: 70 }, page)).toEqual({
        x: 100,
        y: 80,
        width: 1,
        height: 1,
      });
      expect(draggedArea(topLeft, { x: 310, y: 230 }, page)).toEqual({
        x: 299,
        y: 219,
        width: 1,
        height: 1,
      });
    });
  });
});
