import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Vitest globals are not used, so Testing Library's automatic cleanup does not run. Call it explicitly
afterEach(() => {
  cleanup();
});

// jsdom has no ResizeObserver. Sizes never change, so use one that never reports
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
