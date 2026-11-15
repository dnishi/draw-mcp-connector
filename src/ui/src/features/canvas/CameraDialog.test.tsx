import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CameraDialog } from './CameraDialog';

const videoInput = () => ({ kind: 'videoinput' }) as MediaDeviceInfo;

/** A fake camera track */
function createFakeTrack() {
  return { stop: vi.fn() };
}

/** A fake MediaStream (getTracks returns the same tracks every time) */
function createFakeStream() {
  const tracks = [createFakeTrack(), createFakeTrack()];
  return {
    stream: { getTracks: () => tracks } as unknown as MediaStream,
    tracks,
  };
}

/**
 * Replaces navigator.mediaDevices with a fake and decides how getUserMedia behaves.
 * The returned mediaDevices is used to check the getUserMedia calls
 */
function installMediaDevices(
  devices: MediaDeviceInfo[],
  getUserMedia: () => Promise<MediaStream>,
) {
  const listeners = new Set<EventListener>();
  const mediaDevices = {
    enumerateDevices: vi.fn(async () => [...devices]),
    getUserMedia: vi.fn(getUserMedia),
    addEventListener: vi.fn((type: string, listener: EventListener) => {
      if (type === 'devicechange') {
        listeners.add(listener);
      }
    }),
    removeEventListener: vi.fn((type: string, listener: EventListener) => {
      if (type === 'devicechange') {
        listeners.delete(listener);
      }
    }),
  };
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: mediaDevices,
  });
  return mediaDevices;
}

function removeMediaDevices() {
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: undefined,
  });
}

const captureButton = () => screen.getByRole('button', { name: 'Capture' });
const video = () => document.querySelector('video') as HTMLVideoElement;

describe('CameraDialog', () => {
  afterEach(() => {
    removeMediaDevices();
  });

  it('開くと getUserMedia を呼び、映像の要素にストリームを入れる', async () => {
    const { stream } = createFakeStream();
    const mediaDevices = installMediaDevices([videoInput()], () =>
      Promise.resolve(stream),
    );
    render(<CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />);

    await waitFor(() => expect(video().srcObject).toBe(stream));
    expect(mediaDevices.getUserMedia).toHaveBeenCalledWith({
      audio: false,
      video: {
        facingMode: { ideal: 'environment' },
        width: { ideal: 1920 },
        height: { ideal: 1080 },
      },
    });
  });

  it('Cancel を押すと onClose が呼ばれる', async () => {
    const { stream } = createFakeStream();
    installMediaDevices([videoInput()], () => Promise.resolve(stream));
    const onClose = vi.fn();
    render(<CameraDialog open onClose={onClose} onCapture={vi.fn()} />);
    await waitFor(() => expect(video().srcObject).toBe(stream));

    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledOnce();
  });

  it('閉じたら(open を false にして再描画したら)トラックが止まる', async () => {
    const { stream, tracks } = createFakeStream();
    installMediaDevices([videoInput()], () => Promise.resolve(stream));
    const { rerender } = render(
      <CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />,
    );
    await waitFor(() => expect(video().srcObject).toBe(stream));

    rerender(
      <CameraDialog open={false} onClose={vi.fn()} onCapture={vi.fn()} />,
    );

    for (const track of tracks) {
      expect(track.stop).toHaveBeenCalledOnce();
    }
  });

  it('loadeddata の前は Capture を押せない。映像が動いたら押せて、onCapture に映像の要素が渡る。onCapture の Promise が終わるまで押せない', async () => {
    const { stream } = createFakeStream();
    installMediaDevices([videoInput()], () => Promise.resolve(stream));
    // A Promise that never settles stands for "importing"
    const onCapture = vi.fn(() => new Promise<void>(() => {}));
    render(<CameraDialog open onClose={vi.fn()} onCapture={onCapture} />);

    expect(captureButton()).toBeDisabled();

    fireEvent.loadedData(video());
    await waitFor(() => expect(captureButton()).toBeEnabled());

    await userEvent.click(captureButton());
    expect(onCapture).toHaveBeenCalledWith(video());
    expect(captureButton()).toBeDisabled();
  });

  it('getUserMedia が NotAllowedError で reject したら拒否の文を出し、Capture は押せない', async () => {
    installMediaDevices([videoInput()], () =>
      Promise.reject(new DOMException('denied', 'NotAllowedError')),
    );
    render(<CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Camera access was denied. Allow it in the browser settings.',
    );
    expect(captureButton()).toBeDisabled();
  });

  it('videoinput が 2 台なら Switch Camera が出て、押すと前のトラックが止まり、user 向きのカメラが開く', async () => {
    const first = createFakeStream();
    const second = createFakeStream();
    let call = 0;
    const mediaDevices = installMediaDevices(
      [videoInput(), videoInput()],
      () => {
        call += 1;
        return Promise.resolve(call === 1 ? first.stream : second.stream);
      },
    );
    render(<CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />);

    // After permission every camera is listed, so Switch Camera appears
    const switchCamera = await screen.findByRole('button', {
      name: 'Switch Camera',
    });

    await userEvent.click(switchCamera);

    expect(mediaDevices.getUserMedia).toHaveBeenCalledTimes(2);
    expect(mediaDevices.getUserMedia).toHaveBeenLastCalledWith({
      audio: false,
      video: {
        facingMode: { ideal: 'user' },
        width: { ideal: 1920 },
        height: { ideal: 1080 },
      },
    });
    for (const track of first.tracks) {
      expect(track.stop).toHaveBeenCalledOnce();
    }
    await waitFor(() => expect(video().srcObject).toBe(second.stream));
  });

  it('videoinput が 1 台なら Switch Camera は出ない', async () => {
    const { stream } = createFakeStream();
    installMediaDevices([videoInput()], () => Promise.resolve(stream));
    render(<CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />);
    await waitFor(() => expect(video().srcObject).toBe(stream));

    expect(
      screen.queryByRole('button', { name: 'Switch Camera' }),
    ).not.toBeInTheDocument();
  });

  it('開いている間に閉じたあと、遅れて getUserMedia が解決しても、そのトラックは止まる', async () => {
    const { stream, tracks } = createFakeStream();
    let resolveStream: (stream: MediaStream) => void = () => {};
    installMediaDevices(
      [videoInput()],
      () =>
        new Promise<MediaStream>((resolve) => {
          resolveStream = resolve;
        }),
    );
    const { rerender } = render(
      <CameraDialog open onClose={vi.fn()} onCapture={vi.fn()} />,
    );
    rerender(
      <CameraDialog open={false} onClose={vi.fn()} onCapture={vi.fn()} />,
    );

    // Wait for the microtasks until the cleanup in .then (stopCamera) runs
    await act(async () => {
      resolveStream(stream);
    });

    for (const track of tracks) {
      expect(track.stop).toHaveBeenCalledOnce();
    }
  });
});
