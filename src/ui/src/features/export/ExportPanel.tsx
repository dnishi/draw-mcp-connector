import { useState, type KeyboardEvent } from 'react';
import { Button } from '@/components/ui/button';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import type { StrokeRenderer } from '../canvas/StrokeRenderer';
import type { Page } from './areaSelection';
import {
  clampArea,
  defaultBaseName,
  download,
  exportPdf,
  exportPng,
  fileName,
  type Area,
  type ExportFormat,
  type ExportTarget,
  type SheetSource,
} from './exportFile';

interface ExportPanelProps {
  /** The sheet size */
  page: Page;
  /** The chosen area (used for PNG and the PDF of the current sheet) */
  area: Area;
  onAreaChange: (area: Area) => void;
  /** The index of the current sheet (0-based) */
  currentSheetIndex: number;
  format: ExportFormat;
  onFormatChange: (format: ExportFormat) => void;
  target: ExportTarget;
  onTargetChange: (target: ExportTarget) => void;
  renderer: StrokeRenderer | null;
  /** The material to export (the sheets in order) */
  sources: readonly SheetSource[];
  onClose: () => void;
}

/** The look of the row labels */
const LABEL_CLASS = 'w-16 shrink-0 text-sm';

/**
 * The export panel. Overlaid at the bottom center of the canvas area (not a modal).
 * Enter is Export even inside the name field, Esc is Cancel
 */
export function ExportPanel({
  page,
  area,
  onAreaChange,
  currentSheetIndex,
  format,
  onFormatChange,
  target,
  onTargetChange,
  renderer,
  sources,
  onClose,
}: ExportPanelProps) {
  /** Until the name field is touched (null), the default name changes according to Pages */
  const [nameInput, setNameInput] = useState<string | null>(null);
  /** While the export is being made, Cancel and Export cannot be pressed */
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only remembers the time it was opened (the default file name is decided from this time)
  const [openedAt] = useState(() => new Date());

  // All sheets are exported only for PDF All Sheets (PNG is the current sheet even if the Pages choice remains).
  // All sheets cannot choose an area (always the whole sheet)
  const allSheets = format === 'pdf' && target === 'all';
  const showArea = !allSheets;
  const defaultBase = defaultBaseName(
    openedAt,
    allSheets ? null : currentSheetIndex + 1,
  );

  const run = async () => {
    if (busy || renderer === null) {
      return;
    }
    const chosen = clampArea(area, page);
    // For all sheets, the whole sheet for each sheet
    const whole = clampArea(
      { x: 0, y: 0, width: page.width, height: page.height },
      page,
    );
    const current = sources[currentSheetIndex];
    if (current === undefined || chosen === null || whole === null) {
      // A broken state such as the sheet size not being known yet (0×0)
      setError("Couldn't choose the area to export.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const blob =
        format === 'png'
          ? await exportPng(renderer, current, chosen)
          : allSheets
            ? await exportPdf(sources.map((sheet) => ({ sheet, area: whole })))
            : await exportPdf([{ sheet: current, area: chosen }]);
      download(blob, fileName(nameInput ?? '', defaultBase, format));
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
    } else if (
      event.key === 'Enter' &&
      !(event.target instanceof HTMLElement && event.target.closest('button'))
    ) {
      // Pressing Enter inside the name field or elsewhere is Export. Enter on a button
      // (such as Cancel) keeps its usual behavior of pressing that button
      event.preventDefault();
      void run();
    }
  };

  return (
    <div
      role="dialog"
      aria-label="Export"
      onKeyDown={onKeyDown}
      className="absolute inset-x-4 bottom-4 mx-auto max-w-md rounded-xl border bg-background p-4 shadow-md"
    >
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-3">
          <span className={LABEL_CLASS}>Format</span>
          <ToggleGroup
            aria-label="Format"
            size="sm"
            value={[format]}
            onValueChange={(values) => {
              const next = values[0];
              if (next === 'png' || next === 'pdf') {
                onFormatChange(next);
              }
            }}
          >
            <ToggleGroupItem value="png">PNG</ToggleGroupItem>
            <ToggleGroupItem value="pdf">PDF</ToggleGroupItem>
          </ToggleGroup>
        </div>
        {format === 'pdf' && (
          <div className="flex items-center gap-3">
            <span className={LABEL_CLASS}>Pages</span>
            <ToggleGroup
              aria-label="Pages"
              size="sm"
              value={[target]}
              onValueChange={(values) => {
                const next = values[0];
                if (next === 'current' || next === 'all') {
                  onTargetChange(next);
                }
              }}
            >
              <ToggleGroupItem value="current">Current Sheet</ToggleGroupItem>
              <ToggleGroupItem value="all">All Sheets</ToggleGroupItem>
            </ToggleGroup>
          </div>
        )}
        {showArea && (
          <div className="flex items-center gap-3">
            <span className={LABEL_CLASS}>Area</span>
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <p className="min-w-0 flex-1 truncate text-sm text-muted-foreground">
                Drag on the canvas to choose the area.
              </p>
              <span className="shrink-0 text-sm">
                {Math.round(area.width)} × {Math.round(area.height)} px
              </span>
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  onAreaChange({
                    x: 0,
                    y: 0,
                    width: page.width,
                    height: page.height,
                  })
                }
              >
                Select All
              </Button>
            </div>
          </div>
        )}
        <div className="flex items-center gap-3">
          <span className={LABEL_CLASS}>File name</span>
          <input
            aria-label="File name"
            type="text"
            value={nameInput ?? defaultBase}
            onChange={(event) => setNameInput(event.target.value)}
            className="h-8 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-[--color-ring]"
          />
          <span className="shrink-0 text-sm text-muted-foreground">
            .{format}
          </span>
        </div>
        {error !== null && (
          <p className="text-sm text-destructive">Couldn't export: {error}</p>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={busy || renderer === null}
            onClick={() => void run()}
          >
            Export
          </Button>
        </div>
      </div>
    </div>
  );
}
