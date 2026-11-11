import { useEffect, useState, type RefObject } from 'react';
import { requestGpuDevice, StrokeRenderer } from './StrokeRenderer';

interface StrokeRendererState {
  renderer: StrokeRenderer | null;
  error: string | null;
}

/** Prepares a WebGPU renderer for the canvas. If it is unavailable, error holds the reason */
export function useStrokeRenderer(
  canvasRef: RefObject<HTMLCanvasElement | null>,
): StrokeRendererState {
  const [state, setState] = useState<StrokeRendererState>({
    renderer: null,
    error: null,
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    let disposed = false;
    let renderer: StrokeRenderer | null = null;
    const fail = (error: unknown) => {
      if (disposed) return;
      setState({
        renderer: null,
        error: error instanceof Error ? error.message : String(error),
      });
    };

    requestGpuDevice()
      .then((device) => {
        // If it was already cleaned up (e.g. by StrictMode), do nothing so the canvas configuration is not overwritten
        if (disposed) {
          device.destroy();
          return;
        }
        device.lost.then((info) => {
          if (info.reason !== 'destroyed')
            fail(`WebGPU stopped: ${info.message}`);
        });
        renderer = new StrokeRenderer(canvas, device);
        setState({ renderer, error: null });
      })
      .catch(fail);

    return () => {
      disposed = true;
      renderer?.destroy();
    };
  }, [canvasRef]);

  return state;
}
