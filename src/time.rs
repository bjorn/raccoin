use chrono::NaiveDateTime;
use serde::{Deserializer, Serializer, Deserialize};

const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub(crate) fn parse_date_time(raw: &str) -> std::result::Result<NaiveDateTime, chrono::ParseError> {
    NaiveDateTime::parse_from_str(raw.trim(), FORMAT)
}

// deserialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    parse_date_time(raw).map_err(serde::de::Error::custom)
}
pub(crate) fn serialize_date_time<S: Serializer>(date: &NaiveDateTime, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&date.format(FORMAT).to_string())
}
