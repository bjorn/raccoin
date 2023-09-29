use chrono::NaiveDateTime;
use serde::{Deserializer, Serializer, Deserialize};

const FORMAT: &'static str = "%Y-%m-%d %H:%M:%S";

// deserialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(&raw, FORMAT).unwrap())
}
pub(crate) fn serialize_date_time<S: Serializer>(date: &NaiveDateTime, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&date.format(FORMAT).to_string())
}
