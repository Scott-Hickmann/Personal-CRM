use rusqlite::Connection;

use crate::error::Result;

pub(crate) fn mark_all_dirty(connection: &Connection, reason: &str) -> Result<usize> {
    connection
        .execute(
            "INSERT INTO dirty_people(person_id, reason, dirty_at)
             SELECT p.id, ?1, CURRENT_TIMESTAMP FROM people p
             WHERE p.lifecycle_state='active' AND NOT EXISTS (
               SELECT 1 FROM identities i
               WHERE i.person_id=p.id AND i.is_self=1 AND i.active=1
             )
             ON CONFLICT(person_id) DO UPDATE SET
               reason=excluded.reason, dirty_at=excluded.dirty_at",
            [reason],
        )
        .map_err(Into::into)
}

pub(crate) fn mark_dirty_for_sources(
    connection: &Connection,
    all_people: bool,
    sources: &[String],
) -> Result<usize> {
    if all_people {
        return mark_all_dirty(connection, "contacts changed");
    }
    let mut marked = 0;
    for source in sources {
        marked += connection.execute(
            "INSERT INTO dirty_people(person_id, reason, dirty_at)
             SELECT DISTINCT ip.person_id, 'interactions changed', CURRENT_TIMESTAMP
             FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id
             JOIN people p ON p.id=ip.person_id AND p.lifecycle_state='active'
             WHERE i.source_id=?1 AND ip.person_id IS NOT NULL AND NOT EXISTS (
               SELECT 1 FROM identities own
               WHERE own.person_id=ip.person_id AND own.is_self=1 AND own.active=1
             )
             ON CONFLICT(person_id) DO UPDATE SET
               reason=excluded.reason, dirty_at=excluded.dirty_at",
            [source],
        )?;
    }
    Ok(marked)
}
