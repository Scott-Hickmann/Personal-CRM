from contextlib import closing
from pathlib import Path
import sqlite3


def load_live(database, interaction_id):
    uri = f"file:{Path(database).resolve()}?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        row = connection.execute(
            "SELECT channel, occurred_at, direction, subject, body FROM interactions "
            "WHERE id=? AND deleted_at IS NULL",
            [interaction_id],
        ).fetchone()
        if row is None:
            raise ValueError(f"interaction not found: {interaction_id}")
        people = connection.execute(
            "SELECT p.display_name, GROUP_CONCAT(DISTINCT ip.role) "
            "FROM interaction_participants ip JOIN people p ON p.id=ip.person_id "
            "WHERE ip.interaction_id=? AND p.lifecycle_state='active' "
            "AND p.apple_contact_id IS NOT NULL AND NOT EXISTS "
            "(SELECT 1 FROM identities own WHERE own.person_id=p.id AND own.is_self=1 AND own.active=1) "
            "GROUP BY p.id, p.display_name ORDER BY p.display_name COLLATE NOCASE, p.id",
            [interaction_id],
        ).fetchall()
    interaction = {
        "interaction_id": "item-0",
        "channel": row[0],
        "occurred_at": row[1],
        "direction": row[2],
        "subject": row[3],
        "body": row[4][:6000],
        "participants": [
            {"participant_id": f"participant-{index}", "display_name": name, "role": role}
            for index, (name, role) in enumerate(people)
        ],
    }
    return {"name": "live interaction", "input": {"interactions": [interaction]}, "expect": {}}
