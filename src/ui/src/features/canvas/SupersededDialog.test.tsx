import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SupersededDialog } from './SupersededDialog';

describe('SupersededDialog', () => {
  it('開いているときだけ知らせ、OK で閉じる', async () => {
    const onClose = vi.fn();
    const { rerender } = render(
      <SupersededDialog open={false} onClose={onClose} />,
    );
    expect(screen.queryByRole('alertdialog')).toBeNull();

    rerender(<SupersededDialog open onClose={onClose} />);
    expect(screen.getByRole('alertdialog')).toHaveTextContent(
      'Another canvas page reconnected to the PC',
    );

    await userEvent.click(screen.getByRole('button', { name: 'OK' }));
    expect(onClose).toHaveBeenCalled();
  });
});
