BEGIN;

CREATE TABLE IF NOT EXISTS schema_versions (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT OR IGNORE INTO schema_versions(version) VALUES (1);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    account TEXT,
    schema_fingerprint TEXT,
    cursor TEXT,
    last_sync_at TEXT,
    last_reconcile_at TEXT,
    status TEXT NOT NULL DEFAULT 'new',
    error TEXT
);

CREATE TABLE IF NOT EXISTS people (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    apple_contact_id TEXT UNIQUE,
    affinity_score REAL,
    affinity_tier TEXT,
    activity_state TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS identities (
    id TEXT PRIMARY KEY,
    person_id TEXT REFERENCES people(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    is_self INTEGER NOT NULL DEFAULT 0,
    source_id TEXT REFERENCES sources(id),
    UNIQUE(kind, normalized_value)
);

CREATE TABLE IF NOT EXISTS interactions (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id),
    native_id TEXT NOT NULL,
    thread_native_id TEXT,
    channel TEXT NOT NULL,
    kind TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    direction TEXT,
    subject TEXT,
    body TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    deleted_at TEXT,
    analysis_state TEXT NOT NULL DEFAULT 'pending',
    UNIQUE(source_id, native_id)
);

CREATE TABLE IF NOT EXISTS interaction_participants (
    interaction_id TEXT NOT NULL REFERENCES interactions(id) ON DELETE CASCADE,
    person_id TEXT REFERENCES people(id),
    identity_value TEXT,
    role TEXT NOT NULL,
    PRIMARY KEY(interaction_id, role, identity_value)
);

CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS facts (
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(person_id, key)
);

CREATE TABLE IF NOT EXISTS tags (
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY(person_id, tag)
);

CREATE TABLE IF NOT EXISTS relationships (
    id TEXT PRIMARY KEY,
    source_person_id TEXT NOT NULL REFERENCES people(id),
    target_person_id TEXT NOT NULL REFERENCES people(id),
    relationship_type TEXT NOT NULL,
    confidence REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'inferred',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    model_version TEXT,
    first_observed_at TEXT,
    last_observed_at TEXT
);

CREATE TABLE IF NOT EXISTS tombstones (
    source_id TEXT NOT NULL REFERENCES sources(id),
    native_id TEXT NOT NULL,
    deleted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(source_id, native_id)
);

CREATE INDEX IF NOT EXISTS idx_identity_normalized ON identities(kind, normalized_value);
CREATE INDEX IF NOT EXISTS idx_interactions_occurred ON interactions(occurred_at);
CREATE INDEX IF NOT EXISTS idx_interactions_source ON interactions(source_id, native_id);

COMMIT;
