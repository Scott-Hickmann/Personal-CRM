BEGIN;

ALTER TABLE people ADD COLUMN lifecycle_state TEXT NOT NULL DEFAULT 'migration_pending'
    CHECK(lifecycle_state IN ('migration_pending', 'active', 'retired'));
ALTER TABLE people ADD COLUMN retired_at TEXT;
ALTER TABLE people ADD COLUMN last_contact_sync_at TEXT;

ALTER TABLE identities RENAME TO identities_legacy;
CREATE TABLE identities (
    id TEXT PRIMARY KEY,
    person_id TEXT REFERENCES people(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    is_self INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    source_id TEXT REFERENCES sources(id)
);
INSERT INTO identities(id, person_id, kind, value, normalized_value, is_self, active, source_id)
SELECT id, person_id, kind, value, normalized_value, is_self, 1, source_id
FROM identities_legacy;
DROP TABLE identities_legacy;

CREATE UNIQUE INDEX idx_active_identity_normalized
    ON identities(kind, normalized_value) WHERE active = 1;
CREATE INDEX idx_identity_person ON identities(person_id, active);

CREATE TRIGGER people_active_requires_icloud_insert
BEFORE INSERT ON people
WHEN NEW.lifecycle_state IN ('active', 'retired') AND NEW.apple_contact_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'active and retired people require an iCloud contact id');
END;

CREATE TRIGGER people_active_requires_icloud_update
BEFORE UPDATE OF lifecycle_state, apple_contact_id ON people
WHEN NEW.lifecycle_state IN ('active', 'retired') AND NEW.apple_contact_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'active and retired people require an iCloud contact id');
END;

CREATE TABLE review_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN (
        'migration_person', 'identity_collision', 'contact_candidate',
        'google_delete', 'google_collision'
    )),
    subject_key TEXT NOT NULL,
    summary TEXT NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK(status IN ('pending', 'approved', 'rejected', 'resolved')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT
);
CREATE UNIQUE INDEX idx_pending_review_subject
    ON review_items(kind, subject_key) WHERE status = 'pending';

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK(kind IN (
        'contacts', 'communications', 'gmail', 'analysis', 'scoring',
        'photos', 'google_publish', 'suggestions'
    )),
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK(state IN ('queued', 'running', 'complete', 'failed')),
    reason TEXT NOT NULL,
    run_after TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT
);
CREATE UNIQUE INDEX idx_open_job_kind
    ON jobs(kind) WHERE state IN ('queued', 'running');
CREATE INDEX idx_jobs_ready ON jobs(state, run_after, id);

CREATE TABLE daemon_state (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    pid INTEGER,
    started_at TEXT,
    heartbeat_at TEXT,
    stopped_at TEXT,
    last_error TEXT
);

INSERT OR IGNORE INTO schema_versions(version) VALUES (6);
COMMIT;
