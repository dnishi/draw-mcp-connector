import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ImageStore } from './imageStore';

describe('ImageStore', () => {
  // Remember each decoded bitmap as it is created, so we can check whether it was closed
  const created: { width: number; height: number; close: () => void }[] = [];

  beforeEach(() => {
    created.length = 0;
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn(async () => {
        const bitmap = { width: 4, height: 2, close: vi.fn() };
        created.push(bitmap);
        return bitmap;
      }),
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  const putAndWait = async (store: ImageStore, id: string) => {
    store.put(id, new Uint8Array([1]));
    await vi.waitFor(() => expect(store.bitmap(id)).toBeDefined());
  };

  it('retain は指定した ID だけ残し、他は中身を捨てて bitmap を close する', async () => {
    const store = new ImageStore();
    await putAndWait(store, 'a');
    await putAndWait(store, 'b');

    store.retain(['b']);

    expect(store.has('a')).toBe(false);
    expect(store.bitmap('a')).toBeUndefined();
    expect(store.natural('a')).toBeUndefined();
    expect(created[0].close).toHaveBeenCalled();
    expect(store.has('b')).toBe(true);
    expect(created[1].close).not.toHaveBeenCalled();
  });

  it('retain を空で呼ぶと全部捨てる(クリアして履歴が空になったとき)', async () => {
    const store = new ImageStore();
    await putAndWait(store, 'a');

    store.retain([]);

    expect(store.has('a')).toBe(false);
    expect(created[0].close).toHaveBeenCalled();
  });

  it('デコード中に捨てた画像は、デコードが終わっても復活せず bitmap を閉じる', async () => {
    const store = new ImageStore();
    store.put('a', new Uint8Array([1]));
    store.retain([]);

    await vi.waitFor(() => expect(created[0].close).toHaveBeenCalled());
    expect(store.bitmap('a')).toBeUndefined();
  });
});
