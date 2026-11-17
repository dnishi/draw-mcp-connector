import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SendToAiButton } from './SendToAiButton';

describe('SendToAiButton', () => {
  it('押すと onSend が呼ばれる', async () => {
    const onSend = vi.fn();
    render(<SendToAiButton onSend={onSend} />);

    await userEvent.click(screen.getByRole('button', { name: 'Send to AI' }));

    expect(onSend).toHaveBeenCalledOnce();
  });

  it('onSend が null なら押せず、ほかのボタンの押せないときより薄い', () => {
    render(<SendToAiButton onSend={null} />);
    const button = screen.getByRole('button', { name: 'Send to AI' });

    expect(button).toBeDisabled();
    expect(button).toHaveClass('disabled:opacity-25');
    expect(button).not.toHaveClass('disabled:opacity-50');
  });
});
