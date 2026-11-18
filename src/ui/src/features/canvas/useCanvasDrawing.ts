import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react';
import { eraseAlong } from './erase';
import { clampPlacement } from './imageImport';
import type { ImageStore } from './imageStore';
import {
  hitsStroke,
  type Point,
  type Stroke,
  type StrokePoint,
} from './stroke';
import { newStrokeId } from './strokeId';
import { CommittedStrokes, tessellateStroke } from './strokeMesh';
import {
  currentSheet,
  initialStrokeHistory,
  strokeHistoryReducer,
  type ErasedPieces,
  type RemoteAction,
} from './strokeHistory';
import { type ImagePlacement, MAX_SHEETS } from '../sync/protocol';
import type { EraserMode, History, SheetControls, ToolSettings } from './types';
import type { StrokeRenderer } from './StrokeRenderer';
import { useStrokeRenderer } from './useStrokeRenderer';
import {
  INITIAL_VIEW,
  PinchTracker,
  clampView,
  pinchView,
  toCanvasPoint,
  toScreenPoint,
  visibleRange,
  type FingerPair,
  type View,
} from './view';

/** Radius of the whole-stroke eraser (CSS pixels on the view) */
const STROKE_ERASER_RADIUS = 12;
/**
 * With the move tool (Move Image), a press and release that moves less than this (screen CSS pixels) is a click.
 * The image does not move until it is exceeded (so small finger or pen jitter does not turn a click into a move)
 */
const CLICK_SLOP = 4;
/** Empty vertex array passed when no stroke is being drawn (tools other than the pen) */
const EMPTY_VERTICES = new Float32Array(0);

/** Reports image operations with the move tool (Move Image) (opens and closes the popover near the image) */
export interface ImageEvents {
  /** Pressed and released on the image without moving. at is the release point (page coordinates, clientX / clientY) */
  onClick(at: { x: number; y: number }): void;
  /** Pressed and released outside the image without moving */
  onClickOutside(): void;
  /** Started moving the image (drag or pinch) */
  onGestureStart(): void;
}

const NO_IMAGE_EVENTS: ImageEvents = {
  onClick: () => {},
  onClickOutside: () => {},
  onGestureStart: () => {},
};

/** One operation from putting the pen down to lifting it */
type Gesture = {
  pointerId: number;
  /** pen, mouse, or touch (when drawing with a finger) */
  pointerType: string;
  bounds: DOMRect;
} & (
  | { tool: 'pen'; color: string; points: StrokePoint[] }
  | {
      tool: 'eraser';
      mode: EraserMode;
      /** Eraser radius (canvas coordinates), divided by the zoom factor so it keeps the same size on the view */
      radius: number;
      /** Previous eraser position */
      last: StrokePoint;
      /** IDs of erased strokes and what remains of them */
      pieces: Map<string, readonly Stroke[]>;
    }
);

/** One operation that moves the image (from pressing the pointer to releasing it) */
type ImageGesture = {
  pointerId: number;
  /** Canvas point when the press started */
  origin: Point;
  /** Screen point when the press started (tells a click from a drag) */
  downScreen: Point;
  /** Whether the press was on the image (even a press outside can move the image by dragging) */
  onImage: boolean;
  /** Whether it moved beyond CLICK_SLOP. Until then it is treated as a click and the image does not move */
  dragging: boolean;
  /** Placement when the operation started */
  start: ImagePlacement;
  /** Placement while moving (committed on release) */
  working: ImagePlacement;
};

/** Zooming the image with a pinch with the move tool (Move Image) */
type ImagePinch = {
  /** Distance between the two fingers when the pinch started (0 until the first move) */
  startSpread: number;
  /** Placement when the pinch started */
  start: ImagePlacement;
  /** Placement while moving (committed when the fingers are lifted) */
  working: ImagePlacement;
};

/** Ratio to the original size allowed for a pinch with the move tool (Move Image) */
const IMAGE_SCALE_RANGE = { min: 0.05, max: 4 };

/** Strokes visible while the eraser moves (the remaining pieces replace the erased strokes) */
function visibleStrokes(
  strokes: readonly Stroke[],
  pieces: ErasedPieces,
): readonly Stroke[] {
  if (pieces.size === 0) {
    return strokes;
  }
  return strokes.flatMap((stroke) => pieces.get(stroke.id) ?? [stroke]);
}

/** A point on the view, with the origin at the canvas element's top left */
function toScreen(event: PointerEvent, bounds: DOMRect): Point {
  return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
}

/** Converts a pen point into a stroke point in canvas coordinates */
function toStrokePoint(
  event: PointerEvent,
  bounds: DOMRect,
  view: View,
): StrokePoint {
  const { x, y } = toCanvasPoint(view, toScreen(event, bounds));
  return { x, y, pressure: event.pressure };
}

/** Fine-grained points from the previous event to this one. Uses only this event's point if getCoalescedEvents is missing or empty */
function coalescedEvents(event: PointerEvent): PointerEvent[] {
  const events = event.getCoalescedEvents?.() ?? [];
  return events.length > 0 ? events : [event];
}

export function useCanvasDrawing(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  { tool, penColor, eraserMode, eraserSize, fingerDrawing }: ToolSettings,
  /** Image bytes and decoded bitmaps */
  images: ImageStore,
  /** When a pen is used (to switch finger drawing automatically) */
  onPenDetected: () => void,
  /** When the image is clicked or starts moving with the move tool (Move Image) */
  imageEvents: ImageEvents = NO_IMAGE_EVENTS,
) {
  const imageMove = tool === 'move';
  const { renderer, error } = useStrokeRenderer(canvasRef);
  const [state, dispatch] = useReducer(
    strokeHistoryReducer,
    initialStrokeHistory,
  );
  // The current sheet (strokes, image, and history are per sheet)
  const sheet = currentSheet(state);
  const strokes = sheet.strokes;
  const image = sheet.image;

  /** One CommittedStrokes per renderer (recreated when the renderer changes) */
  const committedRef = useRef<{
    renderer: StrokeRenderer;
    committed: CommittedStrokes;
  } | null>(null);
  const strokesRef = useRef(strokes);
  const imageRef = useRef(image);
  const gestureRef = useRef<Gesture | null>(null);
  const frameRef = useRef(0);
  /** Circle showing the eraser size while tracing to erase */
  const eraserCursorRef = useRef<HTMLDivElement>(null);
  /** The image being moved (not put into state until release) */
  const imageGestureRef = useRef<ImageGesture | null>(null);
  const imagePinchRef = useRef<ImagePinch | null>(null);
  /** Div showing the image frame while the move tool (Move Image) is selected (requestRender rewrites its position and size directly) */
  const imageFrameRef = useRef<HTMLDivElement>(null);
  const imageMoveRef = useRef(imageMove);
  // So React does not re-render during a pinch, the view is held in a ref and put into state when the pinch ends
  const viewRef = useRef(INITIAL_VIEW);
  const [view, setView] = useState(INITIAL_VIEW);
  const pinchRef = useRef(new PinchTracker());
  /** Size of the canvas (paper). Same as the size of the canvas element (what is visible at 100%) */
  const [page, setPage] = useState({ width: 0, height: 0 });
  // Used by pinch handling (reads the latest size from functions recreated on every pointer event)
  const pageRef = useRef(page);

  /** Shows the image frame with a dashed line while the move tool (Move Image) is selected (moved in the same coordinate system as the canvas) */
  const updateImageFrame = useCallback((placement: ImagePlacement | null) => {
    const frame = imageFrameRef.current;
    if (!frame) {
      return;
    }
    const view = viewRef.current;
    if (!placement || !imageMoveRef.current) {
      frame.hidden = true;
      return;
    }
    const topLeft = toScreenPoint(view, placement);
    frame.hidden = false;
    frame.style.width = `${placement.width * view.scale}px`;
    frame.style.height = `${placement.height * view.scale}px`;
    frame.style.transform = `translate(${topLeft.x}px, ${topLeft.y}px)`;
  }, []);

  // So React does not re-render on every pen move, drawing is done directly with requestAnimationFrame
  const requestRender = useCallback(() => {
    if (!renderer || frameRef.current) {
      return;
    }
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = 0;
      const gesture = gestureRef.current;
      const strokes =
        gesture && gesture.tool !== 'pen'
          ? visibleStrokes(strokesRef.current, gesture.pieces)
          : strokesRef.current;
      const active = gesture?.tool === 'pen' ? gesture : undefined;
      const placement =
        imagePinchRef.current?.working ??
        imageGestureRef.current?.working ??
        imageRef.current;
      const bitmap = placement ? images.bitmap(placement.id) : undefined;
      if (committedRef.current?.renderer !== renderer) {
        committedRef.current = { renderer, committed: new CommittedStrokes() };
      }
      const { committed } = committedRef.current;
      // While erasing, visibleStrokes makes a new array, so this is a rebuild.
      // Rewriting everything is the same amount of work as before, so it adds no extra cost
      renderer.draw(
        committed.update(strokes),
        active ? tessellateStroke(active.points, active.color) : EMPTY_VERTICES,
        viewRef.current,
        placement && bitmap ? { bitmap, ...placement } : undefined,
      );
      updateImageFrame(placement);
    });
  }, [renderer, images, updateImageFrame]);

  useEffect(() => () => cancelAnimationFrame(frameRef.current), []);

  useEffect(() => {
    strokesRef.current = strokes;
    imageRef.current = image;
    requestRender();
  }, [strokes, image, requestRender]);

  // Keep only images that may be shown (those placed now on any sheet, and those
  // undo or redo can bring back). Wiping a sheet empties its history, so its images are dropped
  useEffect(() => {
    const alive = new Set<string>();
    for (const sheet of state.sheets) {
      if (sheet.image) {
        alive.add(sheet.image.id);
      }
      for (const stack of [sheet.undoStack, sheet.redoStack]) {
        for (const op of stack) {
          if (op.type !== 'image') {
            continue;
          }
          if (op.before) {
            alive.add(op.before.id);
          }
          if (op.after) {
            alive.add(op.after.id);
          }
        }
      }
    }
    images.retain(alive);
  }, [state.sheets, images]);

  useEffect(() => {
    imageMoveRef.current = imageMove;
    requestRender();
  }, [imageMove, requestRender]);

  // Redraw when decoding of an image finishes. Unless CanvasPage also re-renders, the slider that
  // reads the original size stays at its pre-decoding state, so update state too
  const [, setDecoded] = useState(0);
  useEffect(() => {
    images.setOnChange(() => {
      setDecoded((count) => count + 1);
      requestRender();
    });
    return () => images.setOnChange(() => {});
  }, [images, requestRender]);

  // Measure the size even without a renderer (WebGPU unavailable), to report the visible area to the PC
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return;
    }
    const observer = new ResizeObserver(() =>
      setPage({ width: canvas.clientWidth, height: canvas.clientHeight }),
    );
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [canvasRef]);

  useEffect(() => {
    renderer?.resize(page.width, page.height, devicePixelRatio);
    requestRender();
  }, [renderer, page, requestRender]);

  /** Commits the view (when a pinch ends, when resetting, when the paper size changes) */
  const commitView = useCallback(
    (next: View) => {
      viewRef.current = next;
      setView(next);
      requestRender();
    },
    [requestRender],
  );

  // When the current sheet changes (switched on this page or by a switch from the PC),
  // cancel any stroke, erasing, or image move in progress, and reset the view to 100%
  const currentSheetId = state.current;
  const previousSheetIdRef = useRef(currentSheetId);
  useEffect(() => {
    if (previousSheetIdRef.current === currentSheetId) {
      return;
    }
    previousSheetIdRef.current = currentSheetId;
    gestureRef.current = null;
    imageGestureRef.current = null;
    imagePinchRef.current = null;
    if (eraserCursorRef.current) {
      eraserCursorRef.current.hidden = true;
    }
    commitView(INITIAL_VIEW);
    requestRender();
  }, [currentSheetId, commitView, requestRender]);

  // When the screen size (= the paper size) changes, fit the visible area inside the new paper again
  useEffect(() => {
    pageRef.current = page;
    commitView(clampView(viewRef.current, page));
  }, [page, commitView]);

  const extendGesture = (gesture: Gesture, point: StrokePoint) => {
    if (gesture.tool === 'pen') {
      gesture.points.push(point);
      return;
    }
    const from = gesture.last;
    gesture.last = point;
    for (const stroke of strokesRef.current) {
      if (gesture.mode === 'stroke') {
        if (hitsStroke(stroke, point, gesture.radius)) {
          gesture.pieces.set(stroke.id, []);
        }
        continue;
      }
      // Parts left over from earlier erasing are erased too when traced again
      let erased = false;
      const pieces = (gesture.pieces.get(stroke.id) ?? [stroke]).flatMap(
        (piece) => {
          const rest = eraseAlong(piece, from, point, gesture.radius);
          if (rest) {
            erased = true;
          }
          return rest ?? [piece];
        },
      );
      if (erased) {
        gesture.pieces.set(stroke.id, pieces);
      }
    }
  };

  /** Eraser radius (CSS pixels on the view). Keeps the same size on the view when zooming */
  const eraserScreenRadius =
    eraserMode === 'stroke' ? STROKE_ERASER_RADIUS : eraserSize / 2;

  /**
   * Shows the eraser circle at the pointer position (hides it if event is null).
   * With a mouse it replaces the OS pointer the whole time the eraser is selected (so the erased area is visible).
   * With a pen it is shown only while tracing to erase
   */
  const updateEraserCursor = (
    event: ReactPointerEvent<HTMLCanvasElement> | null,
  ) => {
    const cursor = eraserCursorRef.current;
    if (!cursor) {
      return;
    }
    const erasing = gestureRef.current !== null && eraserMode === 'partial';
    const visible =
      event !== null &&
      tool === 'eraser' &&
      (event.pointerType === 'mouse' || erasing);
    if (!visible) {
      cursor.hidden = true;
      return;
    }
    const bounds = event.currentTarget.getBoundingClientRect();
    const { x, y } = toScreen(event.nativeEvent, bounds);
    const radius = eraserScreenRadius;
    cursor.hidden = false;
    cursor.style.width = `${radius * 2}px`;
    cursor.style.height = `${radius * 2}px`;
    cursor.style.transform = `translate(${x - radius}px, ${y - radius}px)`;
  };

  // When the tool or erase mode changes, hide the old circle (it reappears on the next pointer move)
  useEffect(() => {
    if (eraserCursorRef.current) {
      eraserCursorRef.current.hidden = true;
    }
  }, [tool, eraserMode]);

  /** Discards the line being drawn or the erase in progress */
  const cancelGesture = () => {
    gestureRef.current = null;
    updateEraserCursor(null);
    requestRender();
  };

  /** Commits an image being moved, if any (finishing a move or handing over to a pinch) */
  const commitImageGesture = () => {
    const gesture = imageGestureRef.current;
    imageGestureRef.current = null;
    if (gesture) {
      dispatch({ type: 'image', image: gesture.working });
    }
  };

  const spread = ([a, b]: FingerPair) => Math.hypot(b.x - a.x, b.y - a.y);

  /** Pinch with the move tool (Move Image): zooms the image by the finger distance ratio, keeping its center */
  const moveImagePinch = (before: FingerPair, after: FingerPair) => {
    const pinch = imagePinchRef.current;
    if (!pinch) {
      return;
    }
    if (pinch.startSpread === 0) {
      pinch.startSpread = spread(before);
    }
    const factor =
      pinch.startSpread > 0 ? spread(after) / pinch.startSpread : 1;
    const start = pinch.start;
    // Limits are set as a ratio to the original size (pixels)
    const natural = images.natural(start.id)?.width;
    const width = Math.min(
      (natural ?? start.width * 2) * IMAGE_SCALE_RANGE.max,
      Math.max(
        (natural ?? start.width) * IMAGE_SCALE_RANGE.min,
        start.width * factor,
      ),
    );
    const height = start.height * (width / start.width);
    const center = {
      x: start.x + start.width / 2,
      y: start.y + start.height / 2,
    };
    pinch.working = clampPlacement(
      {
        ...start,
        width,
        height,
        x: center.x - width / 2,
        y: center.y - height / 2,
      },
      pageRef.current,
    );
    requestRender();
  };

  const commitImagePinch = () => {
    const pinch = imagePinchRef.current;
    imagePinchRef.current = null;
    if (pinch) {
      dispatch({ type: 'image', image: pinch.working });
    }
  };

  /**
   * Tracks fingers (touch). With two fingers it zooms and pans with a pinch and returns true (not used for writing).
   * If a second finger touches while writing with one, the stroke in progress is canceled and it becomes a pinch.
   * With the move tool (Move Image) the pinch zooms the image instead of the canvas
   */
  const handleTouch = (
    event: ReactPointerEvent<HTMLCanvasElement>,
  ): boolean => {
    const pinch = pinchRef.current;
    const canvas = event.currentTarget;
    const screen = toScreen(event.nativeEvent, canvas.getBoundingClientRect());
    switch (event.type) {
      case 'pointerdown':
        canvas.setPointerCapture(event.pointerId);
        pinch.down(event.pointerId, screen);
        if (pinch.count < 2) {
          return false;
        }
        if (gestureRef.current?.pointerType === 'touch') {
          cancelGesture();
        }
        // If the image was being moved, commit it there before starting the pinch
        const base = imageGestureRef.current?.working ?? imageRef.current;
        commitImageGesture();
        imageEvents.onGestureStart();
        if (imageMoveRef.current && base) {
          imagePinchRef.current = {
            startSpread: 0,
            start: { ...base },
            working: { ...base },
          };
        }
        return true;
      case 'pointermove': {
        const moved = pinch.move(event.pointerId, screen);
        if (!moved) {
          return imagePinchRef.current !== null;
        }
        if (imagePinchRef.current) {
          moveImagePinch(moved.before, moved.after);
          return true;
        }
        const next = pinchView(viewRef.current, moved.before, moved.after);
        viewRef.current = clampView(next, pageRef.current);
        requestRender();
        return true;
      }
      default: {
        const ended = pinch.up(event.pointerId);
        if (imagePinchRef.current) {
          if (ended) {
            commitImagePinch();
          }
          return false;
        }
        if (ended) {
          commitView(viewRef.current);
        }
        return false;
      }
    }
  };

  /** Starts moving the image with one pointer (finger, mouse or pen) with the move tool (Move Image) */
  const startImageMove = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const placement = imageRef.current;
    if (!placement) {
      return;
    }
    const canvas = event.currentTarget;
    canvas.setPointerCapture(event.pointerId);
    const downScreen = toScreen(
      event.nativeEvent,
      canvas.getBoundingClientRect(),
    );
    const origin = toCanvasPoint(viewRef.current, downScreen);
    imageGestureRef.current = {
      pointerId: event.pointerId,
      origin,
      downScreen,
      onImage:
        origin.x >= placement.x &&
        origin.x <= placement.x + placement.width &&
        origin.y >= placement.y &&
        origin.y <= placement.y + placement.height,
      dragging: false,
      start: { ...placement },
      working: { ...placement },
    };
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.pointerType === 'pen') {
      onPenDetected();
    }
    // Handle the pinch check first for fingers (placed before the writing check so that even with
    // "Draw with Finger" OFF, one finger can move the image with the move tool (Move Image))
    if (event.pointerType === 'touch' && handleTouch(event)) {
      return;
    }
    // While something is moving, do not start a new one with another pointer
    if (
      gestureRef.current ||
      imageGestureRef.current ||
      imagePinchRef.current
    ) {
      return;
    }
    // With the move tool (Move Image), one finger (including mouse and pen) moves the image.
    // Even without an image, no pen or eraser operation starts
    if (tool === 'move') {
      if (imageRef.current) {
        startImageMove(event);
      }
      return;
    }
    // When finger drawing is off, do not start writing with a finger
    if (event.pointerType === 'touch' && !fingerDrawing) {
      return;
    }
    if (!renderer) {
      return;
    }

    const canvas = event.currentTarget;
    canvas.setPointerCapture(event.pointerId);
    const bounds = canvas.getBoundingClientRect();
    const view = viewRef.current;
    const point = toStrokePoint(event.nativeEvent, bounds, view);
    const { pointerId, pointerType } = event;
    const gesture: Gesture =
      tool === 'pen'
        ? { pointerId, pointerType, bounds, tool, color: penColor, points: [] }
        : {
            pointerId,
            pointerType,
            bounds,
            tool,
            mode: eraserMode,
            radius: eraserScreenRadius / view.scale,
            last: point,
            pieces: new Map(),
          };
    extendGesture(gesture, point);
    gestureRef.current = gesture;
    updateEraserCursor(event);
    requestRender();
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.pointerType === 'touch' && handleTouch(event)) {
      return;
    }
    const imageGesture = imageGestureRef.current;
    if (imageGesture) {
      if (imageGesture.pointerId !== event.pointerId) {
        return;
      }
      const screen = toScreen(
        event.nativeEvent,
        event.currentTarget.getBoundingClientRect(),
      );
      if (!imageGesture.dragging) {
        const { downScreen } = imageGesture;
        if (
          Math.hypot(screen.x - downScreen.x, screen.y - downScreen.y) <=
          CLICK_SLOP
        ) {
          return;
        }
        imageGesture.dragging = true;
        imageEvents.onGestureStart();
      }
      const point = toCanvasPoint(viewRef.current, screen);
      imageGesture.working = clampPlacement(
        {
          ...imageGesture.start,
          x: imageGesture.start.x + (point.x - imageGesture.origin.x),
          y: imageGesture.start.y + (point.y - imageGesture.origin.y),
        },
        pageRef.current,
      );
      requestRender();
      return;
    }
    const gesture = gestureRef.current;
    // Ignore other pointers moving while drawing
    if (gesture && gesture.pointerId !== event.pointerId) {
      return;
    }

    if (gesture) {
      for (const e of coalescedEvents(event.nativeEvent)) {
        extendGesture(
          gesture,
          toStrokePoint(e, gesture.bounds, viewRef.current),
        );
      }
      requestRender();
    }
    // With a mouse, move the eraser circle even when not pressed (hover)
    updateEraserCursor(event);
  };

  const onPointerUp = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.pointerType === 'touch') {
      handleTouch(event);
    }
    const imageGesture = imageGestureRef.current;
    if (imageGesture) {
      if (imageGesture.pointerId !== event.pointerId) {
        return;
      }
      if (imageGesture.dragging) {
        commitImageGesture();
        requestRender();
      } else {
        // Released without moving: a click
        imageGestureRef.current = null;
        if (imageGesture.onImage) {
          imageEvents.onClick({ x: event.clientX, y: event.clientY });
        } else {
          imageEvents.onClickOutside();
        }
      }
      return;
    }
    const gesture = gestureRef.current;
    if (gesture?.pointerId !== event.pointerId) {
      return;
    }

    gestureRef.current = null;
    updateEraserCursor(event);
    if (gesture.tool === 'pen') {
      const stroke: Stroke = {
        id: newStrokeId(),
        author: 'human',
        color: gesture.color,
        points: gesture.points,
      };
      dispatch({ type: 'add', stroke });
    } else {
      dispatch({ type: 'erase', pieces: gesture.pieces });
    }
    requestRender();
  };

  // When the OS or browser takes over the gesture, discard it
  const onPointerCancel = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.pointerType === 'touch') {
      handleTouch(event);
    }
    if (imageGestureRef.current?.pointerId === event.pointerId) {
      imageGestureRef.current = null;
      requestRender();
      return;
    }
    if (gestureRef.current?.pointerId !== event.pointerId) {
      return;
    }
    cancelGesture();
  };

  // Hide the eraser circle when the pointer leaves the canvas (e.g. into the toolbar)
  const onPointerLeave = () => updateEraserCursor(null);

  /** Applies what arrived from the PC */
  const applyRemote: (action: RemoteAction) => void = dispatch;

  const history: History = {
    canUndo: sheet.undoStack.length > 0,
    canRedo: sheet.redoStack.length > 0,
    undo: () => dispatch({ type: 'undo' }),
    redo: () => dispatch({ type: 'redo' }),
  };

  /** Area of the canvas currently visible on this page (reported to the PC) */
  const viewport = useMemo(
    () => visibleRange(view, page.width, page.height),
    [view, page],
  );

  const sheetIndex = state.sheets.findIndex(({ id }) => id === state.current);
  /** Switching and deleting sheets */
  const sheetControls: SheetControls = {
    number: sheetIndex + 1,
    count: state.sheets.length,
    previous:
      sheetIndex > 0
        ? () =>
            dispatch({
              type: 'selectSheet',
              id: state.sheets[sheetIndex - 1].id,
            })
        : null,
    next:
      sheetIndex < state.sheets.length - 1
        ? () =>
            dispatch({
              type: 'selectSheet',
              id: state.sheets[sheetIndex + 1].id,
            })
        : null,
    add:
      state.sheets.length >= MAX_SHEETS
        ? null
        : () => dispatch({ type: 'newSheet', id: newStrokeId() }),
    remove:
      state.sheets.length > 1 ? () => dispatch({ type: 'removeSheet' }) : null,
    removeAll: () => dispatch({ type: 'resetSheets' }),
  };

  return {
    error,
    history,
    /** The committed view (excluding movement during a pinch) */
    view,
    resetView: () => commitView(INITIAL_VIEW),
    /** Whether the current sheet has any stroke or an image (whether it can be cleared) */
    canClear: strokes.length > 0 || image !== null,
    /** Wipes only the current sheet: removes its strokes and image (also sent to the PC), empties its history, and resets the view to 100% */
    clear: () => {
      dispatch({ type: 'clear' });
      commitView(INITIAL_VIEW);
    },
    /** Switching and deleting sheets */
    sheets: sheetControls,
    viewport,
    page,
    /** Image laid under the current sheet's strokes */
    image,
    /** Adds, moves, resizes, or removes (null) the image. Added to the undo history and sent to the PC */
    setImage: (image: ImagePlacement | null) =>
      dispatch({ type: 'image', image }),
    /** Moves only the display first (while the slider is moving). Committed with setImage on release */
    previewImage: (image: ImagePlacement | null) => {
      imageRef.current = image;
      requestRender();
    },
    /** Operations made on this page that the PC has not acked yet (sent to the PC) */
    pending: state.pending,
    applyRemote,
    eraserCursorRef,
    imageFrameRef,
    pointerHandlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel,
      onPointerLeave,
    },
  };
}
