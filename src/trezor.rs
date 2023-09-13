use std::{error::Error, path::Path};

use chrono::NaiveDateTime;
use serde::Deserialize;

use crate::base::{Transaction, Amount};

#[derive(Debug, Clone, Deserialize)]
enum TrezorTransactionType {
    #[serde(rename = "SENT")]
    Sent,
    #[serde(rename = "RECV")]
    Received,
}

// Stores values loaded from CSV file exported by TREZOR Suite, with the following header:
// Timestamp;Date;Time;Type;Transaction ID;Fee;Fee unit;Address;Label;Amount;Amount unit;Fiat (EUR);Other
#[derive(Debug, Deserialize)]
struct TrezorTransaction<'a> {
    #[serde(rename = "Timestamp")]
    timestamp: i64,
    // #[serde(rename = "Date")]
    // date: &'a str,
    // #[serde(rename = "Time")]
    // time: &'a str,
    #[serde(rename = "Type")]
    type_: TrezorTransactionType,
    #[serde(rename = "Transaction ID")]
    id: &'a str,
    #[serde(rename = "Fee")]
    fee: Option<f64>,
    #[serde(rename = "Fee unit")]
    fee_unit: Option<&'a str>,
    // #[serde(rename = "Address")]
    // address: &'a str,
    #[serde(rename = "Label")]
    label: &'a str,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Amount unit")]
    amount_unit: &'a str,
    // #[serde(rename = "Fiat (EUR)")]
    // fiat_eur: f64,
    // #[serde(rename = "Other")]
    // other: &'a str,
}

impl<'a> From<TrezorTransaction<'a>> for Transaction {
    // todo: translate address?
    fn from(item: TrezorTransaction) -> Self {
        let date_time = NaiveDateTime::from_timestamp_opt(item.timestamp, 0).expect("valid timestamp");
        let mut tx = match item.type_ {
            TrezorTransactionType::Sent => {
                Transaction::send(date_time, item.amount, &item.amount_unit)
            },
            TrezorTransactionType::Received => {
                Transaction::receive(date_time, item.amount, &item.amount_unit)
            },
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label.to_owned()) };
        tx.tx_hash = Some(item.id.to_owned());
        tx.fee = if let Some(fee) = item.fee {
            Some(Amount { quantity: fee, currency: item.fee_unit.unwrap().to_owned() })
        } else {
            None
        };
        tx
    }
}

// loads a TREZOR Suite CSV file into a list of unified transactions
pub(crate) fn load_trezor_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;
    let mut raw_record = csv::StringRecord::new();
    let headers = rdr.headers()?.clone();

    while rdr.read_record(&mut raw_record)? {
        let record: TrezorTransaction = raw_record.deserialize(Some(&headers))?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
