BEGIN;

DROP TABLE IF EXISTS mentions;
DROP TABLE IF EXISTS semantic_chunks;
DROP TABLE IF EXISTS relationship_signals;
DROP TABLE IF EXISTS jobs;

ALTER TABLE interactions DROP COLUMN analysis_state;

ALTER TABLE metrics DROP COLUMN relational_score;
ALTER TABLE metrics DROP COLUMN model_version;

ALTER TABLE relationships DROP COLUMN relationship_type;
ALTER TABLE relationships DROP COLUMN classification_confidence;
ALTER TABLE relationships DROP COLUMN classification_state;
ALTER TABLE relationships DROP COLUMN classification_evidence;
ALTER TABLE relationships DROP COLUMN evidence_message_ids_json;
ALTER TABLE relationships DROP COLUMN model_version;
ALTER TABLE relationships DROP COLUMN prompt_hash;

CREATE TABLE source_sync_state (
    kind TEXT PRIMARY KEY CHECK(kind IN (
        'contacts', 'imessage', 'whatsapp', 'apple_calls', 'whatsapp_calls', 'gmail'
    )),
    state TEXT NOT NULL DEFAULT 'idle'
        CHECK(state IN ('idle', 'pending', 'running', 'failed')),
    step TEXT NOT NULL DEFAULT 'sync'
        CHECK(step IN ('sync', 'relationships', 'dirty_people')),
    reason TEXT,
    run_after TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    requested_generation INTEGER NOT NULL DEFAULT 0,
    running_generation INTEGER,
    completed_generation INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    changed INTEGER NOT NULL DEFAULT 0 CHECK(changed IN (0, 1)),
    affected_sources_json TEXT NOT NULL DEFAULT '[]',
    error TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE maintenance_state (
    kind TEXT PRIMARY KEY CHECK(kind IN (
        'scoring', 'photos', 'google_publish', 'suggestions'
    )),
    state TEXT NOT NULL DEFAULT 'idle'
        CHECK(state IN ('idle', 'pending', 'running', 'failed')),
    reason TEXT,
    run_after TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    requested_generation INTEGER NOT NULL DEFAULT 0,
    running_generation INTEGER,
    completed_generation INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE dirty_people (
    person_id TEXT PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    dirty_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE dirty_conversations (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    thread_native_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    dirty_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(source_id, thread_native_id)
);

INSERT INTO dirty_people(person_id, reason)
SELECT p.id, 'deterministic scoring migration'
FROM people p
WHERE p.lifecycle_state='active'
  AND NOT EXISTS (
      SELECT 1 FROM identities i
      WHERE i.person_id=p.id AND i.is_self=1 AND i.active=1
  );

INSERT INTO maintenance_state(kind, state, reason, requested_generation)
VALUES ('scoring', 'pending', 'deterministic scoring migration', 1);

INSERT OR IGNORE INTO schema_versions(version) VALUES (18);
COMMIT;
