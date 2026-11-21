import { Menu as MenuIcon } from 'lucide-react';
import { Menu } from '@base-ui/react/menu';
import { Button } from '@/components/ui/button';
import type { MenuActions } from './menuItems';
import { useMenuItems } from './menuItems';

/**
 * Hamburger menu at the left end of the menu bar.
 * It does not know the contents of the items; it leaves them all to menuItems.tsx (`useMenuItems`)
 */
export function AppMenu(props: MenuActions) {
  const { items, dialogs } = useMenuItems(props);

  return (
    <>
      <Menu.Root>
        {/* Same size as the toolbar buttons */}
        <Menu.Trigger
          render={<Button variant="ghost" size="icon-lg" aria-label="Menu" />}
        >
          <MenuIcon />
        </Menu.Trigger>
        <Menu.Portal>
          <Menu.Positioner side="bottom" align="start" sideOffset={8}>
            {/* Looks the same as the Toolbar's Popover */}
            <Menu.Popup className="rounded-xl border bg-background p-1 shadow-md outline-none">
              {items}
            </Menu.Popup>
          </Menu.Positioner>
        </Menu.Portal>
      </Menu.Root>
      {dialogs}
    </>
  );
}
