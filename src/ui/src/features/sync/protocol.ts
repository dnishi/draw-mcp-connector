// This file is generated from src/bridge/src/protocol.rs. Do not edit it directly.
// To regenerate it, run `UPDATE_PROTOCOL_TS=1 cargo test protocol` in src/bridge.

/** Whether a stroke was written by a human or the AI */
export type Author = 'human' | 'ai';

/** A change to the canvas. Used both page → bridge and bridge → page.
Stroke and image changes carry the sheet they apply to in `sheet` (the sheet ID) */
export type CanvasOp =
  /** Inserts a stroke at position index (stacking order) on that sheet. Out of range puts it at the end */
  | { type: 'insert'; sheet: string; stroke: Stroke; index: number }
  /** Removes strokes from that sheet. Missing IDs are ignored */
  | { type: 'remove'; sheet: string; ids: string[] }
  /** Replaces the stroke with the same ID on that sheet, keeping the stacking order */
  | { type: 'replace'; sheet: string; stroke: Stroke }
  /** Places the image on that sheet (also used to move or resize it). Replaces any existing image.
The bytes are sent on a separate stream (ImageStream) */
  | { type: 'setImage'; sheet: string; image: ImagePlacement }
  /** Removes the image from that sheet. Ignored if there is no image */
  | { type: 'removeImage'; sheet: string }
  /** Inserts an empty sheet at position index. Out of range puts it at the end */
  | { type: 'addSheet'; id: string; index: number }
  /** Removes a sheet with its contents */
  | { type: 'removeSheet'; id: string }
  /** Changes the current sheet */
  | { type: 'selectSheet'; id: string }
  /** Removes all sheets and makes one empty `INITIAL_SHEET_ID` sheet the current sheet */
  | { type: 'resetSheets' };

/** Canvas page → bridge. One JSON object per line */
export type ClientMessage =
  /** Sent first after connecting. token is the one-time token from the QR code URL */
  | { type: 'hello'; token: string; viewport: Viewport; page?: PageSize | null; protocol?: number; resume?: string | null }
  /** The visible area or paper size changed (screen size, zoom, pan) */
  | { type: 'resize'; viewport: Viewport; page?: PageSize | null }
  | { type: 'op'; op: CanvasOp }
  /** Send to AI was pressed. Tells the AI (MCP's wait_for_canvas) */
  | { type: 'sendToAi' };

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
  /** Approved. The whole canvas held by the bridge (the sheets and the current sheet's ID). The page follows it:
sheets only it has (such as ones written before the bridge restarted) are sent back with addSheet and insert
if they have contents, and empty ones are discarded. The current sheet is the bridge's.
The bridge's images win, but an image only the page has is sent back with setImage.
If the page lacks the bytes of a bridge image, it receives them with an ImageStream request */
  | { type: 'snapshot'; sheets: Sheet[]; current: string }
  | { type: 'op'; op: CanvasOp }
  /** Whether the AI is waiting in wait_for_canvas (waiting) and whether a Send to AI press has not been
picked up by the AI yet (queued). Sent when syncing starts (right after snapshot) and when it changes */
  | { type: 'aiStatus'; waiting: boolean; queued: boolean };

/** Contents of one sheet. The paper size (PageSize) is each connection's screen size and is not kept per sheet */
export interface Sheet {
  /** ID unique across the whole canvas */
  id: string;
  /** Stacking order (later strokes are on top) */
  strokes: Stroke[];
  /** Image placed under the strokes (one per sheet). None if there is none */
  image?: ImagePlacement | null;
}

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

/** Version of the page–bridge protocol. The page sends it in hello, and the bridge rejects a different one.
Pages from before sheets do not send it, so it is taken as 0 */
export const PROTOCOL_VERSION = 1;

/** Maximum number of sheets */
export const MAX_SHEETS = 50;

/** ID of the first sheet, and of the sheet left after deleting all sheets. The bridge and the page start with
the same ID, so after the bridge restarts it matches the first sheet of a reconnecting page */
export const INITIAL_SHEET_ID = 'sheet-1';
