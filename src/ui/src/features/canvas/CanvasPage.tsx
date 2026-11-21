import { useEffect, useMemo, useRef, useState } from 'react';
import type { ConnectionInfo } from '../sync/connectionInfo';
import { useBridgeSync } from '../sync/useBridgeSync';
import { MenuBar } from '../menu/MenuBar';
import { AreaSelectionOverlay } from '../export/AreaSelectionOverlay';
import { ExportPanel } from '../export/ExportPanel';
import type { Area, ExportFormat, ExportTarget } from '../export/exportFile';
import { CameraDialog } from './CameraDialog';
import { useCameraAvailable } from './camera';
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
  /** The tool used before Move Image was selected. Restored when the image is gone, or when clicking outside the image with the move tool still selected */
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
  /**
   * The export panel. When opening, the view is reset to 1:1 and the area to the whole sheet.
   * The format and target are kept here too: this page decides whether to show the area
   * overlay (not shown for All Sheets)
   */
  const [exportOpen, setExportOpen] = useState(false);
  const [exportArea, setExportArea] = useState<Area>({
    x: 0,
    y: 0,
    width: 0,
    height: 0,
  });
  const [exportFormat, setExportFormat] = useState<ExportFormat>('png');
  const [exportTarget, setExportTarget] = useState<ExportTarget>('current');
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  // Image bytes and decoded bitmaps (only this page has them; like strokes, they are gone when the tab is closed)
  // Created only once in the useState initializer so it is not recreated (it is passed to useBridgeSync)
  const [images] = useState(() => new ImageStore());
  const {
    error,
    renderer,
    exportSources,
    currentSheetIndex,
    history,
    pointerHandlers,
    pending,
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
    sheets,
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
      // Clicking outside the image returns to the tool used before Move Image was selected (only called with the move tool).
      // changeTool closes the popover
      onClickOutside: () => changeTool(lastDrawingTool),
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
    sendToAi,
  } = useBridgeSync({
    info: connectionInfo,
    viewport,
    page,
    pending,
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

  /** Opens the export panel: resets the view to 1:1 and the area to the whole sheet. Also closes the image popover */
  const openExport = () => {
    resetView();
    setExportArea({ x: 0, y: 0, width: page.width, height: page.height });
    setImageSettingsAt(null);
    setExportOpen(true);
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

  /** Import: re-encode as PNG, store it, place it at the center of the sheet and send it to the PC */
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

  // When the sheet switches (including a switch from another device), reset the selected area to the whole sheet
  const [prevSheetNumber, setPrevSheetNumber] = useState(sheets.number);
  if (prevSheetNumber !== sheets.number) {
    setPrevSheetNumber(sheets.number);
    if (exportOpen) {
      setExportArea({ x: 0, y: 0, width: page.width, height: page.height });
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
    // The menu bar sits on top, the tool sidebar on the right, and the rest is the canvas.
    // The sheet size (page) is the displayed size of the canvas, so it is narrower by the menu bar's and sidebar's space
    <div className="flex h-svh w-full flex-col overflow-hidden overscroll-none bg-muted select-none">
      <MenuBar
        menu={{
          // Not pressable when there is no renderer (WebGPU unavailable)
          onExport: renderer ? openExport : null,
        }}
        sheets={sheets}
        status={status}
        onReconnect={reconnect}
        onSendToAi={sendToAi}
      />

      <main className="flex min-h-0 flex-1">
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
            // Mouse on a PC: a crosshair for the pen; for the eraser, hide the OS pointer and show the eraser circle (the div below)
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

          {/* Closed unless the move tool is selected (including when the image is deleted). Also closed while the export panel is open */}
          <ImageSettingsPopover
            open={imageSettingsAt !== null && tool === 'move' && !exportOpen}
            onClose={() => setImageSettingsAt(null)}
            anchor={imageSettingsAnchor}
            side={imageSettingsAt?.kind === 'point' ? 'bottom' : 'left'}
            ignoreOutsidePressOn={[canvasRef, moveImageButtonRef]}
            imageScale={imageScale}
            onImageScaleChange={(percent) => applyScale(percent, false)}
            onImageScaleCommit={(percent) => applyScale(percent, true)}
            onImageDelete={() => setImage(null)}
          />

          {/* While the export panel is open, stop input from reaching the canvas.
            For PNG and the current sheet, an area overlay (which receives the input itself);
            for all sheets, only a transparent div that shows no area */}
          {exportOpen &&
            (exportFormat === 'png' || exportTarget === 'current' ? (
              <AreaSelectionOverlay
                page={page}
                area={exportArea}
                onAreaChange={setExportArea}
              />
            ) : (
              <div className="absolute inset-0 touch-none" aria-hidden />
            ))}
          {exportOpen && (
            <ExportPanel
              page={page}
              area={exportArea}
              onAreaChange={setExportArea}
              currentSheetIndex={currentSheetIndex}
              format={exportFormat}
              target={exportTarget}
              onFormatChange={setExportFormat}
              onTargetChange={setExportTarget}
              renderer={renderer}
              sources={exportSources()}
              onClose={() => setExportOpen(false)}
            />
          )}
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

        {/* Tools are laid out in a column. When the screen is short, only the sidebar scrolls */}
        <aside
          aria-label="Sidebar"
          className="flex w-18 shrink-0 flex-col items-center gap-2 overflow-y-auto overscroll-contain border-l bg-background p-2"
        >
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
      </main>

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
    </div>
  );
}
