BEGIN;

ALTER TABLE sources ADD COLUMN content_fingerprint TEXT;

INSERT OR IGNORE INTO schema_versions(version) VALUES (14);

COMMIT;
