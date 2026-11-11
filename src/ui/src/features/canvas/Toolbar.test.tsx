import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState, type ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Toolbar } from './Toolbar';
import type { History } from './types';

function createHistory(overrides: Partial<History> = {}): History {
  return {
    canUndo: true,
    canRedo: true,
    undo: vi.fn(),
    redo: vi.fn(),
    ...overrides,
  };
}

function renderToolbar(props: Partial<ComponentProps<typeof Toolbar>> = {}) {
  const all: ComponentProps<typeof Toolbar> = {
    tool: 'pen',
    onToolChange: vi.fn(),
    fingerDrawing: true,
    onFingerDrawingChange: vi.fn(),
    showGrid: false,
    onShowGridChange: vi.fn(),
    penColor: '#000000',
    onPenColorChange: vi.fn(),
    customColor: null,
    onCustomColorChange: vi.fn(),
    eraserMode: 'stroke',
    onEraserModeChange: vi.fn(),
    eraserSize: 24,
    onEraserSizeChange: vi.fn(),
    history: createHistory(),
    viewScale: null,
    onResetView: vi.fn(),
    canClear: true,
    onClear: vi.fn(),
    ...props,
  };
  render(<Toolbar {...all} />);
  return all;
}

/** Remembers the chosen tool (like CanvasPage, the tool is held outside Toolbar) */
function StatefulToolbar(props: Partial<ComponentProps<typeof Toolbar>>) {
  const [tool, setTool] = useState<ComponentProps<typeof Toolbar>['tool']>(
    props.tool ?? 'pen',
  );
  return (
    <Toolbar
      fingerDrawing
      onFingerDrawingChange={vi.fn()}
      showGrid={false}
      onShowGridChange={vi.fn()}
      penColor="#000000"
      onPenColorChange={vi.fn()}
      customColor={null}
      onCustomColorChange={vi.fn()}
      eraserMode="stroke"
      onEraserModeChange={vi.fn()}
      eraserSize={24}
      onEraserSizeChange={vi.fn()}
      history={createHistory()}
      viewScale={null}
      onResetView={vi.fn()}
      canClear
      onClear={vi.fn()}
      {...props}
      tool={tool}
      onToolChange={setTool}
    />
  );
}

const eraserButton = () => screen.getByRole('button', { name: 'Eraser' });
const modeButton = (name: string) => screen.queryByRole('button', { name });

/** Presses the eraser to open its popover */
async function openEraserSettings() {
  await userEvent.click(eraserButton());
  await screen.findByRole('dialog', { name: 'Eraser Settings' });
}

describe('Toolbar', () => {
  it('選択中のツールが押された状態になる', () => {
    renderToolbar({ tool: 'pen' });

    expect(screen.getByRole('button', { name: 'Pen' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(screen.getByRole('button', { name: 'Eraser' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  it('別のツールを押すと onToolChange が呼ばれる', async () => {
    const { onToolChange } = renderToolbar();

    await userEvent.click(screen.getByRole('button', { name: 'Eraser' }));
    expect(onToolChange).toHaveBeenCalledWith('eraser');
  });

  it('選択中のツールをもう一度押しても選択は外れない', async () => {
    const { onToolChange } = renderToolbar();

    await userEvent.click(screen.getByRole('button', { name: 'Pen' }));

    expect(onToolChange).not.toHaveBeenCalled();
  });

  it('消しゴムの設定は、押すまで出ない', () => {
    renderToolbar({ tool: 'eraser' });
    expect(modeButton('Object Eraser')).not.toBeInTheDocument();
  });

  it('消しゴムを押すと消しゴムに切り替わり、設定の吹き出しが出る', async () => {
    render(<StatefulToolbar tool="pen" />);

    await openEraserSettings();

    expect(eraserButton()).toHaveAttribute('aria-pressed', 'true');
    expect(modeButton('Object Eraser')).toHaveAttribute('aria-pressed', 'true');
  });

  it('選んでいる消しゴムをもう一度押すと吹き出しが閉じる', async () => {
    render(<StatefulToolbar tool="eraser" />);
    await openEraserSettings();

    await userEvent.click(eraserButton());

    expect(
      screen.queryByRole('dialog', { name: 'Eraser Settings' }),
    ).not.toBeInTheDocument();
  });

  it.each(['Pen', 'Eraser'])(
    '%s の吹き出しは、そのボタンの左の真横(上下の中央をそろえて)に出す',
    async (name) => {
      render(<StatefulToolbar tool={name === 'Pen' ? 'eraser' : 'pen'} />);

      await userEvent.click(screen.getByRole('button', { name }));

      const popup = await screen.findByRole('dialog', {
        name: `${name} Settings`,
      });
      expect(popup).toHaveAttribute('data-side', 'left');
      expect(popup).toHaveAttribute('data-align', 'center');
    },
  );

  it('ペンに切り替えると、吹き出しはペンの設定に変わる', async () => {
    render(<StatefulToolbar tool="eraser" />);
    await openEraserSettings();

    await userEvent.click(screen.getByRole('button', { name: 'Pen' }));

    expect(
      screen.queryByRole('dialog', { name: 'Eraser Settings' }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('dialog', { name: 'Pen Settings' }),
    ).toBeInTheDocument();
  });

  it('ペンの吹き出しで、黒・赤・青・緑から色を選べる', async () => {
    const { onPenColorChange } = renderToolbar({ tool: 'pen' });
    await userEvent.click(screen.getByRole('button', { name: 'Pen' }));
    await screen.findByRole('dialog', { name: 'Pen Settings' });

    expect(modeButton('Black')).toHaveAttribute('aria-pressed', 'true');
    for (const name of ['Red', 'Blue', 'Green']) {
      expect(modeButton(name)).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Red' }));

    expect(onPenColorChange).toHaveBeenCalledWith('#ff3b30');
  });

  it('カラーパレットで選んだ色は、カスタムの色として残し、ペンの色にする', async () => {
    const { onPenColorChange, onCustomColorChange } = renderToolbar({
      tool: 'pen',
    });
    await userEvent.click(screen.getByRole('button', { name: 'Pen' }));
    await screen.findByRole('dialog', { name: 'Pen Settings' });

    const palette = screen.getByLabelText('Choose Custom Color');
    fireEvent.input(palette, { target: { value: '#8e44ad' } });

    expect(onCustomColorChange).toHaveBeenCalledWith('#8e44ad');
    expect(onPenColorChange).toHaveBeenCalledWith('#8e44ad');
  });

  describe('最後の設定を選んだら吹き出しを閉じる', () => {
    const dialog = (name: string) => screen.queryByRole('dialog', { name });

    async function openPenSettings() {
      await userEvent.click(screen.getByRole('button', { name: 'Pen' }));
      await screen.findByRole('dialog', { name: 'Pen Settings' });
    }

    it('色の見本を押したら閉じる', async () => {
      renderToolbar({ tool: 'pen' });
      await openPenSettings();

      await userEvent.click(screen.getByRole('button', { name: 'Red' }));

      expect(dialog('Pen Settings')).not.toBeInTheDocument();
    });

    it('カラーパレットは、色を動かしている間は閉じず、決めたら閉じる', async () => {
      renderToolbar({ tool: 'pen' });
      await openPenSettings();
      const palette = screen.getByLabelText('Choose Custom Color');

      fireEvent.input(palette, { target: { value: '#8e44ad' } });
      expect(dialog('Pen Settings')).toBeInTheDocument();

      fireEvent.change(palette, { target: { value: '#8e44ad' } });
      expect(dialog('Pen Settings')).not.toBeInTheDocument();
    });

    it('画ごと消すを選んだら閉じる', async () => {
      renderToolbar({ tool: 'eraser', eraserMode: 'partial' });
      await openEraserSettings();

      await userEvent.click(
        screen.getByRole('button', { name: 'Object Eraser' }),
      );

      expect(dialog('Eraser Settings')).not.toBeInTheDocument();
    });

    it('なぞって消すを選んでも、続けて大きさを選べるよう閉じない', async () => {
      renderToolbar({ tool: 'eraser', eraserMode: 'stroke' });
      await openEraserSettings();

      await userEvent.click(
        screen.getByRole('button', { name: 'Pixel Eraser' }),
      );

      expect(dialog('Eraser Settings')).toBeInTheDocument();
    });

    it('大きさを変えても閉じない', async () => {
      renderToolbar({ tool: 'eraser', eraserMode: 'partial' });
      await openEraserSettings();

      const slider = screen.getByRole('slider', { name: 'Eraser Size' });
      act(() => slider.focus());
      await userEvent.keyboard('{ArrowRight}');

      expect(dialog('Eraser Settings')).toBeInTheDocument();
    });
  });

  it('指で書くかを切り替えられる', async () => {
    const { onFingerDrawingChange } = renderToolbar({ fingerDrawing: true });
    const toggle = screen.getByRole('button', { name: 'Draw with Finger' });
    expect(toggle).toHaveAttribute('aria-pressed', 'true');

    await userEvent.click(toggle);

    expect(onFingerDrawingChange).toHaveBeenCalledWith(false);
  });

  it('グリッドの表示を切り替えられる', async () => {
    const { onShowGridChange } = renderToolbar({ showGrid: false });
    const toggle = screen.getByRole('button', { name: 'Show Grid' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');

    await userEvent.click(toggle);

    expect(onShowGridChange).toHaveBeenCalledWith(true);
  });

  it('ペンのアイコンは、選んでいる色で描く', () => {
    renderToolbar({ penColor: '#ff3b30' });
    const icon = screen
      .getByRole('button', { name: 'Pen' })
      .querySelector('svg');
    expect(icon).toHaveStyle({ color: '#ff3b30' });
  });

  it('カスタムの色があれば、色の選択肢に並べる', async () => {
    renderToolbar({ tool: 'pen', customColor: '#8e44ad' });
    await userEvent.click(screen.getByRole('button', { name: 'Pen' }));
    await screen.findByRole('dialog', { name: 'Pen Settings' });

    expect(modeButton('Custom Color')).toBeInTheDocument();
  });

  it('吹き出しの中で消し方を選べる', async () => {
    const { onEraserModeChange } = renderToolbar({
      tool: 'eraser',
      eraserMode: 'stroke',
    });
    await openEraserSettings();

    await userEvent.click(screen.getByRole('button', { name: 'Pixel Eraser' }));

    expect(onEraserModeChange).toHaveBeenCalledWith('partial');
  });

  it('画ごと消すときは、大きさのスライダーが無効になる', async () => {
    renderToolbar({ tool: 'eraser', eraserMode: 'stroke' });
    await openEraserSettings();

    expect(screen.getByRole('slider', { name: 'Eraser Size' })).toBeDisabled();
  });

  it('なぞって消すときは、大きさのスライダーが使える', async () => {
    renderToolbar({ tool: 'eraser', eraserMode: 'partial' });
    await openEraserSettings();

    expect(screen.getByRole('slider', { name: 'Eraser Size' })).toBeEnabled();
  });

  it('スライダーで消しゴムの大きさを細かく変えられる', async () => {
    const { onEraserSizeChange } = renderToolbar({
      tool: 'eraser',
      eraserMode: 'partial',
      eraserSize: 24,
    });
    await openEraserSettings();
    expect(screen.getByText('24px')).toBeInTheDocument();

    const slider = screen.getByRole('slider', { name: 'Eraser Size' });
    // jsdom has no setPointerCapture, so instead of clicking, focus it and move it with keys
    act(() => slider.focus());
    await userEvent.keyboard('{ArrowRight}');

    expect(onEraserSizeChange).toHaveBeenCalled();
    const [size] = vi.mocked(onEraserSizeChange).mock.lastCall!;
    expect(size).toBeCloseTo(24.1);
  });

  it('取り消し・やり直しを押すと履歴の操作が呼ばれる', async () => {
    const { history } = renderToolbar();

    await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await userEvent.click(screen.getByRole('button', { name: 'Redo' }));

    expect(history.undo).toHaveBeenCalledOnce();
    expect(history.redo).toHaveBeenCalledOnce();
  });

  it('拡大縮小・移動していなければ、倍率のボタンは出ない', () => {
    renderToolbar({ viewScale: null });
    expect(
      screen.queryByRole('button', { name: 'Reset Zoom' }),
    ).not.toBeInTheDocument();
  });

  it('拡大しているときは倍率を出し、押すと元に戻す', async () => {
    const { onResetView } = renderToolbar({ viewScale: 1.5 });

    const reset = screen.getByRole('button', { name: 'Reset Zoom' });
    expect(reset).toHaveTextContent('150%');
    await userEvent.click(reset);

    expect(onResetView).toHaveBeenCalledOnce();
  });

  describe('Clear', () => {
    const clearButton = () => screen.getByRole('button', { name: 'Clear' });

    it('押すと確認し、クリアを選んだら消す', async () => {
      const { onClear } = renderToolbar();

      await userEvent.click(clearButton());
      const dialog = await screen.findByRole('alertdialog');
      expect(dialog).toHaveTextContent('Clear the canvas?');
      expect(dialog).toHaveTextContent(
        "All strokes will be erased. You can't undo this action.",
      );
      expect(onClear).not.toHaveBeenCalled();

      await userEvent.click(
        within(dialog).getByRole('button', { name: 'Clear' }),
      );
      expect(onClear).toHaveBeenCalledOnce();
    });

    it('キャンセルしたら消さない', async () => {
      const { onClear } = renderToolbar();

      await userEvent.click(clearButton());
      const dialog = await screen.findByRole('alertdialog');
      await userEvent.click(
        within(dialog).getByRole('button', { name: 'Cancel' }),
      );

      expect(onClear).not.toHaveBeenCalled();
    });

    it('線が無いときは押せない', () => {
      renderToolbar({ canClear: false });
      expect(clearButton()).toBeDisabled();
    });

    it('ツールバーの最後(下端)にある', () => {
      renderToolbar();
      const buttons = within(
        screen.getByRole('toolbar', { name: 'Drawing Tools' }),
      ).getAllByRole('button');
      expect(buttons.at(-1)).toBe(clearButton());
    });
  });

  it('サイドバーに置くので、ツールバーもツールの組も縦に並べる', () => {
    renderToolbar();

    expect(
      screen.getByRole('toolbar', { name: 'Drawing Tools' }),
    ).toHaveAttribute('aria-orientation', 'vertical');
    expect(screen.getByRole('group', { name: 'Tools' })).toHaveAttribute(
      'data-orientation',
      'vertical',
    );
  });

  it('取り消し・やり直しができないときはボタンが無効になる', () => {
    renderToolbar({
      history: createHistory({ canUndo: false, canRedo: false }),
    });

    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Redo' })).toBeDisabled();
  });
});
