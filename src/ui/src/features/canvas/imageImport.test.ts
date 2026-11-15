import { describe, expect, it } from 'vitest';
import {
  clampPlacement,
  initialPlacement,
  resizePlacement,
} from './imageImport';

const page = { width: 1000, height: 800 };

describe('initialPlacement', () => {
  it('紙の中央に等倍で置く(紙に収まるなら拡大しない)', () => {
    expect(initialPlacement({ width: 400, height: 200 }, page, 'i')).toEqual({
      id: 'i',
      x: 300,
      y: 300,
      width: 400,
      height: 200,
    });
  });

  it('紙より大きい画像は、紙に収まるまで縮めて中央に置く', () => {
    expect(initialPlacement({ width: 4000, height: 2000 }, page, 'i')).toEqual({
      id: 'i',
      x: 0,
      y: 150,
      width: 1000,
      height: 500,
    });
  });
});

describe('clampPlacement', () => {
  const at = (x: number, y: number, width = 100, height = 50) => ({
    id: 'i',
    x,
    y,
    width,
    height,
  });

  it('紙からはみ出さない位置に寄せる', () => {
    expect(clampPlacement(at(-50, 900), page)).toEqual(at(0, 750));
    expect(clampPlacement(at(980, -10), page)).toEqual(at(900, 0));
  });

  it('紙の中なら動かさない', () => {
    expect(clampPlacement(at(10, 20), page)).toEqual(at(10, 20));
  });

  it('画像の方が紙より大きいときは、端を紙の端に合わせられる', () => {
    // Width 1200 > the paper's 1000. To align the right edge, x is allowed down to -200
    expect(clampPlacement(at(100, 0, 1200, 100), page)).toEqual(
      at(0, 0, 1200, 100),
    );
    expect(clampPlacement(at(-500, 0, 1200, 100), page)).toEqual(
      at(-200, 0, 1200, 100),
    );
  });
});

describe('resizePlacement', () => {
  const placed = { id: 'i', x: 100, y: 100, width: 200, height: 100 };
  const natural = { width: 400, height: 200 };

  it('中心を保ったまま、元の大きさに対する割合で大きさを変える', () => {
    // Center (200, 150). 100% is the original size, so it fits in the paper centered
    expect(resizePlacement(placed, natural, 100, page)).toEqual({
      id: 'i',
      x: 0,
      y: 50,
      width: 400,
      height: 200,
    });
    expect(resizePlacement(placed, natural, 50, page)).toEqual(
      // The center stays; at 50% it is 200x100 = the original
      placed,
    );
  });

  it('拡大到着は中心が紙からはみ出さない範囲に寄せる', () => {
    const bottomRight = { id: 'i', x: 940, y: 740, width: 40, height: 40 };
    const grown = resizePlacement(
      bottomRight,
      { width: 40, height: 40 },
      400,
      page,
    );
    expect(grown.width).toBe(160);
    expect(grown.x).toBe(840); // 960 - 160/2 = 880, but limited to the paper's right edge 1000-160=840
    expect(grown.y).toBe(640);
  });
});
