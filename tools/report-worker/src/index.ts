// SPDX-License-Identifier: GPL-3.0-or-later
// The NectarLink report service: takes a report from the app and files it as
// an issue, so people without a GitHub account can send one.
//
//   POST /v1/report        -> 200 { issue, url } or 202 { queued: true }
//   GET  /v1/report/<rid>  -> 200 { issue, url }, 202 while queued, 404
//
// Nothing about the sender is kept: no address, no account. Report text is
// held only while a report waits in the queue.

import { AppClient, type GitHub } from "./github";
import { hex, sha256, verify } from "./pow";
import { MAX_REQUEST, type Report, issueBody, labels, merge, parse, tally, title } from "./report";

export interface Env {
  DB: D1Database;
  REPO: string;
  GITHUB_APP_ID: string;
  GITHUB_INSTALLATION: string;
  /** Secret: the app's private key, PEM. */
  GITHUB_APP_KEY: string;
  POW_BITS: string;
  /** New issues a day before the rest wait in the queue. */
  DAILY_ISSUES: string;
  /** Reports a day from one sender. */
  SENDER_DAILY: string;
}

const DAY = 86_400;
const QUEUE_MAX = 500;
const QUEUE_DAYS = 7;
const KEEP_DAYS = 90;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env, client(env), Math.floor(Date.now() / 1000));
  },
  async scheduled(_event: ScheduledController, env: Env): Promise<void> {
    await sweep(env, client(env), Math.floor(Date.now() / 1000));
  },
} satisfies ExportedHandler<Env>;

function client(env: Env): GitHub {
  return new AppClient(env.GITHUB_APP_ID, env.GITHUB_INSTALLATION, env.GITHUB_APP_KEY, env.REPO);
}

export async function handle(request: Request, env: Env, github: GitHub, now: number): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname === "/v1/report") {
    if (request.method !== "POST") return reply(405, { error: "method" });
    return receive(request, env, github, now);
  }
  const status = url.pathname.match(/^\/v1\/report\/([0-9a-z]{16,32})$/);
  if (status) {
    if (request.method !== "GET") return reply(405, { error: "method" });
    return lookup(status[1], env);
  }
  return reply(404, { error: "not found" });
}

async function receive(request: Request, env: Env, github: GitHub, now: number): Promise<Response> {
  if (!request.headers.get("content-type")?.startsWith("application/json")) {
    return reply(415, { error: "json" });
  }
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > MAX_REQUEST) return reply(413, { error: "size" });
  const raw = new Uint8Array(await request.arrayBuffer());
  if (raw.length > MAX_REQUEST) return reply(413, { error: "size" });

  // The proof first: nothing below touches storage or GitHub without it.
  const requestHash = hex(await sha256(raw));
  if (!(await verify(request.headers.get("x-proof"), requestHash, Number(env.POW_BITS), now))) {
    return reply(403, { error: "proof" });
  }

  let json: unknown;
  try {
    json = JSON.parse(new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(raw));
  } catch {
    return reply(400, { error: "json" });
  }
  const report = parse(json);
  if (typeof report === "string") return reply(400, { error: report });

  const bodyHash = hex(await sha256(report.body));
  const known = await env.DB.prepare("SELECT issue FROM reports WHERE rid = ?1 OR body_hash = ?2 LIMIT 1")
    .bind(report.rid, bodyHash)
    .first<{ issue: number }>();
  if (known) return filed(env, known.issue);
  if (await env.DB.prepare("SELECT 1 FROM queue WHERE rid = ?1").bind(report.rid).first()) {
    return reply(202, { queued: true });
  }

  const day = dayOf(now);
  const sender = await senderHash(env, day, request.headers.get("cf-connecting-ip") ?? "");
  const sent = await env.DB.prepare("SELECT count FROM senders WHERE day = ?1 AND hash = ?2")
    .bind(day, sender)
    .first<{ count: number }>();
  if ((sent?.count ?? 0) >= Number(env.SENDER_DAILY)) return reply(429, { error: "limit" });
  await env.DB.prepare(
    "INSERT INTO senders (day, hash, count) VALUES (?1, ?2, 1) ON CONFLICT (day, hash) DO UPDATE SET count = count + 1",
  )
    .bind(day, sender)
    .run();

  const issue = await file(report, bodyHash, env, github, now);
  if (issue !== undefined) return filed(env, issue);
  const queued = await env.DB.prepare("SELECT COUNT(*) AS n FROM queue").first<{ n: number }>();
  if ((queued?.n ?? 0) >= QUEUE_MAX) return reply(503, { error: "busy" });
  await env.DB.prepare("INSERT INTO queue (rid, payload, at) VALUES (?1, ?2, ?3)")
    .bind(report.rid, JSON.stringify(report), now)
    .run();
  return reply(202, { queued: true });
}

/**
 * Files `report`: a crash whose signature has an open issue is counted on
 * it; anything else becomes a new issue, unless today's limit is reached.
 * `undefined` when it has to wait (the limit, or GitHub failing).
 */
async function file(
  report: Report,
  bodyHash: string,
  env: Env,
  github: GitHub,
  now: number,
): Promise<number | undefined> {
  try {
    const sig = report.sig
      ? await env.DB.prepare("SELECT * FROM sigs WHERE sig = ?1").bind(report.sig).first<SigRow>()
      : null;
    if (sig) {
      const state = await github.issueState(sig.issue);
      if (state !== "open") {
        await env.DB.prepare("DELETE FROM sigs WHERE sig = ?1").bind(sig.sig).run();
      } else if (report.kind === "crash") {
        await count(sig, report, env, github, now);
        await remember(env, report.rid, bodyHash, sig.issue, now);
        return sig.issue;
      }
    }

    const day = dayOf(now);
    const today = await env.DB.prepare("SELECT issues FROM days WHERE day = ?1").bind(day).first<{ issues: number }>();
    if ((today?.issues ?? 0) >= Number(env.DAILY_ISSUES)) return undefined;

    const issue = await github.createIssue(title(report), issueBody(report), labels(report));
    await env.DB.batch([
      env.DB.prepare(
        "INSERT INTO days (day, issues) VALUES (?1, 1) ON CONFLICT (day) DO UPDATE SET issues = issues + 1",
      ).bind(day),
      env.DB.prepare("INSERT OR REPLACE INTO reports (rid, body_hash, issue, at) VALUES (?1, ?2, ?3, ?4)").bind(
        report.rid,
        bodyHash,
        issue,
        now,
      ),
      ...(report.sig
        ? [
            env.DB.prepare(
              "INSERT OR IGNORE INTO sigs (sig, issue, count, versions, platforms, at) VALUES (?1, ?2, 1, ?3, ?4, ?5)",
            ).bind(report.sig, issue, JSON.stringify(report.versions), JSON.stringify(report.platforms), now),
          ]
        : []),
    ]);
    return issue;
  } catch (e) {
    console.error("filing failed", e instanceof Error ? e.message : "unknown");
    return undefined;
  }
}

interface SigRow {
  sig: string;
  issue: number;
  count: number;
  comment: number | null;
  versions: string;
  platforms: string;
}

/** Adds a report to a crash's tally comment. */
async function count(sig: SigRow, report: Report, env: Env, github: GitHub, now: number): Promise<void> {
  const n = sig.count + 1;
  const versions = merge(JSON.parse(sig.versions) as string[], report.versions);
  const platforms = merge(JSON.parse(sig.platforms) as string[], report.platforms);
  const text = tally(n, versions, platforms);
  let comment = sig.comment;
  if (comment === null) comment = await github.createComment(sig.issue, text);
  else await github.updateComment(comment, text);
  await env.DB.prepare(
    "UPDATE sigs SET count = ?2, comment = ?3, versions = ?4, platforms = ?5, at = ?6 WHERE sig = ?1",
  )
    .bind(sig.sig, n, comment, JSON.stringify(versions), JSON.stringify(platforms), now)
    .run();
}

async function remember(env: Env, rid: string, bodyHash: string, issue: number, now: number): Promise<void> {
  await env.DB.prepare("INSERT OR REPLACE INTO reports (rid, body_hash, issue, at) VALUES (?1, ?2, ?3, ?4)")
    .bind(rid, bodyHash, issue, now)
    .run();
}

async function lookup(rid: string, env: Env): Promise<Response> {
  const known = await env.DB.prepare("SELECT issue FROM reports WHERE rid = ?1").bind(rid).first<{ issue: number }>();
  if (known) return filed(env, known.issue);
  if (await env.DB.prepare("SELECT 1 FROM queue WHERE rid = ?1").bind(rid).first()) {
    return reply(202, { queued: true });
  }
  return reply(404, { error: "unknown" });
}

/**
 * Every 15 minutes: files what's waiting, oldest first, stopping at the
 * first failure or the day's limit; then forgets what's past keeping.
 */
export async function sweep(env: Env, github: GitHub, now: number): Promise<void> {
  const waiting = await env.DB.prepare("SELECT rid, payload FROM queue ORDER BY at LIMIT 20").all<{
    rid: string;
    payload: string;
  }>();
  for (const row of waiting.results) {
    const report = parse(JSON.parse(row.payload));
    if (typeof report === "string") {
      await env.DB.prepare("DELETE FROM queue WHERE rid = ?1").bind(row.rid).run();
      continue;
    }
    const issue = await file(report, hex(await sha256(report.body)), env, github, now);
    if (issue === undefined) {
      await env.DB.prepare("UPDATE queue SET tries = tries + 1 WHERE rid = ?1").bind(row.rid).run();
      break;
    }
    await env.DB.prepare("DELETE FROM queue WHERE rid = ?1").bind(row.rid).run();
  }
  const today = dayOf(now);
  await env.DB.batch([
    env.DB.prepare("DELETE FROM queue WHERE at < ?1").bind(now - QUEUE_DAYS * DAY),
    env.DB.prepare("DELETE FROM reports WHERE at < ?1").bind(now - KEEP_DAYS * DAY),
    env.DB.prepare("DELETE FROM sigs WHERE at < ?1").bind(now - KEEP_DAYS * DAY),
    env.DB.prepare("DELETE FROM salts WHERE day <> ?1").bind(today),
    env.DB.prepare("DELETE FROM senders WHERE day <> ?1").bind(today),
    env.DB.prepare("DELETE FROM days WHERE day < ?1").bind(dayOf(now - 30 * DAY)),
  ]);
}

/** The sender's address hashed with today's random salt, which is deleted tomorrow. */
async function senderHash(env: Env, day: string, ip: string): Promise<string> {
  const fresh = hex(crypto.getRandomValues(new Uint8Array(32)));
  await env.DB.prepare("INSERT OR IGNORE INTO salts (day, salt) VALUES (?1, ?2)").bind(day, fresh).run();
  const row = await env.DB.prepare("SELECT salt FROM salts WHERE day = ?1").bind(day).first<{ salt: string }>();
  return hex(await sha256(`${row?.salt ?? fresh}|${ip}`));
}

function dayOf(secs: number): string {
  return new Date(secs * 1000).toISOString().slice(0, 10);
}

function filed(env: Env, issue: number): Response {
  return reply(200, { issue, url: `https://github.com/${env.REPO}/issues/${issue}` });
}

function reply(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}
