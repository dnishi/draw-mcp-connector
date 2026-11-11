/**
 * Resume key. Received on a connection approved by the PC; sending it with hello after disconnecting reconnects without waiting for approval on the PC.
 * Kept in session storage: it survives reloads in the same tab and disappears when the tab is closed
 */
export const RESUME_KEY_STORAGE_KEY = 'draw-mcp:resume-key';

/** The stored key. null if there is none or storage is unavailable (private browsing, etc.) */
export function readResumeKey(): string | null {
  try {
    return sessionStorage.getItem(RESUME_KEY_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function saveResumeKey(key: string): void {
  try {
    sessionStorage.setItem(RESUME_KEY_STORAGE_KEY, key);
  } catch {
    // Not being able to store it is harmless while connected; reconnecting just waits for approval on the PC
  }
}
