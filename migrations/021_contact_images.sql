BEGIN;
CREATE TABLE contact_images (
    apple_contact_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    data BLOB NOT NULL
);
INSERT INTO schema_versions(version) VALUES (21);
COMMIT;
