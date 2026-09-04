BEGIN;

INSERT OR IGNORE INTO source_sync_state(kind) VALUES
    ('contacts'),
    ('imessage'),
    ('whatsapp'),
    ('apple_calls'),
    ('whatsapp_calls'),
    ('gmail');

INSERT OR IGNORE INTO maintenance_state(kind) VALUES
    ('scoring'),
    ('photos'),
    ('google_publish'),
    ('suggestions');

INSERT OR IGNORE INTO schema_versions(version) VALUES (19);
COMMIT;
