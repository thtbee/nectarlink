// SPDX-License-Identifier: GPL-3.0-or-later
// What a report from the app looks like, and the issue made from it.

export const SCHEMAS = [1];
export const MAX_REQUEST = 64 * 1024;
export const MAX_BODY = 60 * 1024;

export const AREAS: Record<string, string> = {
  connection: "Connection",
  "app-windows": "Mirroring and app windows",
  clipboard: "Clipboard",
  audio: "Audio",
  calls: "Calls and messages",
  files: "Files and photos",
  notifications: "Notifications",
  other: "Something else",
};

const PLATFORMS = ["windows", "linux", "macos", "android"];
const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,4}(-[0-9A-Za-z.]{1,20})?$/;
const RID = /^[0-9a-z]{16,32}$/;
const SIG = /^[0-9a-f]{12}$/;
const HASH = /^[0-9a-f]{64}$/;
/** Longest reply from the app, in characters. */
export const MAX_REPLY = 2000;

/** A report as the app sends it. */
export interface Report {
  schema: number;
  /** Random per report, so a resend is filed once. */
  rid: string;
  /** `person` when someone wrote it, `crash` when the crash card sent it. */
  kind: "person" | "crash";
  area: string;
  /** The crash signature, when there is one. */
  sig?: string;
  /** The first words of the description, for the title. */
  summary: string;
  /** NectarLink versions of the devices in the report. */
  versions: string[];
  platforms: string[];
  /** The report as the app showed it to the user. */
  body: string;
  /** SHA-256 (hex) of the key the app keeps for replying from the app. */
  reply_hash?: string;
}

/** Checks `value` field by field. Unknown fields are ignored. */
export function parse(value: unknown): Report | string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return "not an object";
  const v = value as Record<string, unknown>;
  if (typeof v.schema !== "number" || !SCHEMAS.includes(v.schema)) return "schema";
  if (typeof v.rid !== "string" || !RID.test(v.rid)) return "rid";
  if (v.kind !== "person" && v.kind !== "crash") return "kind";
  if (typeof v.area !== "string" || !(v.area in AREAS)) return "area";
  if (v.sig !== undefined && (typeof v.sig !== "string" || !SIG.test(v.sig))) return "sig";
  if (v.kind === "crash" && v.sig === undefined) return "sig";
  if (typeof v.summary !== "string" || v.summary.length > 200) return "summary";
  if (!list(v.versions, (s) => VERSION.test(s))) return "versions";
  if (!list(v.platforms, (s) => PLATFORMS.includes(s))) return "platforms";
  if (typeof v.body !== "string" || v.body.length === 0 || utf8(v.body) > MAX_BODY) return "body";
  if (v.reply_hash !== undefined && (typeof v.reply_hash !== "string" || !HASH.test(v.reply_hash))) {
    return "reply_hash";
  }
  return {
    schema: v.schema,
    rid: v.rid,
    kind: v.kind,
    area: v.area,
    sig: v.sig as string | undefined,
    summary: v.summary,
    versions: v.versions as string[],
    platforms: v.platforms as string[],
    body: v.body,
    reply_hash: v.reply_hash as string | undefined,
  };
}

/**
 * A reply from the app, as a comment: a fixed line, then the text quoted,
 * with mentions and issue links defused so it can't ping anyone, and a
 * marker the app knows its own replies by.
 */
export function replyBody(rid: string, text: string): string {
  const clean = text
    .replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069]/g, "")
    .trim()
    .replace(/@/g, "@\u200b")
    .replace(/#(?=\d)/g, "#\u200b")
    .replace(/</g, "&lt;");
  const quoted = clean
    .split("\n")
    .map((line) => (line.length > 0 ? `> ${line}` : ">"))
    .join("\n");
  return ["The reporter replied from the app:", "", quoted, "", `<!-- nectarlink-reply rid=${rid} -->`].join("\n");
}

function list(value: unknown, ok: (s: string) => boolean): boolean {
  return (
    Array.isArray(value) &&
    value.length <= 4 &&
    value.every((s) => typeof s === "string" && ok(s))
  );
}

function utf8(s: string): number {
  return new TextEncoder().encode(s).length;
}

/** Text that is safe on one line of a title: no controls, no markup that pings. */
export function titleText(s: string, max: number): string {
  const clean = s
    .replace(/[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁦-⁩]/g, " ")
    .replace(/[@#`<>]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const chars = Array.from(clean);
  if (chars.length <= max) return clean;
  const cut = chars.slice(0, max).join("");
  const space = cut.lastIndexOf(" ");
  return `${(space > max / 2 ? cut.slice(0, space) : cut).trimEnd()}...`;
}

export function title(r: Report): string {
  const area = AREAS[r.area];
  if (r.kind === "crash") return `[${area}] Crash ${r.sig}`;
  const words = titleText(r.summary, 60);
  return words ? `[${area}] ${words}` : `[${area}] Problem report`;
}

export function labels(r: Report): string[] {
  const out = ["from-app", "needs-triage", `area:${r.area}`];
  if (r.sig) out.push("crash");
  return out;
}

/**
 * The issue text: a fixed line, then the whole report inside one code fence
 * longer than any run of backticks in it, so nothing in it renders, links
 * or mentions anyone.
 */
export function issueBody(r: Report): string {
  const longest = Math.max(0, ...Array.from(r.body.matchAll(/`+/g), (m) => m[0].length));
  const fence = "`".repeat(Math.max(3, longest + 1));
  const body = r.body.replace(/\r\n?/g, "\n").replace(/\n+$/, "");
  return [
    "Sent from the app's **Report a problem**. Everything below is what the sender saw before sending.",
    "",
    `${fence}text`,
    body,
    fence,
    "",
    `<!-- nectarlink-report rid=${r.rid}${r.sig ? ` sig=${r.sig}` : ""} -->`,
  ].join("\n");
}

/** The tally comment on a crash issue, edited in place. */
export function tally(count: number, versions: string[], platforms: string[]): string {
  return [
    `Reported from the app **${count}** times.`,
    "",
    `- Versions: ${versions.join(", ") || "unknown"}`,
    `- Platforms: ${platforms.join(", ") || "unknown"}`,
  ].join("\n");
}

/** `into` plus anything new from `more`, newest last, at most `max`. */
export function merge(into: string[], more: string[], max = 12): string[] {
  const out = into.filter((v) => !more.includes(v)).concat(more);
  return Array.from(new Set(out)).slice(-max);
}
