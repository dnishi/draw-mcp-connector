/** move moves the image: a one-finger swipe moves it and a two-finger pinch resizes it */
export type Tool = 'pen' | 'eraser' | 'move';

/** The drawing tools. Where the move tool returns to when the image is gone */
export type DrawingTool = Exclude<Tool, 'move'>;

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
