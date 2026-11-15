/** Stores image bytes and the decoded bitmaps used for drawing, by ID.
 *
 * Only the page has them (sent back from the page after a bridge restart; gone with the strokes when the tab is closed).
 * Decoding (createImageBitmap) is asynchronous, so onChange is called when it finishes to redraw
 */
export class ImageStore {
  readonly #bytes = new Map<string, Uint8Array>();
  readonly #bitmaps = new Map<string, ImageBitmap>();
  readonly #natural = new Map<string, { width: number; height: number }>();
  #onChange: () => void = () => {};

  /** Called when decoding finishes and a redraw is needed (set by whoever schedules redrawing) */
  setOnChange(onChange: () => void): void {
    this.#onChange = onChange;
  }

  /** Stores the bytes and starts decoding in the background (the bytes are kept even if decoding fails) */
  put(id: string, bytes: Uint8Array): void {
    this.#bytes.set(id, bytes);
    void this.#decode(id, bytes);
  }

  async #decode(id: string, bytes: Uint8Array): Promise<void> {
    // In environments without createImageBitmap, such as tests (jsdom), continue without being able to draw
    if (typeof createImageBitmap !== 'function') {
      return;
    }
    try {
      const bitmap = await createImageBitmap(
        new Blob([bytes.slice() as BlobPart], { type: 'image/png' }),
      );
      // If it was replaced by another image while loading, discard it (close the bitmap that will not be shown to free it)
      if (this.#bytes.get(id) !== bytes) {
        bitmap.close();
        return;
      }
      this.#bitmaps.set(id, bitmap);
      this.#natural.set(id, { width: bitmap.width, height: bitmap.height });
      this.#onChange();
    } catch (error) {
      // Images that cannot be read are not drawn (it behaves as a canvas with strokes only).
      // So that it is not silent during development, the reason is printed to the console
      console.error('[image] failed to decode', id, error);
    }
  }

  /**
   * Keeps only the images with the given IDs and discards the rest (ImageBitmaps are closed to return memory).
   * Pass only the images that may be shown (the one placed now and those undo/redo can bring back).
   * A discarded image is not sent even if the PC asks for it (it will never be shown again)
   */
  retain(ids: Iterable<string>): void {
    const keep = new Set(ids);
    for (const id of [...this.#bytes.keys()]) {
      if (keep.has(id)) {
        continue;
      }
      this.#bytes.delete(id);
      this.#natural.delete(id);
      this.#bitmaps.get(id)?.close();
      this.#bitmaps.delete(id);
    }
  }

  has(id: string): boolean {
    return this.#bytes.has(id);
  }

  get(id: string): Uint8Array | undefined {
    return this.#bytes.get(id);
  }

  /** The bitmap if decoding has finished (undefined if not yet). Drawn together with the placement */
  bitmap(id: string): ImageBitmap | undefined {
    return this.#bitmaps.get(id);
  }

  /** Original size (pixels). Unknown for images not yet decoded */
  natural(id: string): { width: number; height: number } | undefined {
    return this.#natural.get(id);
  }
}
