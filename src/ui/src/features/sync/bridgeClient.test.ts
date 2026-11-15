import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BridgeHandlers } from './bridgeClient';
import { connectBridge } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';

const info: ConnectionInfo = {
  url: 'https://192.168.0.10:47100/',
  certificateHash: new Uint8Array(32),
  token: 'token',
};

/** Writer of a unidirectional stream. Records what was written and whether it was closed, and can be made to fail */
class FakeWriter {
  chunks: Uint8Array[] = [];
  closed = false;
  fail = false;

  getWriter() {
    return this;
  }
  async write(chunk: Uint8Array) {
    if (this.fail) {
      throw new Error('write failed');
    }
    this.chunks.push(chunk);
  }
  async close() {
    this.closed = true;
  }
}

// Queue requests to open unidirectional streams; the test completes (or fails) them manually
let pendingOpens: {
  resolve: (stream: FakeWriter) => void;
  reject: (error: Error) => void;
}[];
// Receiver for unidirectional streams from the PC. Tests feed them in with pushIncoming
let deliverIncoming: ((stream: ReadableStream<Uint8Array>) => void) | null;
let queuedIncoming: ReadableStream<Uint8Array>[];

class FakeTransport {
  ready = Promise.resolve();
  close() {}
  async createBidirectionalStream() {
    return {
      readable: {
        // The reading side only needs to never finish
        pipeThrough: () => ({
          getReader: () => ({ read: () => new Promise<never>(() => {}) }),
        }),
      },
      writable: {
        getWriter: () => ({ write: () => Promise.resolve() }),
      },
    };
  }
  createUnidirectionalStream() {
    return new Promise<FakeWriter>((resolve, reject) => {
      pendingOpens.push({ resolve, reject });
    });
  }
  incomingUnidirectionalStreams = {
    getReader: () => ({
      read: () =>
        new Promise<{ value: ReadableStream<Uint8Array>; done: false }>(
          (resolve) => {
            const queued = queuedIncoming.shift();
            if (queued) {
              resolve({ value: queued, done: false });
              return;
            }
            deliverIncoming = (value) => {
              deliverIncoming = null;
              resolve({ value, done: false });
            };
          },
        ),
    }),
  };
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();
/** Advances the microtasks and timers that follow sendImageBytes and the like */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

/** A unidirectional stream arrives from the PC asking to send that image */
function pushRequest(id: string) {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(
        encoder.encode(`${JSON.stringify({ type: 'request', id })}\n`),
      );
      controller.close();
    },
  });
  if (deliverIncoming) {
    deliverIncoming(stream);
  } else {
    queuedIncoming.push(stream);
  }
}

let requested: string[];
let handlers: BridgeHandlers;

beforeEach(() => {
  pendingOpens = [];
  deliverIncoming = null;
  queuedIncoming = [];
  requested = [];
  handlers = {
    onMessage: () => {},
    onImageBytes: () => {},
    onImageRequest: (id) => requested.push(id),
    onClose: () => {},
  };
  vi.stubGlobal('WebTransport', FakeTransport);
  // The bidirectional stream's pipeThrough is fake and unused, but this lets new succeed
  vi.stubGlobal('TextDecoderStream', class {});
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('bridgeClient の画像のバイト列', () => {
  it('1 行目にヘッダー、その後にバイト列を載せて閉じる', async () => {
    const connection = await connectBridge(info, handlers);

    connection.sendImageBytes('img', new Uint8Array([1, 2, 3]));
    const stream = new FakeWriter();
    pendingOpens.shift()?.resolve(stream);
    await flush();

    expect(decoder.decode(stream.chunks[0])).toBe(
      '{"type":"provide","id":"img"}\n',
    );
    expect(stream.chunks[1]).toEqual(new Uint8Array([1, 2, 3]));
    expect(stream.closed).toBe(true);
  });

  it('送っていない画像を求められたら、そのまま知らせる', async () => {
    await connectBridge(info, handlers);
    pushRequest('img');
    await flush();
    expect(requested).toEqual(['img']);
  });

  it('送っている途中に求められても、送り終えれば応えない(2 回送らない)', async () => {
    const connection = await connectBridge(info, handlers);

    // The placement reached the PC before the bytes, and the PC asked for the contents
    connection.sendImageBytes('img', new Uint8Array([1]));
    pushRequest('img');
    await flush();
    expect(requested).toEqual([]);

    pendingOpens.shift()?.resolve(new FakeWriter());
    await flush();
    expect(requested).toEqual([]);

    // A request after sending finished is answered
    pushRequest('img');
    await flush();
    expect(requested).toEqual(['img']);
  });

  it('送るのに失敗したら、途中で求められていた画像に応える', async () => {
    const connection = await connectBridge(info, handlers);

    // The stream could not be opened (the PC has not received even the header)
    connection.sendImageBytes('img', new Uint8Array([1]));
    pushRequest('img');
    await flush();
    pendingOpens.shift()?.reject(new Error('ストリームを開けない'));
    await flush();
    expect(requested).toEqual(['img']);

    // It failed while writing
    connection.sendImageBytes('other', new Uint8Array([2]));
    pushRequest('other');
    await flush();
    const stream = new FakeWriter();
    stream.fail = true;
    pendingOpens.shift()?.resolve(stream);
    await flush();
    expect(requested).toEqual(['img', 'other']);
  });

  it('送るのに失敗しても、求められていなければ知らせない', async () => {
    const connection = await connectBridge(info, handlers);

    connection.sendImageBytes('img', new Uint8Array([1]));
    pendingOpens.shift()?.reject(new Error('ストリームを開けない'));
    await flush();
    expect(requested).toEqual([]);
  });
});
