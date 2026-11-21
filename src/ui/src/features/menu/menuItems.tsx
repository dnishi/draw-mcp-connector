import { Menu } from '@base-ui/react/menu';
import type { ReactNode } from 'react';

/**
 * Items lined up in the hamburger menu (left end of the menu bar).
 * Public repository version: the only item is Export
 * The difference from the private repository is kept in this file, so AppMenu.tsx and MenuBar.tsx are shared
 */

export interface MenuActions {
  /** When "Export" is pressed. null when there is no renderer (WebGPU unavailable), then it can't be pressed */
  onExport: (() => void) | null;
}

/** Look of the items (same style as the Toolbar's Popover) */
const ITEM_CLASS =
  'cursor-default rounded-lg px-3 py-2 text-sm outline-none data-[highlighted]:bg-muted data-[disabled]:opacity-25';

export function useMenuItems({ onExport }: MenuActions): {
  items: ReactNode;
  dialogs: ReactNode;
} {
  return {
    items: (
      <Menu.Item
        disabled={onExport === null}
        onClick={() => onExport?.()}
        className={ITEM_CLASS}
      >
        Export
      </Menu.Item>
    ),
    // Nothing is drawn outside the menu (dialogs)
    dialogs: null,
  };
}
