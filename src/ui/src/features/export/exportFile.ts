import type { ImageUnderlay, StrokeRenderer } from '../canvas/StrokeRenderer';
import { strokeColor, toRgb } from '../canvas/color';
import type { Stroke } from '../canvas/stroke';
import { radiusAt, smoothPoints } from '../canvas/stroke';
import { strokesMesh } from '../canvas/strokeMesh';
import {
  buildPdf,
  imageFromRgba,
  pt,
  type PdfImage,
  type PdfPage,
} from './pdf';

/** The export resolution: 1 CSS pixel = 2 pixels */
export const EXPORT_SCALE = 2;
/** The upper bound in pixels for the long side of a PNG export. Larger areas use a lower scale */
export const MAX_EXPORT_SIDE = 4096;
/** The size in pt of one CSS pixel in a PDF. 96 CSS px = 1 in, 72 pt = 1 in */
export const PT_PER_CSS_PIXEL = 0.75;

export type ExportFormat = 'png' | 'pdf';
export type ExportTarget = 'current' | 'all';

export interface Area {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The scale for a PNG export. Only when the long side exceeds MAX_EXPORT_SIDE pixels at the default 2×,
 * lower the scale so that the long side is MAX_EXPORT_SIDE pixels */
export function exportScale(area: Area): number {
  return Math.min(
    EXPORT_SCALE,
    MAX_EXPORT_SIDE / Math.max(area.width, area.height),
  );
}

/** The default file name (without the extension). The time is local. sheet is the sheet number of a single-sheet export (1-based), null for all sheets */
export function defaultBaseName(now: Date, sheet: number | null): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}`;
  const base = `draw-mcp-${date}-${time}`;
  return sheet === null ? base : `${base}-sheet-${sheet}`;
}

/** Appends the extension to a user-entered name. Uses the fallback if it is only whitespace. Does not append it if the name already ends with the same extension (case-insensitive) */
export function fileName(
  input: string,
  fallback: string,
  format: ExportFormat,
): string {
  const base = input.trim() === '' ? fallback : input;
  const extension = `.${format}`;
  return base.toLowerCase().endsWith(extension) ? base : `${base}${extension}`;
}

/** Clamps the area inside the sheet. null if they do not overlap */
export function clampArea(
  area: Area,
  page: { width: number; height: number },
): Area | null {
  const left = Math.min(Math.max(area.x, 0), page.width);
  const top = Math.min(Math.max(area.y, 0), page.height);
  const right = Math.min(Math.max(area.x + area.width, 0), page.width);
  const bottom = Math.min(Math.max(area.y + area.height, 0), page.height);
  const width = right - left;
  const height = bottom - top;
  if (width <= 0 || height <= 0) {
    return null;
  }
  return { x: left, y: top, width, height };
}

/** The material of one sheet to export */
export interface SheetSource {
  strokes: readonly Stroke[];
  image: ImageUnderlay | undefined;
}

/** The pixels of the sheet's area drawn at the export scale (PNG only) */
function renderSheet(renderer: StrokeRenderer, sheet: SheetSource, area: Area) {
  return renderer.renderArea(
    strokesMesh(sheet.strokes),
    sheet.image,
    area,
    exportScale(area),
  );
}

/** Builds the PNG Blob (for the sheet's area) */
export async function exportPng(
  renderer: StrokeRenderer,
  sheet: SheetSource,
  area: Area,
): Promise<Blob> {
  const { width, height, rgba } = await renderSheet(renderer, sheet, area);
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  // Encode the image to PNG with the 2D context
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error("Couldn't get a 2D context to export the image.");
  }
  context.putImageData(new ImageData(rgba, width, height), 0, 0);
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) =>
        blob
          ? resolve(blob)
          : reject(new Error('The PNG file could not be created.')),
      'image/png',
    );
  });
}

/** Formats a number for the content (canvas coordinates) to at most 2 decimal places, dropping trailing zeros and the dot */
function number(value: number): string {
  return value.toFixed(2).replace(/0+$/, '').replace(/\.$/, '');
}

/**
 * Builds the content of a PDF page (the Contents stream). Cut to the area → map the
 * canvas coordinates onto the page → the image (under the strokes) → the strokes,
 * in that order. The strokes are written not as the rendered pixels but as lines
 * with the same points (`smoothPoints`) and widths (`radiusAt`) as the page rendering
 */
export function pageContent(
  area: Area,
  strokes: readonly Stroke[],
  image: ImageUnderlay | undefined,
): string {
  const width = pt(area.width * PT_PER_CSS_PIXEL);
  const height = pt(area.height * PT_PER_CSS_PIXEL);
  const lines = [
    // 1. Cut away everything outside the page (outside the area)
    `0 0 ${width} ${height} re W n`,
    // 2. Map the canvas coordinates (CSS pixels, y down) onto the page (pt, y up)
    `${pt(PT_PER_CSS_PIXEL)} 0 0 -${pt(PT_PER_CSS_PIXEL)} ${pt(-area.x * PT_PER_CSS_PIXEL)} ${pt((area.y + area.height) * PT_PER_CSS_PIXEL)} cm`,
  ];
  // 3. The image (placed as in the canvas. The coordinates are flipped in y, so the
  //    height is negated and the origin set at the bottom edge to keep it upright)
  if (image) {
    lines.push(
      `q ${number(image.width)} 0 0 -${number(image.height)} ${number(image.x)} ${number(image.y + image.height)} cm /Im0 Do Q`,
    );
  }
  // 4. The strokes (in overlap order)
  for (const stroke of strokes) {
    // A stroke with a different color shape (a data error) is drawn black (as in the page rendering)
    const [red, green, blue] = toRgb(strokeColor(stroke));
    lines.push(
      `${red.toFixed(3)} ${green.toFixed(3)} ${blue.toFixed(3)} RG 1 J 1 j`,
    );
    const points = smoothPoints(stroke.points);
    if (points.length === 1) {
      // With one point only, a zero-length line to the same point (the round cap makes a circle)
      const only = points[0];
      lines.push(
        `${number(radiusAt(only.pressure) * 2)} w ${number(only.x)} ${number(only.y)} m ${number(only.x)} ${number(only.y)} l S`,
      );
    } else {
      for (let i = 0; i + 1 < points.length; i++) {
        // The width is the average of the two points' radii × 2 (= the sum of the radii)
        const lineWidth =
          radiusAt(points[i].pressure) + radiusAt(points[i + 1].pressure);
        lines.push(
          `${number(lineWidth)} w ${number(points[i].x)} ${number(points[i].y)} m ${number(points[i + 1].x)} ${number(points[i + 1].y)} l S`,
        );
      }
    }
  }
  return `${lines.join('\n')}\n`;
}

/** Reads the bitmap of an imported image as RGBA (values whose colors are not multiplied by α) */
async function imageFromUnderlay(image: ImageUnderlay): Promise<PdfImage> {
  const { bitmap } = image;
  const canvas = document.createElement('canvas');
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error("Couldn't get a 2D context to read the image.");
  }
  context.drawImage(bitmap, 0, 0);
  const data = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
  return imageFromRgba(bitmap.width, bitmap.height, data);
}

/** Builds the PDF Blob (the sheets in order, one page per area).
 * The strokes are written as vectors by the content, so no renderer is needed */
export async function exportPdf(
  pages: readonly { sheet: SheetSource; area: Area }[],
): Promise<Blob> {
  const pdfPages: PdfPage[] = [];
  for (const { sheet, area } of pages) {
    pdfPages.push({
      widthPt: area.width * PT_PER_CSS_PIXEL,
      heightPt: area.height * PT_PER_CSS_PIXEL,
      content: new TextEncoder().encode(
        pageContent(area, sheet.strokes, sheet.image),
      ),
      // An imported image is embedded as the original image (color and α), not the rendered pixels
      image: sheet.image ? await imageFromUnderlay(sheet.image) : null,
    });
  }
  return new Blob([buildPdf(pdfPages)], { type: 'application/pdf' });
}

/** Downloads the Blob as a file. The blob URL is released a little later (so it is not released before the download starts on iPad Safari) */
export function download(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 60 * 1000);
}
