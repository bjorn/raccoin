use std::{error::Error, path::Path};

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{time::deserialize_date_time, base::{Transaction, Amount}};

#[derive(Debug, Deserialize)]
struct ElectrumHistoryItem {
    transaction_hash: String,
    label: String,
    // confirmations: u64,
    value: Decimal,
    // fiat_value: Decimal,
    fee: Option<Decimal>,
    // fiat_fee: Option<Decimal>,
    #[serde(deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
}

impl From<ElectrumHistoryItem> for Transaction {
    fn from(item: ElectrumHistoryItem) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.timestamp).unwrap().naive_utc();
        let mut tx = if item.value < Decimal::ZERO {
            let amount = -item.value - item.fee.unwrap_or_default();
            Transaction::send(utc_time, Amount::new(amount, "BTC".to_owned()))
        } else {
            Transaction::receive(utc_time, Amount::new(item.value, "BTC".to_owned()))
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label) };
        tx.tx_hash = Some(item.transaction_hash);
        tx.blockchain = Some("BTC".to_owned());
        tx.fee = item.fee.map(|f| Amount::new(f, "BTC".to_string()));
        tx
    }
}

// loads an Electrum CSV file into a list of unified transactions
pub(crate) fn load_electrum_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: ElectrumHistoryItem = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
