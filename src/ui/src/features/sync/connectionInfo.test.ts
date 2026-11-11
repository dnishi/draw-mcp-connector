import { describe, expect, it } from 'vitest';
import { hashParams, parseConnectionInfo } from './connectionInfo';

const HASH = 'ab'.repeat(32);

describe('parseConnectionInfo', () => {
  it('URL の # 以降から接続先・証明書のハッシュ・トークンを読む', () => {
    const info = parseConnectionInfo(
      hashParams(`#ip=192.168.0.10&port=47100&hash=${HASH}&token=t0`),
    );
    expect(info?.url).toBe('https://192.168.0.10:47100/');
    expect(info?.certificateHash).toEqual(new Uint8Array(32).fill(0xab));
    expect(info?.token).toBe('t0');
  });

  it('IPv6 のアドレスは [] で囲む', () => {
    const info = parseConnectionInfo(
      hashParams(`#ip=fe80::1&port=1&hash=${HASH}&token=t`),
    );
    expect(info?.url).toBe('https://[fe80::1]:1/');
  });

  it.each([
    '',
    `#port=1&hash=${HASH}&token=t`,
    `#ip=1.2.3.4&port=x&hash=${HASH}&token=t`,
    '#ip=1.2.3.4&port=1&hash=abc&token=t',
    `#ip=1.2.3.4&port=1&hash=${HASH}`,
  ])('足りない・正しくない情報なら null(%s)', (hash) => {
    expect(parseConnectionInfo(hashParams(hash))).toBeNull();
  });
});
