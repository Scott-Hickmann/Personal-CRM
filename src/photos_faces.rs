use std::path::Path;

use rusqlite::params;

use crate::error::{CrmError, Result};
use crate::source::ReadOnlySource;

const SAMPLES_PER_PERSON: i64 = 3;
const SUPPORTED_FACEPRINT_VERSION: i64 = 15;

pub(crate) struct PhotoFaceprints {
    pub named_people: usize,
    pub references: Vec<PhotoFaceprint>,
}

pub(crate) struct PhotoFaceprint {
    pub person_id: String,
    pub name: String,
    pub data: Vec<u8>,
}

pub(crate) fn load(database_path: &Path) -> Result<PhotoFaceprints> {
    let source = ReadOnlySource::open(database_path)?;
    source.require_columns(
        "ZPERSON",
        &["Z_PK", "ZDISPLAYNAME", "ZPERSONUUID", "ZKEYFACE"],
    )?;
    source.require_columns(
        "ZDETECTEDFACE",
        &[
            "Z_PK",
            "ZPERSONFORFACE",
            "ZFACEPRINT",
            "ZQUALITY",
            "ZHIDDEN",
            "ZISINTRASH",
        ],
    )?;
    source.require_columns(
        "ZDETECTEDFACEPRINT",
        &["Z_PK", "ZFACEPRINTVERSION", "ZDATA"],
    )?;

    let connection = source.connection();
    let incompatible: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM ZDETECTEDFACEPRINT fp
         JOIN ZDETECTEDFACE f ON f.ZFACEPRINT = fp.Z_PK
         JOIN ZPERSON p ON p.Z_PK = f.ZPERSONFORFACE
         WHERE COALESCE(TRIM(p.ZDISPLAYNAME), '') <> ''
           AND fp.ZFACEPRINTVERSION <> ?1",
        [SUPPORTED_FACEPRINT_VERSION],
        |row| row.get(0),
    )?;
    if incompatible > 0 {
        return Err(CrmError::IncompatibleSource(format!(
            "Photos contains a faceprint version other than {SUPPORTED_FACEPRINT_VERSION}"
        )));
    }

    let named_people = connection.query_row(
        "SELECT COUNT(*) FROM ZPERSON WHERE COALESCE(TRIM(ZDISPLAYNAME), '') <> ''",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let named_people = usize::try_from(named_people).map_err(|_| {
        CrmError::IncompatibleSource("Photos returned an invalid named-person count".into())
    })?;
    let mut statement = connection.prepare(
        "WITH ranked AS (
             SELECT p.ZPERSONUUID AS person_id,
                    p.ZDISPLAYNAME AS name,
                    fp.ZDATA AS data,
                    ROW_NUMBER() OVER (
                        PARTITION BY p.Z_PK
                        ORDER BY CASE WHEN p.ZKEYFACE = f.Z_PK THEN 0 ELSE 1 END,
                                 COALESCE(f.ZQUALITY, 0) DESC
                    ) AS sample_rank
             FROM ZPERSON p
             JOIN ZDETECTEDFACE f ON f.ZPERSONFORFACE = p.Z_PK
             JOIN ZDETECTEDFACEPRINT fp ON fp.Z_PK = f.ZFACEPRINT
             WHERE COALESCE(TRIM(p.ZDISPLAYNAME), '') <> ''
               AND p.ZPERSONUUID IS NOT NULL
               AND fp.ZDATA IS NOT NULL
               AND fp.ZFACEPRINTVERSION = ?1
               AND COALESCE(f.ZHIDDEN, 0) = 0
               AND COALESCE(f.ZISINTRASH, 0) = 0
         )
         SELECT person_id, name, data
         FROM ranked
         WHERE sample_rank <= ?2
         ORDER BY name COLLATE NOCASE, person_id, sample_rank",
    )?;
    let references = statement
        .query_map(
            params![SUPPORTED_FACEPRINT_VERSION, SAMPLES_PER_PERSON],
            |row| {
                Ok(PhotoFaceprint {
                    person_id: row.get(0)?,
                    name: row.get(1)?,
                    data: row.get(2)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(PhotoFaceprints {
        named_people,
        references,
    })
}
