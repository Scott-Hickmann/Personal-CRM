use rusqlite::Connection;

use crate::error::Result;
use crate::graph;

use super::{Overview, OverviewPerson, separated};

pub fn load(connection: &Connection) -> Result<Overview> {
    let mut statement = connection.prepare(
        "SELECT p.id, p.display_name, p.lifecycle_state, p.affinity_score,
                p.affinity_tier, p.activity_state,
                (SELECT COUNT(DISTINCT i.id) FROM interactions i
                 JOIN interaction_participants ip ON ip.interaction_id=i.id
                 WHERE ip.person_id=p.id AND i.deleted_at IS NULL),
                (SELECT MAX(i.occurred_at) FROM interactions i
                 JOIN interaction_participants ip ON ip.interaction_id=i.id
                 WHERE ip.person_id=p.id AND i.deleted_at IS NULL),
                EXISTS(SELECT 1 FROM identities i WHERE i.person_id=p.id AND i.is_self=1),
                COALESCE((SELECT GROUP_CONCAT(tag, char(31)) FROM tags WHERE person_id=p.id), ''),
                COALESCE((SELECT GROUP_CONCAT(value, char(31)) FROM identities
                          WHERE person_id=p.id AND (active=1 OR p.lifecycle_state='retired')), '')
         FROM people p
         WHERE p.lifecycle_state IN ('active', 'retired')
           AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)
         ORDER BY p.affinity_score IS NULL, p.affinity_score DESC, p.display_name COLLATE NOCASE",
    )?;
    let people = statement
        .query_map([], |row| {
            Ok(OverviewPerson {
                id: row.get(0)?,
                display_name: row.get(1)?,
                lifecycle_state: row.get(2)?,
                affinity_score: row.get(3)?,
                affinity_tier: row.get(4)?,
                activity_state: row.get(5)?,
                interaction_count: row.get(6)?,
                last_interaction_at: row.get(7)?,
                is_self: row.get::<_, i64>(8)? != 0,
                tags: separated(row.get(9)?),
                identities: separated(row.get(10)?)
                    .into_iter()
                    .map(|value| crate::phone::format_for_display(&value))
                    .collect(),
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Overview {
        people,
        graph: graph::build(connection, None, 0.0)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn overview_includes_searchable_person_context() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-alex', 'active');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('identity', 'person', 'email', 'alex@example.com', 'alex@example.com', 1);
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('phone', 'person', 'phone', '+16264648098', '16264648098', 1);
             INSERT INTO tags(person_id, tag) VALUES ('person', 'friend');
             INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('other', 'Blair', 'apple-blair', 'active');
             INSERT INTO relationships(
                 id, source_person_id, target_person_id, relationship_type,
                 classification_confidence, classification_state,
                 first_observed_at, last_observed_at, shared_context_count
             ) VALUES (
                 'relationship', 'other', 'person', 'friend', 0.9, 'complete',
                 '2026-01-01', '2026-01-01', 1
             );",
            )
            .unwrap();

        let overview = load(&connection).unwrap();

        assert_eq!(overview.people.len(), 2);
        let alex = overview
            .people
            .iter()
            .find(|person| person.id == "person")
            .unwrap();
        assert_eq!(alex.tags, ["friend"]);
        assert_eq!(alex.identities, ["alex@example.com", "+1 (626) 464-8098"]);
        assert_eq!(overview.graph.nodes.len(), 2);
        assert_eq!(overview.graph.edges.len(), 1);
    }
}
