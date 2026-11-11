import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CUSTOM_COLOR_KEY, useCustomColor } from './customColor';

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('useCustomColor', () => {
  it('選んだ色をローカルストレージに残し、次に開いたときも使う', () => {
    const first = renderHook(() => useCustomColor());
    expect(first.result.current[0]).toBeNull();

    act(() => first.result.current[1]('#8e44ad'));
    expect(first.result.current[0]).toBe('#8e44ad');
    expect(localStorage.getItem(CUSTOM_COLOR_KEY)).toBe('#8e44ad');

    const next = renderHook(() => useCustomColor());
    expect(next.result.current[0]).toBe('#8e44ad');
  });

  it('壊れた値は使わない', () => {
    localStorage.setItem(CUSTOM_COLOR_KEY, 'purple');
    const { result } = renderHook(() => useCustomColor());
    expect(result.current[0]).toBeNull();
  });

  it('ストレージを使えなくても、開いている間は色を使える', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('使えない');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('使えない');
    });
    const { result } = renderHook(() => useCustomColor());
    expect(result.current[0]).toBeNull();

    act(() => result.current[1]('#8e44ad'));
    expect(result.current[0]).toBe('#8e44ad');
  });
});
