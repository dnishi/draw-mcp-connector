import { describe, expect, it } from 'vitest';
import { LineReader } from './lineReader';

describe('LineReader', () => {
  it('途中で切れて届いた行をつなぎ、揃った行だけを返す', () => {
    const reader = new LineReader();
    expect(reader.push('{"a"')).toEqual([]);
    expect(reader.push(':1}\n{"b":2}\n{"c"')).toEqual(['{"a":1}', '{"b":2}']);
    expect(reader.push(':3}\n')).toEqual(['{"c":3}']);
  });

  it('1 つの chunk に複数の行があるときは全部返す', () => {
    const reader = new LineReader();
    expect(reader.push('a\nb\nc\n')).toEqual(['a', 'b', 'c']);
    expect(reader.push('d')).toEqual([]);
  });

  it('空の行は返さない', () => {
    const reader = new LineReader();
    expect(reader.push('\n\na\n\n')).toEqual(['a']);
  });

  it('改行の無い chunk では何も返さない', () => {
    const reader = new LineReader();
    expect(reader.push('一部の')).toEqual([]);
    expect(reader.push('行')).toEqual([]);
    expect(reader.push('列\n')).toEqual(['一部の行列']);
  });
});
