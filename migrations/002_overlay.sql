BEGIN;

CREATE TABLE IF NOT EXISTS identity_candidates (
    id TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL,
    source_value TEXT NOT NULL,
    candidate_person_id TEXT NOT NULL REFERENCES people(id),
    confidence REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS important_dates (
    id TEXT PRIMARY KEY,
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    date TEXT NOT NULL,
    recurring INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS followups (
    id TEXT PRIMARY KEY,
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    due_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS cadences (
    person_id TEXT PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    interval_days INTEGER NOT NULL CHECK(interval_days > 0),
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    interaction_id TEXT NOT NULL REFERENCES interactions(id) ON DELETE CASCADE,
    filename TEXT,
    mime_type TEXT,
    size_bytes INTEGER,
    source_reference TEXT
);

CREATE TABLE IF NOT EXISTS mentions (
    id TEXT PRIMARY KEY,
    interaction_id TEXT NOT NULL REFERENCES interactions(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    person_id TEXT REFERENCES people(id),
    confidence REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'unresolved'
);

CREATE TABLE IF NOT EXISTS metrics (
    person_id TEXT PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    behavioral_score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    components_json TEXT NOT NULL,
    model_version TEXT,
    calculated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS semantic_chunks (
    id TEXT PRIMARY KEY,
    person_id TEXT REFERENCES people(id) ON DELETE CASCADE,
    interaction_ids_json TEXT NOT NULL,
    summary TEXT,
    embedding_json TEXT,
    model_version TEXT NOT NULL,
    prompt_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO schema_versions(version) VALUES (2);
COMMIT;
