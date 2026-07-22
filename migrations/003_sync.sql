BEGIN;
ALTER TABLE interactions ADD COLUMN last_seen_at TEXT;
INSERT OR IGNORE INTO schema_versions(version) VALUES (3);
COMMIT;
