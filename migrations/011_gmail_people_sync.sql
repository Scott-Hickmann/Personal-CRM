BEGIN;

CREATE TABLE gmail_sync_scopes (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    scope_key TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('contact', 'discovery')),
    query TEXT NOT NULL,
    page_token TEXT,
    messages_found INTEGER NOT NULL DEFAULT 0,
    completed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(source_id, scope_key)
);
CREATE INDEX idx_gmail_sync_scopes_pending
    ON gmail_sync_scopes(source_id, completed_at, updated_at);

CREATE TABLE gmail_message_state (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    known_scope INTEGER NOT NULL DEFAULT 0 CHECK(known_scope IN (0, 1)),
    discovery_scope INTEGER NOT NULL DEFAULT 0 CHECK(discovery_scope IN (0, 1)),
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK(status IN ('queued', 'accepted', 'skipped', 'deleted')),
    reason TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(source_id, message_id)
);
CREATE INDEX idx_gmail_message_state_queue
    ON gmail_message_state(source_id, status, updated_at);

INSERT OR IGNORE INTO schema_versions(version) VALUES (11);
COMMIT;
