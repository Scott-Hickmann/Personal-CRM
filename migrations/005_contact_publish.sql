BEGIN;

CREATE TABLE IF NOT EXISTS contact_mirrors (
    apple_contact_id TEXT NOT NULL,
    google_account TEXT NOT NULL,
    google_resource_name TEXT NOT NULL,
    google_etag TEXT,
    content_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(apple_contact_id, google_account),
    UNIQUE(google_account, google_resource_name)
);

INSERT OR IGNORE INTO schema_versions(version) VALUES (5);
COMMIT;
