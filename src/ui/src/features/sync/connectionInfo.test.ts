import { describe, expect, it } from 'vitest';
import { hashParams, isAllowedIp, parseConnectionInfo } from './connectionInfo';

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
    `#ip=192.168.0.10&port=x&hash=${HASH}&token=t`,
    '#ip=192.168.0.10&port=1&hash=abc&token=t',
    `#ip=192.168.0.10&port=1&hash=${HASH}`,
    // A disallowed ip (a global IPv4 address) is treated like missing connection info
    `#ip=8.8.8.8&port=1&hash=${HASH}&token=t`,
    `#ip=attacker.example&port=1&hash=${HASH}&token=t`,
  ])('足りない・正しくない情報なら null(%s)', (hash) => {
    expect(parseConnectionInfo(hashParams(hash))).toBeNull();
  });
});

describe('isAllowedIp', () => {
  it.each([
    '192.168.0.2',
    '10.1.2.3',
    '172.31.255.255',
    '127.0.0.1',
    '169.254.1.1',
    '100.64.0.1',
    '::1',
    'fd12:3456::1',
    'fe80::1',
  ])('プライベート(LAN)の範囲は許す(%s)', (ip) => {
    expect(isAllowedIp(ip)).toBe(true);
  });

  it.each([
    'attacker.example',
    '8.8.8.8',
    '172.32.0.1',
    '100.128.0.1',
    '192.168.0.256',
    '192.168.01.2',
    '1.2.3',
    '2001:db8::1',
    '',
  ])('ドメイン名・グローバルの IP・書式の誤りは許さない(%s)', (ip) => {
    expect(isAllowedIp(ip)).toBe(false);
  });
});
