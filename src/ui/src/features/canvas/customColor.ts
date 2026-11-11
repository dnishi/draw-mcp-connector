import { useCallback, useState } from 'react';
import { isHexColor } from './color';

/** Local storage key that keeps the custom color */
export const CUSTOM_COLOR_KEY = 'draw-mcp:custom-pen-color';

/** The stored custom color. null if there is none, it is corrupt, or storage is unavailable (private browsing, etc.) */
function readCustomColor(): string | null {
  try {
    const stored = localStorage.getItem(CUSTOM_COLOR_KEY);
    return stored && isHexColor(stored) ? stored : null;
  } catch {
    return null;
  }
}

/** Custom color chosen in the color palette. Kept in local storage and used next time too */
export function useCustomColor(): [string | null, (color: string) => void] {
  const [color, setColor] = useState(readCustomColor);
  const save = useCallback((next: string) => {
    setColor(next);
    try {
      localStorage.setItem(CUSTOM_COLOR_KEY, next);
    } catch {
      // Even if it cannot be stored, it works while this page is open
    }
  }, []);
  return [color, save];
}
