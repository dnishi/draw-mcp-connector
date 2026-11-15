import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef, useState, type ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ImageSettingsPopover } from './ImageSettingsPopover';

type Props = ComponentProps<typeof ImageSettingsPopover>;

/** Parent that holds open/close (in place of CanvasPage). Places the anchor button and a canvas that does not close it when pressed */
function Harness(
  props: Partial<
    Omit<Props, 'open' | 'onClose' | 'anchor' | 'ignoreOutsidePressOn'>
  >,
) {
  const anchor = useRef<HTMLButtonElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [open, setOpen] = useState(true);
  return (
    <>
      <button ref={anchor}>Move Image</button>
      <canvas ref={canvas} aria-label="Canvas" />
      <button>Elsewhere</button>
      <ImageSettingsPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        side="left"
        ignoreOutsidePressOn={[canvas]}
        imageScale={100}
        onImageScaleChange={vi.fn()}
        onImageScaleCommit={vi.fn()}
        onImageDelete={vi.fn()}
        {...props}
      />
    </>
  );
}

const popup = () => screen.queryByRole('dialog', { name: 'Image Settings' });

describe('ImageSettingsPopover', () => {
  it('大きさ(元の画像に対する割合)と削除を出す', async () => {
    render(<Harness imageScale={100} />);

    expect(
      await screen.findByRole('dialog', { name: 'Image Settings' }),
    ).toBeInTheDocument();
    expect(screen.getByText('100%')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Delete Image' }),
    ).toBeInTheDocument();
  });

  it('デコードが終わるまでは、大きさのスライダーは無効', async () => {
    render(<Harness imageScale={null} />);
    await screen.findByRole('dialog', { name: 'Image Settings' });

    expect(screen.getByRole('slider', { name: 'Image Size' })).toBeDisabled();
  });

  // Committing (releasing the slider) happens on pointerup, which jsdom cannot drive.
  // Check only that it changes while moving
  it('スライダーを動かすと大きさが変わる', async () => {
    const onImageScaleChange = vi.fn();
    render(<Harness onImageScaleChange={onImageScaleChange} />);
    await screen.findByRole('dialog', { name: 'Image Settings' });

    const slider = screen.getByRole('slider', { name: 'Image Size' });
    act(() => slider.focus());
    await userEvent.keyboard('{ArrowRight}');

    const [percent] = vi.mocked(onImageScaleChange).mock.lastCall!;
    expect(percent).toBeCloseTo(101);
  });

  it('削除を押すと画像を消し、吹き出しは閉じる', async () => {
    const onImageDelete = vi.fn();
    render(<Harness onImageDelete={onImageDelete} />);
    await screen.findByRole('dialog', { name: 'Image Settings' });

    await userEvent.click(screen.getByRole('button', { name: 'Delete Image' }));

    expect(onImageDelete).toHaveBeenCalledOnce();
    expect(popup()).not.toBeInTheDocument();
  });

  it('キャンバスを押しても閉じない(キャンバスの上のクリックは親が扱う)', async () => {
    render(<Harness />);
    await screen.findByRole('dialog', { name: 'Image Settings' });

    await userEvent.click(screen.getByLabelText('Canvas'));

    expect(popup()).toBeInTheDocument();
  });

  it('吹き出しとキャンバスの外を押すと閉じる', async () => {
    render(<Harness />);
    await screen.findByRole('dialog', { name: 'Image Settings' });

    await userEvent.click(screen.getByRole('button', { name: 'Elsewhere' }));

    expect(popup()).not.toBeInTheDocument();
  });
});
