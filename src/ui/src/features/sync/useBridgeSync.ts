import { useCallback, useEffect, useRef, useState } from 'react';
import {
  type LegacyCanvasOp,
  type RemoteAction,
} from '../canvas/strokeHistory';
import type { ConnectionStatus } from '../canvas/types';
import { connectBridge, type BridgeConnection } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';
import type { SheetOp } from './canvasOp';
import {
  type CanvasOp,
  type ImagePlacement,
  type PageSize,
  PROTOCOL_VERSION,
  type ServerMessage,
  type Stroke,
  type Viewport,
} from './protocol';
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

const DISCONNECTED_ON_PC_MESSAGE =
  'Disconnected on the PC. Select the status at the top right to reconnect.';

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The snapshot shape of bridges up to v0.3.0 (strokes and image without sheets).
It arrives as JSON, so it cannot be told apart from the ServerMessage type above */
interface LegacySnapshot {
  type: 'snapshot';
  strokes: Stroke[];
  image?: ImagePlacement | null;
}

/** Sheet changes (addSheet / removeSheet / selectSheet / resetSheets) */
const isSheetChange = (op: CanvasOp): op is SheetOp =>
  op.type === 'addSheet' ||
  op.type === 'removeSheet' ||
  op.type === 'selectSheet' ||
  op.type === 'resetSheets';

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
  // Whether the AI is waiting in wait_for_canvas (waiting in the bridge's aiStatus).
  // Old bridges do not send aiStatus, so it stays false and sendToAi, which they cannot read, is not sent
  // (sending it would close the connection)
  const [aiWaiting, setAiWaiting] = useState(false);
  // Another page reconnected with this page's key and this connection was closed (tell the user)
  const [superseded, setSuperseded] = useState(false);
  // Whether syncing with an old bridge (up to v0.3.0). Handles a single sheet and sends no sheet changes
  const [singleSheet, setSingleSheet] = useState(false);
  const legacyBridgeRef = useRef(false);
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
      setAiWaiting(false);
      legacyBridgeRef.current = false;
      setSingleSheet(false);
    };

    const onMessage = (received: ServerMessage) => {
      switch (received.type) {
        case 'awaitingApproval':
          setStatus('waiting');
          return;
        case 'resumeKey':
          saveResumeKey(received.key);
          return;
        case 'snapshot': {
          // Snapshots from old bridges (up to v0.3.0) have no sheets (it arrives as JSON, so
          // the types cannot tell them apart)
          const snapshot = received as
            Extract<ServerMessage, { type: 'snapshot' }> | LegacySnapshot;
          const legacy = !('sheets' in snapshot);
          legacyBridgeRef.current = legacy;
          setSingleSheet(legacy);
          if (legacy) {
            applyRemote({
              type: 'legacySnapshot',
              strokes: snapshot.strokes,
              image: snapshot.image,
            });
            if (snapshot.image && !images.has(snapshot.image.id)) {
              connection?.requestImageBytes(snapshot.image.id);
            }
          } else {
            applyRemote({
              type: 'snapshot',
              sheets: snapshot.sheets,
              current: snapshot.current,
            });
            // Request the bytes of images on any sheet that this page does not have
            for (const sheet of snapshot.sheets) {
              if (sheet.image && !images.has(sheet.image.id)) {
                connection?.requestImageBytes(sheet.image.id);
              }
            }
          }
          // Even on the same connection, the whole canvas is resent after missed messages. Keep the sent record then
          if (syncingRef.current !== connection) {
            sentImagesRef.current.clear();
          }
          syncingRef.current = connection;
          setStatus('connected');
          setMessage(null);
          return;
        }
        case 'aiStatus':
          setAiWaiting(received.waiting);
          return;
        case 'op':
          if (legacyBridgeRef.current) {
            // Changes from old bridges have no sheet. Treat them as changes to the current sheet
            applyRemote({
              type: 'legacyRemote',
              op: received.op as LegacyCanvasOp,
            });
          } else {
            applyRemote({ type: 'remote', op: received.op });
          }
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
        case 'disconnected':
          rejectedReason = DISCONNECTED_ON_PC_MESSAGE;
          disconnect(DISCONNECTED_ON_PC_MESSAGE);
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
          protocol: PROTOCOL_VERSION,
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
    setAiWaiting(false);
    setAttempt((count) => count + 1);
  }, []);
  const dismissSuperseded = useCallback(() => setSuperseded(false), []);
  // Tells the AI that you have finished writing. Sent only while the AI waits in wait_for_canvas.
  // Pressing it while the AI is not waiting would not reach the AI, so it is disabled
  const sendToAi =
    status === 'connected' && aiWaiting
      ? () => {
          syncingRef.current?.send({ type: 'sendToAi' });
        }
      : null;

  // For the first setImage of an image, send the bytes on a separate stream first (so that the bridge can draw
  // the image when the op arrives). For images already sent, moves and resizes send only the placement
  useEffect(() => {
    const connection = syncingRef.current;
    if (!connection) {
      return;
    }
    for (const op of outgoing) {
      // Old bridges cannot read the sheet change types and sending them closes the connection, so they are not sent
      if (legacyBridgeRef.current && isSheetChange(op)) {
        continue;
      }
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
    sendToAi,
    /** Whether syncing with an old bridge (up to v0.3.0). Used to hide the sheet switcher */
    singleSheet,
  };
}
