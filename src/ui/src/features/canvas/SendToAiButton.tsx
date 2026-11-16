import { Send } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from 'cn';

interface SendToAiButtonProps {
  /** Sends the notice. When null (not connected), the button cannot be pressed */
  onSend: (() => void) | null;
  /** Pressed, but the AI (wait_for_canvas) has not picked it up yet */
  queued: boolean;
}

/**
 * Tells the AI that the user finished writing (the bridge hands it to MCP's wait_for_canvas).
 * If no AI is waiting, the bridge remembers it and the next call returns right away
 */
export function SendToAiButton({ onSend, queued }: SendToAiButtonProps) {
  const name = queued
    ? 'Send to AI. Sent, waiting for the AI to read it'
    : 'Send to AI';
  return (
    <Button
      variant="ghost"
      size="icon-lg"
      aria-label={name}
      title={name}
      disabled={onSend === null}
      onClick={onSend ?? undefined}
      className={cn(queued && 'relative')}
    >
      <Send />
      {queued && (
        // Show a dot at the top right until the AI picks it up
        <span
          aria-hidden
          className="absolute top-0 right-0 size-1.5 rounded-full bg-primary"
        />
      )}
    </Button>
  );
}
