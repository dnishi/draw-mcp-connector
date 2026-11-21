/**
 * Builds a PDF with the page content (stroke lines, etc.) and images.
 *
 * Writing the objects in number order and building the cross-reference table is
 * this file's job; exportFile.ts builds the page content (Contents). The object
 * order and the xref table format follow ISO 32000-1:2008. The image data is a
 * sequence of rows, each prefixed with filter 0, compressed with zlib (RFC 1950)
 * (the same shape as concatenated PNG IDAT data), so it can be read as-is with
 * FlateDecode and the PNG predictor (Predictor 15)
 */

/** An image to embed in the PDF (built from RGBA whose colors are not multiplied by α) */
export interface PdfImage {
  width: number;
  height: number;
  /** The RGB rows, each prefixed with filter 0, compressed with zlib */
  rgb: Uint8Array<ArrayBuffer>;
  /** The α channel (one channel of the same shape). null if it is all 255 */
  alpha: Uint8Array<ArrayBuffer> | null;
}

/** One page of the PDF */
export interface PdfPage {
  /** The page size (pt) */
  widthPt: number;
  heightPt: number;
  /** The page content (the Contents stream) */
  content: Uint8Array;
  /** The image the content refers to as `/Im0` */
  image: PdfImage | null;
}

/**
 * Compresses pixel rows of `colors` channels with filter 0 prefixed to each row,
 * in zlib format. Used for both RGB (3 channels) and α (1 channel).
 * "deflate" in the Compression Streams spec is the zlib format (RFC 1950)
 */
async function zlibRows(
  pixels: Uint8Array,
  width: number,
  colors: number,
): Promise<Uint8Array<ArrayBuffer>> {
  const height = pixels.length / (width * colors);
  // Each row: 1 byte of filter type (0 = none) + width × channel count
  const rows = new Uint8Array(height * (1 + width * colors));
  for (let y = 0; y < height; y++) {
    const rowStart = y * (1 + width * colors);
    rows[rowStart] = 0;
    rows.set(
      pixels.subarray(y * width * colors, (y + 1) * width * colors),
      rowStart + 1,
    );
  }
  const stream = new CompressionStream('deflate');
  const writer = stream.writable.getWriter();
  await writer.write(rows);
  await writer.close();
  return new Uint8Array(await new Response(stream.readable).arrayBuffer());
}

/** Builds a PdfImage from RGBA whose colors are not multiplied by α. The alpha is null if it is all 255 */
export async function imageFromRgba(
  width: number,
  height: number,
  rgba: Uint8ClampedArray | Uint8Array,
): Promise<PdfImage> {
  const pixels = width * height;
  if (rgba.length !== pixels * 4) {
    throw new Error('The RGBA length does not match the width and height.');
  }
  const rgb = new Uint8Array(pixels * 3);
  const alpha = new Uint8Array(pixels);
  let hasAlpha = false;
  for (let i = 0; i < pixels; i++) {
    rgb[i * 3] = rgba[i * 4];
    rgb[i * 3 + 1] = rgba[i * 4 + 1];
    rgb[i * 3 + 2] = rgba[i * 4 + 2];
    const a = rgba[i * 4 + 3];
    alpha[i] = a;
    if (a !== 255) {
      hasAlpha = true;
    }
  }
  return {
    width,
    height,
    rgb: await zlibRows(rgb, width, 3),
    alpha: hasAlpha ? await zlibRows(alpha, width, 1) : null,
  };
}

/** The bytes of a PDF made of the pages in order */
export function buildPdf(pages: readonly PdfPage[]): Uint8Array<ArrayBuffer> {
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [];
  const offsets: number[] = [];
  let position = 0;
  const write = (bytes: Uint8Array | string): void => {
    const data = typeof bytes === 'string' ? encoder.encode(bytes) : bytes;
    chunks.push(data);
    position += data.length;
  };
  // Record the object's start position and write `N 0 obj`
  const startObject = (number: number): void => {
    offsets.push(position);
    write(`${number} 0 obj\n`);
  };
  const dictObject = (number: number, dict: string): void => {
    startObject(number);
    write(`${dict}\nendobj\n`);
  };
  // Contents: not compressed. /Length is the byte count
  const writeContents = (number: number, content: Uint8Array): void => {
    startObject(number);
    write(`<< /Length ${content.length} >>\nstream\n`);
    write(content);
    write('\nendstream\nendobj\n');
  };
  // Image: after the dictionary, the zlib data (rows each prefixed with filter 0)
  const writeImageObject = (number: number, dict: string, data: Uint8Array) => {
    startObject(number);
    write(`${dict} /Length ${data.length} >>\nstream\n`);
    write(data);
    write('\nendstream\nendobj\n');
  };
  // The image dictionary. /SMask only when there is α
  const imageDict = (
    width: number,
    height: number,
    colorSpace: string,
    colors: number,
    smask?: number,
  ): string =>
    `<< /Type /XObject /Subtype /Image /Width ${width} /Height ${height} /ColorSpace ${colorSpace} /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors ${colors} /BitsPerComponent 8 /Columns ${width} >>${smask === undefined ? '' : ` /SMask ${smask} 0 R`}`;

  // Header. The second line is the 4 bytes marking the file as binary (it becomes UTF-8 if written as a string, so write bytes)
  write('%PDF-1.4\n%');
  write(new Uint8Array([0xe2, 0xe3, 0xcf, 0xd3]));
  write('\n');

  // Object numbers: assigned in writing order starting from 1 (1 = Catalog, 2 = Pages).
  // Every page always has a Page and Contents. /Resources is empty without an image,
  // and there is no SMask without α
  const perPage = pages.map(
    (page) => 2 + (page.image ? 1 : 0) + (page.image?.alpha ? 1 : 0),
  );
  dictObject(1, '<< /Type /Catalog /Pages 2 0 R >>');
  const pageNumbers: number[] = [];
  let next = 3;
  for (const size of perPage) {
    pageNumbers.push(next);
    next += size;
  }
  const kids = pageNumbers.map((number) => `${number} 0 R`).join(' ');
  dictObject(2, `<< /Type /Pages /Kids [${kids}] /Count ${pages.length} >>`);
  for (let i = 0; i < pages.length; i++) {
    const page = pages[i];
    const number = pageNumbers[i];
    const image = page.image;
    const imageNumber = image ? number + 2 : undefined;
    const smaskNumber = image?.alpha ? number + 3 : undefined;
    const resources = imageNumber
      ? `<< /XObject << /Im0 ${imageNumber} 0 R >> >>`
      : '<< >>';
    dictObject(
      number,
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${pt(page.widthPt)} ${pt(page.heightPt)}] /Resources ${resources} /Contents ${number + 1} 0 R >>`,
    );
    writeContents(number + 1, page.content);
    if (image) {
      writeImageObject(
        number + 2,
        imageDict(image.width, image.height, '/DeviceRGB', 3, smaskNumber),
        image.rgb,
      );
      // SMask only when there is α
      if (image.alpha) {
        writeImageObject(
          number + 3,
          imageDict(image.width, image.height, '/DeviceGray', 1),
          image.alpha,
        );
      }
    }
  }

  // Cross-reference table: records the start position of each object (each row exactly 20 bytes)
  const xrefPosition = position;
  const size = offsets.length + 1; // the free object (number 0) and the objects
  write(`xref\n0 ${size}\n`);
  write('0000000000 65535 f \n');
  for (const offset of offsets) {
    write(`${String(offset).padStart(10, '0')} 00000 n \n`);
  }
  write(
    `trailer\n<< /Size ${size} /Root 1 0 R >>\nstartxref\n${xrefPosition}\n%%EOF\n`,
  );

  const pdf = new Uint8Array(
    chunks.reduce((sum, chunk) => sum + chunk.length, 0),
  );
  let offset = 0;
  for (const chunk of chunks) {
    pdf.set(chunk, offset);
    offset += chunk.length;
  }
  return pdf;
}

/** Formats a number (pt) to at most 3 decimal places, dropping trailing zeros and the dot (600 → `600`, 450.75 → `450.75`) */
export function pt(value: number): string {
  return value.toFixed(3).replace(/0+$/, '').replace(/\.$/, '');
}
