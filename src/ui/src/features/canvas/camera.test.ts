import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  cameraErrorMessage,
  openCamera,
  stopCamera,
  useCameraAvailable,
} from './camera';

/** jsdom has no navigator.mediaDevices, so the tests put a mock */
function installMediaDevices(value: unknown) {
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value,
  });
}

function removeMediaDevices() {
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: undefined,
  });
}

/** A mock camera track */
function createMockTrack() {
  return { stop: vi.fn() };
}

/** A mock MediaStream (getTracks returns the same tracks every time) */
function createMockStream() {
  const tracks = [createMockTrack(), createMockTrack()];
  return {
    stream: { getTracks: () => tracks } as unknown as MediaStream,
    tracks,
  };
}

/** A mock MediaDevices that can fire devicechange (devices are read on every call) */
function createMockMediaDevices(devices: MediaDeviceInfo[]) {
  const listeners = new Set<EventListener>();
  const mediaDevices = {
    enumerateDevices: vi.fn(async () => [...devices]),
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
  return {
    mediaDevices,
    fireDeviceChange: () => {
      for (const listener of [...listeners]) {
        listener(new Event('devicechange'));
      }
    },
  };
}

const videoInput = () => ({ kind: 'videoinput' }) as MediaDeviceInfo;

describe('useCameraAvailable', () => {
  afterEach(() => {
    removeMediaDevices();
  });

  it('navigator.mediaDevices が無ければ false', async () => {
    const { result } = renderHook(() => useCameraAvailable());

    await act(async () => {});

    expect(result.current).toBe(false);
  });

  it('videoinput があるなら true', async () => {
    installMediaDevices(createMockMediaDevices([videoInput()]).mediaDevices);

    const { result } = renderHook(() => useCameraAvailable());

    await waitFor(() => expect(result.current).toBe(true));
  });

  it('audioinput だけなら false', async () => {
    installMediaDevices(
      createMockMediaDevices([{ kind: 'audioinput' } as MediaDeviceInfo])
        .mediaDevices,
    );

    const { result } = renderHook(() => useCameraAvailable());

    await act(async () => {});
    expect(result.current).toBe(false);
  });

  it('enumerateDevices が reject したら false', async () => {
    const mediaDevices = createMockMediaDevices([videoInput()]).mediaDevices;
    vi.mocked(mediaDevices.enumerateDevices).mockRejectedValue(
      new Error('denied'),
    );
    installMediaDevices(mediaDevices);

    const { result } = renderHook(() => useCameraAvailable());

    await act(async () => {});
    expect(result.current).toBe(false);
  });

  it('devicechange で数え直して変わる', async () => {
    const devices = [videoInput()];
    const { mediaDevices, fireDeviceChange } = createMockMediaDevices(devices);
    installMediaDevices(mediaDevices);

    const { result } = renderHook(() => useCameraAvailable());
    await waitFor(() => expect(result.current).toBe(true));

    devices.length = 0;
    fireDeviceChange();
    await waitFor(() => expect(result.current).toBe(false));

    devices.push(videoInput());
    fireDeviceChange();
    await waitFor(() => expect(result.current).toBe(true));
  });
});

describe('openCamera', () => {
  afterEach(() => {
    removeMediaDevices();
  });

  it('getUserMedia に、制約(audio: false、facingMode と幅・高さは ideal)を渡す', async () => {
    const { stream } = createMockStream();
    const getUserMedia = vi.fn(async () => stream);
    installMediaDevices({ getUserMedia });

    const opened = await openCamera('user');

    expect(opened).toBe(stream);
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: false,
      video: {
        facingMode: { ideal: 'user' },
        width: { ideal: 1920 },
        height: { ideal: 1080 },
      },
    });
  });

  it('navigator.mediaDevices が無ければ NotFoundError で reject する', async () => {
    await expect(openCamera('environment')).rejects.toMatchObject({
      name: 'NotFoundError',
    });
  });
});

describe('stopCamera', () => {
  it('すべてのトラックの stop を呼ぶ', () => {
    const { stream, tracks } = createMockStream();

    stopCamera(stream);

    for (const track of tracks) {
      expect(track.stop).toHaveBeenCalledOnce();
    }
  });
});

describe('cameraErrorMessage', () => {
  it.each([
    [
      'NotAllowedError',
      'Camera access was denied. Allow it in the browser settings.',
    ],
    ['NotFoundError', 'No camera was found.'],
    ['OverconstrainedError', 'No camera was found.'],
    ['NotReadableError', 'The camera is being used by another app.'],
  ])('%s → %s', (name, message) => {
    expect(cameraErrorMessage(new DOMException('failed', name))).toBe(message);
  });

  it('それ以外は共通の文章', () => {
    expect(cameraErrorMessage(new Error('boom'))).toBe(
      'Could not start the camera.',
    );
    expect(cameraErrorMessage('boom')).toBe('Could not start the camera.');
    expect(cameraErrorMessage(null)).toBe('Could not start the camera.');
  });
});

// countCameras itself is tested through the useCameraAvailable tests
