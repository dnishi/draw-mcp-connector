import type { ReactNode } from 'react';

/** Drawn like lucide icons (24 coordinates, line width 2, round caps and joins) */
function LineIcon({ children }: { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

/** Erase whole strokes: a wavy line and an × (the whole line disappears) */
export function StrokeEraseIcon() {
  return (
    <LineIcon>
      <path d="M3 17C6 11 9 9 12 12C15 15 18 13 21 7" />
      <path d="M16 16l5 5M21 16l-5 5" />
    </LineIcon>
  );
}

/**
 * Erase by tracing: a hollow circle (the eraser) sits on the line, and the line inside it is gone.
 * The circle has the same shape as the eraser circle shown on screen while tracing
 */
export function PartialEraseIcon() {
  return (
    <LineIcon>
      <circle cx="12" cy="12" r="5.5" />
      {/* The line stops at the circle's edge (it is erased inside the circle) */}
      <path d="M2 18Q4 15 6.83 13.88" />
      <path d="M16.98 9.68Q19.5 9 22 6" />
    </LineIcon>
  );
}
