import { describe, expect, it } from 'vitest';
import { cn } from './utils';

describe('cn', () => {
  it('条件付きのクラスを結合する', () => {
    const isHidden = false;
    expect(
      cn('p-2', isHidden && 'hidden', { 'text-sm': true, block: false }),
    ).toBe('p-2 text-sm');
  });

  it('競合する Tailwind のクラスは後に書いたものを残す', () => {
    expect(cn('px-2 py-1 bg-red-500', 'px-4', 'bg-blue-500')).toBe(
      'py-1 px-4 bg-blue-500',
    );
  });
});
