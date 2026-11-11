import { describe, expect, it } from 'vitest';
import { LineReader } from './lineReader';

describe('LineReader', () => {
  it('途中で切れて届いた行をつなぎ、揃った行だけを返す', () => {
    const reader = new LineReader();
    expect(reader.push('{"a"')).toEqual([]);
    expect(reader.push(':1}\n{"b":2}\n{"c"')).toEqual(['{"a":1}', '{"b":2}']);
    expect(reader.push(':3}\n')).toEqual(['{"c":3}']);
  });
});
