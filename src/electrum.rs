use std::error::Error;

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time, base::{Transaction, Amount}};

#[derive(Debug, Deserialize)]
struct ElectrumHistoryItem {
    transaction_hash: String,
    label: String,
    // confirmations: u64,
    value: f64,
    // fiat_value: f64,
    fee: Option<f64>,
    // fiat_fee: Option<f64>,
    #[serde(deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
}

impl From<ElectrumHistoryItem> for Transaction {
    fn from(item: ElectrumHistoryItem) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.timestamp).unwrap().naive_utc();
        let mut tx = if item.value < 0.0 {
            let amount = -item.value - item.fee.unwrap_or(0.0);
            Transaction::send(utc_time, amount, "BTC")
        } else {
            Transaction::receive(utc_time, item.value, "BTC")
        };
        tx.description = item.label;
        tx.tx_hash = Some(item.transaction_hash);
        tx.fee = item.fee.and_then(|f| Some(Amount { quantity: f, currency: "BTC".to_string() }));
        tx
    }
}

// loads an Electrum CSV file into a list of unified transactions
pub(crate) fn load_electrum_csv(input_path: &str) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    println!("Loading {}", input_path);

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: ElectrumHistoryItem = result?;
        transactions.push(record.into());
        println!("Loaded {:?}", transactions.last().unwrap());
    }

    Ok(transactions)
}

pub(crate) fn convert_electrum_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: ElectrumHistoryItem = result?;
        let utc_time = Berlin.from_local_datetime(&record.timestamp).unwrap().naive_utc();

        wtr.serialize(CtcTx {
            id: Some(&record.transaction_hash),
            description: Some(&record.label),
            fee_amount: record.fee,
            fee_currency: if record.fee.is_none() { None } else { Some("BTC") },
            blockchain: Some("BTC"),
            ..CtcTx::new(
                utc_time,
                if record.value < 0.0 { CtcTxType::Send } else { CtcTxType::Receive },
                "BTC",
                record.value.abs() - record.fee.unwrap_or(0.0))
        })?;
    }

    Ok(())
}
