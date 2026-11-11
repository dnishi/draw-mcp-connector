/** Length of human stroke IDs (bytes). 12 hex characters */
const STROKE_ID_BYTES = 6;

/**
 * A new ID for a human stroke. A random value shorter than a UUID (36 characters) keeps the stroke list given to the AI short.
 * With 48 bits, the chance of an ID collision is about 2×10⁻⁷ even with 10,000 strokes.
 * AI stroke IDs are created by the bridge with an "ai-" prefix, so they never collide with these
 */
export function newStrokeId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(STROKE_ID_BYTES));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join(
    '',
  );
}
