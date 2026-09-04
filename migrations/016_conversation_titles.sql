BEGIN;

ALTER TABLE conversation_memberships ADD COLUMN conversation_title TEXT;

INSERT OR IGNORE INTO schema_versions(version) VALUES (16);
COMMIT;
