import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { SheetControls } from '../canvas/types';
import type { MenuActions } from './menuItems';
import { MenuBar } from './MenuBar';

// Same construction as createControls in SheetSwitcher.test.tsx
const createControls = (props: Partial<SheetControls> = {}): SheetControls => ({
  number: 1,
  count: 1,
  previous: null,
  next: null,
  add: vi.fn(),
  remove: null,
  removeAll: vi.fn(),
  ...props,
});

const actions: MenuActions = { onExport: () => {} };

describe('MenuBar', () => {
  it('メニューバーには、左から Menu・シートの切り替え・Send to AI・接続の状態がある', () => {
    render(
      <MenuBar
        menu={actions}
        sheets={createControls()}
        status="disconnected"
        onSendToAi={() => {}}
      />,
    );
    const bar = screen.getByRole('banner', { name: 'Menu Bar' });
    const menu = within(bar).getByRole('button', { name: 'Menu' });
    const sheet = within(bar).getByRole('button', { name: 'Sheet 1 / 1' });
    const send = within(bar).getByRole('button', { name: 'Send to AI' });
    const status = within(bar).getByRole('status');

    // Order: verified in DOM order (a.compareDocumentPosition(b) & PRECEDING is true when b is before a)
    expect(
      sheet.compareDocumentPosition(menu) & Node.DOCUMENT_POSITION_PRECEDING,
    ).toBeTruthy();
    expect(
      send.compareDocumentPosition(sheet) & Node.DOCUMENT_POSITION_PRECEDING,
    ).toBeTruthy();
    expect(
      status.compareDocumentPosition(send) & Node.DOCUMENT_POSITION_PRECEDING,
    ).toBeTruthy();
  });

  it('onSendToAi が null なら Send to AI は押せない', () => {
    render(
      <MenuBar
        menu={actions}
        sheets={createControls()}
        status="disconnected"
        onSendToAi={null}
      />,
    );

    expect(
      within(screen.getByRole('banner', { name: 'Menu Bar' })).getByRole(
        'button',
        { name: 'Send to AI' },
      ),
    ).toBeDisabled();
  });
});
