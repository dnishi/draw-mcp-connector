import { describe, expect, it } from 'vitest';
import { buildPdf, imageFromRgba, type PdfPage } from './pdf';

function page(widthPt: number, heightPt: number): PdfPage {
  return { widthPt, heightPt, content: new Uint8Array(), image: null };
}

/** A content page embedding a 2×2 image (α is 128 and 255, so there is an SMask) as /Im0 */
async function pageWithImage(): Promise<PdfPage> {
  return {
    widthPt: 150,
    heightPt: 100,
    content: new TextEncoder().encode('q 2 0 0 -2 0 2 cm /Im0 Do Q\n'),
    image: await imageFromRgba(
      2,
      2,
      new Uint8ClampedArray([
        255, 0, 0, 128, 0, 0, 255, 128, 1, 2, 3, 255, 4, 5, 6, 255,
      ]),
    ),
  };
}

/** A page embedding an image whose α is all 255 (no SMask) */
async function opaquePage(): Promise<PdfPage> {
  const result = page(300, 200);
  result.image = await imageFromRgba(
    2,
    2,
    new Uint8ClampedArray([
      1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255,
    ]),
  );
  return result;
}

function ascii(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

/** Decompresses a zlib-format byte sequence with DecompressionStream('deflate') */
async function inflate(bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array> {
  const stream = new DecompressionStream('deflate');
  const writer = stream.writable.getWriter();
  await writer.write(bytes);
  await writer.close();
  return new Uint8Array(await new Response(stream.readable).arrayBuffer());
}

function indexOf(haystack: Uint8Array, needle: Uint8Array): number {
  for (let i = 0; i + needle.length <= haystack.length; i++) {
    let found = true;
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) {
        found = false;
        break;
      }
    }
    if (found) {
      return i;
    }
  }
  return -1;
}

function lastIndexOf(haystack: Uint8Array, needle: Uint8Array): number {
  for (let i = haystack.length - needle.length; i >= 0; i--) {
    if (indexOf(haystack.subarray(i), needle) === 0) {
      return i;
    }
  }
  return -1;
}

/** Whether the string matches needle from the start of haystack */
function startsWith(haystack: Uint8Array, needle: Uint8Array): boolean {
  return indexOf(haystack, needle) === 0;
}

/**
 * Reads the position of each object (in number order) from the cross-reference table.
 * It reads from the position startxref points to (so it works even if the image data contains the string "xref")
 */
function xrefOffsets(pdf: Uint8Array): number[] {
  // The trailer's startxref is the last one in the file
  const marker = lastIndexOf(pdf, ascii('startxref'));
  expect(marker).toBeGreaterThan(-1);
  // The digits after "startxref\n" are the position of the xref table
  let pos = marker + 'startxref\n'.length;
  const lineEnd = indexOf(pdf.subarray(pos), ascii('\n'));
  const xref = Number(
    new TextDecoder().decode(pdf.subarray(pos, pos + lineEnd)),
  );
  pos = xref + 'xref\n'.length;
  const countLine = indexOf(pdf.subarray(pos), ascii('\n'));
  const count = Number(
    new TextDecoder().decode(pdf.subarray(pos + 2, pos + countLine)),
  );
  pos += countLine + 1;
  return Array.from({ length: count }, () => {
    const offset = Number(
      new TextDecoder().decode(pdf.subarray(pos, pos + 10)),
    );
    pos += 20;
    return offset;
  });
}

/** Reads the number right after `name` from a dictionary string */
function dictNumber(dict: string, name: string): number {
  const rest = dict.slice(dict.indexOf(name) + name.length);
  const start = rest.search(/[0-9]/);
  const end = rest.slice(start).search(/\D/);
  return Number(rest.slice(start, start + end));
}

/** Reads all the one-line (object dictionary) lines containing `kind` */
function linesContaining(pdf: Uint8Array, kind: string): string[] {
  return new TextDecoder()
    .decode(pdf)
    .split('\n')
    .filter((line) => line.includes(kind));
}

/** Extracts the bytes of the stream (data part) right after a dictionary line */
function streamAfterDict(pdf: Uint8Array, dict: string): Uint8Array {
  const dictStart = indexOf(pdf, ascii(dict)) + dict.length;
  const stream =
    dictStart +
    indexOf(pdf.subarray(dictStart), ascii('stream\n')) +
    'stream\n'.length;
  const length = dictNumber(dict, '/Length');
  return pdf.subarray(stream, stream + length);
}

describe('imageFromRgba', () => {
  it('DecompressionStream で戻すと、各行が 0 + RGB の並びになる(α は 0 + α)', async () => {
    const image = await imageFromRgba(
      2,
      2,
      new Uint8ClampedArray([
        255, 0, 0, 128, 0, 0, 255, 128, 1, 2, 3, 255, 4, 5, 6, 255,
      ]),
    );
    // Each row: filter 0 + width (2) × channel count (RGB is 7 bytes per row, 2 rows in total)
    expect(await inflate(image.rgb)).toEqual(
      new Uint8Array([0, 255, 0, 0, 0, 0, 255, 0, 1, 2, 3, 4, 5, 6]),
    );
    // α is 3 bytes per row
    expect(await inflate(image.alpha as Uint8Array<ArrayBuffer>)).toEqual(
      new Uint8Array([0, 128, 128, 0, 255, 255]),
    );
  });

  it('α が全部 255 なら無い', async () => {
    const image = await imageFromRgba(
      2,
      1,
      new Uint8ClampedArray([9, 9, 9, 255, 8, 8, 8, 255]),
    );
    expect(image.alpha).toBeNull();
    expect(await inflate(image.rgb)).toEqual(
      new Uint8Array([0, 9, 9, 9, 8, 8, 8]),
    );
  });

  it('RGBA の長さが幅と高さに合わなければエラー', async () => {
    await expect(
      imageFromRgba(2, 1, new Uint8ClampedArray([0, 0, 0])),
    ).rejects.toThrow();
  });
});

describe('buildPdf', () => {
  it('相互参照表の各位置が `N 0 obj` の先頭を指す(1 ページ・2 ページ・画像あり・画像なし)', async () => {
    // Pages without an image (2), with an image but no α (3), and with α (4)
    for (const [pages, perPage] of [
      [[page(600, 450)], [2]],
      [
        [page(600, 450), page(300, 200)],
        [2, 2],
      ],
      [
        [page(600, 450), await pageWithImage(), await opaquePage()],
        [2, 4, 3],
      ],
    ]) {
      const pdf = buildPdf(pages as PdfPage[]);
      const offsets = xrefOffsets(pdf);
      // The Catalog, the Pages, the per-page objects, and the free object
      expect(offsets.length).toBe(
        3 + (perPage as number[]).reduce((sum, n) => sum + n, 0),
      );
      expect(offsets[0]).toBe(0);
      for (let number = 1; number < offsets.length; number++) {
        expect(
          startsWith(pdf.subarray(offsets[number]), ascii(`${number} 0 obj`)),
        ).toBe(true);
      }
    }
  });

  it('/Count と /Kids がページ数に合い、MediaBox の数は末尾の 0 を落として書く', async () => {
    // A page without an image uses 2 objects, and a page with an image that has α uses 4
    const text = new TextDecoder().decode(
      buildPdf([page(600, 450), await pageWithImage()]),
    );
    expect(text).toContain('<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>');
    expect(text).toContain('/MediaBox [0 0 600 450]');
    const one = new TextDecoder().decode(buildPdf([page(450.75, 200)]));
    expect(one).toContain('/MediaBox [0 0 450.75 200]');
  });

  it('Page が Contents と画像を指す(画像が無ければ /Resources が空)', async () => {
    const pdf = buildPdf([await pageWithImage()]);
    const pageDict = linesContaining(pdf, '/Type /Page ').pop() as string;
    // The content (4), the image (5), and the SMask (6)
    expect(pageDict).toContain('/Resources << /XObject << /Im0 5 0 R >> >>');
    expect(pageDict).toContain('/Contents 4 0 R');
    // The content's /Im0 goes in as-is
    expect(new TextDecoder().decode(pdf)).toContain('/Im0 Do');

    const noImage = buildPdf([page(100, 100)]);
    const dict = linesContaining(noImage, '/Type /Page ').pop() as string;
    expect(dict).toContain('/Resources << >>');
  });

  it('Contents は圧縮しない(/Length はバイト数)', () => {
    const content = new TextEncoder().encode('0 0 600 450 re W n');
    const single = page(800, 600);
    single.content = content;
    const text = new TextDecoder().decode(buildPdf([single]));
    expect(text).toContain(
      '<< /Length 18 >>\nstream\n0 0 600 450 re W n\nendstream\nendobj\n',
    );
  });

  it('Image オブジェクトが RGBA に合う(α があれば SMask)', async () => {
    const pdf = buildPdf([await pageWithImage()]);
    // The two Images: the RGB (5) and the SMask (6)
    const dicts = linesContaining(pdf, '/Subtype /Image');
    expect(dicts.length).toBe(2);
    const color = dicts.filter((dict) => dict.includes('/DeviceRGB'));
    const smask = dicts.filter((dict) => !dict.includes('/DeviceRGB'));
    expect(color.length).toBe(1);
    expect(smask.length).toBe(1);
    expect(dictNumber(color[0], '/Width')).toBe(2);
    expect(dictNumber(color[0], '/Height')).toBe(2);
    expect(dictNumber(color[0], '/Columns')).toBe(2);
    expect(dictNumber(color[0], '/Colors')).toBe(3);
    expect(color[0]).toContain('/Predictor 15');
    expect(dictNumber(color[0], '/SMask')).toBe(6);
    expect(smask[0]).toContain('/DeviceGray');
    expect(dictNumber(smask[0], '/Colors')).toBe(1);
    expect(dictNumber(smask[0], '/Columns')).toBe(2);

    // The stream is in zlib format (the first 2 bytes are `78 xx`), and its length is /Length
    for (const dict of [color[0], smask[0]]) {
      const data = streamAfterDict(pdf, dict);
      expect(data.length).toBe(dictNumber(dict, '/Length'));
      expect(data[0]).toBe(0x78);
    }
  });

  it('α が全部 255 なら /SMask が無い', async () => {
    const pdf = buildPdf([await opaquePage()]);
    const text = new TextDecoder().decode(pdf);
    expect(text).not.toContain('/SMask');
    // Only the content (4) and one Image (5), and /Im0 points to it
    const pageDict = linesContaining(pdf, '/Type /Page ').pop() as string;
    expect(pageDict).toContain('/Im0 5 0 R');
  });
});
