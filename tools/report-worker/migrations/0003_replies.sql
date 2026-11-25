-- Replies from the app: a report can carry the SHA-256 of a key only the
-- sending device knows, so the reporter can answer on the issue without a
-- GitHub account. Only the hash is kept, with the report's row (90 days).
ALTER TABLE reports ADD COLUMN reply_hash TEXT;

-- Replies per report per day, for the limit. Cleared with the day.
CREATE TABLE replies (
  rid TEXT NOT NULL,
  day TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (rid, day)
);
