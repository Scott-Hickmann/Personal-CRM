BEGIN;

DROP TABLE relationships;

CREATE TABLE conversation_memberships (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    thread_native_id TEXT NOT NULL,
    person_id TEXT REFERENCES people(id) ON DELETE SET NULL,
    identity_value TEXT NOT NULL,
    display_name TEXT,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    PRIMARY KEY(source_id, thread_native_id, identity_value)
);
CREATE INDEX idx_conversation_memberships_person
    ON conversation_memberships(person_id, source_id, thread_native_id);

CREATE TABLE relationship_contexts (
    source_person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    target_person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    thread_native_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    first_observed_at TEXT NOT NULL,
    last_observed_at TEXT NOT NULL,
    message_count INTEGER NOT NULL,
    PRIMARY KEY(source_person_id, target_person_id, source_id, thread_native_id),
    CHECK(source_person_id < target_person_id)
);
CREATE INDEX idx_relationship_contexts_source
    ON relationship_contexts(source_id, thread_native_id);

CREATE TABLE relationships (
    id TEXT PRIMARY KEY,
    source_person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    target_person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    relationship_type TEXT NOT NULL DEFAULT 'unclear',
    classification_confidence REAL CHECK(
        classification_confidence IS NULL
        OR classification_confidence BETWEEN 0 AND 1
    ),
    classification_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(classification_state IN ('pending', 'complete')),
    classification_evidence TEXT NOT NULL DEFAULT '',
    evidence_message_ids_json TEXT NOT NULL DEFAULT '[]',
    model_version TEXT,
    prompt_hash TEXT,
    structure_revision INTEGER NOT NULL DEFAULT 1,
    first_observed_at TEXT NOT NULL,
    last_observed_at TEXT NOT NULL,
    shared_context_count INTEGER NOT NULL,
    UNIQUE(source_person_id, target_person_id),
    CHECK(source_person_id < target_person_id)
);

INSERT INTO conversation_memberships(
    source_id, thread_native_id, person_id, identity_value, display_name
)
SELECT i.source_id, i.thread_native_id, MAX(ip.person_id),
       COALESCE(ip.identity_value, 'person:' || ip.person_id), MAX(ip.display_name)
FROM interactions i
JOIN interaction_participants ip ON ip.interaction_id=i.id
WHERE i.deleted_at IS NULL AND i.thread_native_id IS NOT NULL
  AND ip.person_id IS NOT NULL
GROUP BY i.source_id, i.thread_native_id,
         COALESCE(ip.identity_value, 'person:' || ip.person_id);

WITH conversation_people AS (
    SELECT DISTINCT cm.source_id, cm.thread_native_id, cm.person_id
    FROM conversation_memberships cm
    JOIN people p ON p.id=cm.person_id AND p.lifecycle_state='active'
    WHERE cm.active=1 AND cm.person_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1 FROM identities own
        WHERE own.person_id=cm.person_id AND own.is_self=1 AND own.active=1
      )
), conversation_activity AS (
    SELECT source_id, thread_native_id, MIN(channel) AS channel,
           MIN(occurred_at) AS first_observed_at,
           MAX(occurred_at) AS last_observed_at,
           COUNT(DISTINCT id) AS message_count
    FROM interactions
    WHERE deleted_at IS NULL AND thread_native_id IS NOT NULL
    GROUP BY source_id, thread_native_id
)
INSERT INTO relationship_contexts(
    source_person_id, target_person_id, source_id, thread_native_id, channel,
    first_observed_at, last_observed_at, message_count
)
SELECT a.person_id, b.person_id, a.source_id, a.thread_native_id, activity.channel,
       activity.first_observed_at, activity.last_observed_at, activity.message_count
FROM conversation_people a
JOIN conversation_people b
  ON b.source_id=a.source_id AND b.thread_native_id=a.thread_native_id
 AND b.person_id>a.person_id
JOIN conversation_activity activity
  ON activity.source_id=a.source_id AND activity.thread_native_id=a.thread_native_id;

INSERT INTO relationships(
    id, source_person_id, target_person_id, first_observed_at,
    last_observed_at, shared_context_count
)
SELECT source_person_id || ':' || target_person_id, source_person_id, target_person_id,
       MIN(first_observed_at), MAX(last_observed_at), COUNT(*)
FROM relationship_contexts
GROUP BY source_person_id, target_person_id;

INSERT OR IGNORE INTO schema_versions(version) VALUES (15);
COMMIT;
