BEGIN;

CREATE TABLE IF NOT EXISTS photo_links (
    person_id TEXT PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    photos_person_uuid TEXT,
    photos_name_snapshot TEXT,
    photos_asset_id TEXT,
    selected_face_index INTEGER,
    selected_face_bounds_json TEXT,
    source_sha256 TEXT,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK(state IN ('pending', 'deferred', 'asset_linked', 'person_linked', 'not_applicable', 'stale')),
    reviewed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_photo_links_person_uuid
    ON photo_links(photos_person_uuid)
    WHERE photos_person_uuid IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_photo_links_asset_id ON photo_links(photos_asset_id);
CREATE INDEX IF NOT EXISTS idx_photo_links_source_sha256 ON photo_links(source_sha256);

INSERT OR IGNORE INTO schema_versions(version) VALUES (4);
COMMIT;
