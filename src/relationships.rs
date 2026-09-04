use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;

pub(crate) struct ConversationMember<'a> {
    pub identity: &'a str,
    pub display_name: Option<&'a str>,
}

pub(crate) fn observe_member(
    connection: &Connection,
    source_id: &str,
    thread_native_id: &str,
    member: ConversationMember<'_>,
) -> Result<()> {
    let person_id = resolve_person(connection, member.identity)?;
    connection.execute(
        "INSERT INTO conversation_memberships(
             source_id, thread_native_id, person_id, identity_value, display_name, active
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1)
         ON CONFLICT(source_id, thread_native_id, identity_value) DO UPDATE SET
           person_id=excluded.person_id,
           display_name=COALESCE(excluded.display_name, conversation_memberships.display_name),
           active=1",
        params![
            source_id,
            thread_native_id,
            person_id,
            member.identity,
            member.display_name
        ],
    )?;
    Ok(())
}

pub(crate) fn replace_members(
    connection: &Connection,
    source_id: &str,
    thread_native_id: &str,
    conversation_title: Option<&str>,
    members: &[ConversationMember<'_>],
) -> Result<()> {
    connection.execute(
        "UPDATE conversation_memberships SET active=0
         WHERE source_id=?1 AND thread_native_id=?2",
        params![source_id, thread_native_id],
    )?;
    for member in members {
        observe_member(
            connection,
            source_id,
            thread_native_id,
            ConversationMember {
                identity: member.identity,
                display_name: member.display_name,
            },
        )?;
    }
    connection.execute(
        "UPDATE conversation_memberships SET conversation_title=?3
         WHERE source_id=?1 AND thread_native_id=?2 AND active=1",
        params![source_id, thread_native_id, conversation_title],
    )?;
    Ok(())
}

pub(crate) fn rebuild_members_from_interactions(
    connection: &Connection,
    source_id: &str,
) -> Result<()> {
    connection.execute(
        "DELETE FROM conversation_memberships WHERE source_id=?1",
        [source_id],
    )?;
    connection.execute(
        "INSERT INTO conversation_memberships(
             source_id, thread_native_id, person_id, identity_value, display_name
         )
         SELECT i.source_id, i.thread_native_id, MAX(ip.person_id),
                COALESCE(ip.identity_value, 'person:' || ip.person_id), MAX(ip.display_name)
         FROM interactions i
         JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE i.source_id=?1 AND i.deleted_at IS NULL AND i.thread_native_id IS NOT NULL
           AND (ip.identity_value IS NOT NULL OR ip.person_id IS NOT NULL)
         GROUP BY i.source_id, i.thread_native_id,
                  COALESCE(ip.identity_value, 'person:' || ip.person_id)",
        [source_id],
    )?;
    Ok(())
}

pub(crate) fn rebind_unresolved_members(connection: &Connection) -> Result<usize> {
    let mut statement = connection.prepare(
        "SELECT source_id, thread_native_id, identity_value
         FROM conversation_memberships WHERE person_id IS NULL",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let mut rebound = 0;
    for (source_id, thread_native_id, identity) in rows {
        if let Some(person_id) = resolve_person(connection, &identity)? {
            rebound += connection.execute(
                "UPDATE conversation_memberships SET person_id=?4
                 WHERE source_id=?1 AND thread_native_id=?2 AND identity_value=?3",
                params![source_id, thread_native_id, identity, person_id],
            )?;
        }
    }
    Ok(rebound)
}

pub(crate) fn reconcile_source(connection: &Connection, source_id: &str) -> Result<usize> {
    let transaction = crate::db::immediate_transaction(connection)?;
    let previous = context_versions(&transaction, source_id)?;
    transaction.execute(
        "DELETE FROM relationship_contexts WHERE source_id=?1",
        [source_id],
    )?;
    transaction.execute(
        "WITH conversation_people AS (
             SELECT DISTINCT cm.thread_native_id, cm.person_id
             FROM conversation_memberships cm
             JOIN people p ON p.id=cm.person_id AND p.lifecycle_state='active'
             WHERE cm.source_id=?1 AND cm.active=1 AND cm.person_id IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM identities own
                 WHERE own.person_id=cm.person_id AND own.is_self=1 AND own.active=1
               )
         ), conversation_activity AS (
             SELECT thread_native_id, MIN(channel) AS channel,
                    MIN(occurred_at) AS first_observed_at,
                    MAX(occurred_at) AS last_observed_at,
                    COUNT(DISTINCT id) AS message_count
             FROM interactions
             WHERE source_id=?1 AND deleted_at IS NULL AND thread_native_id IS NOT NULL
             GROUP BY thread_native_id
         )
         INSERT INTO relationship_contexts(
             source_person_id, target_person_id, source_id, thread_native_id, channel,
             first_observed_at, last_observed_at, message_count
         )
         SELECT a.person_id, b.person_id, ?1, a.thread_native_id, activity.channel,
                activity.first_observed_at, activity.last_observed_at, activity.message_count
         FROM conversation_people a
         JOIN conversation_people b
           ON b.thread_native_id=a.thread_native_id AND b.person_id>a.person_id
         JOIN conversation_activity activity ON activity.thread_native_id=a.thread_native_id",
        [source_id],
    )?;
    let current = context_versions(&transaction, source_id)?;
    transaction.execute(
        "INSERT INTO relationships(
             id, source_person_id, target_person_id, first_observed_at,
             last_observed_at, shared_context_count
         )
         SELECT source_person_id || ':' || target_person_id,
                source_person_id, target_person_id,
                MIN(first_observed_at), MAX(last_observed_at), COUNT(*)
         FROM relationship_contexts
         GROUP BY source_person_id, target_person_id
         ON CONFLICT(source_person_id, target_person_id) DO UPDATE SET
           first_observed_at=excluded.first_observed_at,
           last_observed_at=excluded.last_observed_at,
           shared_context_count=excluded.shared_context_count",
        [],
    )?;
    let pairs = previous
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    for pair in pairs {
        if previous.get(&pair) != current.get(&pair) {
            transaction.execute(
                "UPDATE relationships SET relationship_type='unclear',
                 classification_confidence=NULL, classification_state='pending',
                 classification_evidence='', evidence_message_ids_json='[]',
                 model_version=NULL, prompt_hash=NULL,
                 structure_revision=structure_revision+1
                 WHERE source_person_id=?1 AND target_person_id=?2",
                params![pair.0, pair.1],
            )?;
        }
    }
    transaction.execute(
        "DELETE FROM relationships
         WHERE NOT EXISTS (
           SELECT 1 FROM relationship_contexts rc
           WHERE rc.source_person_id=relationships.source_person_id
             AND rc.target_person_id=relationships.target_person_id
         )",
        [],
    )?;
    let count = transaction.query_row(
        "SELECT COUNT(*) FROM relationships WHERE classification_state='pending'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.commit()?;
    Ok(count as usize)
}

fn context_versions(
    connection: &Connection,
    source_id: &str,
) -> Result<std::collections::HashMap<(String, String), Vec<String>>> {
    let mut statement = connection.prepare(
        "SELECT source_person_id, target_person_id, thread_native_id, channel,
                first_observed_at, last_observed_at, message_count
         FROM relationship_contexts WHERE source_id=?1
         ORDER BY source_person_id, target_person_id, thread_native_id",
    )?;
    let rows = statement
        .query_map([source_id], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                format!(
                    "{}\0{}\0{}\0{}\0{}",
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut versions = std::collections::HashMap::<_, Vec<_>>::new();
    for (pair, version) in rows {
        versions.entry(pair).or_default().push(version);
    }
    Ok(versions)
}

pub(crate) fn reconcile_all(connection: &Connection) -> Result<usize> {
    let mut statement = connection
        .prepare("SELECT DISTINCT source_id FROM conversation_memberships ORDER BY source_id")?;
    let sources: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for source in sources {
        reconcile_source(connection, &source)?;
    }
    connection
        .query_row(
            "SELECT COUNT(*) FROM relationships WHERE classification_state='pending'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(Into::into)
}

fn resolve_person(connection: &Connection, identity: &str) -> Result<Option<String>> {
    let normalized = crate::repository::normalize_observed_identity(identity);
    connection
        .query_row(
            "SELECT i.person_id FROM identities i JOIN people p ON p.id=i.person_id
             WHERE i.normalized_value=?1 AND i.active=1 AND p.lifecycle_state='active'
             LIMIT 1",
            [normalized],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

#[cfg(test)]
#[path = "relationships/tests.rs"]
mod tests;
