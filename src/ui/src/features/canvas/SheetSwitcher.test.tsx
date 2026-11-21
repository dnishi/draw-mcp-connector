import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SheetSwitcher } from './SheetSwitcher';
import type { SheetControls } from './types';

const createControls = (props: Partial<SheetControls> = {}): SheetControls => ({
  number: 2,
  count: 5,
  previous: vi.fn(),
  next: vi.fn(),
  add: vi.fn(),
  remove: vi.fn(),
  removeAll: vi.fn(),
  ...props,
});

const openMenu = async () => {
  await userEvent.click(screen.getByRole('button', { name: 'Sheet 2 / 5' }));
  await screen.findByRole('menu');
};

describe('SheetSwitcher', () => {
  it('今のシートの番号と枚数を出し、前・次・新規のボタンがそれぞれ呼ばれる', async () => {
    const controls = createControls();
    render(<SheetSwitcher sheets={controls} />);

    expect(
      screen.getByRole('button', { name: 'Sheet 2 / 5' }),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole('button', { name: 'Previous Sheet' }),
    );
    await userEvent.click(screen.getByRole('button', { name: 'Next Sheet' }));
    await userEvent.click(screen.getByRole('button', { name: 'New Sheet' }));

    expect(controls.previous).toHaveBeenCalledOnce();
    expect(controls.next).toHaveBeenCalledOnce();
    expect(controls.add).toHaveBeenCalledOnce();
  });

  it('前のシート・次のシート・新しいシートが作れない(null)ときは、それぞれのボタンが押せない', () => {
    render(
      <SheetSwitcher
        sheets={createControls({
          previous: null,
          next: null,
          add: null,
        })}
      />,
    );

    expect(
      screen.getByRole('button', { name: 'Previous Sheet' }),
    ).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Next Sheet' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'New Sheet' })).toBeDisabled();
  });

  it('押せないボタンは、Send to AI と同じくほかの押せないボタンより薄い(25%)', () => {
    render(<SheetSwitcher sheets={createControls()} />);

    for (const name of ['Previous Sheet', 'Next Sheet', 'New Sheet']) {
      const button = screen.getByRole('button', { name });
      expect(button).toHaveClass('disabled:opacity-25');
      expect(button).not.toHaveClass('disabled:opacity-50');
    }
  });

  it('メニューの「Delete Sheet…」→「Delete」で今のシートを消し、「Cancel」では消さない', async () => {
    const controls = createControls();
    render(<SheetSwitcher sheets={controls} />);

    await openMenu();
    await userEvent.click(
      screen.getByRole('menuitem', { name: 'Delete Sheet…' }),
    );
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent('Delete this sheet?');
    expect(dialog).toHaveTextContent(
      "Sheet 2 and everything on it will be deleted. You can't undo this action.",
    );

    await userEvent.click(
      within(dialog).getByRole('button', { name: 'Cancel' }),
    );
    expect(controls.remove).not.toHaveBeenCalled();

    await openMenu();
    await userEvent.click(
      screen.getByRole('menuitem', { name: 'Delete Sheet…' }),
    );
    await screen.findByRole('alertdialog');
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(controls.remove).toHaveBeenCalledOnce();
  });

  it('メニューの「Delete All Sheets…」→「Delete All」で全部のシートを消す', async () => {
    const controls = createControls();
    render(<SheetSwitcher sheets={controls} />);

    await openMenu();
    await userEvent.click(
      screen.getByRole('menuitem', { name: 'Delete All Sheets…' }),
    );
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent('Delete all sheets?');
    expect(dialog).toHaveTextContent(
      "All sheets will be deleted, leaving one blank sheet. You can't undo this action.",
    );

    await userEvent.click(
      within(dialog).getByRole('button', { name: 'Delete All' }),
    );
    expect(controls.removeAll).toHaveBeenCalledOnce();
  });

  it('シートを消せない(null)ときは「Delete Sheet…」が押せない', async () => {
    render(<SheetSwitcher sheets={createControls({ remove: null })} />);

    await openMenu();
    // Base UI marks a disabled menu item with aria-disabled instead of the disabled attribute
    expect(
      screen.getByRole('menuitem', { name: 'Delete Sheet…' }),
    ).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByRole('menuitem', { name: 'Delete Sheet…' })).toHaveClass(
      'data-[disabled]:opacity-25',
    );
    expect(
      screen.getByRole('menuitem', { name: 'Delete All Sheets…' }),
    ).not.toHaveAttribute('aria-disabled');
  });
});
