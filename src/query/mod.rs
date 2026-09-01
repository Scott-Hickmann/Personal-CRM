mod filter;

use rusqlite::Connection;
use rusqlite::params_from_iter;
use rusqlite::types::ValueRef;
use serde::Serialize;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Entity {
    People,
    Interactions,
    Relationships,
    Mentions,
    Followups,
    Sources,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub entity: String,
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
}

pub struct QueryOptions<'a> {
    pub select: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub group: Option<&'a str>,
    pub limit: u32,
}

pub fn execute(
    connection: &Connection,
    entity: Entity,
    options: QueryOptions<'_>,
) -> Result<QueryResult> {
    if options.limit == 0 || options.limit > 10_000 {
        return Err(CrmError::InvalidQuery(
            "limit must be between 1 and 10000".into(),
        ));
    }
    let specification = specification(entity);
    let (select_sql, columns) = if let Some(group) = options.group {
        let field = resolve_field(specification, group)?;
        (
            format!("{field} AS \"{group}\", COUNT(*) AS \"count\""),
            vec![group.into(), "count".into()],
        )
    } else {
        select_fields(specification, options.select)?
    };
    let mut sql = format!("SELECT {select_sql} FROM {}", specification.from);
    let mut params = Vec::new();
    if let Some(expression) = options.filter {
        let (where_sql, where_params) =
            filter::compile(expression, |field| lookup_field(specification, field))?;
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
        params = where_params;
    }
    if options.group.is_some() {
        sql.push_str(" GROUP BY 1");
    }
    if let Some(sort) = options.sort {
        sql.push_str(" ORDER BY ");
        sql.push_str(&sort_fields(specification, sort, options.group)?);
    }
    sql.push_str(" LIMIT ?");
    params.push(rusqlite::types::Value::Integer(i64::from(options.limit)));
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(params_from_iter(params), |row| {
            let mut object = serde_json::Map::new();
            for (index, column) in columns.iter().enumerate() {
                object.insert(column.clone(), json_value(row.get_ref(index)?));
            }
            Ok(serde_json::Value::Object(object))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(QueryResult {
        entity: specification.name.into(),
        columns,
        rows,
    })
}

struct Specification {
    name: &'static str,
    from: &'static str,
    defaults: &'static [&'static str],
    fields: &'static [(&'static str, &'static str)],
}

fn specification(entity: Entity) -> &'static Specification {
    match entity {
        Entity::People => &PEOPLE,
        Entity::Interactions => &INTERACTIONS,
        Entity::Relationships => &RELATIONSHIPS,
        Entity::Mentions => &MENTIONS,
        Entity::Followups => &FOLLOWUPS,
        Entity::Sources => &SOURCES,
    }
}

fn select_fields(spec: &Specification, select: Option<&str>) -> Result<(String, Vec<String>)> {
    let names: Vec<_> = select
        .map(|value| value.split(',').map(str::trim).collect())
        .unwrap_or_else(|| spec.defaults.to_vec());
    let sql = names
        .iter()
        .map(|name| resolve_field(spec, name).map(|field| format!("{field} AS \"{name}\"")))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    Ok((sql, names.into_iter().map(str::to_owned).collect()))
}

fn sort_fields(spec: &Specification, sort: &str, group: Option<&str>) -> Result<String> {
    sort.split(',')
        .map(str::trim)
        .map(|value| {
            let (direction, name) = match value.as_bytes().first() {
                Some(b'-') => ("DESC", &value[1..]),
                Some(b'+') => ("ASC", &value[1..]),
                _ => ("ASC", value),
            };
            if name == "count" && group.is_some() {
                return Ok(format!("count {direction}"));
            }
            resolve_field(spec, name).map(|field| format!("{field} {direction}"))
        })
        .collect::<Result<Vec<_>>>()
        .map(|fields| fields.join(", "))
}

fn resolve_field(spec: &Specification, name: &str) -> Result<&'static str> {
    lookup_field(spec, name)
        .ok_or_else(|| CrmError::InvalidQuery(format!("unknown field `{name}` for {}", spec.name)))
}
fn lookup_field(spec: &Specification, name: &str) -> Option<&'static str> {
    spec.fields
        .iter()
        .find_map(|(candidate, sql)| (*candidate == name).then_some(*sql))
}

fn json_value(value: ValueRef<'_>) -> serde_json::Value {
    match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(value) => value.into(),
        ValueRef::Real(value) => serde_json::json!(value),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned().into(),
        ValueRef::Blob(value) => format!("<{} bytes>", value.len()).into(),
    }
}

const PEOPLE: Specification = Specification {
    name: "people",
    from: "(SELECT * FROM people WHERE lifecycle_state='active') p",
    defaults: &[
        "id",
        "display_name",
        "affinity_score",
        "affinity_tier",
        "activity_state",
        "days_since_meaningful_interaction",
    ],
    fields: &[
        ("id", "p.id"),
        ("display_name", "p.display_name"),
        ("affinity_score", "p.affinity_score"),
        ("affinity_tier", "p.affinity_tier"),
        ("activity_state", "p.activity_state"),
        ("apple_contact_id", "p.apple_contact_id"),
        ("lifecycle_state", "p.lifecycle_state"),
        (
            "is_self",
            "EXISTS(SELECT 1 FROM identities self_identity WHERE self_identity.person_id=p.id AND self_identity.is_self=1)",
        ),
        (
            "days_since_meaningful_interaction",
            "CAST(julianday('now') - julianday((SELECT MAX(i.occurred_at) FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id WHERE ip.person_id=p.id AND i.deleted_at IS NULL)) AS INTEGER)",
        ),
        (
            "interaction_count",
            "(SELECT COUNT(*) FROM interaction_participants ip JOIN interactions i ON i.id=ip.interaction_id WHERE ip.person_id=p.id AND i.deleted_at IS NULL)",
        ),
    ],
};
const INTERACTIONS: Specification = Specification {
    name: "interactions",
    from: "interactions i",
    defaults: &[
        "id",
        "source_id",
        "channel",
        "kind",
        "occurred_at",
        "direction",
        "subject",
    ],
    fields: &[
        ("id", "i.id"),
        ("source_id", "i.source_id"),
        ("native_id", "i.native_id"),
        ("channel", "i.channel"),
        ("kind", "i.kind"),
        ("occurred_at", "i.occurred_at"),
        ("direction", "i.direction"),
        ("subject", "i.subject"),
        ("body", "i.body"),
        ("deleted_at", "i.deleted_at"),
        ("analysis_state", "i.analysis_state"),
    ],
};
const RELATIONSHIPS: Specification = Specification {
    name: "relationships",
    from: "relationships r",
    defaults: &[
        "id",
        "source_person_id",
        "target_person_id",
        "relationship_type",
        "confidence",
        "status",
    ],
    fields: &[
        ("id", "r.id"),
        ("source_person_id", "r.source_person_id"),
        ("target_person_id", "r.target_person_id"),
        ("relationship_type", "r.relationship_type"),
        ("confidence", "r.confidence"),
        ("status", "r.status"),
        ("last_observed_at", "r.last_observed_at"),
    ],
};
const MENTIONS: Specification = Specification {
    name: "mentions",
    from: "mentions m",
    defaults: &[
        "id",
        "interaction_id",
        "text",
        "person_id",
        "confidence",
        "status",
    ],
    fields: &[
        ("id", "m.id"),
        ("interaction_id", "m.interaction_id"),
        ("text", "m.text"),
        ("person_id", "m.person_id"),
        ("confidence", "m.confidence"),
        ("status", "m.status"),
    ],
};
const FOLLOWUPS: Specification = Specification {
    name: "followups",
    from: "followups f",
    defaults: &["id", "person_id", "body", "due_at", "completed_at"],
    fields: &[
        ("id", "f.id"),
        ("person_id", "f.person_id"),
        ("body", "f.body"),
        ("due_at", "f.due_at"),
        ("completed_at", "f.completed_at"),
        ("created_at", "f.created_at"),
    ],
};
const SOURCES: Specification = Specification {
    name: "sources",
    from: "sources s",
    defaults: &["id", "kind", "account", "status", "last_sync_at", "error"],
    fields: &[
        ("id", "s.id"),
        ("kind", "s.kind"),
        ("account", "s.account"),
        ("status", "s.status"),
        ("last_sync_at", "s.last_sync_at"),
        ("last_reconcile_at", "s.last_reconcile_at"),
        ("error", "s.error"),
    ],
};
