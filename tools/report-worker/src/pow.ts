// SPDX-License-Identifier: GPL-3.0-or-later
// Proof of work: the app spends about a second finding a counter, so
// sending reports in bulk costs real compute. No web view, no third party.
//
// The app hashes the exact request body (SHA-256, hex), then finds `n` so
// that SHA-256 of `<body hash>|<t>|<n>` starts with `bits` zero bits, `t`
// being the time in seconds. It sends `X-Proof: <t>.<n>`.

/** How far `t` may be from now, in seconds. */
export const WINDOW = 600;

export async function sha256(data: string | Uint8Array): Promise<Uint8Array> {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}

export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function leadingZeros(bytes: Uint8Array): number {
  let n = 0;
  for (const b of bytes) {
    if (b === 0) {
      n += 8;
      continue;
    }
    return n + Math.clz32(b) - 24;
  }
  return n;
}

/** Whether `header` proves the work for a request whose body hashes to `bodyHash`. */
export async function verify(
  header: string | null,
  bodyHash: string,
  bits: number,
  now: number,
): Promise<boolean> {
  const m = header?.match(/^(\d{1,12})\.(\d{1,12})$/);
  if (!m) return false;
  const t = Number(m[1]);
  if (Math.abs(now - t) > WINDOW) return false;
  const digest = await sha256(`${bodyHash}|${m[1]}|${m[2]}`);
  return leadingZeros(digest) >= bits;
}

/** Finds a proof; what the app does, for tests. */
export async function solve(bodyHash: string, bits: number, t: number): Promise<string> {
  for (let n = 0; ; n++) {
    if (leadingZeros(await sha256(`${bodyHash}|${t}|${n}`)) >= bits) return `${t}.${n}`;
  }
}
