// SPDX-License-Identifier: GPL-3.0-or-later
// GitHub, as the NectarLink Reports app: one repository, Issues only.

export interface GitHub {
  createIssue(title: string, body: string, labels: string[]): Promise<number>;
  /** `open`, `closed`, or `gone` when deleted or moved away. */
  issueState(issue: number): Promise<"open" | "closed" | "gone">;
  createComment(issue: number, body: string): Promise<number>;
  updateComment(comment: number, body: string): Promise<void>;
}

export class GitHubError extends Error {
  constructor(readonly status: number) {
    super(`github ${status}`);
  }
}

const API = "https://api.github.com";

interface Token {
  value: string;
  /** Milliseconds since 1970. */
  expires: number;
}

/** Kept between requests in the same isolate; GitHub's tokens last an hour. */
let cached: Token | undefined;

export class AppClient implements GitHub {
  constructor(
    private readonly appId: string,
    private readonly installation: string,
    private readonly privateKey: string,
    private readonly repo: string,
  ) {}

  async createIssue(title: string, body: string, labels: string[]): Promise<number> {
    const res = await this.call("POST", `/repos/${this.repo}/issues`, { title, body, labels });
    return (res as { number: number }).number;
  }

  async issueState(issue: number): Promise<"open" | "closed" | "gone"> {
    try {
      const res = (await this.call("GET", `/repos/${this.repo}/issues/${issue}`)) as {
        state: string;
        repository_url: string;
      };
      if (!res.repository_url.endsWith(`/repos/${this.repo}`)) return "gone";
      return res.state === "open" ? "open" : "closed";
    } catch (e) {
      if (e instanceof GitHubError && (e.status === 404 || e.status === 410)) return "gone";
      throw e;
    }
  }

  async createComment(issue: number, body: string): Promise<number> {
    const res = await this.call("POST", `/repos/${this.repo}/issues/${issue}/comments`, { body });
    return (res as { id: number }).id;
  }

  async updateComment(comment: number, body: string): Promise<void> {
    await this.call("PATCH", `/repos/${this.repo}/issues/comments/${comment}`, { body });
  }

  private async call(method: string, path: string, body?: unknown): Promise<unknown> {
    const token = await this.token();
    const res = await fetch(API + path, {
      method,
      headers: { ...HEADERS, authorization: `Bearer ${token}` },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) {
      if (res.status === 401) cached = undefined;
      throw new GitHubError(res.status);
    }
    return res.json();
  }

  private async token(): Promise<string> {
    if (cached && cached.expires - Date.now() > 5 * 60_000) return cached.value;
    const jwt = await appJwt(this.appId, this.privateKey, Math.floor(Date.now() / 1000));
    const res = await fetch(`${API}/app/installations/${this.installation}/access_tokens`, {
      method: "POST",
      headers: { ...HEADERS, authorization: `Bearer ${jwt}` },
    });
    if (!res.ok) throw new GitHubError(res.status);
    const json = (await res.json()) as { token: string; expires_at: string };
    cached = { value: json.token, expires: Date.parse(json.expires_at) };
    return cached.value;
  }
}

const HEADERS = {
  accept: "application/vnd.github+json",
  "x-github-api-version": "2022-11-28",
  "user-agent": "nectarlink-reports",
  "content-type": "application/json",
};

/** The short-lived token that proves this is the app (RS256, 9 minutes). */
export async function appJwt(appId: string, pem: string, now: number): Promise<string> {
  const key = await crypto.subtle.importKey(
    "pkcs8",
    pkcs8(pem),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const head = b64url(JSON.stringify({ alg: "RS256", typ: "JWT" }));
  const claims = b64url(JSON.stringify({ iat: now - 60, exp: now + 540, iss: appId }));
  const signed = `${head}.${claims}`;
  const sig = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", key, new TextEncoder().encode(signed));
  return `${signed}.${b64url(new Uint8Array(sig))}`;
}

/**
 * The key as PKCS#8 DER. GitHub hands out PKCS#1 (`BEGIN RSA PRIVATE KEY`),
 * which WebCrypto can't import, so that is wrapped; PKCS#8 passes through.
 */
export function pkcs8(pem: string): Uint8Array {
  const rsa = pem.includes("BEGIN RSA PRIVATE KEY");
  const b64 = pem.replace(/-----[^-]+-----/g, "").replace(/\s+/g, "");
  const der = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  if (!rsa) return der;
  const algorithm = [0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00];
  return seq([...[0x02, 0x01, 0x00], ...algorithm, ...tlv(0x04, der)]);
}

function seq(content: number[]): Uint8Array {
  return Uint8Array.from(tlv(0x30, Uint8Array.from(content)));
}

function tlv(tag: number, content: Uint8Array): number[] {
  const n = content.length;
  const len = n < 0x80 ? [n] : n < 0x100 ? [0x81, n] : n < 0x10000 ? [0x82, n >> 8, n & 0xff] : [0x83, n >> 16, (n >> 8) & 0xff, n & 0xff];
  return [tag, ...len, ...content];
}

function b64url(data: string | Uint8Array): string {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
