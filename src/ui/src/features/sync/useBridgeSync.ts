import { useCallback, useEffect, useRef, useState } from 'react';
import type { RemoteAction } from '../canvas/strokeHistory';
import type { ConnectionStatus } from '../canvas/types';
import { connectBridge, type BridgeConnection } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';
import type { CanvasOp, PageSize, ServerMessage, Viewport } from './protocol';
import { readResumeKey, saveResumeKey } from './resumeKey';

interface Options {
  /** Connection info. null if it is not in the URL (no connection) */
  info: ConnectionInfo | null;
  /** Area of the canvas currently visible on this page (sent to the PC whenever it changes) */
  viewport: Viewport;
  /** Size of the canvas (paper) (sent to the PC whenever it changes) */
  page: PageSize;
  /** Changes made by this canvas page (sent to the PC whenever they change) */
  outgoing: readonly CanvasOp[];
  applyRemote: (action: RemoteAction) => void;
}

const SUPERSEDED_MESSAGE =
  'Another canvas page reconnected to the PC in place of this one.';

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Connects to the PC (bridge) and sends canvas changes both ways */
export function useBridgeSync({
  info,
  viewport,
  page,
  outgoing,
  applyRemote,
}: Options) {
  // info is fixed when the page opens and never changes afterwards
  const [status, setStatus] = useState<ConnectionStatus>(
    info ? 'connecting' : 'disconnected',
  );
  const [message, setMessage] = useState<string | null>(null);
  // Another page reconnected with this page's key and this connection was closed (tell the user)
  const [superseded, setSuperseded] = useState(false);
  // Incrementing it reconnects (when the user chooses to after a disconnect)
  const [attempt, setAttempt] = useState(0);
  // Holds only a connection that is approved and syncing
  const syncingRef = useRef<BridgeConnection | null>(null);
  // Used to send the area and paper size at connection time in hello. Changes do not cause a reconnect
  const viewRef = useRef({ viewport, page });
  useEffect(() => {
    viewRef.current = { viewport, page };
  }, [viewport, page]);

  useEffect(() => {
    if (!info) return;
    let cancelled = false;
    let connection: BridgeConnection | null = null;
    // Reason for refusal. Keep showing it even if the connection closes afterwards
    let rejectedReason: string | null = null;

    const disconnect = (text: string) => {
      syncingRef.current = null;
      setStatus('disconnected');
      setMessage(text);
    };

    const onMessage = (received: ServerMessage) => {
      switch (received.type) {
        case 'awaitingApproval':
          setStatus('waiting');
          return;
        case 'resumeKey':
          saveResumeKey(received.key);
          return;
        case 'snapshot':
          applyRemote({ type: 'snapshot', strokes: received.strokes });
          syncingRef.current = connection;
          setStatus('connected');
          setMessage(null);
          return;
        case 'op':
          applyRemote({ type: 'remote', op: received.op });
          return;
        case 'rejected':
          rejectedReason = received.reason;
          disconnect(received.reason);
          return;
        case 'superseded':
          rejectedReason = SUPERSEDED_MESSAGE;
          disconnect(SUPERSEDED_MESSAGE);
          setSuperseded(true);
          return;
      }
    };

    connectBridge(info, {
      onMessage,
      onClose: () =>
        disconnect(
          rejectedReason ??
            'Disconnected from the PC. Select the status at the top right to reconnect.',
        ),
    })
      .then((opened) => {
        connection = opened;
        if (cancelled) {
          opened.close();
          return;
        }
        opened.send({
          type: 'hello',
          token: info.token,
          resume: readResumeKey(),
          ...viewRef.current,
        });
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          disconnect(`Can't connect to the PC: ${errorText(error)}`);
        }
      });

    return () => {
      cancelled = true;
      syncingRef.current = null;
      connection?.close();
    };
  }, [info, applyRemote, attempt]);

  const canReconnect = info !== null && status === 'disconnected';
  const reconnect = useCallback(() => {
    setStatus('connecting');
    setMessage(null);
    setSuperseded(false);
    setAttempt((count) => count + 1);
  }, []);
  const dismissSuperseded = useCallback(() => setSuperseded(false), []);

  useEffect(() => {
    for (const op of outgoing) syncingRef.current?.send({ type: 'op', op });
  }, [outgoing]);

  // Report when the visible area (screen size, zoom, pan) or the paper size changes.
  // It does not change during a pinch, only once when it ends
  useEffect(() => {
    if (status !== 'connected') return;
    syncingRef.current?.send({ type: 'resize', viewport, page });
  }, [viewport, page, status]);

  return {
    status,
    message,
    reconnect: canReconnect ? reconnect : null,
    superseded,
    dismissSuperseded,
  };
}
