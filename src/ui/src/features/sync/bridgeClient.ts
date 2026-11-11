import type { ConnectionInfo } from './connectionInfo';
import { LineReader } from './lineReader';
import type { ClientMessage, ServerMessage } from './protocol';

export interface BridgeConnection {
  send(message: ClientMessage): void;
  close(): void;
}

export interface BridgeHandlers {
  onMessage(message: ServerMessage): void;
  /** The connection ended (the peer closed it or the connection dropped). Not called when we close it ourselves */
  onClose(error?: unknown): void;
}

/**
 * Connects to the PC (bridge) over WebTransport and opens one bidirectional stream.
 * The certificate is self-signed, so its hash is checked with serverCertificateHashes.
 * Messages are one JSON object per line (src/bridge/src/protocol.rs)
 */
export async function connectBridge(
  info: ConnectionInfo,
  handlers: BridgeHandlers,
): Promise<BridgeConnection> {
  const transport = new WebTransport(info.url, {
    serverCertificateHashes: [
      { algorithm: 'sha-256', value: info.certificateHash },
    ],
  });
  await transport.ready;
  const stream = await transport.createBidirectionalStream();
  const writer = stream.writable.getWriter();
  const encoder = new TextEncoder();
  let closedByUs = false;

  const close = () => {
    closedByUs = true;
    transport.close();
  };

  const finish = (error?: unknown) => {
    if (!closedByUs) handlers.onClose(error);
  };

  void (async () => {
    const lines = new LineReader();
    const reader = stream.readable
      .pipeThrough(new TextDecoderStream())
      .getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        for (const line of lines.push(value)) {
          handlers.onMessage(JSON.parse(line) as ServerMessage);
        }
      }
      finish();
    } catch (error) {
      finish(error);
    }
  })();

  return {
    send(message) {
      // Writes are processed in order, so queue them without waiting. Failures are reported by the reader ending
      writer
        .write(encoder.encode(`${JSON.stringify(message)}\n`))
        .catch(() => {});
    },
    close,
  };
}
