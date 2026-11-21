-- Daily counts: one report a day from each install, no ID. Each report is
-- added to these totals and thrown away. 400 days.
CREATE TABLE usage (
  day TEXT NOT NULL,
  platform TEXT NOT NULL,
  app TEXT NOT NULL,
  channel TEXT NOT NULL,
  os TEXT NOT NULL,
  hardware TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (day, platform, app, channel, os, hardware)
);

-- How features did, per day and platform: outcome counts added up.
CREATE TABLE feature_day (
  day TEXT NOT NULL,
  platform TEXT NOT NULL,
  feature TEXT NOT NULL,
  code TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (day, platform, feature, code)
);

-- Daily reports per sender, by the same daily salted hash as reports.
CREATE TABLE pingers (
  day TEXT NOT NULL,
  hash TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (day, hash)
);
