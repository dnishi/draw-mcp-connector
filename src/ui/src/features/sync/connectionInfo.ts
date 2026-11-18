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
  if (!/^(?:[0-9a-f]{2})+$/i.test(hex)) {
    return null;
  }
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

export function hashParams(hash: string): URLSearchParams {
  return new URLSearchParams(hash.replace(/^#/, ''));
}

/**
 * Limits the address to the IP the bridge puts in the QR code (the IPv4 source address of the default route;
 * lan_ip in src/bridge/src/net.rs).
 *
 * If any host name were allowed, a link like `#ip=attacker.example&…` could make
 * the official site connect to an attacker's server, so the format and range are checked:
 *
 * - IPv4: private/LAN (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16),
 *   this PC (127.0.0.0/8), link-local (169.254.0.0/16),
 *   CGNAT (100.64.0.0/10, used by Tailscale and others)
 * - IPv6: loopback (::1), unique local (fc00::/7), link-local (fe80::/10)
 *
 * Domain names, global IPs and malformed values are false
 */
export function isAllowedIp(ip: string): boolean {
  if (ip.includes(':')) {
    return isAllowedIpv6(ip);
  }
  return isAllowedIpv4(ip);
}

/** An IPv4 address of four decimal octets (0–255, no leading zeros) in an allowed range */
function isAllowedIpv4(ip: string): boolean {
  const parts = ip.split('.');
  if (parts.length !== 4) {
    return false;
  }
  const octets = parts.map((part) => {
    if (!/^(0|[1-9][0-9]{0,2})$/.test(part)) {
      return -1;
    }
    const n = Number(part);
    return n <= 255 ? n : -1;
  });
  if (octets.some((n) => n < 0)) {
    return false;
  }
  const [a, b] = octets;
  return (
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 100 && b >= 64 && b <= 127)
  );
}

/** The format is left to the browser's URL parser; the range is judged by the first 16 bits of the normalized hostname */
function isAllowedIpv6(ip: string): boolean {
  let hostname: string;
  try {
    hostname = new URL(`https://[${ip}]/`).hostname;
  } catch {
    return false;
  }
  const address = hostname.slice(1, -1);
  if (address === '::1') {
    return true;
  }
  const first = address.split(':')[0];
  const top16 = first === '' ? 0 : parseInt(first, 16);
  // fc00::/7 (first 7 bits 1111110), fe80::/10 (first 10 bits 1111111010)
  return (top16 & 0xfe00) === 0xfc00 || (top16 & 0xffc0) === 0xfe80;
}

export function parseConnectionInfo(
  params: URLSearchParams,
): ConnectionInfo | null {
  const ip = params.get('ip');
  const port = Number(params.get('port'));
  const certificateHash = hexToBytes(params.get('hash') ?? '');
  const token = params.get('token');
  // If ip is not in an allowed format and range, treat it like any other missing value
  if (
    !ip ||
    !isAllowedIp(ip) ||
    !Number.isInteger(port) ||
    port <= 0 ||
    !token
  ) {
    return null;
  }
  if (certificateHash?.length !== 32) {
    return null;
  }

  // IPv6 addresses are enclosed in [] in URLs
  const host = ip.includes(':') ? `[${ip}]` : ip;
  return { url: `https://${host}:${port}/`, certificateHash, token };
}
