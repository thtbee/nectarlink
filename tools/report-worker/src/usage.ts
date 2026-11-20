// SPDX-License-Identifier: GPL-3.0-or-later
// The daily count: once a day each install says which version and platform
// it is and how its features did, with no ID. Each report is added to daily
// totals and thrown away; the owner reads the totals on /stats.

import type { Env } from "./index";
import { hex, sha256, verify } from "./pow";

/** Zero bits the daily report's proof needs: a few milliseconds. */
export const PING_BITS = 16;
const MAX_PING = 8 * 1024;
/** Daily reports from one address (households and offices share one). */
const PER_SENDER = 20;
const KEEP_DAYS = 400;

const PLATFORMS = ["windows", "linux", "macos", "android"];
const CHANNELS = ["stable", "nightly", "dev"];
const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,4}(-[0-9A-Za-z.]{1,20})?$/;
const TEXT = /^[0-9A-Za-z ._()+/-]{0,60}$/;
const NAME = /^[a-z][a-z0-9_]{0,23}$/;

export interface Ping {
  app: string;
  channel: string;
  platform: string;
  os: string;
  hardware: string;
  features: Record<string, Record<string, number>>;
}

export function parsePing(value: unknown): Ping | string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return "not an object";
  const v = value as Record<string, unknown>;
  if (v.v !== 1) return "v";
  if (typeof v.app !== "string" || !VERSION.test(v.app)) return "app";
  if (typeof v.channel !== "string" || !CHANNELS.includes(v.channel)) return "channel";
  if (typeof v.platform !== "string" || !PLATFORMS.includes(v.platform)) return "platform";
  if (typeof v.os !== "string" || !TEXT.test(v.os)) return "os";
  const hardware = v.hardware ?? "";
  if (typeof hardware !== "string" || !TEXT.test(hardware)) return "hardware";
  const features: Record<string, Record<string, number>> = {};
  const raw = v.features ?? {};
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return "features";
  const entries = Object.entries(raw as Record<string, unknown>);
  if (entries.length > 16) return "features";
  for (const [feature, codes] of entries) {
    if (!NAME.test(feature) || typeof codes !== "object" || codes === null || Array.isArray(codes)) return "features";
    const list = Object.entries(codes as Record<string, unknown>);
    if (list.length > 16) return "features";
    features[feature] = {};
    for (const [code, n] of list) {
      if (!NAME.test(code) || typeof n !== "number" || !Number.isInteger(n) || n < 0 || n > 100_000) return "features";
      features[feature][code] = n;
    }
  }
  return { app: v.app, channel: v.channel, platform: v.platform, os: v.os.trim(), hardware: hardware.trim(), features };
}

export async function receivePing(request: Request, env: Env, now: number, sender: string): Promise<Response> {
  const raw = new Uint8Array(await request.arrayBuffer());
  if (raw.length > MAX_PING) return json(413, { error: "size" });
  const proofBits = Math.min(PING_BITS, Number(env.POW_BITS));
  if (!(await verify(request.headers.get("x-proof"), hex(await sha256(raw)), proofBits, now))) {
    return json(403, { error: "proof" });
  }
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder().decode(raw));
  } catch {
    return json(400, { error: "json" });
  }
  const ping = parsePing(value);
  if (typeof ping === "string") return json(400, { error: ping });

  const day = new Date(now * 1000).toISOString().slice(0, 10);
  const seen = await env.DB.prepare("SELECT count FROM pingers WHERE day = ?1 AND hash = ?2")
    .bind(day, sender)
    .first<{ count: number }>();
  if ((seen?.count ?? 0) >= PER_SENDER) return new Response(null, { status: 204 });

  const statements = [
    env.DB.prepare(
      "INSERT INTO pingers (day, hash, count) VALUES (?1, ?2, 1) ON CONFLICT (day, hash) DO UPDATE SET count = count + 1",
    ).bind(day, sender),
    env.DB.prepare(
      `INSERT INTO usage (day, platform, app, channel, os, hardware, count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
       ON CONFLICT (day, platform, app, channel, os, hardware) DO UPDATE SET count = count + 1`,
    ).bind(day, ping.platform, ping.app, ping.channel, ping.os, ping.hardware),
  ];
  for (const [feature, codes] of Object.entries(ping.features)) {
    for (const [code, n] of Object.entries(codes)) {
      if (n === 0) continue;
      statements.push(
        env.DB.prepare(
          `INSERT INTO feature_day (day, platform, feature, code, count) VALUES (?1, ?2, ?3, ?4, ?5)
           ON CONFLICT (day, platform, feature, code) DO UPDATE SET count = count + ?5`,
        ).bind(day, ping.platform, feature, code, n),
      );
    }
  }
  await env.DB.batch(statements);
  return new Response(null, { status: 204 });
}

export function sweepUsage(env: Env, now: number): D1PreparedStatement[] {
  const today = new Date(now * 1000).toISOString().slice(0, 10);
  const oldest = new Date((now - KEEP_DAYS * 86_400) * 1000).toISOString().slice(0, 10);
  return [
    env.DB.prepare("DELETE FROM pingers WHERE day <> ?1").bind(today),
    env.DB.prepare("DELETE FROM usage WHERE day < ?1").bind(oldest),
    env.DB.prepare("DELETE FROM feature_day WHERE day < ?1").bind(oldest),
  ];
}

// ---- The stats page ----

/** Whether `given` is the stats key, compared in constant time. */
export async function isStatsKey(env: Env, given: string | null): Promise<boolean> {
  if (!env.STATS_KEY || !given) return false;
  const [a, b] = [await sha256(env.STATS_KEY), await sha256(given)];
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}

interface Row {
  [key: string]: string | number;
}

export async function statsPage(env: Env, now: number): Promise<Response> {
  const day = (offset: number) => new Date((now - offset * 86_400) * 1000).toISOString().slice(0, 10);
  const [today, yesterday, week, month] = [day(0), day(1), day(7), day(30)];
  const all = async (sql: string, ...args: unknown[]) =>
    (await env.DB.prepare(sql).bind(...args).all<Row>()).results;

  const daily = await all(
    "SELECT day, platform, SUM(count) AS n FROM usage WHERE day >= ?1 GROUP BY day, platform ORDER BY day",
    month,
  );
  const average = async (from: string) =>
    (await all("SELECT SUM(count) AS n, COUNT(DISTINCT day) AS days FROM usage WHERE day >= ?1 AND day < ?2", from, today))[0];
  const [avg7, avg30] = [await average(week), await average(month)];
  const latest = (await all("SELECT MAX(day) AS d FROM usage WHERE day < ?1", today))[0]?.d ?? yesterday;
  const by = (column: string) =>
    all(`SELECT ${column} AS name, SUM(count) AS n FROM usage WHERE day = ?1 GROUP BY ${column} ORDER BY n DESC LIMIT 25`, latest);
  const [platforms, versions, systems, hardware] = [await by("platform"), await by("app"), await by("os"), await by("hardware")];
  const failing = await all(
    `SELECT platform, feature, code, SUM(count) AS n FROM feature_day
     WHERE day >= ?1 AND code NOT IN ('ok', 'lan', 'usb', 'relay', 'cancelled')
     GROUP BY platform, feature, code ORDER BY n DESC LIMIT 30`,
    week,
  );

  const days = Array.from(new Set(daily.map((r) => String(r.day))));
  const platformsSeen = Array.from(new Set(daily.map((r) => String(r.platform)))).sort();
  const total = (d: string) => daily.filter((r) => r.day === d).reduce((s, r) => s + Number(r.n), 0);
  const peak = Math.max(1, ...days.map(total));
  const perDay = (r: Row | undefined) =>
    r && Number(r.days) > 0 ? Math.round(Number(r.n) / Number(r.days)).toLocaleString("en") : "–";

  const bars = days
    .map((d) => {
      const n = total(d);
      const split = platformsSeen.map((p) => `${p} ${daily.find((r) => r.day === d && r.platform === p)?.n ?? 0}`).join(", ");
      return `<div class="bar" title="${esc(`${d}: ${n} (${split})`)}"><span style="height:${Math.max(2, (n / peak) * 100)}%"></span><em>${esc(d.slice(5))}</em></div>`;
    })
    .join("");

  const table = (title: string, rows: Row[], columns: [string, string][]) => `
    <section><h2>${esc(title)}</h2>${
      rows.length === 0
        ? `<p class="none">Nothing yet.</p>`
        : `<table><thead><tr>${columns.map(([, label]) => `<th>${esc(label)}</th>`).join("")}</tr></thead><tbody>${rows
            .map((r) => `<tr>${columns.map(([key]) => `<td>${esc(String(r[key] ?? ""))}</td>`).join("")}</tr>`)
            .join("")}</tbody></table>`
    }</section>`;

  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex"><title>NectarLink stats</title><style>
:root{--bg:#fbf8f2;--fg:#1d1b16;--muted:#6b665b;--line:#e4dfd3;--accent:#c27c0e}
@media (prefers-color-scheme:dark){:root{--bg:#161512;--fg:#ece7dc;--muted:#a49e91;--line:#2e2b25;--accent:#f0a93a}}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif}
main{max-width:960px;margin:0 auto;padding:24px 16px 48px}
h1{font-size:24px;margin:0 0 4px}h2{font-size:16px;margin:28px 0 8px}
.sub,.none,em{color:var(--muted)}.none{margin:0}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-top:20px}
.tile{border:1px solid var(--line);border-radius:12px;padding:12px 14px}.tile b{display:block;font-size:26px}
.chart{display:flex;align-items:flex-end;gap:3px;height:160px;border-bottom:1px solid var(--line);overflow-x:auto}
.bar{flex:1 0 14px;display:flex;flex-direction:column;justify-content:flex-end;height:100%;position:relative}
.bar span{background:var(--accent);border-radius:3px 3px 0 0;display:block}
.bar em{font-size:10px;font-style:normal;text-align:center;position:absolute;bottom:-18px;width:100%}
.chart+p{margin-top:24px}table{border-collapse:collapse;width:100%}
td,th{text-align:left;padding:6px 8px;border-bottom:1px solid var(--line)}th{color:var(--muted);font-weight:500}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:0 24px}
</style></head><body><main>
<h1>NectarLink stats</h1>
<p class="sub">Installs that sent their daily count. There's no ID, so a device counts once a day, and a week's numbers are a daily average.</p>
<div class="tiles">
  <div class="tile"><em>${esc(String(latest))}</em><b>${total(String(latest)).toLocaleString("en")}</b>devices</div>
  <div class="tile"><em>Last 7 days</em><b>${perDay(avg7)}</b>a day on average</div>
  <div class="tile"><em>Last 30 days</em><b>${perDay(avg30)}</b>a day on average</div>
  <div class="tile"><em>Today so far</em><b>${total(today).toLocaleString("en")}</b>devices</div>
</div>
<h2>Devices a day, last 30 days</h2>
<div class="chart">${bars || `<p class="none">Nothing yet.</p>`}</div>
<p class="sub">Hover a bar for the split by platform.</p>
<div class="grid">
${table(`Platforms on ${latest}`, platforms, [["name", "Platform"], ["n", "Devices"]])}
${table(`Versions on ${latest}`, versions, [["name", "Version"], ["n", "Devices"]])}
${table(`Systems on ${latest}`, systems, [["name", "OS"], ["n", "Devices"]])}
${table(`Phones and graphics on ${latest}`, hardware, [["name", "Model or GPU"], ["n", "Devices"]])}
</div>
${table("What failed most, last 7 days", failing, [["platform", "Platform"], ["feature", "Feature"], ["code", "Outcome"], ["n", "Times"]])}
</main></body></html>`;
  return new Response(html, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "x-robots-tag": "noindex",
      "referrer-policy": "no-referrer",
      "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'",
    },
  });
}

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}
