/**
 * Connection info for the PC (bridge), found after # in the QR code URL
 * (the browser never sends the part after # to the server, so the token never reaches the public site)
 */
export interface ConnectionInfo {
  /** WebTransport endpoint (https://IP:port/) */
  url: string;
  /** SHA-256 of the self-signed certificate (DER) created by the PC */
  certificateHash: Uint8Array<ArrayBuffer>;
  token: string;
}

function hexToBytes(hex: string): Uint8Array<ArrayBuffer> | null {
  if (!/^(?:[0-9a-f]{2})+$/i.test(hex)) return null;
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

export function hashParams(hash: string): URLSearchParams {
  return new URLSearchParams(hash.replace(/^#/, ''));
}

export function parseConnectionInfo(
  params: URLSearchParams,
): ConnectionInfo | null {
  const ip = params.get('ip');
  const port = Number(params.get('port'));
  const certificateHash = hexToBytes(params.get('hash') ?? '');
  const token = params.get('token');
  if (!ip || !Number.isInteger(port) || port <= 0 || !token) return null;
  if (certificateHash?.length !== 32) return null;

  // IPv6 addresses are enclosed in [] in URLs
  const host = ip.includes(':') ? `[${ip}]` : ip;
  return { url: `https://${host}:${port}/`, certificateHash, token };
}
