BEGIN;

CREATE TABLE person_merges (
    source_person_id TEXT PRIMARY KEY REFERENCES people(id),
    target_person_id TEXT NOT NULL REFERENCES people(id),
    merged_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK(source_person_id <> target_person_id)
);

INSERT OR IGNORE INTO schema_versions(version) VALUES (9);
COMMIT;
