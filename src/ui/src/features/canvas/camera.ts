import { useEffect, useState } from 'react';

/** The camera direction. environment is the back camera (a phone's rear camera), user is the front */
export type FacingMode = 'environment' | 'user';

/**
 * The number of available cameras (videoinput).
 * Before permission, enumerateDevices returns at most one empty entry per kind,
 * so the count is only used to tell whether there is at least one (it is recounted while shooting, after permission)
 */
export async function countCameras(): Promise<number> {
  const mediaDevices = navigator.mediaDevices;
  if (!mediaDevices?.enumerateDevices) {
    return 0;
  }
  try {
    const devices = await mediaDevices.enumerateDevices();
    return devices.filter((device) => device.kind === 'videoinput').length;
  } catch {
    return 0;
  }
}

/**
 * Whether a camera can be used (there is a video input). Starts as false and is decided by counting
 * on mount and again when the set of devices changes (devicechange)
 */
export function useCameraAvailable(): boolean {
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    let mounted = true;
    const update = () => {
      void countCameras().then((count) => {
        // Don't change the state if the result arrives after unmount
        if (mounted) {
          setAvailable(count > 0);
        }
      });
    };
    update();
    const mediaDevices = navigator.mediaDevices;
    mediaDevices?.addEventListener('devicechange', update);
    return () => {
      mounted = false;
      mediaDevices?.removeEventListener('devicechange', update);
    };
  }, []);
  return available;
}

/**
 * Opens the camera and returns the video stream.
 * The constraints are ideal so that PC webcams without facing information
 * don't fail for not meeting them.
 * Imports are shrunk to MAX_IMAGE_SIDE (2000px) on the long side, so ask for 1920×1080, which stays within it
 */
export function openCamera(facingMode: FacingMode): Promise<MediaStream> {
  const mediaDevices = navigator.mediaDevices;
  if (!mediaDevices) {
    return Promise.reject(
      new DOMException('Media devices are not available.', 'NotFoundError'),
    );
  }
  return mediaDevices.getUserMedia({
    audio: false,
    video: {
      facingMode: { ideal: facingMode },
      width: { ideal: 1920 },
      height: { ideal: 1080 },
    },
  });
}

/** Stops every track of the stream (to turn off the camera light) */
export function stopCamera(stream: MediaStream): void {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}

/** The message shown in the dialog when the camera could not be opened */
export function cameraErrorMessage(error: unknown): string {
  const name = error instanceof DOMException ? error.name : '';
  switch (name) {
    case 'NotFoundError':
    case 'OverconstrainedError':
      return 'No camera was found.';
    case 'NotAllowedError':
      return 'Camera access was denied. Allow it in the browser settings.';
    case 'NotReadableError':
      return 'The camera is being used by another app.';
    default:
      return 'Could not start the camera.';
  }
}
