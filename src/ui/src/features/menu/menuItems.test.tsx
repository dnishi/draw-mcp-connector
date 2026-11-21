import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AppMenu } from './AppMenu';

describe('menuItems', () => {
  /** Open the menu (Base UI opens it with requestAnimationFrame) */
  const openMenu = async () => {
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    await screen.findByRole('menu');
  };

  it('「Menu」を押すと Export だけが出る', async () => {
    render(<AppMenu onExport={() => {}} />);

    await openMenu();

    expect(screen.getAllByRole('menuitem')).toHaveLength(1);
    expect(
      screen.getByRole('menuitem', { name: 'Export' }),
    ).toBeInTheDocument();
  });

  it('「Export」で onExport が呼ばれる', async () => {
    const onExport = vi.fn();
    render(<AppMenu onExport={onExport} />);

    await openMenu();
    await userEvent.click(screen.getByRole('menuitem', { name: 'Export' }));

    expect(onExport).toHaveBeenCalledTimes(1);
  });

  it('onExport が null なら「Export」は押せない', async () => {
    render(<AppMenu onExport={null} />);

    await openMenu();

    // The item is a div, so it gets aria-disabled instead of disabled
    expect(screen.getByRole('menuitem', { name: 'Export' })).toHaveAttribute(
      'aria-disabled',
      'true',
    );
  });
});
