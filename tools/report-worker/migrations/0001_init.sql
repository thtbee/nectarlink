-- Reports that were filed: a resend returns the same issue. 90 days.
CREATE TABLE reports (
  rid TEXT PRIMARY KEY,
  body_hash TEXT NOT NULL,
  issue INTEGER NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX reports_body_hash ON reports (body_hash);
CREATE INDEX reports_at ON reports (at);

-- One issue per crash signature while it's open, with its tally comment.
CREATE TABLE sigs (
  sig TEXT PRIMARY KEY,
  issue INTEGER NOT NULL,
  count INTEGER NOT NULL,
  comment INTEGER,
  versions TEXT NOT NULL,
  platforms TEXT NOT NULL,
  at INTEGER NOT NULL
);

-- Reports waiting for GitHub: an outage, or the day's limit reached. The
-- report text is kept only here, and only until it's filed. 7 days.
CREATE TABLE queue (
  rid TEXT PRIMARY KEY,
  payload TEXT NOT NULL,
  at INTEGER NOT NULL,
  tries INTEGER NOT NULL DEFAULT 0
);

-- Today's random salt for counting senders. Deleted the next day, so the
-- counts below can't be matched to an address afterwards.
CREATE TABLE salts (
  day TEXT PRIMARY KEY,
  salt TEXT NOT NULL
);

-- Reports per sender per day, by salted hash. Never the address.
CREATE TABLE senders (
  day TEXT NOT NULL,
  hash TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (day, hash)
);

-- New issues per day, for the global limit.
CREATE TABLE days (
  day TEXT PRIMARY KEY,
  issues INTEGER NOT NULL
);
