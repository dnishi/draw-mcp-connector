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
  | { type: 'replace'; stroke: Stroke }
  /** Places an image (also used to replace it with a new position or size). Replaces any existing image.
The bytes are sent on a separate stream (ImageStream) */
  | { type: 'setImage'; image: ImagePlacement }
  /** Removes the image. Ignored if there is no image */
  | { type: 'removeImage' };

/** Canvas page → bridge. One JSON object per line */
export type ClientMessage =
  /** Sent first after connecting. token is the one-time token from the QR code URL */
  | { type: 'hello'; token: string; viewport: Viewport; page?: PageSize | null; resume?: string | null }
  /** The visible area or paper size changed (screen size, zoom, pan) */
  | { type: 'resize'; viewport: Viewport; page?: PageSize | null }
  | { type: 'op'; op: CanvasOp };

/** Position and size of an image (canvas coordinates, CSS pixels). Images are placed under all strokes.
Only one per canvas for now (referenced by ID, anticipating several in the future) */
export interface ImagePlacement {
  /** ID unique across the canvas. Shared with the bytes, which are sent on a separate stream (ImageStream) */
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Header on the first line of a separate stream that carries image bytes (opened one-to-one, apart from the bidirectional stream).
provide is followed by the PNG bytes; request has nothing after it. Either the page or the bridge can open one */
export type ImageStream =
  /** Sends the bytes of the image with the given ID */
  | { type: 'provide'; id: string }
  /** The bytes of the image with the given ID are requested. Not sent if it does not have them */
  | { type: 'request'; id: string };

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
  /** The connection was closed from the PC's menu. The connection is closed after this */
  | { type: 'disconnected' }
  /** Approved. The whole canvas the bridge has. The page matches it,
keeps strokes only it has (such as ones written before the bridge restarted), and sends them back with insert.
The bridge's image wins, but an image the page has and the bridge does not is sent back with setImage.
If the page does not have the bytes of the bridge's image, it receives them with an ImageStream request */
  | { type: 'snapshot'; strokes: Stroke[]; image?: ImagePlacement | null }
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
