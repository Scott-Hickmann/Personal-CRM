BEGIN;

CREATE TABLE jobs_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK(kind IN (
        'contacts', 'imessage', 'whatsapp', 'apple_calls', 'whatsapp_calls',
        'gmail', 'analysis', 'scoring', 'photos', 'google_publish', 'suggestions'
    )),
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK(state IN ('queued', 'running', 'complete', 'failed')),
    reason TEXT NOT NULL,
    run_after TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attempts INTEGER NOT NULL DEFAULT 0,
    rerun_requested INTEGER NOT NULL DEFAULT 0 CHECK(rerun_requested IN (0, 1)),
    rerun_after TEXT,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT
);

INSERT INTO jobs_next(
    id, kind, state, reason, run_after, attempts, error,
    created_at, updated_at, completed_at
)
SELECT id, kind, state, reason, run_after, attempts, error,
       created_at, updated_at, completed_at
FROM jobs
WHERE kind != 'communications';

INSERT INTO jobs_next(
    kind, state, reason, run_after, attempts, error,
    created_at, updated_at, completed_at
)
SELECT split.kind, jobs.state, jobs.reason, jobs.run_after, jobs.attempts, jobs.error,
       jobs.created_at, jobs.updated_at, jobs.completed_at
FROM jobs
JOIN (
    SELECT 'imessage' AS kind
    UNION ALL SELECT 'whatsapp'
    UNION ALL SELECT 'apple_calls'
    UNION ALL SELECT 'whatsapp_calls'
) split
WHERE jobs.kind = 'communications';

DROP TABLE jobs;
ALTER TABLE jobs_next RENAME TO jobs;
CREATE UNIQUE INDEX idx_open_job_kind
    ON jobs(kind) WHERE state IN ('queued', 'running');
CREATE INDEX idx_jobs_ready ON jobs(state, run_after, id);

INSERT OR IGNORE INTO schema_versions(version) VALUES (13);
COMMIT;
