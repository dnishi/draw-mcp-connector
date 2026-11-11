import { useCallback, useState } from 'react';

interface FingerDrawingState {
  enabled: boolean;
  /** Whether the user decided in the toolbar. Once decided, using a pen does not switch it automatically */
  chosen: boolean;
}

/**
 * Whether one finger draws. Fingers draw at first so devices without a pen can draw.
 * Once a pen is used, finger drawing is turned off so a palm or finger touching the screen does not draw lines.
 * If the user decides in the toolbar, that choice is followed
 */
export function useFingerDrawing() {
  const [state, setState] = useState<FingerDrawingState>({
    enabled: true,
    chosen: false,
  });

  const onPenDetected = useCallback(() => {
    setState((current) =>
      current.chosen || !current.enabled
        ? current
        : { enabled: false, chosen: false },
    );
  }, []);

  const setEnabled = useCallback((enabled: boolean) => {
    setState({ enabled, chosen: true });
  }, []);

  return { enabled: state.enabled, setEnabled, onPenDetected };
}
