//! Conversions between domain types and SQLite column values.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Row;
use rusqlite::types::Type;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::model::Timestamp;

pub fn ts(value: &Timestamp) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn opt_ts(value: &Option<Timestamp>) -> Option<String> {
    value.as_ref().map(ts)
}

pub fn id(value: &Uuid) -> String {
    value.to_string()
}

pub fn opt_id(value: &Option<Uuid>) -> Option<String> {
    value.as_ref().map(Uuid::to_string)
}

pub fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn conversion_error(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

/// Typed column accessors for domain values stored as text.
pub trait RowExt {
    fn uuid(&self, index: usize) -> rusqlite::Result<Uuid>;
    fn opt_uuid(&self, index: usize) -> rusqlite::Result<Option<Uuid>>;
    fn timestamp(&self, index: usize) -> rusqlite::Result<Timestamp>;
    fn opt_timestamp(&self, index: usize) -> rusqlite::Result<Option<Timestamp>>;
    fn json<T: DeserializeOwned>(&self, index: usize) -> rusqlite::Result<T>;
}

impl RowExt for Row<'_> {
    fn uuid(&self, index: usize) -> rusqlite::Result<Uuid> {
        let text: String = self.get(index)?;
        Uuid::parse_str(&text).map_err(|error| conversion_error(index, error))
    }

    fn opt_uuid(&self, index: usize) -> rusqlite::Result<Option<Uuid>> {
        let text: Option<String> = self.get(index)?;
        text.map(|value| Uuid::parse_str(&value).map_err(|error| conversion_error(index, error)))
            .transpose()
    }

    fn timestamp(&self, index: usize) -> rusqlite::Result<Timestamp> {
        let text: String = self.get(index)?;
        parse_timestamp(&text).map_err(|error| conversion_error(index, error))
    }

    fn opt_timestamp(&self, index: usize) -> rusqlite::Result<Option<Timestamp>> {
        let text: Option<String> = self.get(index)?;
        text.map(|value| parse_timestamp(&value).map_err(|error| conversion_error(index, error)))
            .transpose()
    }

    fn json<T: DeserializeOwned>(&self, index: usize) -> rusqlite::Result<T> {
        let text: String = self.get(index)?;
        serde_json::from_str(&text).map_err(|error| conversion_error(index, error))
    }
}

pub fn parse_timestamp(text: &str) -> Result<Timestamp, chrono::ParseError> {
    DateTime::parse_from_rfc3339(text).map(|value| value.with_timezone(&Utc))
}
