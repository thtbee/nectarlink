// SPDX-License-Identifier: GPL-3.0-or-later
import { env } from "cloudflare:test";
import { beforeEach, describe, expect, it } from "vitest";

import { GitHubError, type GitHub, appJwt, pkcs8 } from "../src/github";
import { handle, sweep } from "../src/index";
import { hex, leadingZeros, sha256, solve, verify } from "../src/pow";
import { issueBody, merge, parse, title, titleText } from "../src/report";

const NOW = 1_795_000_000;
const BITS = Number(env.POW_BITS);

class FakeGitHub implements GitHub {
  issues: { number: number; title: string; body: string; labels: string[]; state: "open" | "closed" }[] = [];
  comments = new Map<number, { issue: number; body: string }>();
  down = false;

  async createIssue(title: string, body: string, labels: string[]): Promise<number> {
    if (this.down) throw new GitHubError(503);
    const number = 100 + this.issues.length;
    this.issues.push({ number, title, body, labels, state: "open" });
    return number;
  }
  async issueState(issue: number): Promise<"open" | "closed" | "gone"> {
    if (this.down) throw new GitHubError(503);
    return this.issues.find((i) => i.number === issue)?.state ?? "gone";
  }
  async createComment(issue: number, body: string): Promise<number> {
    if (this.down) throw new GitHubError(503);
    const id = 9000 + this.comments.size;
    this.comments.set(id, { issue, body });
    return id;
  }
  async updateComment(comment: number, body: string): Promise<void> {
    if (this.down) throw new GitHubError(503);
    const c = this.comments.get(comment);
    if (!c) throw new GitHubError(404);
    c.body = body;
  }
}

let rids = 0;
function report(over: Record<string, unknown> = {}): Record<string, unknown> {
  rids += 1;
  return {
    schema: 1,
    rid: `rid${String(rids).padStart(16, "0")}`,
    kind: "person",
    area: "connection",
    summary: "The phone drops off every few minutes",
    versions: ["0.1.6"],
    platforms: ["windows", "android"],
    body: `NectarLink problem report (format 1, id ${rids})\nArea: Connection\n`,
    ...over,
  };
}

async function post(
  github: GitHub,
  value: unknown,
  opts: { ip?: string; proof?: string | null; now?: number } = {},
): Promise<Response> {
  const text = JSON.stringify(value);
  const now = opts.now ?? NOW;
  const proof =
    opts.proof === undefined ? await solve(hex(await sha256(text)), BITS, now) : opts.proof;
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "cf-connecting-ip": opts.ip ?? "198.51.100.7",
  };
  if (proof !== null) headers["x-proof"] = proof;
  return handle(new Request("https://r.example/v1/report", { method: "POST", headers, body: text }), env, github, now);
}

async function get(github: GitHub, rid: string): Promise<Response> {
  return handle(new Request(`https://r.example/v1/report/${rid}`), env, github, NOW);
}

beforeEach(async () => {
  for (const t of ["reports", "sigs", "queue", "salts", "senders", "days"]) {
    await env.DB.prepare(`DELETE FROM ${t}`).run();
  }
});

describe("proof of work", () => {
  it("counts leading zero bits", () => {
    expect(leadingZeros(Uint8Array.from([0, 0, 0x0f]))).toBe(20);
    expect(leadingZeros(Uint8Array.from([0x80]))).toBe(0);
    expect(leadingZeros(Uint8Array.from([0, 1]))).toBe(15);
  });

  it("is bound to the body and the time", async () => {
    const proof = await solve("aa", BITS, NOW);
    expect(await verify(proof, "aa", BITS, NOW)).toBe(true);
    expect(await verify(proof, "aa", BITS, NOW + 601)).toBe(false);
    expect(await verify("12.x", "aa", BITS, NOW)).toBe(false);
    expect(await verify(null, "aa", BITS, NOW)).toBe(false);
  });

  it("matches the app's test vector", async () => {
    // core/nectarlink-core/src/diag/send.rs, proof_matches_the_worker
    expect(hex(await sha256("abc|1795000000|0"))).toBe(
      "096f166aa433a9b7c070d18c9b70d8727ffbcf0d9f0b81f4ae94221aeefe2a89",
    );
  });

  it("is checked before anything is stored or sent", async () => {
    const github = new FakeGitHub();
    const value = report();
    const hash = hex(await sha256(JSON.stringify(value)));
    let wrong = 0;
    while (await verify(`${NOW}.${wrong}`, hash, BITS, NOW)) wrong++;
    for (const proof of [null, `${NOW}.${wrong}`, `${NOW - 601}.0`, "x"]) {
      expect((await post(github, value, { proof })).status).toBe(403);
    }
    expect(github.issues).toHaveLength(0);
    const stored = await env.DB.prepare("SELECT COUNT(*) AS n FROM senders").first<{ n: number }>();
    expect(stored?.n).toBe(0);
  });
});

describe("reports", () => {
  it("files a new issue with a fixed title, labels and a fenced body", async () => {
    const github = new FakeGitHub();
    const res = await post(github, report());
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ issue: 100, url: "https://github.com/thtbee/nectarlink/issues/100" });
    const [issue] = github.issues;
    expect(issue.title).toBe("[Connection] The phone drops off every few minutes");
    expect(issue.labels).toEqual(["from-app", "needs-triage", "area:connection"]);
    expect(issue.body).toContain("```text\nNectarLink problem report");
  });

  it("rejects what doesn't match the schema", async () => {
    const github = new FakeGitHub();
    for (const bad of [
      { area: "everything" },
      { schema: 9 },
      { rid: "UPPER-case-rid-0000" },
      { versions: ["1.0"] },
      { platforms: ["amiga"] },
      { kind: "crash" },
      { sig: "zzzzzzzzzzzz" },
      { body: "" },
      { body: "x".repeat(61 * 1024) },
    ]) {
      const res = await post(github, report(bad));
      expect(res.status, Object.keys(bad).join()).toBe(400);
    }
    expect(github.issues).toHaveLength(0);
  });

  it("rejects bodies that are too big or not JSON", async () => {
    const github = new FakeGitHub();
    const big = await handle(
      new Request("https://r.example/v1/report", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "x".repeat(65 * 1024),
      }),
      env,
      github,
      NOW,
    );
    expect(big.status).toBe(413);
    const text = await handle(
      new Request("https://r.example/v1/report", { method: "POST", body: "{}" }),
      env,
      github,
      NOW,
    );
    expect(text.status).toBe(415);
  });

  it("files a resend once", async () => {
    const github = new FakeGitHub();
    const value = report();
    expect((await post(github, value)).status).toBe(200);
    const again = await post(github, value);
    expect(await again.json()).toMatchObject({ issue: 100 });
    expect(github.issues).toHaveLength(1);
  });

  it("limits each sender per day without keeping the address", async () => {
    const github = new FakeGitHub();
    for (let i = 0; i < 5; i++) expect((await post(github, report())).status).toBe(200);
    expect((await post(github, report())).status).toBe(429);
    expect((await post(github, report(), { ip: "203.0.113.9" })).status).toBe(200);
    const rows = await env.DB.prepare("SELECT hash FROM senders").all<{ hash: string }>();
    for (const row of rows.results) expect(row.hash).toMatch(/^[0-9a-f]{64}$/);
    const dump = JSON.stringify((await env.DB.prepare("SELECT * FROM senders").all()).results);
    expect(dump).not.toContain("198.51.100.7");
  });

  it("waits in the queue past the day's limit", async () => {
    const github = new FakeGitHub();
    const limited = { ...env, DAILY_ISSUES: "1" };
    const send = async (v: unknown, ip: string) => {
      const text = JSON.stringify(v);
      const proof = await solve(hex(await sha256(text)), BITS, NOW);
      return handle(
        new Request("https://r.example/v1/report", {
          method: "POST",
          headers: { "content-type": "application/json", "x-proof": proof, "cf-connecting-ip": ip },
          body: text,
        }),
        limited,
        github,
        NOW,
      );
    };
    expect((await send(report(), "a")).status).toBe(200);
    const second = report();
    const res = await send(second, "b");
    expect(res.status).toBe(202);
    expect((await get(github, second.rid as string)).status).toBe(202);
    await sweep(limited, github, NOW + 86_400);
    expect(github.issues).toHaveLength(2);
    expect(await (await get(github, second.rid as string)).json()).toMatchObject({ issue: 101 });
  });
});

describe("crashes", () => {
  it("counts a known crash on its open issue with one tally comment", async () => {
    const github = new FakeGitHub();
    const crash = (v: string, p: string) =>
      report({ kind: "crash", sig: "b3af6c6e31db", summary: "", versions: [v], platforms: [p] });
    expect(await (await post(github, crash("0.1.6", "windows"))).json()).toMatchObject({ issue: 100 });
    expect(github.issues[0].title).toBe("[Connection] Crash b3af6c6e31db");
    expect(github.issues[0].labels).toContain("crash");
    expect(await (await post(github, crash("0.1.6", "android"))).json()).toMatchObject({ issue: 100 });
    expect(await (await post(github, crash("0.1.7", "android"))).json()).toMatchObject({ issue: 100 });
    expect(github.issues).toHaveLength(1);
    expect(github.comments.size).toBe(1);
    const tally = [...github.comments.values()][0].body;
    expect(tally).toContain("**3** times");
    expect(tally).toContain("0.1.6, 0.1.7");
    expect(tally).toContain("windows, android");
  });

  it("opens a new issue once the old one is closed", async () => {
    const github = new FakeGitHub();
    const crash = () => report({ kind: "crash", sig: "b3af6c6e31db", summary: "" });
    await post(github, crash());
    github.issues[0].state = "closed";
    expect(await (await post(github, crash())).json()).toMatchObject({ issue: 101 });
  });

  it("gives a person's report its own issue even with a known crash", async () => {
    const github = new FakeGitHub();
    await post(github, report({ kind: "crash", sig: "b3af6c6e31db", summary: "" }));
    const res = await post(github, report({ sig: "b3af6c6e31db" }));
    expect(await res.json()).toMatchObject({ issue: 101 });
  });
});

describe("GitHub failing", () => {
  it("queues the report and files it on the next sweep", async () => {
    const github = new FakeGitHub();
    github.down = true;
    const value = report();
    const res = await post(github, value);
    expect(res.status).toBe(202);
    expect(await res.json()).toEqual({ queued: true });
    expect((await post(github, value)).status).toBe(202);
    await sweep(env, github, NOW + 900);
    expect(github.issues).toHaveLength(0);
    github.down = false;
    await sweep(env, github, NOW + 1800);
    expect(github.issues).toHaveLength(1);
    expect((await get(github, value.rid as string)).status).toBe(200);
    const left = await env.DB.prepare("SELECT COUNT(*) AS n FROM queue").first<{ n: number }>();
    expect(left?.n).toBe(0);
  });

  it("drops what waited more than a week", async () => {
    const github = new FakeGitHub();
    github.down = true;
    const value = report();
    await post(github, value);
    await sweep(env, github, NOW + 8 * 86_400);
    expect((await get(github, value.rid as string)).status).toBe(404);
  });
});

describe("rendering", () => {
  it("fences the report longer than any backticks in it", () => {
    const r = parse(report({ body: "a ```` b\n```\n@someone #12 <b>x</b>" }));
    if (typeof r === "string") throw new Error(r);
    const body = issueBody(r);
    expect(body).toContain("`````text\n");
    expect(body.trimEnd().split("\n").at(-3)).toBe("`````");
  });

  it("keeps titles to plain short text", () => {
    expect(titleText("hi‮ @team #1 `x`\n<b>", 60)).toBe("hi team 1 x b");
    expect(titleText("word ".repeat(30), 20)).toBe("word word word word...");
    const r = parse(report({ summary: "" }));
    if (typeof r === "string") throw new Error(r);
    expect(title(r)).toBe("[Connection] Problem report");
  });

  it("merges lists newest last without repeats", () => {
    expect(merge(["a", "b"], ["b", "c"])).toEqual(["a", "b", "c"]);
  });
});

describe("the GitHub app key", () => {
  it("signs a token with a PKCS#1 key as GitHub hands it out", async () => {
    const pair = (await crypto.subtle.generateKey(
      { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
      true,
      ["sign", "verify"],
    )) as CryptoKeyPair;
    const der = new Uint8Array((await crypto.subtle.exportKey("pkcs8", pair.privateKey)) as ArrayBuffer);
    // PKCS#8 wraps PKCS#1 in an OCTET STRING after a fixed 26-byte prefix for 2048-bit keys.
    const pkcs1 = der.slice(26);
    const pem = (label: string, bytes: Uint8Array) =>
      `-----BEGIN ${label}-----\n${btoa(String.fromCharCode(...bytes)).replace(/(.{64})/g, "$1\n")}\n-----END ${label}-----\n`;
    expect(pkcs8(pem("RSA PRIVATE KEY", pkcs1))).toEqual(der);
    expect(pkcs8(pem("PRIVATE KEY", der))).toEqual(der);

    const jwt = await appJwt("5845100", pem("RSA PRIVATE KEY", pkcs1), NOW);
    const [head, claims, sig] = jwt.split(".");
    const decode = (s: string) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
    expect(JSON.parse(new TextDecoder().decode(decode(claims)))).toEqual({ iat: NOW - 60, exp: NOW + 540, iss: "5845100" });
    const ok = await crypto.subtle.verify(
      "RSASSA-PKCS1-v1_5",
      pair.publicKey,
      decode(sig),
      new TextEncoder().encode(`${head}.${claims}`),
    );
    expect(ok).toBe(true);
  });
});
