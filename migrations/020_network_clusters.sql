BEGIN;
CREATE TABLE network_cluster_cache (
    level TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE network_cluster_names (
    cluster_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT OR IGNORE INTO schema_versions(version) VALUES (20);
COMMIT;
