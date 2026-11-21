import { describe, expect, it, vi } from 'vitest';
import type { Stroke } from '../canvas/stroke';
import {
  clampArea,
  defaultBaseName,
  download,
  EXPORT_SCALE,
  exportPdf,
  exportScale,
  fileName,
  pageContent,
  type Area,
} from './exportFile';

describe('defaultBaseName', () => {
  it('ローカルの時刻を使い、シートの番号があるときだけ -sheet-N を付ける', () => {
    const now = new Date(2026, 10, 19, 15, 30);
    expect(defaultBaseName(now, 2)).toBe('draw-mcp-2026-11-19-1530-sheet-2');
    expect(defaultBaseName(now, null)).toBe('draw-mcp-2026-11-19-1530');
  });
});

describe('fileName', () => {
  it('拡張子がないなら付ける', () => {
    expect(fileName('memo', 'fallback', 'png')).toBe('memo.png');
  });

  it('末尾が同じ拡張子(大文字)なら付けない', () => {
    expect(fileName('memo.PNG', 'fallback', 'png')).toBe('memo.PNG');
  });

  it('末尾の拡張子が違えば、正しい拡張子を付ける', () => {
    expect(fileName('memo.pdf', 'fallback', 'png')).toBe('memo.pdf.png');
  });

  it('空白だけなら fallback を使う', () => {
    expect(fileName('  ', 'draw-mcp-2026-11-19-1530', 'pdf')).toBe(
      'draw-mcp-2026-11-19-1530.pdf',
    );
  });
});

describe('clampArea', () => {
  const sheet = { width: 100, height: 50 };

  it('シートからはみ出した部分を切る', () => {
    expect(clampArea({ x: -10, y: -5, width: 60, height: 80 }, sheet)).toEqual({
      x: 0,
      y: 0,
      width: 50,
      height: 50,
    });
  });

  it('シートの外だけなら null', () => {
    expect(
      clampArea({ x: 100, y: 0, width: 10, height: 10 }, sheet),
    ).toBeNull();
    expect(clampArea({ x: 0, y: 50, width: 10, height: 10 }, sheet)).toBeNull();
    expect(
      clampArea({ x: 200, y: 200, width: 10, height: 10 }, sheet),
    ).toBeNull();
  });
});

describe('exportScale', () => {
  it('長い辺が 4096 画素以下なら 2 倍', () => {
    // An area the size of an iPad stays at 2× (2360×1640 pixels)
    expect(exportScale({ x: 0, y: 0, width: 1180, height: 820 })).toBe(
      EXPORT_SCALE,
    );
  });

  it('長い辺が超えるときだけ、長い辺が 4096 画素になる倍率に下げる', () => {
    const area: Area = { x: 0, y: 0, width: 7608, height: 4320 };
    const scale = exportScale(area);
    expect(scale).toBeLessThan(EXPORT_SCALE);
    expect(scale).toBeCloseTo(4096 / 7608);
    // The long side becomes 4096 pixels
    expect(area.width * scale).toBeCloseTo(4096);
    expect(area.height * scale).toBeLessThan(4096);
  });
});

describe('pageContent', () => {
  const area: Area = { x: 20, y: 10, width: 40, height: 30 };

  function stroke(color: string | undefined, points: Stroke['points']): Stroke {
    return { id: 's', author: 'human', color, points };
  }

  it('範囲で切り、座標を写し、筆跡は線として書く(bridge と同じ文字列)', () => {
    const onePoint = stroke(undefined, [{ x: 30, y: 20, pressure: 1 }]);
    const red = stroke('#ff3b30', [
      { x: 10, y: 10, pressure: 1 },
      { x: 50, y: 30, pressure: 1 },
    ]);
    const content = pageContent(area, [onePoint, red], undefined);

    // 1. Cut away the outside of the page (40px × 30px = 30pt × 22.5pt)
    expect(content).toContain('0 0 30 22.5 re W n\n');
    // 2. Map the canvas coordinates onto the page (tx = -20 × 0.75, ty = (10 + 30) × 0.75)
    expect(content).toContain('0.75 0 0 -0.75 -15 30 cm\n');
    // 4. The color and line for each stroke (the width is the average of radius 3 × 2 = 6).
    // The black stroke has one point, so it is a zero-length line
    expect(content).toContain(
      '0.000 0.000 0.000 RG 1 J 1 j\n6 w 30 20 m 30 20 l S\n',
    );
    // The red stroke
    expect(content).toContain(
      '1.000 0.231 0.188 RG 1 J 1 j\n6 w 10 10 m 50 30 l S\n',
    );
    // The start of the content is the clip to the area
    expect(content.startsWith('0 0 30 22.5 re W n\n')).toBe(true);
  });

  it('画像があれば、座標の写像の後に配置で /Im0 を貼る(筆跡の下)', () => {
    const content = pageContent(
      { x: 0, y: 0, width: 100, height: 50 },
      [stroke(undefined, [{ x: 1, y: 2, pressure: 1 }])],
      { bitmap: {} as ImageBitmap, x: 5, y: 7, width: 10, height: 4 },
    );
    // The placement (10 × 4, the bottom edge of y at 11)
    expect(content).toContain('q 10 0 0 -4 5 11 cm /Im0 Do Q\n');
    const mapping = content.indexOf('cm\n');
    const image = content.indexOf('/Im0 Do Q\n');
    const strokeLine = content.indexOf(' m ');
    expect(mapping).toBeLessThan(image);
    expect(image).toBeLessThan(strokeLine);
  });
});

describe('exportPdf', () => {
  it('描画器が無くても、ページの数だけ area ごとに 1 ページを作る', async () => {
    const pages = [
      {
        sheet: { strokes: [], image: undefined },
        area: { x: 0, y: 0, width: 800, height: 600 },
      },
      {
        sheet: { strokes: [], image: undefined },
        area: { x: 0, y: 0, width: 100, height: 10 },
      },
    ];
    const blob = await exportPdf(pages);
    expect(blob.type).toBe('application/pdf');
    const text = new TextDecoder().decode(
      new Uint8Array(await blob.arrayBuffer()),
    );
    expect(text.startsWith('%PDF-')).toBe(true);
    // The page size is area × 0.75pt (800×600 → 600×450)
    expect(text).toContain('/MediaBox [0 0 600 450]');
    expect(text).toContain('/Count 2');
  });

  it('取り込んだ画像は元の RGBA を SMask(α)付きで貼る', async () => {
    // jsdom has no canvas 2D, so fake the canvas that reads the image
    const rgba = new Uint8ClampedArray([255, 0, 0, 128]);
    const fakeCanvas = {
      width: 0,
      height: 0,
      getContext: (kind: string) =>
        kind === '2d'
          ? {
              drawImage: () => {},
              getImageData: () => ({ data: rgba }),
            }
          : null,
    };
    const createElement = vi
      .spyOn(document, 'createElement')
      .mockReturnValue(fakeCanvas as unknown as HTMLCanvasElement);
    try {
      const blob = await exportPdf([
        {
          sheet: {
            strokes: [],
            image: {
              bitmap: { width: 1, height: 1 } as ImageBitmap,
              x: 5,
              y: 7,
              width: 10,
              height: 4,
            },
          },
          area: { x: 0, y: 0, width: 100, height: 50 },
        },
      ]);
      const text = new TextDecoder().decode(
        new Uint8Array(await blob.arrayBuffer()),
      );
      expect(text).toContain('q 10 0 0 -4 5 11 cm /Im0 Do Q\n');
      // The image is 1×1 RGB, and because there is α (128), there is also an SMask (gray)
      expect(text).toContain(
        '/Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB',
      );
      expect(text).toContain('/DeviceGray');
      expect(text).toContain('/SMask');
    } finally {
      createElement.mockRestore();
    }
  });
});

describe('download', () => {
  it('download 属性に名前を付けた <a> を押し、少し後に URL を解放する', () => {
    vi.useFakeTimers();
    const createObjectURL = vi.fn(() => 'blob:test');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
    // No navigation. It checks the download attribute of the clicked <a>
    const clicked: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(this.download);
    });

    const blob = new Blob(['hello']);
    download(blob, 'memo.png');
    expect(createObjectURL).toHaveBeenCalledWith(blob);
    expect(clicked).toEqual(['memo.png']);
    expect(revokeObjectURL).not.toHaveBeenCalled();
    vi.advanceTimersByTime(60 * 1000);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:test');

    vi.useRealTimers();
    vi.unstubAllGlobals();
  });
});
