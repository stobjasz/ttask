use rusqlite::Connection;
use rusqlite::types::Type;
use ttask_core::jiff::civil::{Date, Time};

/// Repository access over one connection or transaction. Obtain it from
/// [`Store::db`](crate::Store::db) or [`Store::write`](crate::Store::write).
pub struct Db<'a> {
    pub(crate) conn: &'a Connection,
}

impl Db<'_> {
    /// Raw connection, for maintenance code and tests.
    pub fn conn(&self) -> &Connection {
        self.conn
    }

    /// A small stored setting (e.g. the active context).
    pub fn meta(&self, key: &str) -> crate::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Sets or (with `None`) removes a stored setting.
    pub fn set_meta(&self, key: &str, value: Option<&str>) -> crate::Result<()> {
        match value {
            Some(v) => self.conn.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, v],
            )?,
            None => self
                .conn
                .execute("DELETE FROM meta WHERE key = ?1", [key])?,
        };
        Ok(())
    }
}

pub(crate) fn date_sql(date: Option<Date>) -> Option<String> {
    date.map(|d| d.to_string())
}

pub(crate) fn time_sql(time: Option<Time>) -> Option<String> {
    time.map(ttask_core::date::format_time)
}

pub(crate) fn parse_date_col(
    value: Option<String>,
    index: usize,
) -> rusqlite::Result<Option<Date>> {
    value
        .map(|s| {
            s.parse::<Date>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(e))
            })
        })
        .transpose()
}

pub(crate) fn parse_time_col(
    value: Option<String>,
    index: usize,
) -> rusqlite::Result<Option<Time>> {
    value
        .map(|s| {
            ttask_core::date::parse_time(&s).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    Type::Text,
                    format!("bad time `{s}`").into(),
                )
            })
        })
        .transpose()
}

pub(crate) fn conversion_error(
    index: usize,
    sql_type: Type,
    err: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, sql_type, err.into())
}
