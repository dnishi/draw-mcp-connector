import { Dialog } from '@base-ui/react/dialog';
import { Camera, SwitchCamera } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { DIALOG_BACKDROP_CLASS, DIALOG_POPUP_CLASS } from '@/lib/dialogStyles';
import { cn } from '@/lib/utils';
import { useEffect, useRef, useState } from 'react';
import {
  cameraErrorMessage,
  countCameras,
  openCamera,
  stopCamera,
  type FacingMode,
} from './camera';

interface CameraDialogProps {
  open: boolean;
  onClose: () => void;
  /** When the shutter is pressed. Passes the video element. The shutter is disabled until the import finishes (the Promise settles) */
  onCapture: (video: HTMLVideoElement) => Promise<void>;
}

/**
 * A dialog that shows the camera feed and takes a photo.
 * The camera runs only while open; its tracks are stopped on close, on switching cameras and on unmount
 */
export function CameraDialog({ open, onClose, onCapture }: CameraDialogProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  /** Starts with the back camera. PCs without one switch to the front camera */
  const [facingMode, setFacingMode] = useState<FacingMode>('environment');
  /** The current video stream. When null, the video element shows nothing */
  const [stream, setStream] = useState<MediaStream | null>(null);
  /** The message shown when the camera could not be opened. The shutter is disabled while it is shown */
  const [error, setError] = useState<string | null>(null);
  /** Whether the video has started (loadeddata) */
  const [ready, setReady] = useState(false);
  /** Whether the import (the onCapture Promise) is pending. The shutter is disabled until it finishes */
  const [capturing, setCapturing] = useState(false);
  /** The number of cameras. Known in full after permission. Shows "Switch Camera" when there are two or more */
  const [cameraCount, setCameraCount] = useState(0);

  // Reset the state when closed (adjusted during render, as in CanvasPage)
  const [prevOpen, setPrevOpen] = useState(open);
  if (open !== prevOpen) {
    setPrevOpen(open);
    if (!open) {
      setStream(null);
      setError(null);
      setReady(false);
      setCapturing(false);
      setCameraCount(0);
    }
  }

  useEffect(() => {
    if (!open) {
      return;
    }
    let disposed = false;
    // The stream this effect opened (stopped in the cleanup; kept apart from the stream state)
    let ownStream: MediaStream | null = null;
    void openCamera(facingMode)
      .then((opened) => {
        ownStream = opened;
        if (disposed) {
          // Closed or switched cameras before it opened (the effect cleanup ran first), so stop the stream right away
          stopCamera(opened);
          return;
        }
        setStream(opened);
        void countCameras().then(setCameraCount);
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setError(cameraErrorMessage(cause));
        }
      });
    return () => {
      disposed = true;
      if (ownStream) {
        stopCamera(ownStream);
      }
    };
  }, [open, facingMode]);

  // React doesn't take srcObject as a prop, so set it with a ref and an effect
  useEffect(() => {
    const video = videoRef.current;
    if (video) {
      video.srcObject = stream;
    }
  }, [stream]);

  const onSwitchCamera = () => {
    // Disable the shutter until the new stream starts (until loadeddata)
    setReady(false);
    setCameraCount(0);
    setFacingMode((mode) => (mode === 'environment' ? 'user' : 'environment'));
  };

  const onCapturePressed = async () => {
    const video = videoRef.current;
    if (!video || error || !ready || capturing) {
      return;
    }
    setCapturing(true);
    try {
      await onCapture(video);
    } finally {
      setCapturing(false);
    }
  };

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          onClose();
        }
      }}
    >
      <Dialog.Portal>
        <Dialog.Backdrop className={DIALOG_BACKDROP_CLASS} />
        <Dialog.Popup
          className={cn(
            DIALOG_POPUP_CLASS,
            'flex w-[min(92vw,48rem)] flex-col',
          )}
        >
          <Dialog.Title className="text-base font-medium">
            Take Photo
          </Dialog.Title>
          {/* iOS needs playsInline and muted to play the video inside the page */}
          <video
            ref={videoRef}
            autoPlay
            playsInline
            muted
            onLoadedData={() => setReady(true)}
            className="mt-3 max-h-[60svh] w-full object-contain"
          />
          {error && (
            <p role="alert" className="mt-3 text-sm text-destructive">
              {error}
            </p>
          )}
          <div className="mt-4 flex items-center justify-end gap-2">
            {cameraCount >= 2 && (
              <Button variant="outline" onClick={onSwitchCamera}>
                <SwitchCamera />
                Switch Camera
              </Button>
            )}
            <Dialog.Close render={<Button variant="outline" />}>
              Cancel
            </Dialog.Close>
            <Button
              disabled={!ready || error !== null || capturing}
              onClick={() => void onCapturePressed()}
            >
              <Camera />
              Capture
            </Button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
