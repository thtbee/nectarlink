# Report service

Takes a problem report from the app's **Report a problem › Send** and files
it as an issue here, so people without a GitHub account can report too. It
runs as a Cloudflare Worker with a D1 database, and files issues as the
**NectarLink Reports** GitHub App.

| Endpoint | Answer |
|---|---|
| `POST /v1/report` | `200 { issue, url }`, or `202 { queued: true }` when it has to wait |
| `GET /v1/report/<rid>` | `200 { issue, url }`, `202` while queued, `404` when unknown |
| `POST /v1/ping` | `204`: the daily count |
| `GET /stats?key=<key>` | The owner's page of daily totals; `404` without the key |

## The daily count

Once a UTC day each app sends its version and channel, platform, OS
version, phone model (or nothing on a PC), and how many times each feature
worked or failed the day before: `{ v, app, channel, platform, os,
hardware, features }`, at most 8 KB, with a 16-bit proof of work. There's
no ID. The service adds it to totals per day and throws the report away;
at most 20 a day count from one address. Totals are kept 400 days.

**The stats page** shows devices per day for the last 30 days, the 7- and
30-day daily averages, the split by platform, version, OS and model, and
what failed most in the last week. Without an ID a device can't be told
apart from day to day, so a week's figure is a daily average, not unique
devices.

## Crashes already fixed

`.github/known-crashes.json` on `main` lists crashes a release fixed:
`[{ "sig": "b3af6c6e31db", "issue": 482, "fixed_in": "0.1.7" }]`, 16 KB at
most. The apps' crash card checks its crash against it and offers the
update instead of a report. Add an entry when a release fixes a crash
issue; only the version is read, and only when it's plain digits.

## What it keeps

Nothing about the sender. Reports per sender per day are counted by a hash
of the address with that day's random salt, and both are deleted the next
day. Report text is stored only while a report waits in the queue (at most
7 days); after that it lives only in the issue. Which report went to which
issue is kept for 90 days, so a resend isn't filed twice.

## Abuse limits

- **Proof of work.** The app hashes the request body, then finds a counter
  `n` so that SHA-256 of `<body hash hex>|<time>|<n>` starts with 20 zero
  bits (about a second on a phone), and sends `X-Proof: <time>.<n>`. It's
  checked before anything is stored or sent to GitHub, and is valid for
  10 minutes.
- 5 reports a day from one sender.
- 50 new issues a day in all. The rest wait in the queue and are filed the
  next day.
- A strict schema: areas, versions, platforms and signatures from fixed
  lists or patterns, 64 KB at most.

## Issues it files

- The title is the area and up to 60 characters of the description, with
  anything that could mention, link or format removed.
- The report goes inside one code fence longer than any run of backticks in
  it, so nothing in it renders.
- Labels: `from-app`, `needs-triage`, `area:<area>`, and `crash` when there
  is a crash signature.
- A crash sent from the crash card whose signature already has an open
  issue is counted on that issue in one comment, edited in place, instead
  of opening another. A report someone wrote always gets its own issue.

When GitHub fails, the report waits in the queue, and a sweep every 15
minutes files what's waiting.

## Deploying

Once, as the repository owner:

1. **GitHub App** (done: NectarLink Reports, App ID 5845100, installed on
   this repository only as installation 183952957). Its only permission is
   Repository › Issues: Read and write, and its webhook is off. Generate a
   private key on the app's page.
2. **Labels.** `from-app`, `needs-triage`, `crash`, and `area:connection`,
   `area:app-windows`, `area:clipboard`, `area:audio`, `area:calls`,
   `area:files`, `area:notifications`, `area:other`.
3. **Cloudflare**, from this folder:

   ```sh
   npm ci
   npx wrangler login
   npx wrangler d1 create nectarlink-reports      # put its database_id in wrangler.toml
   npx wrangler d1 migrations apply nectarlink-reports --remote
   npx wrangler secret put GITHUB_APP_KEY < path/to/the-key.pem
   npx wrangler secret put STATS_KEY      # a long random string: the stats page's key
   npx wrangler deploy
   ```

   Delete the `.pem` once the secret is in; a new key can be generated on
   the app's page any time, and old ones revoked there.
4. Put the Worker's address in `REPORT_SERVICE` in
   `core/nectarlink-core/src/diag/send.rs`.

## Testing

```sh
npm ci
npx tsc --noEmit
npx vitest run
```

The tests run in the Workers runtime with a real D1 database and a fake
GitHub.
