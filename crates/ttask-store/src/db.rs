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
