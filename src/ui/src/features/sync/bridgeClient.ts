import type { ConnectionInfo } from './connectionInfo';
import { LineReader } from './lineReader';
import type { ClientMessage, ImageStream, ServerMessage } from './protocol';

export interface BridgeConnection {
  send(message: ClientMessage): void;
  /** Sends image bytes on a separate (unidirectional) stream */
  sendImageBytes(id: string, bytes: Uint8Array): void;
  /** Asks the PC for the bytes of an image it does not have */
  requestImageBytes(id: string): void;
  close(): void;
}

export interface BridgeHandlers {
  onMessage(message: ServerMessage): void;
  /** Image bytes received from the PC (they arrive on a separate stream) */
  onImageBytes(id: string, bytes: Uint8Array): void;
  /**
   * The PC does not have the bytes of that image and wants them sent.
   * Requests for an image still being sent are not reported; they are reported only if sending fails
   */
  onImageRequest(id: string): void;
  /** The connection ended (the peer closed it or the connection dropped). Not called when we close it ourselves */
  onClose(error?: unknown): void;
}

const NEWLINE = 0x0a;

function concat(parts: readonly Uint8Array[]): Uint8Array<ArrayBuffer> {
  const total = parts.reduce((sum, part) => sum + part.byteLength, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

/**
 * Connects to the PC (bridge) over WebTransport and opens one bidirectional stream.
 * The certificate is self-signed, so its hash is checked with serverCertificateHashes.
 * Messages are one JSON per line (src/bridge/src/protocol.rs).
 * Only image bytes would grow about 1.33 times in JSON, so a one-to-one unidirectional stream is
 * opened each time, with a header (ImageStream) on the first line and, for provide, the raw PNG after it
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
  // IDs of images whose bytes are being sent now, and whether the PC asked for them meanwhile.
  // The placement (setImage) often reaches the PC before the bytes, and the PC then asks for the contents.
  // Answering as is would send the same image twice, so it is not answered once sending finishes, only when it fails
  const sendingImages = new Map<string, boolean>();

  const close = () => {
    closedByUs = true;
    transport.close();
  };

  const finish = (error?: unknown) => {
    if (!closedByUs) {
      handlers.onClose(error);
    }
  };

  void (async () => {
    const lines = new LineReader();
    const reader = stream.readable
      .pipeThrough(new TextDecoderStream())
      .getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) {
          break;
        }
        for (const line of lines.push(value)) {
          handlers.onMessage(JSON.parse(line) as ServerMessage);
        }
      }
      finish();
    } catch (error) {
      finish(error);
    }
  })();

  // Receives a unidirectional stream with a header on the first line and, for provide, the raw bytes after it
  const receiveImageStream = async (
    imageStream: ReadableStream<Uint8Array>,
  ) => {
    const reader = imageStream.getReader();
    // Find the header on the first line (up to \n)
    let buffered = new Uint8Array(0);
    let headerEnd = -1;
    while (headerEnd < 0) {
      const { value, done } = await reader.read();
      if (done) {
        return;
      }
      buffered = concat([buffered, value]);
      headerEnd = buffered.indexOf(NEWLINE);
    }
    const header = JSON.parse(
      new TextDecoder().decode(buffered.subarray(0, headerEnd)),
    ) as ImageStream;
    if (header.type === 'request') {
      // If it is still being sent, arrival is enough, so do not answer (answer if sending fails)
      if (sendingImages.has(header.id)) {
        sendingImages.set(header.id, true);
      } else {
        handlers.onImageRequest(header.id);
      }
      return;
    }
    const parts: Uint8Array[] = [];
    if (buffered.byteLength > headerEnd + 1) {
      parts.push(buffered.subarray(headerEnd + 1));
    }
    for (;;) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      parts.push(value);
    }
    handlers.onImageBytes(header.id, concat(parts));
  };

  // Keep receiving image bytes from the PC
  void (async () => {
    const streams = transport.incomingUnidirectionalStreams.getReader();
    try {
      for (;;) {
        const { value, done } = await streams.read();
        if (done) {
          return;
        }
        void receiveImageStream(value).catch(() => {});
      }
    } catch {
      // When the whole connection drops, the bidirectional stream side reports it
    }
  })();

  const openImageStream = async (header: ImageStream, body?: Uint8Array) => {
    const writable = (await transport.createUnidirectionalStream()).getWriter();
    await writable.write(encoder.encode(`${JSON.stringify(header)}\n`));
    if (body) {
      await writable.write(body as Uint8Array<ArrayBuffer>);
    }
    await writable.close();
  };

  return {
    send(message) {
      // Writes are processed in order, so queue them without waiting. Failures are reported by the reader ending
      writer
        .write(encoder.encode(`${JSON.stringify(message)}\n`))
        .catch(() => {});
    },
    sendImageBytes(id, bytes) {
      sendingImages.set(id, false);
      openImageStream({ type: 'provide', id }, bytes).then(
        () => {
          sendingImages.delete(id);
        },
        () => {
          const asked = sendingImages.get(id) === true;
          sendingImages.delete(id);
          if (asked) {
            handlers.onImageRequest(id);
          }
        },
      );
    },
    requestImageBytes(id) {
      openImageStream({ type: 'request', id }).catch(() => {});
    },
    close,
  };
}
