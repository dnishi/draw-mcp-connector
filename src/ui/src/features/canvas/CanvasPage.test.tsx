import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

  it('グリッドの表示を切り替えるボタンは無い', () => {
    render(<CanvasPage />);
    expect(screen.queryByRole('button', { name: 'Show Grid' })).toBeNull();
  });

  describe('画像の取り込み', () => {
    // jsdom has no createImageBitmap or canvas 2D, so fake them as tools that work
    // up to re-encoding into PNG (still no WebGPU, so drawing cannot be tested)
    beforeEach(() => {
      vi.stubGlobal(
        'createImageBitmap',
        vi.fn(async () => ({ width: 400, height: 200, close: vi.fn() })),
      );
      vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
        drawImage: vi.fn(),
      } as unknown as CanvasRenderingContext2D);
      vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(
        (callback) =>
          callback(new Blob([new Uint8Array([1])], { type: 'image/png' })),
      );
    });
    afterEach(() => {
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
    });

    const pickFile = async () => {
      const input = document.querySelector(
        'input[type=file]',
      ) as HTMLInputElement;
      fireEvent.change(input, {
        target: {
          files: [
            new File([new Uint8Array([1, 2, 3])], 'a.png', {
              type: 'image/png',
            }),
          ],
        },
      });
    };

    it('ファイルを選ぶと画像が置いてあり、移動モードが使えるようになる', async () => {
      render(<CanvasPage />);
      expect(screen.getByRole('button', { name: 'Move Image' })).toBeDisabled();

      await pickFile();

      // Once a placement is in, Move can be pressed, and Clear works even without lines
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: 'Move Image' }),
        ).toBeEnabled(),
      );
      expect(screen.getByRole('button', { name: 'Clear' })).toBeEnabled();
    });

    describe('画像の大きさと削除の吹き出し', () => {
      // Without a paper size the image is placed as 0×0 and clicks never hit it, so
      // fake the canvas size (800×600) and the notice of its change
      beforeEach(() => {
        vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(
          800,
        );
        vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(
          600,
        );
        vi.stubGlobal(
          'ResizeObserver',
          class {
            #callback: (entries: unknown[]) => void;
            constructor(callback: (entries: unknown[]) => void) {
              this.#callback = callback;
            }
            observe() {
              this.#callback([]);
            }
            unobserve() {}
            disconnect() {}
          },
        );
        HTMLCanvasElement.prototype.setPointerCapture = vi.fn();
      });

      const moveButton = () =>
        screen.getByRole('button', { name: 'Move Image' });
      const popup = () =>
        screen.queryByRole('dialog', { name: 'Image Settings' });
      /** Presses and releases the mouse at (x, y) on the canvas without moving it */
      const clickCanvas = async (x: number, y: number) => {
        await userEvent.pointer({
          keys: '[MouseLeft]',
          target: screen.getByLabelText('Canvas'),
          coords: { clientX: x, clientY: y },
        });
      };
      const placeImage = async () => {
        render(<CanvasPage />);
        await pickFile();
        await waitFor(() => expect(moveButton()).toBeEnabled());
      };

      it('「Move Image」で移動モードにすると出し、OFF にすると閉じる', async () => {
        await placeImage();

        await userEvent.click(moveButton());
        expect(
          await screen.findByRole('dialog', { name: 'Image Settings' }),
        ).toBeInTheDocument();

        await userEvent.click(moveButton());
        await waitFor(() => expect(popup()).not.toBeInTheDocument());
      });

      it('画像をクリックすると出し、もう一度クリックしても閉じず、画像の外のクリックで閉じる', async () => {
        await placeImage();
        await userEvent.click(moveButton());
        // Close the popover opened by the button, then check that clicking the image opens it
        await clickCanvas(5, 5);
        await waitFor(() => expect(popup()).not.toBeInTheDocument());

        // A 400×200 image is placed in the middle of the 800×600 paper (200 to 600, 200 to 400)
        await clickCanvas(400, 300);
        expect(
          await screen.findByRole('dialog', { name: 'Image Settings' }),
        ).toBeInTheDocument();

        await clickCanvas(420, 310);
        expect(popup()).toBeInTheDocument();

        await clickCanvas(5, 5);
        await waitFor(() => expect(popup()).not.toBeInTheDocument());
      });
    });

    // In jsdom the WebGPU error is shown first (notice prefers error), so
    // only check that the image is not placed
    it('読み込めない画像は、画像を置かない', async () => {
      vi.mocked(createImageBitmap).mockRejectedValue(new Error('bad image'));
      render(<CanvasPage />);

      await pickFile();
      await waitFor(() => expect(createImageBitmap).toHaveBeenCalled());

      expect(screen.getByRole('button', { name: 'Move Image' })).toBeDisabled();
    });
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
