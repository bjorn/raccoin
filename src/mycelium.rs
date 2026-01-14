use std::path::Path;

use anyhow::Result;
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use csv::Trim;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount}, CsvSpec, TransactionSourceType};
use linkme::distributed_slice;

// serialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%MZ").unwrap())
}

// Stores values loaded from CSV file exported by Mycelium, with the following header:
// Account, Transaction ID, Destination Address, Timestamp, Value, Currency, Transaction Label
#[derive(Debug, Deserialize)]
struct MyceliumTransaction {
    // #[serde(rename = "Account")]
    // account: String,
    #[serde(rename = "Transaction ID")]
    id: String,
    // #[serde(rename = "Destination Address")]
    // address: String,
    #[serde(rename = "Timestamp", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(rename = "Value")]
    value: Decimal,
    // #[serde(rename = "Currency")]
    // currency: String,
    #[serde(rename = "Transaction Label")]
    label: String,
}

impl From<MyceliumTransaction> for Transaction {
    // todo: translate address?
    fn from(item: MyceliumTransaction) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.timestamp).unwrap().naive_utc();
        let mut tx = if item.value < Decimal::ZERO {
            Transaction::send(utc_time, Amount::new(-item.value, "BTC".to_owned()))
        } else {
            Transaction::receive(utc_time, Amount::new(item.value, "BTC".to_owned()))
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label) };
        tx.tx_hash = Some(item.id);
        tx.blockchain = Some("BTC".to_owned());
        tx
    }
}

// loads a Mycelium CSV file into a list of unified transactions
fn load_mycelium_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .trim(Trim::Headers)
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: MyceliumTransaction = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static MYCELIUM_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "MyceliumCsv",
    label: "Mycelium (CSV)",
    csv: &[CsvSpec::new(&[
        "Account",
        "Transaction ID",
        "Destination Address",
        "Timestamp",
        "Value",
        "Currency",
        "Transaction Label",
    ])],
    detect: None,
    load_sync: Some(load_mycelium_csv),
    load_async: None,
};
