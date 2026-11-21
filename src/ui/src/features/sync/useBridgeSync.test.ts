import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { PendingOp } from '../canvas/strokeHistory';
import type { BridgeConnection, BridgeHandlers } from './bridgeClient';
import { connectBridge } from './bridgeClient';
import type { ConnectionInfo } from './connectionInfo';
import {
  type CanvasOp,
  type ClientMessage,
  type ImagePlacement,
  INITIAL_SHEET_ID,
  PROTOCOL_VERSION,
  type ServerMessage,
  type Viewport,
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
const placement: ImagePlacement = {
  id: 'img',
  x: 1,
  y: 2,
  width: 30,
  height: 20,
};
/** Pass the same object on rerender to show that the area did not change */
const unchanged = viewport();
const page = { width: 800, height: 600 };

function setup(pending: readonly PendingOp[] = []) {
  const applyRemote = vi.fn();
  const hook = renderHook(
    ({ pending, viewport }) =>
      useBridgeSync({
        info,
        viewport,
        page,
        pending,
        applyRemote,
        images: imageStore,
      }),
    { initialProps: { pending, viewport: unchanged } },
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
    expect(sent[0]).toMatchObject({
      type: 'hello',
      token: 'token',
      protocol: PROTOCOL_VERSION,
    });

    await receive({ type: 'awaitingApproval' });
    expect(result.current.status).toBe('waiting');

    await receive({
      type: 'snapshot',
      sheets: [],
      current: INITIAL_SHEET_ID,
    });
    expect(result.current.status).toBe('connected');
    expect(applyRemote).toHaveBeenCalledWith({
      type: 'snapshot',
      sheets: [],
      current: INITIAL_SHEET_ID,
    });

    const op: CanvasOp = {
      type: 'remove',
      sheet: INITIAL_SHEET_ID,
      ids: ['a'],
    };
    rerender({ pending: [{ seq: 1, op }], viewport: unchanged });
    expect(sent.at(-1)).toEqual({ type: 'op', op, seq: 1 });
  });

  it('見えている範囲が変わったら知らせる', async () => {
    const { rerender } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({
      type: 'hello',
      viewport: unchanged,
      page,
    });
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    rerender({ pending: [], viewport: viewport(120) });

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
      pending: [
        { seq: 1, op: { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['a'] } },
      ],
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
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });
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

    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });
    expect(result.current.status).toBe('connected');
  });

  it('受け取った再接続用の鍵を残し、繋ぎ直すときに hello に添える', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({ type: 'hello', resume: null });

    await receive({ type: 'resumeKey', key: 'key-1' });
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });
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
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    await receive({ type: 'superseded' });
    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');
    expect(result.current.superseded).toBe(true);
    expect(result.current.message).toMatch(/reconnected to the PC/);

    act(() => result.current.dismissSuperseded());
    expect(result.current.superseded).toBe(false);
    expect(result.current.reconnect).not.toBeNull();
  });

  it('PC 側で切られたら、状態の文に出す', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    await receive({ type: 'disconnected' });
    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');
    expect(result.current.message).toMatch(/Disconnected on the PC/);
    expect(result.current.superseded).toBe(false);
    expect(result.current.reconnect).not.toBeNull();
  });

  it('aiStatus を受け取るまで sendToAi は使えず、AI が待っているときだけ { type: "sendToAi" } を送る', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));

    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });
    expect(result.current.status).toBe('connected');
    expect(result.current.sendToAi).toBeNull();

    await receive({ type: 'aiStatus', waiting: false, queued: false });
    expect(result.current.sendToAi).toBeNull();

    await receive({ type: 'aiStatus', waiting: true, queued: false });
    expect(result.current.sendToAi).not.toBeNull();
    act(() => result.current.sendToAi?.());
    expect(sent.at(-1)).toEqual({ type: 'sendToAi' });
  });

  it('AI が待たなくなったら sendToAi が null に戻る', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    await receive({ type: 'aiStatus', waiting: true, queued: false });
    expect(result.current.sendToAi).not.toBeNull();

    await receive({ type: 'aiStatus', waiting: false, queued: false });
    expect(result.current.sendToAi).toBeNull();
  });

  it('切れたら sendToAi が null になる', async () => {
    const { result } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });
    await receive({ type: 'aiStatus', waiting: true, queued: false });
    expect(result.current.sendToAi).not.toBeNull();

    act(() => handlers.onClose());
    expect(result.current.status).toBe('disconnected');
    expect(result.current.sendToAi).toBeNull();
  });

  it('接続情報が無ければ、繋ぎ直せない', () => {
    const { result } = renderHook(() =>
      useBridgeSync({
        info: null,
        viewport: unchanged,
        page,
        pending: [],
        applyRemote: vi.fn(),
        images: imageStore,
      }),
    );

    expect(result.current.status).toBe('disconnected');
    expect(result.current.reconnect).toBeNull();
  });

  it('ack を受けると applyRemote に渡す', async () => {
    const { applyRemote } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));

    await receive({ type: 'ack', seq: 7 });

    expect(applyRemote).toHaveBeenCalledWith({ type: 'ack', seq: 7 });
  });

  it('切れたら、繋ぎ直すまで送らない', async () => {
    const { result, rerender } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    act(() => handlers.onClose());
    rerender({
      pending: [
        { seq: 1, op: { type: 'remove', sheet: INITIAL_SHEET_ID, ids: ['a'] } },
      ],
      viewport: unchanged,
    });

    expect(sent.filter((message) => message.type === 'op')).toEqual([]);
    expect(result.current.reconnect).not.toBeNull();
  });

  it('繋ぎ直して snapshot を受けると、ack の来ていない操作が同じ seq で送り直される。ack が来た操作は送られない', async () => {
    const { result, rerender } = setup();
    await waitFor(() => expect(sent).toHaveLength(1));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    const op1: CanvasOp = {
      type: 'remove',
      sheet: INITIAL_SHEET_ID,
      ids: ['a'],
    };
    const op2: CanvasOp = {
      type: 'remove',
      sheet: INITIAL_SHEET_ID,
      ids: ['b'],
    };
    rerender({
      pending: [
        { seq: 1, op: op1 },
        { seq: 2, op: op2 },
      ],
      viewport: unchanged,
    });
    expect(sent.filter((message) => message.type === 'op')).toEqual([
      { type: 'op', op: op1, seq: 1 },
      { type: 'op', op: op2, seq: 2 },
    ]);

    // ack 1 arrives and operation 1 is removed from pending. 2 is still there
    await receive({ type: 'ack', seq: 1 });
    rerender({ pending: [{ seq: 2, op: op2 }], viewport: unchanged });
    expect(sent.filter((message) => message.type === 'op')).toHaveLength(2);

    // Disconnect and reconnect
    act(() => handlers.onClose());
    act(() => result.current.reconnect?.());
    const hellos = () => sent.filter(({ type }) => type === 'hello');
    await waitFor(() => expect(hellos()).toHaveLength(2));
    await receive({ type: 'snapshot', sheets: [], current: INITIAL_SHEET_ID });

    // Only operation 2 is resent, with the same seq
    expect(sent.filter((message) => message.type === 'op')).toEqual([
      { type: 'op', op: op1, seq: 1 },
      { type: 'op', op: op2, seq: 2 },
      { type: 'op', op: op2, seq: 2 },
    ]);
  });

  it('置かれている画像のバイト列は、どのシートのものでも求める', async () => {
    setup();
    await waitFor(() => expect(sent).toHaveLength(1));

    await receive({
      type: 'snapshot',
      sheets: [
        { id: INITIAL_SHEET_ID, strokes: [], image: placement },
        {
          id: 's2',
          strokes: [],
          image: { ...placement, id: 'img2' },
        },
      ],
      current: 's2',
    });

    expect(requestedImages).toEqual(['img', 'img2']);
  });

  describe('画像のバイト列(JSON とは別のストリーム)', () => {
    it('自分の setImage は、先にバイト列を一方向ストリームで送る', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [],
        current: INITIAL_SHEET_ID,
      });
      imageStore.seed('img', new Uint8Array([1, 2, 3]));

      rerender({
        pending: [
          {
            seq: 1,
            op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
          },
        ],
        viewport: unchanged,
      });

      expect(sentImageBytes).toEqual([['img', new Uint8Array([1, 2, 3])]]);
      expect(sent.at(-1)).toEqual({
        type: 'op',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
        seq: 1,
      });
    });

    it('同じ画像を動かしても、バイト列は最初の1回だけ送る', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [],
        current: INITIAL_SHEET_ID,
      });
      imageStore.seed('img', new Uint8Array([1, 2, 3]));
      const moved = { ...placement, x: 50 };

      rerender({
        pending: [
          {
            seq: 1,
            op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
          },
        ],
        viewport: unchanged,
      });
      // The move operation is queued after the earlier operation
      rerender({
        pending: [
          {
            seq: 1,
            op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
          },
          {
            seq: 2,
            op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: moved },
          },
        ],
        viewport: unchanged,
      });

      expect(sentImageBytes).toHaveLength(1);
      expect(sent.at(-1)).toEqual({
        type: 'op',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: moved },
        seq: 2,
      });
    });

    it('bridge から受け取った画像は、動かしてもバイト列を送らない', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: placement }],
        current: INITIAL_SHEET_ID,
      });
      act(() => handlers.onImageBytes('img', new Uint8Array([4])));

      rerender({
        pending: [
          {
            seq: 1,
            op: {
              type: 'setImage',
              sheet: INITIAL_SHEET_ID,
              image: { ...placement, x: 50 },
            },
          },
        ],
        viewport: unchanged,
      });

      expect(sentImageBytes).toEqual([]);
    });

    it('bridge に求められたら、持っているバイト列を送る', async () => {
      setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [],
        current: INITIAL_SHEET_ID,
      });
      imageStore.seed('img', new Uint8Array([5]));

      act(() => handlers.onImageRequest('img'));
      act(() => handlers.onImageRequest('unknown'));

      expect(sentImageBytes).toEqual([['img', new Uint8Array([5])]]);
    });

    it('bridge の snapshot にあった画像のバイト列を知らなければ、求める', async () => {
      const { applyRemote } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));

      await receive({
        type: 'snapshot',
        sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: placement }],
        current: INITIAL_SHEET_ID,
      });

      expect(applyRemote).toHaveBeenCalledWith({
        type: 'snapshot',
        sheets: [{ id: INITIAL_SHEET_ID, strokes: [], image: placement }],
        current: INITIAL_SHEET_ID,
      });
      expect(requestedImages).toEqual(['img']);
    });

    it('他のページが置いた画像のバイト列を知らなければ、求める', async () => {
      const { applyRemote } = setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [],
        current: INITIAL_SHEET_ID,
      });

      await receive({
        type: 'op',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
      });

      expect(applyRemote).toHaveBeenCalledWith({
        type: 'remote',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
      });
      expect(requestedImages).toEqual(['img']);
    });

    it('知っている画像は求めず、受け取ったバイト列は保管する', async () => {
      setup();
      await waitFor(() => expect(sent).toHaveLength(1));
      await receive({
        type: 'snapshot',
        sheets: [],
        current: INITIAL_SHEET_ID,
      });
      imageStore.seed('img', new Uint8Array([9]));

      await receive({
        type: 'op',
        op: { type: 'setImage', sheet: INITIAL_SHEET_ID, image: placement },
      });
      expect(requestedImages).toEqual([]);

      act(() => handlers.onImageBytes('img2', new Uint8Array([7, 8])));
      expect(imageStore.get('img2')).toEqual(new Uint8Array([7, 8]));
    });
  });
});
