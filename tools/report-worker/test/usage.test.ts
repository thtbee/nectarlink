// SPDX-License-Identifier: GPL-3.0-or-later
import { env } from "cloudflare:test";
import { beforeEach, describe, expect, it } from "vitest";

import { type GitHub } from "../src/github";
import { handle, sweep } from "../src/index";
import { hex, sha256, solve } from "../src/pow";
import { parsePing } from "../src/usage";

const NOW = 1_795_000_000;
const BITS = Number(env.POW_BITS);
const github = {} as GitHub;

function ping(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    v: 1,
    app: "0.1.6",
    channel: "stable",
    platform: "android",
    os: "Android 16",
    hardware: "Google Pixel 9",
    features: { connect: { lan: 3, unreachable: 1 }, mirror: { ok: 2 } },
    ...over,
  };
}

async function send(value: unknown, ip = "198.51.100.7", now = NOW): Promise<Response> {
  const text = JSON.stringify(value);
  const proof = await solve(hex(await sha256(text)), BITS, now);
  return handle(
    new Request("https://r.example/v1/ping", {
      method: "POST",
      headers: { "content-type": "application/json", "x-proof": proof, "cf-connecting-ip": ip },
      body: text,
    }),
    env,
    github,
    now,
  );
}

async function stats(key: string | null, now = NOW): Promise<Response> {
  const url = key === null ? "https://r.example/stats" : `https://r.example/stats?key=${key}`;
  return handle(new Request(url), env, github, now);
}

beforeEach(async () => {
  for (const t of ["usage", "feature_day", "pingers", "salts"]) await env.DB.prepare(`DELETE FROM ${t}`).run();
});

describe("the daily count", () => {
  it("adds each report to the day's totals and keeps nothing else", async () => {
    expect((await send(ping())).status).toBe(204);
    expect((await send(ping({ platform: "windows", os: "Windows 11", hardware: "intel" }), "203.0.113.9")).status).toBe(204);
    expect((await send(ping(), "203.0.113.10")).status).toBe(204);
    const usage = await env.DB.prepare("SELECT platform, count FROM usage ORDER BY platform").all();
    expect(usage.results).toEqual([
      { platform: "android", count: 2 },
      { platform: "windows", count: 1 },
    ]);
    const failed = await env.DB.prepare("SELECT count FROM feature_day WHERE platform = 'android' AND code = 'unreachable'").first();
    expect(failed).toEqual({ count: 2 });
    const dump = JSON.stringify((await env.DB.prepare("SELECT * FROM pingers").all()).results);
    expect(dump).not.toContain("198.51.100.7");
  });

  it("checks the proof and the schema", async () => {
    const text = JSON.stringify(ping());
    const res = await handle(
      new Request("https://r.example/v1/ping", { method: "POST", headers: { "content-type": "application/json" }, body: text }),
      env,
      github,
      NOW,
    );
    expect(res.status).toBe(403);
    for (const bad of [{ v: 2 }, { app: "x" }, { platform: "amiga" }, { os: "<script>" }, { features: { "Bad Name": { ok: 1 } } }, { features: { a: { ok: -1 } } }]) {
      expect(typeof parsePing(ping(bad)), JSON.stringify(bad)).toBe("string");
    }
    expect((await send(ping({ os: "<b>" }))).status).toBe(400);
  });

  it("counts at most 20 a day from one address", async () => {
    for (let i = 0; i < 25; i++) await send(ping());
    const row = await env.DB.prepare("SELECT count FROM usage").first<{ count: number }>();
    expect(row?.count).toBe(20);
  });

  it("forgets senders the next day", async () => {
    await send(ping());
    await sweep(env, github, NOW + 86_400);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM pingers").first<{ n: number }>())?.n).toBe(0);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM usage").first<{ n: number }>())?.n).toBe(1);
  });
});

describe("the stats page", () => {
  it("is only for the key", async () => {
    expect((await stats(null)).status).toBe(404);
    expect((await stats("wrong")).status).toBe(404);
    const res = await stats("test-stats-key");
    expect(res.status).toBe(200);
    expect(res.headers.get("x-robots-tag")).toBe("noindex");
  });

  it("shows the totals", async () => {
    await send(ping({ hardware: "Pixel 9 (A)" }), "a", NOW - 86_400);
    await send(ping({ platform: "windows", os: "Windows 11", hardware: "nvidia" }), "b", NOW - 86_400);
    const html = await (await stats("test-stats-key")).text();
    expect(html).toContain("NectarLink stats");
    expect(html).toContain("<b>2</b>devices");
    expect(html).toContain("Pixel 9 (A)");
    expect(html).toContain("unreachable");
  });
});
