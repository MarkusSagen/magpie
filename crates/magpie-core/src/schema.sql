CREATE TABLE IF NOT EXISTS apps (
  id            INTEGER PRIMARY KEY,
  identifier    TEXT NOT NULL UNIQUE,
  display_name  TEXT NOT NULL,
  icon_path     TEXT
);

CREATE TABLE IF NOT EXISTS entries (
  id                 INTEGER PRIMARY KEY,
  content_hash       TEXT NOT NULL UNIQUE,
  kind               TEXT NOT NULL,
  preview_text       TEXT NOT NULL,
  full_text          TEXT NOT NULL,
  image_path         TEXT,
  byte_size          INTEGER NOT NULL,
  char_count         INTEGER NOT NULL,
  word_count         INTEGER NOT NULL,
  line_count         INTEGER NOT NULL,
  first_copied_at_ms INTEGER NOT NULL,
  last_copied_at_ms  INTEGER NOT NULL,
  copy_count         INTEGER NOT NULL DEFAULT 1,
  pinned             INTEGER NOT NULL DEFAULT 0,
  source_app_id      INTEGER REFERENCES apps(id)
);

CREATE INDEX IF NOT EXISTS idx_entries_last_copied ON entries(last_copied_at_ms);
CREATE INDEX IF NOT EXISTS idx_entries_kind        ON entries(kind);
CREATE INDEX IF NOT EXISTS idx_entries_app         ON entries(source_app_id);

CREATE TABLE IF NOT EXISTS copy_events (
  id            INTEGER PRIMARY KEY,
  entry_id      INTEGER NOT NULL REFERENCES entries(id),
  copied_at_ms  INTEGER NOT NULL,
  source_app_id INTEGER REFERENCES apps(id)
);

CREATE INDEX IF NOT EXISTS idx_copy_events_entry ON copy_events(entry_id);
CREATE INDEX IF NOT EXISTS idx_copy_events_time  ON copy_events(copied_at_ms);

CREATE TABLE IF NOT EXISTS slots (
  slot     INTEGER PRIMARY KEY CHECK(slot BETWEEN 1 AND 9),
  entry_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS entry_tags (
  entry_id INTEGER NOT NULL,
  tag      TEXT NOT NULL,
  PRIMARY KEY (entry_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags(tag);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
  full_text,
  content='entries',
  content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, full_text) VALUES (new.id, new.full_text);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, full_text) VALUES ('delete', old.id, old.full_text);
END;
CREATE TRIGGER IF NOT EXISTS entries_au AFTER UPDATE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, full_text) VALUES ('delete', old.id, old.full_text);
  INSERT INTO entries_fts(rowid, full_text) VALUES (new.id, new.full_text);
END;

CREATE TABLE IF NOT EXISTS notes (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  is_daily        INTEGER NOT NULL DEFAULT 0,
  body            TEXT NOT NULL DEFAULT '',
  created_at_ms   INTEGER NOT NULL,
  updated_at_ms   INTEGER NOT NULL,
  source_app_id   INTEGER,
  source_entry_id INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_notes_name ON notes(name);
CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at_ms);
