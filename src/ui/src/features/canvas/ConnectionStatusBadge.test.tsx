import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ConnectionStatusBadge } from './ConnectionStatusBadge';
import type { ConnectionStatus } from './types';

describe('ConnectionStatusBadge', () => {
  it.each<[ConnectionStatus, string]>([
    ['disconnected', 'Not Connected'],
    ['connecting', 'Connecting'],
    ['waiting', 'Waiting for Approval'],
    ['connected', 'Connected'],
  ])('%s のときは「%s」と表示する', (status, label) => {
    render(<ConnectionStatusBadge status={status} />);

    expect(screen.getByRole('status')).toHaveTextContent(label);
  });

  it('画面にはアイコンだけを出し、状態の名前は読み上げとマウスを重ねたときの表示にだけ使う', () => {
    render(<ConnectionStatusBadge status="waiting" />);
    const status = screen.getByRole('status');

    expect(status).toHaveAttribute('title', 'Waiting for Approval');
    expect(within(status).getByText('Waiting for Approval')).toHaveClass(
      'sr-only',
    );
  });

  it('接続済みのときだけ、アイコンを緑にする', () => {
    const { rerender } = render(<ConnectionStatusBadge status="connected" />);
    const icon = () => screen.getByRole('status').querySelector('svg');
    expect(icon()).toHaveClass('text-[#34c759]');

    rerender(<ConnectionStatusBadge status="waiting" />);
    expect(icon()).not.toHaveClass('text-[#34c759]');
  });

  it('接続中で AI が待っているときは「Connected. AI is waiting」で、アイコンを明滅させる', () => {
    render(<ConnectionStatusBadge status="connected" aiWaiting />);
    const status = screen.getByRole('status');

    expect(status).toHaveAttribute('title', 'Connected. AI is waiting');
    expect(within(status).getByText('Connected. AI is waiting')).toHaveClass(
      'sr-only',
    );
    expect(status.querySelector('svg')).toHaveClass(
      'motion-safe:animate-pulse',
    );
  });

  it('接続中でなければ、aiWaiting でも明滅させない', () => {
    render(<ConnectionStatusBadge status="disconnected" aiWaiting />);
    const status = screen.getByRole('status');

    expect(status).toHaveAttribute('title', 'Not Connected');
    expect(status.querySelector('svg')).not.toHaveClass(
      'motion-safe:animate-pulse',
    );
  });

  it('aiWaiting が無ければ従来通り', () => {
    render(<ConnectionStatusBadge status="connected" />);
    const status = screen.getByRole('status');

    expect(status).toHaveAttribute('title', 'Connected');
    expect(status.querySelector('svg')).not.toHaveClass(
      'motion-safe:animate-pulse',
    );
  });

  it('繋ぎ直せるときは、押すと繋ぎ直すボタンになる', async () => {
    const onReconnect = vi.fn();
    render(
      <ConnectionStatusBadge status="disconnected" onReconnect={onReconnect} />,
    );

    await userEvent.click(
      screen.getByRole('button', { name: /Not Connected.*Reconnect/ }),
    );

    expect(onReconnect).toHaveBeenCalledOnce();
  });

  it('繋ぎ直せないときは、ボタンにしない', () => {
    render(<ConnectionStatusBadge status="disconnected" />);

    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });
});
