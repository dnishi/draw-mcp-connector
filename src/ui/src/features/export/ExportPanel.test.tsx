import { useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { StrokeRenderer } from '../canvas/StrokeRenderer';
import {
  download,
  exportPdf,
  exportPng,
  type ExportFormat,
  type ExportTarget,
  type SheetSource,
} from './exportFile';
import { ExportPanel } from './ExportPanel';

// Only mock the creation and download (exportFile.ts). The default name and the way the extension is added use the real thing
vi.mock('./exportFile', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./exportFile')>();
  return {
    ...actual,
    exportPng: vi.fn(),
    exportPdf: vi.fn(),
    download: vi.fn(),
  };
});

// The renderer does not build the export content (exportPng / exportPdf are mocks), so an empty one is enough
const renderer = {} as unknown as StrokeRenderer;
const page = { width: 400, height: 300 };
const sources: readonly SheetSource[] = [
  { strokes: [], image: undefined },
  { strokes: [], image: undefined },
];

interface PanelProps {
  area?: { x: number; y: number; width: number; height: number };
  onAreaChange?: (area: {
    x: number;
    y: number;
    width: number;
    height: number;
  }) => void;
  currentSheetIndex?: number;
  format?: ExportFormat;
  onFormatChange?: (format: ExportFormat) => void;
  target?: ExportTarget;
  onTargetChange?: (target: ExportTarget) => void;
  onClose?: () => void;
}

/** Holds Format and Pages by itself (so the tests can see the panel's content change with switching) */
function Panel(props: PanelProps) {
  const [format, setFormat] = useState(props.format ?? 'png');
  const [target, setTarget] = useState(props.target ?? 'current');
  return (
    <ExportPanel
      page={page}
      area={props.area ?? { x: 10, y: 20, width: 30, height: 40 }}
      onAreaChange={props.onAreaChange ?? (() => {})}
      currentSheetIndex={props.currentSheetIndex ?? 0}
      format={format}
      onFormatChange={(next) => {
        setFormat(next);
        props.onFormatChange?.(next);
      }}
      target={target}
      onTargetChange={(next) => {
        setTarget(next);
        props.onTargetChange?.(next);
      }}
      renderer={renderer}
      sources={sources}
      onClose={props.onClose ?? (() => {})}
    />
  );
}

describe('ExportPanel', () => {
  // Sets the time used for the default file name
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 19, 15, 30));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('既定の名前と拡張子を出す', () => {
    render(<Panel currentSheetIndex={1} />);

    expect(screen.getByRole('textbox', { name: 'File name' })).toHaveValue(
      'draw-mcp-2026-11-19-1530-sheet-2',
    );
    expect(screen.getByText('.png')).toBeInTheDocument();
  });

  it('Format と Pages で出る欄が変わる', () => {
    render(<Panel />);
    const areaText = 'Drag on the canvas.';

    // PNG: the Pages row stays but can't be pressed (only the current sheet), the area is shown
    expect(screen.getByRole('button', { name: 'All Sheets' })).toBeDisabled();
    expect(screen.getByText(areaText)).toBeInTheDocument();
    // PDF: Pages can be pressed. While on Current Sheet, the area is shown too
    fireEvent.click(screen.getByRole('button', { name: 'PDF' }));
    expect(screen.getByRole('button', { name: 'All Sheets' })).toBeEnabled();
    expect(screen.getByText(areaText)).toBeInTheDocument();
    // All Sheets: no area can be chosen, so the area row stays but can't be pressed (it shows the whole sheet size)
    fireEvent.click(screen.getByRole('button', { name: 'All Sheets' }));
    expect(screen.getByRole('button', { name: 'Select All' })).toBeDisabled();
    expect(screen.getByText('400 × 300 px')).toBeInTheDocument();
    // Don't make it look like the area can be chosen by dragging
    expect(screen.queryByText(areaText)).toBeNull();
    expect(screen.getByText('Whole sheets.')).toBeInTheDocument();
  });

  it('名前は欄を変えていない間だけ、Pages に合わせて既定の名前が変わる', () => {
    render(<Panel currentSheetIndex={1} />);
    const input = screen.getByRole('textbox', { name: 'File name' });
    expect(input).toHaveValue('draw-mcp-2026-11-19-1530-sheet-2');

    // Switching to All Sheets drops the sheet number
    fireEvent.click(screen.getByRole('button', { name: 'PDF' }));
    fireEvent.click(screen.getByRole('button', { name: 'All Sheets' }));
    expect(input).toHaveValue('draw-mcp-2026-11-19-1530');

    // After changing the name, it does not change when Pages is switched
    fireEvent.change(input, { target: { value: 'memo' } });
    fireEvent.click(screen.getByRole('button', { name: 'Current Sheet' }));
    expect(input).toHaveValue('memo');
  });

  it('Select All で範囲をシート全体にする', () => {
    const onAreaChange = vi.fn();
    render(<Panel onAreaChange={onAreaChange} />);

    fireEvent.click(screen.getByRole('button', { name: 'Select All' }));

    expect(onAreaChange).toHaveBeenCalledWith({
      x: 0,
      y: 0,
      width: 400,
      height: 300,
    });
  });

  it('PNG を Export すると、選んだ範囲で exportPng が呼ばれ、ダウンロードされる', async () => {
    vi.mocked(exportPng).mockResolvedValue(new Blob(['png']));
    const onClose = vi.fn();
    render(<Panel onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));
    // run() proceeds after the mock resolves (flush the Promises)
    await act(async () => {});

    expect(exportPng).toHaveBeenCalledWith(renderer, sources[0], {
      x: 10,
      y: 20,
      width: 30,
      height: 40,
    });
    expect(download).toHaveBeenCalledWith(
      new Blob(['png']),
      'draw-mcp-2026-11-19-1530-sheet-1.png',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('今のシートの PDF を Export すると、そのシートだけのページで exportPdf が呼ばれる', async () => {
    vi.mocked(exportPdf).mockResolvedValue(new Blob(['pdf']));
    render(<Panel format="pdf" target="current" />);

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));
    await act(async () => {});

    expect(exportPdf).toHaveBeenCalledWith([
      { sheet: sources[0], area: { x: 10, y: 20, width: 30, height: 40 } },
    ]);
  });

  it('全部のシートの PDF を Export すると、シートごとにシート全体のページで exportPdf が呼ばれる', async () => {
    vi.mocked(exportPdf).mockResolvedValue(new Blob(['pdf']));
    render(<Panel format="pdf" target="all" currentSheetIndex={1} />);

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));
    await act(async () => {});

    expect(exportPdf).toHaveBeenCalledWith([
      { sheet: sources[0], area: { x: 0, y: 0, width: 400, height: 300 } },
      { sheet: sources[1], area: { x: 0, y: 0, width: 400, height: 300 } },
    ]);
    expect(download).toHaveBeenCalledWith(
      new Blob(['pdf']),
      'draw-mcp-2026-11-19-1530.pdf',
    );
  });

  it('作っている間、Cancel と Export を押せない。終わったら閉じる', async () => {
    let resolveExport: (blob: Blob) => void = () => {};
    vi.mocked(exportPng).mockImplementation(
      () =>
        new Promise<Blob>((resolve) => {
          resolveExport = resolve;
        }),
    );
    const onClose = vi.fn();
    render(<Panel onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));
    await act(async () => {});
    expect(exportPng).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Export' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();

    resolveExport(new Blob(['png']));
    await act(async () => {});
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('失敗したら閉じず、パネルの中にエラーの文を出す', async () => {
    vi.mocked(exportPng).mockRejectedValue(new Error('boom'));
    const onClose = vi.fn();
    render(<Panel onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: 'Export' }));
    await act(async () => {});

    expect(screen.getByText("Couldn't export: boom")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Export' })).toBeEnabled();
  });

  it('Cancel を押すと閉じる', () => {
    const onClose = vi.fn();
    render(<Panel onClose={onClose} />);

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Esc で Cancel、Enter(名前の欄の中でも)で Export', async () => {
    vi.mocked(exportPng).mockResolvedValue(new Blob(['png']));
    const onClose = vi.fn();
    render(<Panel onClose={onClose} />);
    const dialog = screen.getByRole('dialog', { name: 'Export' });

    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    onClose.mockClear();

    // Press Enter while the name field has focus (fires on the field and reaches the panel)
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'File name' }), {
      key: 'Enter',
    });
    await act(async () => {});
    expect(exportPng).toHaveBeenCalledTimes(1);
  });

  it('Cancel のボタンの上で Enter を押しても Export しない', () => {
    render(<Panel />);

    fireEvent.keyDown(screen.getByRole('button', { name: 'Cancel' }), {
      key: 'Enter',
    });

    expect(exportPng).not.toHaveBeenCalled();
  });

  it('PDF の All Sheets を選んでから PNG に戻すと、今のシートの名前と範囲に戻る', () => {
    render(<Panel format="pdf" target="all" currentSheetIndex={1} />);
    expect(screen.getByRole('textbox', { name: 'File name' })).toHaveValue(
      'draw-mcp-2026-11-19-1530',
    );

    fireEvent.click(screen.getByRole('button', { name: 'PNG' }));

    expect(screen.getByRole('textbox', { name: 'File name' })).toHaveValue(
      'draw-mcp-2026-11-19-1530-sheet-2',
    );
    expect(screen.getByRole('button', { name: 'Select All' })).toBeEnabled();
  });
});
