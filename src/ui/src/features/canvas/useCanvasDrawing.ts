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
import {
  hitsStroke,
  type Point,
  type Stroke,
  type StrokePoint,
} from './stroke';
import { newStrokeId } from './strokeId';
import { buildSceneVertices } from './strokeMesh';
import {
  initialStrokeHistory,
  strokeHistoryReducer,
  type ErasedPieces,
  type RemoteAction,
} from './strokeHistory';
import type { EraserMode, History, ToolSettings } from './types';
import { useStrokeRenderer } from './useStrokeRenderer';
import {
  INITIAL_VIEW,
  PinchTracker,
  clampView,
  pinchView,
  toCanvasPoint,
  visibleRange,
  type View,
} from './view';

/** Radius of the whole-stroke eraser (CSS pixels on the view) */
const STROKE_ERASER_RADIUS = 12;

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

/** Strokes visible while the eraser moves (remaining parts replace the erased strokes) */
function visibleStrokes(
  strokes: readonly Stroke[],
  pieces: ErasedPieces,
): readonly Stroke[] {
  if (pieces.size === 0) return strokes;
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
  {
    tool,
    penColor,
    eraserMode,
    eraserSize,
    fingerDrawing,
    showGrid,
  }: ToolSettings,
  /** Called when a pen is used (to switch finger drawing automatically) */
  onPenDetected: () => void,
) {
  const { renderer, error } = useStrokeRenderer(canvasRef);
  const [state, dispatch] = useReducer(
    strokeHistoryReducer,
    initialStrokeHistory,
  );

  const strokesRef = useRef(state.strokes);
  const gestureRef = useRef<Gesture | null>(null);
  const frameRef = useRef(0);
  /** Circle showing the eraser size while tracing to erase */
  const eraserCursorRef = useRef<HTMLDivElement>(null);
  // Keep the view in a ref during a pinch so React does not re-render, and commit it to state when the pinch ends
  const viewRef = useRef(INITIAL_VIEW);
  const [view, setView] = useState(INITIAL_VIEW);
  const pinchRef = useRef(new PinchTracker());
  const showGridRef = useRef(showGrid);
  /** Size of the canvas (paper). Same as the canvas element's size (what is visible at 100%) */
  const [page, setPage] = useState({ width: 0, height: 0 });
  // Used by pinch handling (reads the latest size from functions recreated on every pointer event)
  const pageRef = useRef(page);

  // Draw directly with requestAnimationFrame so React does not re-render on every pen movement
  const requestRender = useCallback(() => {
    if (!renderer || frameRef.current) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = 0;
      const gesture = gestureRef.current;
      const strokes =
        gesture && gesture.tool !== 'pen'
          ? visibleStrokes(strokesRef.current, gesture.pieces)
          : strokesRef.current;
      const active = gesture?.tool === 'pen' ? gesture : undefined;
      renderer.draw(
        buildSceneVertices(strokes, active),
        viewRef.current,
        showGridRef.current,
      );
    });
  }, [renderer]);

  useEffect(() => () => cancelAnimationFrame(frameRef.current), []);

  useEffect(() => {
    strokesRef.current = state.strokes;
    requestRender();
  }, [state.strokes, requestRender]);

  useEffect(() => {
    showGridRef.current = showGrid;
    requestRender();
  }, [showGrid, requestRender]);

  // Measure the size even without a renderer (WebGPU unavailable), to report the visible area to the PC
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
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

  // When the canvas element's size (= paper size) changes, fit the visible area inside the new paper
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
          if (rest) erased = true;
          return rest ?? [piece];
        },
      );
      if (erased) gesture.pieces.set(stroke.id, pieces);
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
    if (!cursor) return;
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
    if (eraserCursorRef.current) eraserCursorRef.current.hidden = true;
  }, [tool, eraserMode]);

  /** Discards the line being drawn or the erase in progress */
  const cancelGesture = () => {
    gestureRef.current = null;
    updateEraserCursor(null);
    requestRender();
  };

  /**
   * Tracks fingers (touch). With two fingers, zooms and pans by pinching and returns true (not used for drawing).
   * If a second finger touches while drawing with one, the stroke in progress is canceled and it becomes a pinch
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
        if (pinch.count < 2) return false;
        if (gestureRef.current?.pointerType === 'touch') cancelGesture();
        return true;
      case 'pointermove': {
        const moved = pinch.move(event.pointerId, screen);
        if (!moved) return false;
        const next = pinchView(viewRef.current, moved.before, moved.after);
        viewRef.current = clampView(next, pageRef.current);
        requestRender();
        return true;
      }
      default:
        if (pinch.up(event.pointerId)) commitView(viewRef.current);
        return false;
    }
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (event.pointerType === 'pen') onPenDetected();
    // Fingers do not draw when used for a pinch or when finger drawing is off
    if (event.pointerType === 'touch' && (handleTouch(event) || !fingerDrawing))
      return;
    // Another pointer (a palm, etc.) touching while drawing does not start a new stroke
    if (gestureRef.current || !renderer) return;

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
    if (event.pointerType === 'touch' && handleTouch(event)) return;
    const gesture = gestureRef.current;
    // Ignore other pointers moving while drawing
    if (gesture && gesture.pointerId !== event.pointerId) return;

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
    if (event.pointerType === 'touch') handleTouch(event);
    const gesture = gestureRef.current;
    if (gesture?.pointerId !== event.pointerId) return;

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
    if (event.pointerType === 'touch') handleTouch(event);
    if (gestureRef.current?.pointerId !== event.pointerId) return;
    cancelGesture();
  };

  // Hide the eraser circle when the pointer leaves the canvas (e.g. into the toolbar)
  const onPointerLeave = () => updateEraserCursor(null);

  /** Applies what arrived from the PC */
  const applyRemote: (action: RemoteAction) => void = dispatch;

  const history: History = {
    canUndo: state.undoStack.length > 0,
    canRedo: state.redoStack.length > 0,
    undo: () => dispatch({ type: 'undo' }),
    redo: () => dispatch({ type: 'redo' }),
  };

  /** Area of the canvas currently visible on this page (reported to the PC) */
  const viewport = useMemo(
    () => visibleRange(view, page.width, page.height),
    [view, page],
  );

  return {
    error,
    history,
    /** The committed view (excluding movement during a pinch) */
    view,
    resetView: () => commitView(INITIAL_VIEW),
    /** Whether there is at least one stroke (whether it can be cleared) */
    canClear: state.strokes.length > 0,
    /** Returns to a blank initial state: removes every stroke (also sent to the PC), empties the history, and resets the view to 100% */
    clear: () => {
      dispatch({ type: 'clear' });
      commitView(INITIAL_VIEW);
    },
    viewport,
    page,
    /** Changes this canvas page made in the last action (sent to the PC) */
    outgoing: state.outgoing,
    applyRemote,
    eraserCursorRef,
    pointerHandlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel,
      onPointerLeave,
    },
  };
}
