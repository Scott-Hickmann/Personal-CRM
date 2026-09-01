BEGIN;
ALTER TABLE interaction_participants ADD COLUMN display_name TEXT;
INSERT OR IGNORE INTO schema_versions(version) VALUES (7);
COMMIT;
