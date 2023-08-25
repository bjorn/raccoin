use chrono::NaiveDateTime;
use serde::{Deserializer, Serializer, Deserialize};

// serialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(&raw, "%Y-%m-%d %H:%M:%S").unwrap())
}
pub(crate) fn serialize_date_time<S: Serializer>(date: &NaiveDateTime, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&date.format("%Y-%m-%d %H:%M:%S").to_string())
}
