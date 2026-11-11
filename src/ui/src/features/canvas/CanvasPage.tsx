import { useRef, useState } from 'react';
import { Separator } from '@/components/ui/separator';
import type { ConnectionInfo } from '../sync/connectionInfo';
import { useBridgeSync } from '../sync/useBridgeSync';
import { ConnectionStatusBadge } from './ConnectionStatusBadge';
import { SupersededDialog } from './SupersededDialog';
import { BLACK } from './color';
import { useCustomColor } from './customColor';
import { useFingerDrawing } from './fingerDrawing';
import { ERASER_SIZE } from './erase';
import type { EraserMode, Tool } from './types';
import { Toolbar } from './Toolbar';
import { useCanvasDrawing } from './useCanvasDrawing';
import { isInitialView } from './view';

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
  const [showGrid, setShowGrid] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const {
    error,
    history,
    pointerHandlers,
    outgoing,
    applyRemote,
    eraserCursorRef,
    view,
    resetView,
    viewport,
    page,
    canClear,
    clear,
  } = useCanvasDrawing(
    canvasRef,
    {
      tool,
      penColor,
      eraserMode,
      eraserSize,
      fingerDrawing: fingerDrawing.enabled,
      showGrid,
    },
    fingerDrawing.onPenDetected,
  );
  const { status, message, reconnect, superseded, dismissSuperseded } =
    useBridgeSync({
      info: connectionInfo,
      viewport,
      page,
      outgoing,
      applyRemote,
    });
  const notice = error ?? message;

  return (
    // The tool sidebar sits on the right and the rest is the canvas.
    // The paper size (page) is the displayed size of the canvas, so it is narrower by the sidebar's width
    <main className="flex h-svh w-full overflow-hidden overscroll-none bg-muted select-none">
      <div className="relative min-w-0 flex-1">
        {/* touch-none: keep pen and touch drawing from scrolling or zooming the page (zoom is handled by the page itself) */}
        <canvas
          ref={canvasRef}
          aria-label="Canvas"
          data-tool={tool}
          // Mouse on a PC: crosshair for the pen; for the eraser, hide the OS pointer and show the eraser circle (the div below)
          className="absolute inset-0 size-full touch-none bg-white data-[tool=eraser]:cursor-none data-[tool=pen]:cursor-crosshair"
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

        {notice && (
          <p
            role="alert"
            className="absolute inset-x-4 bottom-4 rounded-lg border bg-background p-3 text-center text-sm text-destructive"
          >
            {notice}
          </p>
        )}
      </div>

      {/* Connection status at the top, tools below it. When the screen is short, only the sidebar scrolls */}
      <aside
        aria-label="Sidebar"
        className="flex w-18 shrink-0 flex-col items-center gap-2 overflow-y-auto overscroll-contain border-l bg-background p-2"
      >
        <ConnectionStatusBadge status={status} onReconnect={reconnect} />
        {/* Separates the status icon from the tool buttons so they can be told apart */}
        <Separator />
        <Toolbar
          tool={tool}
          onToolChange={setTool}
          fingerDrawing={fingerDrawing.enabled}
          onFingerDrawingChange={fingerDrawing.setEnabled}
          showGrid={showGrid}
          onShowGridChange={setShowGrid}
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
        />
      </aside>

      <SupersededDialog open={superseded} onClose={dismissSuperseded} />
    </main>
  );
}
