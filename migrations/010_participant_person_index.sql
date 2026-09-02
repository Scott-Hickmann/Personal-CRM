BEGIN;

CREATE INDEX idx_interaction_participants_person
    ON interaction_participants(person_id, interaction_id);

INSERT OR IGNORE INTO schema_versions(version) VALUES (10);
COMMIT;
