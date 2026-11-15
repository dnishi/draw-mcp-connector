import { useCallback, useEffect, useRef, useState } from 'react';
import type { RemoteAction } from '../canvas/strokeHistory';
import type { ConnectionStatus } from '../canvas/types';
import { connectBridge, type BridgeConnection } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';
import type { CanvasOp, PageSize, ServerMessage, Viewport } from './protocol';
import { readResumeKey, saveResumeKey } from './resumeKey';

/** Store of image bytes (ImageStore in features/canvas) */
export interface ImageBytes {
  has(id: string): boolean;
  get(id: string): Uint8Array | undefined;
  put(id: string, bytes: Uint8Array): void;
}

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
  /**
   * Image bytes (exchanged on streams separate from the JSON).
   * Pass an object that is never discarded once created and keeps its identity (a change causes a reconnect)
   */
  images: ImageBytes;
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
  images,
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
  // IDs of images whose contents were sent on the current connection. Contents are sent only once; moves and resizes send only the placement.
  // Emptied on reconnect (if the bridge restarted, the contents are sent again)
  const sentImagesRef = useRef(new Set<string>());
  // To send the range and paper size at connection time with hello. Does not reconnect when they change
  const viewRef = useRef({ viewport, page });
  useEffect(() => {
    viewRef.current = { viewport, page };
  }, [viewport, page]);

  useEffect(() => {
    if (!info) {
      return;
    }
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
          applyRemote({
            type: 'snapshot',
            strokes: received.strokes,
            image: received.image,
          });
          // Even on the same connection, the whole canvas is resent when changes were missed. Keep the sent record then
          if (syncingRef.current !== connection) {
            sentImagesRef.current.clear();
          }
          syncingRef.current = connection;
          setStatus('connected');
          setMessage(null);
          // If the page does not know the bytes of the bridge's image, receive them on a separate stream
          if (received.image && !images.has(received.image.id)) {
            connection?.requestImageBytes(received.image.id);
          }
          return;
        case 'op':
          applyRemote({ type: 'remote', op: received.op });
          if (
            received.op.type === 'setImage' &&
            !images.has(received.op.image.id)
          ) {
            connection?.requestImageBytes(received.op.image.id);
          }
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
      // Contents that arrived from the bridge are held by the bridge, so they are not resent even when moved
      onImageBytes: (id, bytes) => {
        images.put(id, bytes);
        sentImagesRef.current.add(id);
      },
      // Requested when the bridge does not have the contents (e.g. after the oldest were dropped).
      // Images that failed to send are also routed here by bridgeClient if they were requested
      onImageRequest: (id) => {
        const bytes = images.get(id);
        if (!bytes) {
          return;
        }
        connection?.sendImageBytes(id, bytes);
        sentImagesRef.current.add(id);
      },
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
  }, [info, applyRemote, attempt, images]);

  const canReconnect = info !== null && status === 'disconnected';
  const reconnect = useCallback(() => {
    setStatus('connecting');
    setMessage(null);
    setSuperseded(false);
    setAttempt((count) => count + 1);
  }, []);
  const dismissSuperseded = useCallback(() => setSuperseded(false), []);

  // For the first setImage of an image, send the bytes on a separate stream first (so that the bridge can draw
  // the image when the op arrives). For images already sent, moves and resizes send only the placement
  useEffect(() => {
    const connection = syncingRef.current;
    if (!connection) {
      return;
    }
    for (const op of outgoing) {
      if (op.type === 'setImage' && !sentImagesRef.current.has(op.image.id)) {
        const bytes = images.get(op.image.id);
        if (bytes) {
          connection.sendImageBytes(op.image.id, bytes);
          sentImagesRef.current.add(op.image.id);
        }
      }
      connection.send({ type: 'op', op });
    }
  }, [outgoing, images]);

  // Report when the visible area (screen size, zoom, pan) or the paper size changes.
  // It does not change during a pinch, only once when it ends
  useEffect(() => {
    if (status !== 'connected') {
      return;
    }
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
