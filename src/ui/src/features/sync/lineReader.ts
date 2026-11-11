/** Splits text arriving on a stream into lines separated by newlines */
export class LineReader {
  private rest = '';

  /** Adds the received text and returns the completed lines (without the trailing newline) */
  push(chunk: string): string[] {
    const lines = (this.rest + chunk).split('\n');
    this.rest = lines.pop() ?? '';
    return lines.filter((line) => line.length > 0);
  }
}
