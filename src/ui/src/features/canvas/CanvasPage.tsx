import { useEffect, useMemo, useRef, useState } from 'react';
import { Separator } from '@/components/ui/separator';
import type { ConnectionInfo } from '../sync/connectionInfo';
import { useBridgeSync } from '../sync/useBridgeSync';
import { CameraDialog } from './CameraDialog';
import { useCameraAvailable } from './camera';
import { ConnectionStatusBadge } from './ConnectionStatusBadge';
import { SendToAiButton } from './SendToAiButton';
import { SupersededDialog } from './SupersededDialog';
import { BLACK } from './color';
import { useCustomColor } from './customColor';
import { useFingerDrawing } from './fingerDrawing';
import { ERASER_SIZE } from './erase';
import { encodeImage, initialPlacement, resizePlacement } from './imageImport';
import { ImageSettingsPopover } from './ImageSettingsPopover';
import { ImageStore } from './imageStore';
import { newStrokeId } from './strokeId';
import type { DrawingTool, EraserMode, Tool } from './types';
import { Toolbar } from './Toolbar';
import { useCanvasDrawing } from './useCanvasDrawing';
import { isInitialView } from './view';

/** Anchor of the image size and delete popover */
type ImageSettingsAnchor =
  { kind: 'button' } | { kind: 'point'; x: number; y: number };

interface CanvasPageProps {
  /** Connection info for the PC (bridge). Without it, the page only draws and does not connect */
  connectionInfo?: ConnectionInfo | null;
}

export function CanvasPage({ connectionInfo = null }: CanvasPageProps) {
  const [tool, setTool] = useState<Tool>('pen');
  const [penColor, setPenColor] = useState(BLACK);
  const [customColor, setCustomColor] = useCustomColor();
  const fingerDrawing = useFingerDrawing();
  const [eraserMode, setEraserMode] = useState<EraserMode>('stroke');
  const [eraserSize, setEraserSize] = useState<number>(ERASER_SIZE.initial);
  /** The tool used before Move Image was selected. Restored when the image is gone */
  const [lastDrawingTool, setLastDrawingTool] = useState<DrawingTool>('pen');
  /**
   * Where the image size and delete popover is anchored (null when closed).
   * The button when Move Image is selected, or the clicked point when the image is clicked
   */
  const [imageSettingsAt, setImageSettingsAt] =
    useState<ImageSettingsAnchor | null>(null);
  const moveImageButtonRef = useRef<HTMLButtonElement>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const cameraAvailable = useCameraAvailable();
  const [cameraOpen, setCameraOpen] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  // Image bytes and decoded bitmaps (only this page has them; like strokes, they are gone when the tab is closed)
  // Created only once in the useState initializer so it is not recreated (it is passed to useBridgeSync)
  const [images] = useState(() => new ImageStore());
  const {
    error,
    history,
    pointerHandlers,
    outgoing,
    applyRemote,
    eraserCursorRef,
    imageFrameRef,
    view,
    resetView,
    viewport,
    page,
    canClear,
    clear,
    image,
    setImage,
    previewImage,
  } = useCanvasDrawing(
    canvasRef,
    {
      tool,
      penColor,
      eraserMode,
      eraserSize,
      fingerDrawing: fingerDrawing.enabled,
    },
    images,
    fingerDrawing.onPenDetected,
    {
      onClick: (at) => setImageSettingsAt({ kind: 'point', ...at }),
      onClickOutside: () => setImageSettingsAt(null),
      // Close once it starts moving (not shown after a drag either; it is shown only on a click)
      onGestureStart: () => setImageSettingsAt(null),
    },
  );
  const {
    status,
    message,
    reconnect,
    superseded,
    dismissSuperseded,
    aiWaiting,
    sendQueued,
    sendToAi,
  } = useBridgeSync({
    info: connectionInfo,
    viewport,
    page,
    outgoing,
    applyRemote,
    images,
  });
  const notice = error ?? importError ?? message;

  /** Changes the tool (Toolbar's onToolChange) */
  const changeTool = (next: Tool) => {
    setTool(next);
    if (next !== 'move') {
      // Remember the drawing tool (to return to when the image is gone)
      setLastDrawingTool(next);
      // Left the move tool, so close the image popover
      setImageSettingsAt(null);
    }
  };

  /**
   * When Move Image is pressed. If it isn't selected, opens the popover at the button;
   * if it is, toggles the popover (closes it when open, opens it when closed).
   * Pressing Move Image doesn't close the popover as an outside press (ignoreOutsidePressOn),
   * so the current open state can be used here.
   * Why `tool` of this render is used: the order of the ToggleGroup value change (onToolChange)
   * and this onClick isn't guaranteed, so whether the move tool was selected is read from
   * `tool` at the time of the press (this render)
   */
  const onMoveImagePress = () => {
    setImageSettingsAt((at) =>
      tool === 'move' && at !== null ? null : { kind: 'button' },
    );
  };

  /** Import: re-encode as PNG, store it, place it at the center of the paper and send it to the PC */
  const importImage = async (source: Blob | HTMLVideoElement) => {
    try {
      const { png, natural } = await encodeImage(source);
      const id = newStrokeId();
      images.put(id, png);
      setImage(initialPlacement(natural, page, id));
      setImportError(null);
    } catch {
      setImportError('Could not load that image.');
    }
  };
  // Paste (PC browsers). importImage is recreated every time, so the paste side calls it through a ref
  const importRef = useRef(importImage);
  useEffect(() => {
    importRef.current = importImage;
  });
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      const file = Array.from(event.clipboardData?.files ?? []).find((file) =>
        file.type.startsWith('image/'),
      );
      if (file) {
        void importRef.current(file);
      }
    };
    window.addEventListener('paste', onPaste);
    return () => window.removeEventListener('paste', onPaste);
  }, []);

  // When the image is gone (deleted, cleared, undone, including from the PC), return from the move tool
  // to the previous tool. Compare with the previous value during render rather than watching it in an effect (the form React recommends)
  const [prevImage, setPrevImage] = useState(image);
  if (prevImage !== image) {
    setPrevImage(image);
    if (!image && tool === 'move') {
      setTool(lastDrawingTool);
    }
  }

  /** Ratio (%) to the original size. null before decoding or when the size is unknown */
  const natural = image ? images.natural(image.id) : undefined;
  const imageScale =
    image && natural && natural.width > 0
      ? Math.round((image.width / natural.width) * 100)
      : null;
  const applyScale = (percent: number, commit: boolean) => {
    if (!image || !natural) {
      return;
    }
    const placement = resizePlacement(image, natural, percent, page);
    if (commit) {
      setImage(placement);
    } else {
      previewImage(placement);
    }
  };
  // Anchor of the popover. The clicked point is passed as a zero-size rectangle (VirtualElement)
  const imageSettingsAnchor = useMemo(() => {
    if (imageSettingsAt?.kind !== 'point') {
      return moveImageButtonRef;
    }
    const { x, y } = imageSettingsAt;
    return {
      getBoundingClientRect: () =>
        DOMRect.fromRect({ x, y, width: 0, height: 0 }),
    };
  }, [imageSettingsAt]);
  const importImageFile = (files: Iterable<File>) => {
    const file = Array.from(files).find((file) =>
      file.type.startsWith('image/'),
    );
    if (file) {
      void importImage(file);
    }
  };

  return (
    // The tool sidebar sits on the right and the rest is the canvas.
    // The paper size (page) is the displayed size of the canvas, so it is narrower by the sidebar's width
    <main className="flex h-svh w-full overflow-hidden overscroll-none bg-muted select-none">
      <div
        className="relative min-w-0 flex-1"
        // PC browsers: dragging and dropping a file also imports it
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => {
          event.preventDefault();
          importImageFile(event.dataTransfer.files);
        }}
      >
        {/* touch-none: keep pen and touch drawing from scrolling or zooming the page (zooming is done by our own code) */}
        <canvas
          ref={canvasRef}
          aria-label="Canvas"
          data-tool={tool}
          // Mouse on a PC: a crosshair for the pen; for the eraser, hide the OS pointer and show the eraser circle (the div below); a move cursor for the move tool
          className="absolute inset-0 size-full touch-none bg-white data-[tool=eraser]:cursor-none data-[tool=move]:cursor-move data-[tool=pen]:cursor-crosshair"
          {...pointerHandlers}
        />
        {/* Eraser circle (with a mouse while the eraser is selected, with a pen while tracing to erase). useCanvasDrawing writes its position and size directly */}
        <div
          ref={eraserCursorRef}
          data-testid="eraser-cursor"
          hidden
          aria-hidden
          className="pointer-events-none absolute top-0 left-0 rounded-full border border-neutral-500 bg-white/40"
        />
        {/* While the move tool (Move Image) is selected, a dashed frame shows the image position and size. useCanvasDrawing writes the position and size directly */}
        <div
          ref={imageFrameRef}
          data-testid="image-frame"
          hidden
          aria-hidden
          className="pointer-events-none absolute top-0 left-0 border border-dashed border-sky-500"
        />
        {/* File picker opened by the import button (album, files, etc.) */}
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          className="sr-only"
          onChange={(event) => {
            importImageFile(event.target.files ?? []);
            // Allow importing again even if the same file is chosen twice
            event.target.value = '';
          }}
        />

        {/* Closed unless the move tool is selected (including when the image is deleted) */}
        <ImageSettingsPopover
          open={imageSettingsAt !== null && tool === 'move'}
          onClose={() => setImageSettingsAt(null)}
          anchor={imageSettingsAnchor}
          side={imageSettingsAt?.kind === 'point' ? 'bottom' : 'left'}
          ignoreOutsidePressOn={[canvasRef, moveImageButtonRef]}
          imageScale={imageScale}
          onImageScaleChange={(percent) => applyScale(percent, false)}
          onImageScaleCommit={(percent) => applyScale(percent, true)}
          onImageDelete={() => setImage(null)}
        />

        {notice && (
          <p
            role="alert"
            className="absolute inset-x-4 bottom-4 rounded-lg border bg-background p-3 text-center text-sm text-destructive"
          >
            {/* Center the block of text and left-align its lines (so the list of supported browsers lines up).
                whitespace-pre-line: show multi-line messages with their line breaks */}
            <span className="inline-block text-left whitespace-pre-line">
              {notice}
            </span>
          </p>
        )}
      </div>

      {/* Connection status at the top, Send to AI below it, then the tools. When the screen is short, only the sidebar scrolls */}
      <aside
        aria-label="Sidebar"
        className="flex w-18 shrink-0 flex-col items-center gap-2 overflow-y-auto overscroll-contain border-l bg-background p-2"
      >
        <ConnectionStatusBadge
          status={status}
          onReconnect={reconnect}
          aiWaiting={aiWaiting}
        />
        <SendToAiButton onSend={sendToAi} queued={sendQueued} />
        {/* Separates the status icon from the tool buttons so they can be told apart */}
        <Separator />
        <Toolbar
          tool={tool}
          onToolChange={changeTool}
          fingerDrawing={fingerDrawing.enabled}
          onFingerDrawingChange={fingerDrawing.setEnabled}
          penColor={penColor}
          onPenColorChange={setPenColor}
          customColor={customColor}
          onCustomColorChange={setCustomColor}
          eraserMode={eraserMode}
          onEraserModeChange={setEraserMode}
          eraserSize={eraserSize}
          onEraserSizeChange={setEraserSize}
          history={history}
          viewScale={isInitialView(view) ? null : view.scale}
          onResetView={resetView}
          canClear={canClear}
          onClear={clear}
          onImport={() => fileInputRef.current?.click()}
          onTakePhoto={cameraAvailable ? () => setCameraOpen(true) : null}
          onMoveImagePress={onMoveImagePress}
          moveImageButtonRef={moveImageButtonRef}
          canMoveImage={image !== null}
        />
      </aside>

      <SupersededDialog open={superseded} onClose={dismissSuperseded} />
      {/* Close after the import finishes (closing stops the camera, so don't stop it before the frame is read) */}
      <CameraDialog
        open={cameraOpen}
        onClose={() => setCameraOpen(false)}
        onCapture={async (video) => {
          await importImage(video);
          setCameraOpen(false);
        }}
      />
    </main>
  );
}
