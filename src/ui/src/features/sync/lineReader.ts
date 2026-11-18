/** Splits text arriving on a stream into lines separated by newlines */
export class LineReader {
  /** Pieces of the partial line. Collected here until a newline arrives, then joined */
  private pending: string[] = [];

  /** Adds the received text and returns the completed lines (without the trailing newline) */
  push(chunk: string): string[] {
    const parts = chunk.split('\n');
    const lines: string[] = [];
    // Look for newlines only in chunk. The last piece is incomplete, so it stays in pending
    for (let i = 0; i + 1 < parts.length; i++) {
      this.pending.push(parts[i]);
      lines.push(this.pending.join(''));
      this.pending = [];
    }
    this.pending.push(parts[parts.length - 1]);
    return lines.filter((line) => line.length > 0);
  }
}
