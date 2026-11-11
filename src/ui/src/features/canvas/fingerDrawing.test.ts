import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useFingerDrawing } from './fingerDrawing';

describe('useFingerDrawing', () => {
  it('初めは指で書け、ペンが使われたら指では書かない', () => {
    const { result } = renderHook(() => useFingerDrawing());
    expect(result.current.enabled).toBe(true);

    act(() => result.current.onPenDetected());

    expect(result.current.enabled).toBe(false);
  });

  it('利用者が決めたら、ペンを使っても切り替えない', () => {
    const { result } = renderHook(() => useFingerDrawing());
    act(() => result.current.onPenDetected());
    act(() => result.current.setEnabled(true));

    act(() => result.current.onPenDetected());

    expect(result.current.enabled).toBe(true);
  });
});
