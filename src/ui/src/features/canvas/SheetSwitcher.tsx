import { ChevronDown, ChevronLeft, ChevronRight, Plus } from 'lucide-react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { Menu } from '@base-ui/react/menu';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { AlertDialogPopup } from './AlertDialogPopup';
import type { SheetControls } from './types';

/** Look of the items (same style as AppMenu) */
const ITEM_CLASS =
  'cursor-default rounded-lg px-3 py-2 text-sm outline-none data-[highlighted]:bg-muted data-[disabled]:opacity-25';

/** Opacity of disabled buttons. 25%, the same as Send to AI, so they are easy to tell from enabled ones */
const DISABLED_CLASS = 'disabled:opacity-25';

interface SheetSwitcherProps {
  /** Switching and deleting sheets (sheets from useCanvasDrawing) */
  sheets: SheetControls;
}

/**
 * Sheet switcher, overlaid at the top right of the canvas.
 * Previous, next, and new sheet are buttons; deleting is in the menu of the number button
 */
export function SheetSwitcher({ sheets }: SheetSwitcherProps) {
  /**
   * Delete confirmation dialogs. They open from menu items (no Trigger),
   * so their open state is kept here
   */
  const [removeOpen, setRemoveOpen] = useState(false);
  const [removeAllOpen, setRemoveAllOpen] = useState(false);

  return (
    <div className="flex items-center gap-1 rounded-xl border bg-background p-1 shadow-sm">
      <Button
        variant="ghost"
        size="icon"
        aria-label="Previous Sheet"
        className={DISABLED_CLASS}
        title="Previous Sheet"
        disabled={sheets.previous === null}
        onClick={() => sheets.previous?.()}
      >
        <ChevronLeft />
      </Button>
      <Menu.Root>
        <Menu.Trigger render={<Button variant="ghost" />}>
          Sheet {sheets.number} / {sheets.count}
          <ChevronDown className="size-3.5" />
        </Menu.Trigger>
        <Menu.Portal>
          <Menu.Positioner side="bottom" align="end" sideOffset={4}>
            <Menu.Popup className="rounded-xl border bg-background p-1 shadow-md outline-none">
              <Menu.Item
                disabled={sheets.remove === null}
                onClick={() => setRemoveOpen(true)}
                className={ITEM_CLASS}
              >
                Delete Sheet…
              </Menu.Item>
              <Menu.Item
                onClick={() => setRemoveAllOpen(true)}
                className={ITEM_CLASS}
              >
                Delete All Sheets…
              </Menu.Item>
            </Menu.Popup>
          </Menu.Positioner>
        </Menu.Portal>
      </Menu.Root>
      <Button
        variant="ghost"
        size="icon"
        aria-label="Next Sheet"
        className={DISABLED_CLASS}
        title="Next Sheet"
        disabled={sheets.next === null}
        onClick={() => sheets.next?.()}
      >
        <ChevronRight />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        aria-label="New Sheet"
        className={DISABLED_CLASS}
        title="New Sheet"
        disabled={sheets.add === null}
        onClick={() => sheets.add?.()}
      >
        <Plus />
      </Button>

      {/* The confirmation text and buttons follow the same approach as Clear in Toolbar (Apple HIG, Alerts) */}
      <AlertDialog.Root open={removeOpen} onOpenChange={setRemoveOpen}>
        <AlertDialogPopup
          title="Delete this sheet?"
          description={`Sheet ${sheets.number} and everything on it will be deleted. You can't undo this action.`}
        >
          <AlertDialog.Close render={<Button variant="outline" />}>
            Cancel
          </AlertDialog.Close>
          <AlertDialog.Close
            render={<Button onClick={() => sheets.remove?.()} />}
          >
            Delete
          </AlertDialog.Close>
        </AlertDialogPopup>
      </AlertDialog.Root>
      <AlertDialog.Root open={removeAllOpen} onOpenChange={setRemoveAllOpen}>
        <AlertDialogPopup
          title="Delete all sheets?"
          description="All sheets will be deleted, leaving one blank sheet. You can't undo this action."
        >
          <AlertDialog.Close render={<Button variant="outline" />}>
            Cancel
          </AlertDialog.Close>
          <AlertDialog.Close render={<Button onClick={sheets.removeAll} />}>
            Delete All
          </AlertDialog.Close>
        </AlertDialogPopup>
      </AlertDialog.Root>
    </div>
  );
}
