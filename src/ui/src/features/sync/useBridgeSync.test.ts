import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BridgeConnection, BridgeHandlers } from './bridgeClient';
import { connectBridge } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';
import type {
  CanvasOp,
  ClientMessage,
  ImagePlacement,
  ServerMessage,
  Viewport,
} from './protocol';
import { RESUME_KEY_STORAGE_KEY } from './resumeKey';
import type { ImageBytes } from './useBridgeSync';
import { useBridgeSync } from './useBridgeSync';

vi.mock('./bridgeClient', () => ({ connectBridge: vi.fn() }));

const info: ConnectionInfo = {
  url: 'https://192.168.0.10:47100/',
  certificateHash: new Uint8Array(32),
  token: 'token',
};

let handlers: BridgeHandlers;
let sent: ClientMessage[];
let sentImageBytes: [string, Uint8Array][];
let requestedImages: string[];
let imageStore: ImageBytes & { seed(id: string, bytes: Uint8Array): void };

beforeEach(() => {
  sent = [];
  sentImageBytes = [];
  requestedImages = [];
  sessionStorage.clear();
  const bytes = new Map<string, Uint8Array>();
  imageStore = {
    has: (id) => bytes.has(id),
    get: (id) => bytes.get(id),
    put: (id, value) => void bytes.set(id, value),
    seed: (id, value) => void bytes.set(id, value),
  };
  vi.mocked(connectBridge).mockImplementation(async (_, h) => {
    handlers = h;
    const connection: BridgeConnection = {
      send: (message) => sent.push(message),
      sendImageBytes: (id, value) => sentImageBytes.push([id, value]),
      requestImageBytes: (id) => requestedImages.push(id),
      close: () => {},
    };
    return connection;
  });
});

const viewport = (x = 0): Viewport => ({ x, y: 0, width: 800, height: 600 });
/** Passes the same object on rerender to show that the area has not changed */
const unchanged = viewport();
const page = { width: 800, height: 600 };

function setup(outgoing: readonly CanvasOp[] = []) {
  const applyRemote = vi.fn();
  const hook = renderHook(
    ({ outgoing, viewport }) =>
      useBridgeSync({
        info,
        viewport,
        page,
        outgoing,
        applyRemote,
        images: imageStore,
      }),
    { initialProps: { outgoing, viewport: unchanged } },
  );
  return { ...hook, applyRemote };
}

const receive = (message: ServerMessage) =>
  act(() => handlers.onMessage(message));

describe('useBridgeSync', () => {
  it('hello を送り、許可されたら同期を始める', async () => {
    const { result, rerender, applyRemote } = setup();
    expect(result.current.status).toBe('connecting');
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({ type: 'hello', token: 'token' });

    await receive({ type: 'awaitingApproval' });
    expect(result.current.status).toBe('waiting');

    await receive({ type: 'snapshot', strokes: [] });
    expect(result.current.status).toBe('connected');
    expect(applyRemote).toHaveBeenCalledWith({ type: 'snapshot', strokes: [] });

    const op: CanvasOp = { type: 'remove', ids: ['a'] };
    rerender({ outgoing: [op], viewport: unchanged });
    expect(sent.at(-1)).toEqual({ type: 'op', op });
  });

  it('見えている範囲が変わったら知らせる', async () => {
    const { rerender } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({
      type: 'hello',
      viewport: unchanged,
      page,
    });
    await receive({ type: 'snapshot', strokes: [] });

    rerender({ outgoing: [], viewport: viewport(120) });

    expect(sent.at(-1)).toEqual({
      type: 'resize',
      viewport: viewport(120),
      page,
    });
  });

  it('許可される前の変更は送らない', async () => {
    const { rerender } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    rerender({
      outgoing: [{ type: 'remove', ids: ['a'] }],
      viewport: unchanged,
    });
    expect(sent).toHaveLength(1);
  });

  it('断られた理由は、その後に通信が閉じても表示に残る', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));

    await receive({ type: 'rejected', reason: '拒否されました' });
    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');
    expect(result.current.message).toBe('拒否されました');
  });

  it('切れた後に繋ぎ直せる', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', strokes: [] });
    expect(result.current.reconnect).toBeNull();

    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');

    act(() => result.current.reconnect?.());
    expect(result.current.status).toBe('connecting');
    expect(result.current.message).toBeNull();
    expect(result.current.reconnect).toBeNull();
    const hellos = () => sent.filter(({ type }) => type === 'hello');
    await waitFor(() => expect(hellos()).toHaveLength(2));
    expect(hellos()[1]).toMatchObject({ token: 'token' });

    await receive({ type: 'snapshot', strokes: [] });
    expect(result.current.status).toBe('connected');
  });

  it('受け取った再接続用の鍵を残し、繋ぎ直すときに hello に添える', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({ type: 'hello', resume: null });

    await receive({ type: 'resumeKey', key: 'key-1' });
    await receive({ type: 'snapshot', strokes: [] });
    expect(sessionStorage.getItem(RESUME_KEY_STORAGE_KEY)).toBe('key-1');

    act(() => handlers.onClose());
    act(() => result.current.reconnect?.());
    const hellos = () => sent.filter(({ type }) => type === 'hello');
    await waitFor(() => expect(hellos()).toHaveLength(2));
    expect(hellos()[1]).toMatchObject({ resume: 'key-1' });
  });

  it('別のページがこのページの鍵で繋ぎ直したら、切れたことを知らせる', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', strokes: [] });

    await receive({ type: 'superseded' });
    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');
    expect(result.current.superseded).toBe(true);
    expect(result.current.message).toMatch(/reconnected to the PC/);

    act(() => result.current.dismissSuperseded());
    expect(result.current.superseded).toBe(false);
    expect(result.current.reconnect).not.toBeNull();
  });

  it('接続情報が無ければ、繋ぎ直せない', () => {
    const { result } = renderHook(() =>
      useBridgeSync({
        info: null,
        viewport: unchanged,
        page,
        outgoing: [],
        applyRemote: vi.fn(),
        images: imageStore,
      }),
    );

    expect(result.current.status).toBe('disconnected');
    expect(result.current.reconnect).toBeNull();
  });

  describe('画像のバイト列(JSON とは別のストリーム)', () => {
    const placement: ImagePlacement = {
      id: 'img',
      x: 1,
      y: 2,
      width: 30,
      height: 20,
    };

    it('自分の setImage は、先にバイト列を一方向ストリームで送る', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [] });
      imageStore.seed('img', new Uint8Array([1, 2, 3]));

      rerender({
        outgoing: [{ type: 'setImage', image: placement }],
        viewport: unchanged,
      });

      expect(sentImageBytes).toEqual([['img', new Uint8Array([1, 2, 3])]]);
      expect(sent.at(-1)).toEqual({
        type: 'op',
        op: { type: 'setImage', image: placement },
      });
    });

    it('同じ画像を動かしても、バイト列は最初の1回だけ送る', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [] });
      imageStore.seed('img', new Uint8Array([1, 2, 3]));
      const moved = { ...placement, x: 50 };

      rerender({
        outgoing: [{ type: 'setImage', image: placement }],
        viewport: unchanged,
      });
      rerender({
        outgoing: [{ type: 'setImage', image: moved }],
        viewport: unchanged,
      });

      expect(sentImageBytes).toHaveLength(1);
      expect(sent.at(-1)).toEqual({
        type: 'op',
        op: { type: 'setImage', image: moved },
      });
    });

    it('bridge から受け取った画像は、動かしてもバイト列を送らない', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [], image: placement });
      act(() => handlers.onImageBytes('img', new Uint8Array([4])));

      rerender({
        outgoing: [{ type: 'setImage', image: { ...placement, x: 50 } }],
        viewport: unchanged,
      });

      expect(sentImageBytes).toEqual([]);
    });

    it('bridge に求められたら、持っているバイト列を送る', async () => {
      setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [] });
      imageStore.seed('img', new Uint8Array([5]));

      act(() => handlers.onImageRequest('img'));
      act(() => handlers.onImageRequest('unknown'));

      expect(sentImageBytes).toEqual([['img', new Uint8Array([5])]]);
    });

    it('bridge の snapshot にあった画像のバイト列を知らなければ、求める', async () => {
      const { applyRemote } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));

      await receive({ type: 'snapshot', strokes: [], image: placement });

      expect(applyRemote).toHaveBeenCalledWith({
        type: 'snapshot',
        strokes: [],
        image: placement,
      });
      expect(requestedImages).toEqual(['img']);
    });

    it('他のページが置いた画像のバイト列を知らなければ、求める', async () => {
      const { applyRemote } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [] });

      await receive({ type: 'op', op: { type: 'setImage', image: placement } });

      expect(applyRemote).toHaveBeenCalledWith({
        type: 'remote',
        op: { type: 'setImage', image: placement },
      });
      expect(requestedImages).toEqual(['img']);
    });

    it('知っている画像は求めず、受け取ったバイト列は保管する', async () => {
      setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({ type: 'snapshot', strokes: [] });
      imageStore.seed('img', new Uint8Array([9]));

      await receive({ type: 'op', op: { type: 'setImage', image: placement } });
      expect(requestedImages).toEqual([]);

      act(() => handlers.onImageBytes('img2', new Uint8Array([7, 8])));
      expect(imageStore.get('img2')).toEqual(new Uint8Array([7, 8]));
    });
  });
});
