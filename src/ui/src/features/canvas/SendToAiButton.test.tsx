import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SendToAiButton } from './SendToAiButton';

describe('SendToAiButton', () => {
  it('押すと onSend が呼ばれる', async () => {
    const onSend = vi.fn();
    render(<SendToAiButton onSend={onSend} queued={false} />);

    await userEvent.click(screen.getByRole('button', { name: 'Send to AI' }));

    expect(onSend).toHaveBeenCalledOnce();
  });

  it('onSend が null なら押せない', () => {
    render(<SendToAiButton onSend={null} queued={false} />);

    expect(screen.getByRole('button', { name: 'Send to AI' })).toBeDisabled();
  });

  it('queued なら名前が変わり点が出る', () => {
    render(<SendToAiButton onSend={vi.fn()} queued />);
    const button = screen.getByRole('button', {
      name: 'Send to AI. Sent, waiting for the AI to read it',
    });

    expect(button).not.toBeDisabled();
    expect(button).toHaveAttribute(
      'title',
      'Send to AI. Sent, waiting for the AI to read it',
    );
    expect(button.querySelector('span.bg-primary')).toBeInTheDocument();
  });

  it('queued でなければ点は出ない', () => {
    render(<SendToAiButton onSend={vi.fn()} queued={false} />);
    const button = screen.getByRole('button', { name: 'Send to AI' });

    expect(button.querySelector('span.bg-primary')).toBeNull();
  });
});
