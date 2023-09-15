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
    // #[serde(rename = "Address")]
    // address: String,
    #[serde(rename = "Amount (BTC)", alias = "Amount (PPC)")]
    amount: f64,
    #[serde(rename = "ID")]
    id: String,
}

impl BitcoinCoreAction {
    // todo: translate address?
    fn to_tx(self, currency: &str) -> Transaction {
        let utc_time = Berlin.from_local_datetime(&self.date).unwrap().naive_utc();
        let mut tx = match self.type_ {
            TransferType::SentTo => {
                Transaction::send(utc_time, -self.amount, currency)
            },
            TransferType::ReceivedWith => {
                Transaction::receive(utc_time, self.amount, currency)
            },
        };
        tx.description = if self.label.is_empty() { None } else { Some(self.label) };
        tx.tx_hash = Some(self.id);
        tx
    }
}

// loads a Bitcoin Core CSV file into a list of unified transactions
fn load_transactions(input_path: &Path, currency: &str) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinCoreAction = result?;
        transactions.push(record.to_tx(currency));
    }

    Ok(transactions)
}

// loads a Bitcoin Core CSV file into a list of unified transactions
pub(crate) fn load_bitcoin_core_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    load_transactions(input_path, "BTC")
}

// loads a Peercoin CSV file into a list of unified transactions
pub(crate) fn load_peercoin_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    load_transactions(input_path, "PPC")
}
