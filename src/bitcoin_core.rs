use std::{error::Error, path::Path};

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::base::Transaction;

#[derive(Debug, Clone, Deserialize)]
enum TransferType {
    #[serde(rename = "Sent to")]
    SentTo,
    #[serde(rename = "Received with")]
    ReceivedWith,
}

#[derive(Debug, Deserialize)]
struct BitcoinCoreAction {
    // #[serde(rename = "Confirmed")]
    // confirmed: bool,
    #[serde(rename = "Date")]
    date: NaiveDateTime,
    #[serde(rename = "Type")]
    type_: TransferType,
    #[serde(rename = "Label")]
    label: String,
    #[serde(rename = "Address")]
    address: String,
    #[serde(rename = "Amount (BTC)")]
    amount: f64,
    #[serde(rename = "ID")]
    id: String,
}

impl From<BitcoinCoreAction> for Transaction {
    // todo: translate address?
    fn from(item: BitcoinCoreAction) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        let mut tx = match item.type_ {
            TransferType::SentTo => {
                Transaction::send(utc_time, -item.amount, "BTC")
            },
            TransferType::ReceivedWith => {
                Transaction::receive(utc_time, item.amount, "BTC")
            },
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label) };
        tx.tx_hash = Some(item.id);
        tx
    }
}

// loads a bitcoin.de CSV file into a list of unified transactions
pub(crate) fn load_bitcoin_core_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinCoreAction = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
