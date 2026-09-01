BEGIN;

CREATE TABLE excluded_icloud_identities (
    apple_contact_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    PRIMARY KEY (apple_contact_id, kind, normalized_value)
);
CREATE INDEX idx_excluded_icloud_identity_normalized
    ON excluded_icloud_identities(normalized_value);

INSERT OR IGNORE INTO schema_versions(version) VALUES (8);
COMMIT;
