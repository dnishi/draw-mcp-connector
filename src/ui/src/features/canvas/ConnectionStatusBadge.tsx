import {
  Hourglass,
  LoaderCircle,
  Wifi,
  WifiOff,
  type LucideIcon,
} from 'lucide-react';
import { cn } from 'cn';
import type { ConnectionStatus } from './types';

/** Color scheme of a state (background and text) */
const TONE_CLASS = {
  secondary: 'bg-secondary text-secondary-foreground',
  destructive: 'bg-destructive/10 text-destructive',
} as const;

const STATUS_VIEW: Record<
  ConnectionStatus,
  {
    label: string;
    icon: LucideIcon;
    tone: keyof typeof TONE_CLASS;
    iconClassName?: string;
  }
> = {
  disconnected: {
    label: 'Not Connected',
    icon: WifiOff,
    tone: 'destructive',
  },
  connecting: {
    label: 'Connecting',
    icon: LoaderCircle,
    tone: 'secondary',
    iconClassName: 'animate-spin',
  },
  waiting: {
    label: 'Waiting for Approval',
    icon: Hourglass,
    tone: 'secondary',
  },
  // The green is the same Apple system color as "connected" in the PC menu bar icon (src/bridge/src/icon.rs)
  connected: {
    label: 'Connected',
    icon: Wifi,
    tone: 'secondary',
    iconClassName: 'text-[#34c759] dark:text-[#30d158]',
  },
};

/**
 * Shows only the icon, in a square the same size as the tool buttons.
 * The state name is not shown on screen; it is used only for screen readers (sr-only) and the hover text (title)
 */
const ITEM_CLASS =
  'flex size-9 shrink-0 items-center justify-center rounded-lg [&_svg]:size-4';

interface ConnectionStatusBadgeProps {
  status: ConnectionStatus;
  /** Passed only when reconnecting is possible (after a disconnect). When passed, pressing it reconnects */
  onReconnect?: (() => void) | null;
  /** The AI is waiting in wait_for_canvas. Used only while connected */
  aiWaiting?: boolean;
}

export function ConnectionStatusBadge({
  status,
  onReconnect,
  aiWaiting,
}: ConnectionStatusBadgeProps) {
  const { label, icon: Icon, tone, iconClassName } = STATUS_VIEW[status];
  // While the AI is waiting, the icon pulses slowly and the status name is "Connected. AI is waiting"
  const waitingForAi = status === 'connected' && aiWaiting === true;
  const name = waitingForAi ? `${label}. AI is waiting` : label;
  const className = cn(ITEM_CLASS, TONE_CLASS[tone]);
  const icon = (
    <Icon
      aria-hidden
      className={cn(iconClassName, waitingForAi && 'motion-safe:animate-pulse')}
    />
  );

  if (onReconnect) {
    return (
      <button
        type="button"
        onClick={onReconnect}
        title={`${label}. Reconnect`}
        className={cn(
          className,
          'cursor-pointer outline-none hover:bg-destructive/20 focus-visible:ring-3 focus-visible:ring-destructive/20',
        )}
      >
        {icon}
        <span className="sr-only">{label}. Reconnect</span>
      </button>
    );
  }

  return (
    <span
      role="status"
      title={name}
      className={cn(className, waitingForAi && 'relative')}
    >
      {icon}
      {/* For people with reduced motion (motion-reduce), show a dot at the top right of the square instead of pulsing */}
      {waitingForAi && (
        <span
          aria-hidden
          className="absolute top-0 right-0 hidden size-1.5 rounded-full bg-[#34c759] motion-reduce:block dark:bg-[#30d158]"
        />
      )}
      <span className="sr-only">{name}</span>
    </span>
  );
}
