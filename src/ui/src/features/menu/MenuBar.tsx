import type { ConnectionStatus, SheetControls } from '../canvas/types';
import { ConnectionStatusBadge } from '../canvas/ConnectionStatusBadge';
import { SendToAiButton } from '../canvas/SendToAiButton';
import { SheetSwitcher } from '../canvas/SheetSwitcher';
import { AppMenu } from './AppMenu';
import type { MenuActions } from './menuItems';

interface MenuBarProps {
  /** Functions called by the hamburger menu items (menuItems.tsx) */
  menu: MenuActions;
  /** Switching and deleting sheets (sheets from useCanvasDrawing) */
  sheets: SheetControls;
  status: ConnectionStatus;
  /** Passed only when a reconnect is possible (after a drop). When passed, it can be pressed to reconnect (same as ConnectionStatusBadge) */
  onReconnect?: (() => void) | null;
  /** Function that sends the notice. Not pressable if null (same as `onSend` of SendToAiButton) */
  onSendToAi: (() => void) | null;
}

/**
 * The band at the top of the screen. It holds only the layout (the item contents are
 * held by AppMenu, SheetSwitcher, etc.). It does not overlap the canvas, but narrows
 * the canvas height by its own height
 */
export function MenuBar({
  menu,
  sheets,
  status,
  onReconnect,
  onSendToAi,
}: MenuBarProps) {
  return (
    <header
      aria-label="Menu Bar"
      className="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-2 border-b bg-background px-2 py-1"
    >
      {/* To place the center column at the center of the screen, the left and right columns are the same width (1fr) */}
      <div className="justify-self-start">
        <AppMenu {...menu} />
      </div>
      <SheetSwitcher sheets={sheets} />
      <div className="flex items-center gap-1 justify-self-end">
        <SendToAiButton onSend={onSendToAi} />
        <ConnectionStatusBadge status={status} onReconnect={onReconnect} />
      </div>
    </header>
  );
}
