BEGIN;

ALTER TABLE metrics RENAME COLUMN semantic_score TO relational_score;

CREATE TABLE relationship_signals (
    interaction_id TEXT NOT NULL REFERENCES interactions(id) ON DELETE CASCADE,
    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    intimacy REAL NOT NULL CHECK(intimacy BETWEEN 0 AND 3),
    emotional_support REAL NOT NULL CHECK(emotional_support BETWEEN 0 AND 3),
    practical_support REAL NOT NULL CHECK(practical_support BETWEEN 0 AND 3),
    affection REAL NOT NULL CHECK(affection BETWEEN 0 AND 3),
    shared_activity REAL NOT NULL CHECK(shared_activity BETWEEN 0 AND 3),
    conflict_repair REAL NOT NULL CHECK(conflict_repair BETWEEN 0 AND 3),
    confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
    evidence TEXT NOT NULL,
    model_version TEXT NOT NULL,
    prompt_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(interaction_id, person_id)
);
CREATE INDEX idx_relationship_signals_person
    ON relationship_signals(person_id, interaction_id);

CREATE TABLE affinity_ratings (
    person_id TEXT PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    rating INTEGER NOT NULL CHECK(rating BETWEEN 1 AND 7),
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

DELETE FROM metrics;
UPDATE people
SET affinity_score=NULL, affinity_tier=NULL, activity_state=NULL,
    updated_at=CURRENT_TIMESTAMP;
UPDATE interactions
SET analysis_state='pending'
WHERE analysis_state='complete' AND body IS NOT NULL AND trim(body) != '';

INSERT OR IGNORE INTO schema_versions(version) VALUES (12);
COMMIT;
