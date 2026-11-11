import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { CanvasPage } from './CanvasPage';

describe('CanvasPage', () => {
  // jsdom has no navigator.gpu, so it behaves like a browser without WebGPU
  it('WebGPU を使えないときは理由を表示する', async () => {
    render(<CanvasPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent('WebGPU');
  });

  it('右のサイドバーに、ツールと接続の状態をまとめる', () => {
    render(<CanvasPage />);
    const sidebar = within(
      screen.getByRole('complementary', { name: 'Sidebar' }),
    );

    expect(
      sidebar.getByRole('toolbar', { name: 'Drawing Tools' }),
    ).toBeInTheDocument();
    // A page without connection info does not connect
    expect(sidebar.getByRole('status')).toHaveTextContent('Not Connected');
    // The sidebar does not overlap the canvas; it sits to the right of it
    expect(sidebar.queryByLabelText('Canvas')).toBeNull();
  });

  it('接続の状態はサイドバーの一番上(ツールより前)に置く', () => {
    render(<CanvasPage />);
    const sidebar = screen.getByRole('complementary', { name: 'Sidebar' });
    const status = within(sidebar).getByRole('status');

    expect(sidebar.firstElementChild).toBe(status);
  });

  it('指で書くは初めはオンで、ペンを使うと自動でオフになる', () => {
    render(<CanvasPage />);
    const toggle = screen.getByRole('button', { name: 'Draw with Finger' });
    expect(toggle).toHaveAttribute('aria-pressed', 'true');

    fireEvent.pointerDown(screen.getByLabelText('Canvas'), {
      pointerType: 'pen',
    });

    expect(toggle).toHaveAttribute('aria-pressed', 'false');
  });

  it('グリッドは初めは表示せず、ツールバーで切り替えられる', async () => {
    render(<CanvasPage />);
    const toggle = screen.getByRole('button', { name: 'Show Grid' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');

    await userEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-pressed', 'true');
  });

  describe('PC のマウスのポインタ', () => {
    const canvas = () => screen.getByLabelText('Canvas');
    const eraserCursor = () => screen.getByTestId('eraser-cursor');
    const hoverWithMouse = (x: number, y: number) =>
      fireEvent.pointerMove(canvas(), {
        pointerType: 'mouse',
        clientX: x,
        clientY: y,
      });

    it('ペンでは十字のポインタにし、消しゴムの円は出さない', () => {
      render(<CanvasPage />);
      expect(canvas()).toHaveClass('data-[tool=pen]:cursor-crosshair');
      expect(canvas()).toHaveAttribute('data-tool', 'pen');

      hoverWithMouse(50, 40);

      expect(eraserCursor()).not.toBeVisible();
    });

    it('消しゴムでは、ポインタの代わりに消しゴムの大きさの円がマウスに付いてくる', async () => {
      render(<CanvasPage />);
      await userEvent.click(screen.getByRole('button', { name: 'Eraser' }));
      expect(canvas()).toHaveAttribute('data-tool', 'eraser');

      hoverWithMouse(50, 40);

      // The hit radius of the whole-stroke eraser is 12px
      expect(eraserCursor()).toBeVisible();
      expect(eraserCursor()).toHaveStyle({
        width: '24px',
        height: '24px',
        transform: 'translate(38px, 28px)',
      });

      fireEvent.pointerLeave(canvas(), { pointerType: 'mouse' });
      expect(eraserCursor()).not.toBeVisible();
    });
  });
});
