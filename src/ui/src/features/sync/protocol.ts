// This file is generated from src/bridge/src/protocol.rs. Do not edit it directly.
// To regenerate it, run `UPDATE_PROTOCOL_TS=1 cargo test protocol` in src/bridge.

/** Whether a stroke was written by a human or the AI */
export type Author = 'human' | 'ai';

/** A change to the canvas. Used both page → bridge and bridge → page */
export type CanvasOp =
  /** Inserts a stroke at position index (stacking order). Appends it if out of range */
  | { type: 'insert'; stroke: Stroke; index: number }
  /** Removes strokes. Unknown IDs are ignored */
  | { type: 'remove'; ids: string[] }
  /** Replaces the stroke with the same ID, keeping its stacking order */
  | { type: 'replace'; stroke: Stroke };

/** Canvas page → bridge. One JSON object per line */
export type ClientMessage =
  /** Sent first after connecting. token is the one-time token from the QR code URL */
  | { type: 'hello'; token: string; viewport: Viewport; page?: PageSize | null; resume?: string | null }
  /** The visible area or paper size changed (screen size, zoom, pan) */
  | { type: 'resize'; viewport: Viewport; page?: PageSize | null }
  | { type: 'op'; op: CanvasOp };

/** Size of the canvas (paper) in CSS pixels. The paper starts at (0, 0) and matches the canvas area on the page (what is visible at 100%) */
export interface PageSize {
  width: number;
  height: number;
}

/** Bridge → canvas page. One JSON object per line */
export type ServerMessage =
  /** The token is valid; waiting for approval on the PC */
  | { type: 'awaitingApproval' }
  /** The connection was refused. The connection is closed after this */
  | { type: 'rejected'; reason: string }
  /** Resume key. Sent when approved (followed by snapshot) and when the key is replaced while connected.
Sending the last received key with hello reconnects without waiting for approval on the PC (usable once) */
  | { type: 'resumeKey'; key: string }
  /** Another connection reconnected with the key handed to this connection. The connection is closed after this */
  | { type: 'superseded' }
  /** Approved. The whole canvas held by the bridge. The page adopts it,
keeps strokes only it has (such as ones written before the bridge restarted), and sends them back with insert */
  | { type: 'snapshot'; strokes: Stroke[] }
  | { type: 'op'; op: CanvasOp };

export interface Stroke {
  /** ID unique across the whole canvas */
  id: string;
  author: Author;
  points: StrokePoint[];
  /** Stroke color ("#rrggbb"). Absent on strokes from before colors existed, treated as black */
  color?: string;
}

/** A point of a stroke. Coordinates are CSS pixels with the origin at the top left of the canvas */
export interface StrokePoint {
  x: number;
  y: number;
  /** Pen pressure (0 to 1) */
  pressure: number;
}

/** Area currently visible on the canvas page (canvas coordinates, CSS pixels). Changes with zoom and pan */
export interface Viewport {
  width: number;
  height: number;
  /** Top left of the visible area. Canvas pages from before zoom support do not send it, so it is treated as 0 */
  x?: number;
  y?: number;
}
