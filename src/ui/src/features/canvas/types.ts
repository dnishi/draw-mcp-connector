export type Tool = 'pen' | 'eraser';

/** How the eraser erases. stroke erases every stroke it touches; partial erases only the traced part */
export type EraserMode = 'stroke' | 'partial';

/** What is chosen in the toolbar */
export interface ToolSettings {
  tool: Tool;
  /** Pen color ("#rrggbb") */
  penColor: string;
  eraserMode: EraserMode;
  /** Diameter of the tracing eraser (CSS pixels) */
  eraserSize: number;
  /** Whether one finger draws (erases). Two fingers always pinch */
  fingerDrawing: boolean;
  /** Whether the image is in move mode. When ON, a one-finger swipe moves the image and a two-finger pinch zooms the image */
  imageMove: boolean;
}

export type ConnectionStatus =
  | 'disconnected'
  | 'connecting'
  /** Waiting for approval on the PC */
  | 'waiting'
  | 'connected';

export interface History {
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
}
