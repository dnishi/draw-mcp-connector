import { Send } from 'lucide-react';
import { Button } from '@/components/ui/button';

interface SendToAiButtonProps {
  /** Function that sends the notice. If null (not connected, or the AI is not waiting), the button is disabled */
  onSend: (() => void) | null;
}

/**
 * Tells the AI that you have finished writing (the bridge hands it to the MCP tool wait_for_canvas).
 * It can be pressed only while the AI is waiting; otherwise it is faded (disabled).
 * To tell it apart from when it can be pressed, it is fainter than other disabled buttons (half opacity)
 */
export function SendToAiButton({ onSend }: SendToAiButtonProps) {
  return (
    <Button
      variant="ghost"
      size="icon-lg"
      aria-label="Send to AI"
      title="Send to AI"
      disabled={onSend === null}
      onClick={onSend ?? undefined}
      className="disabled:opacity-25"
    >
      <Send />
    </Button>
  );
}
