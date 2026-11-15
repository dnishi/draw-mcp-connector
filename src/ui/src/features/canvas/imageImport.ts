import type { ImagePlacement, PageSize } from '../sync/protocol';
import { newStrokeId } from './strokeId';

/** Upper limit of the longer side of an image to send (pixels). Keeps transfer, memory and PC-side drawing costs down */
export const MAX_IMAGE_SIDE = 2000;

export interface ImportedSize {
  width: number;
  height: number;
}

/**
 * Re-encodes an imported image into PNG, shrinking the longer side to the limit.
 * The format is unified to PNG, so the PC (bridge) side only has to read PNG
 */
export async function encodeImageFile(
  file: Blob,
): Promise<{ png: Uint8Array; natural: ImportedSize }> {
  const bitmap = await createImageBitmap(file);
  const shrink = Math.min(
    1,
    MAX_IMAGE_SIDE / bitmap.width,
    MAX_IMAGE_SIDE / bitmap.height,
  );
  const width = Math.max(1, Math.round(bitmap.width * shrink));
  const height = Math.max(1, Math.round(bitmap.height * shrink));

  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error("Couldn't get a 2D canvas to encode the image.");
  }
  context.drawImage(bitmap, 0, 0, width, height);
  bitmap.close();

  const blob = await new Promise<Blob>((resolve, reject) =>
    canvas.toBlob(
      (encoded) =>
        encoded
          ? resolve(encoded)
          : reject(new Error("Couldn't encode the image as PNG.")),
      'image/png',
    ),
  );
  return {
    png: new Uint8Array(await blob.arrayBuffer()),
    natural: { width, height },
  };
}

/**
 * Placement right after import: in the middle of the paper, at the original size or smaller (the smaller of the original size and the fit), enlarged until the longer side fits the paper
 */
export function initialPlacement(
  natural: ImportedSize,
  page: PageSize,
  id: string = newStrokeId(),
): ImagePlacement {
  const scale = Math.min(
    1,
    page.width / natural.width,
    page.height / natural.height,
  );
  const width = natural.width * scale;
  const height = natural.height * scale;
  return {
    id,
    x: (page.width - width) / 2,
    y: (page.height - height) / 2,
    width,
    height,
  };
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/** Moves the image to a position where it does not stick out of the paper (when the image is larger, allows the visible part) */
export function clampPlacement(
  placement: ImagePlacement,
  page: PageSize,
): ImagePlacement {
  if (page.width === 0 || page.height === 0) {
    return placement;
  }
  const axis = (value: number, size: number, paper: number) =>
    clamp(value, Math.min(0, paper - size), Math.max(0, paper - size));
  return {
    ...placement,
    x: axis(placement.x, placement.width, page.width),
    y: axis(placement.y, placement.height, page.height),
  };
}

/** Resizes by a ratio (%) to the original size, keeping the center (for the slider) */
export function resizePlacement(
  placement: ImagePlacement,
  natural: ImportedSize,
  percent: number,
  page: PageSize,
): ImagePlacement {
  const width = (natural.width * percent) / 100;
  const height = (natural.height * percent) / 100;
  const center = {
    x: placement.x + placement.width / 2,
    y: placement.y + placement.height / 2,
  };
  return clampPlacement(
    {
      ...placement,
      width,
      height,
      x: center.x - width / 2,
      y: center.y - height / 2,
    },
    page,
  );
}
