use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::base::{Transaction, Amount};

#[derive(Debug, Clone, Deserialize)]
enum TrezorTransactionType {
    #[serde(rename = "SENT")]
    Sent,
    #[serde(rename = "RECV")]
    Received,
}

#[derive(Debug, Clone, Deserialize)]
enum TrezorAmount {
    Quantity(Decimal),
    TokenId(String),
}

fn deserialize_amount<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<TrezorAmount, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    match Decimal::try_from(raw) {
        Ok(quantity) => Ok(TrezorAmount::Quantity(quantity)),
        Err(_) => Ok(TrezorAmount::TokenId(raw.trim_start_matches("ID ").to_owned())),
    }
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
    fee: Option<Decimal>,
    #[serde(rename = "Fee unit")]
    fee_unit: Option<&'a str>,
    // #[serde(rename = "Address")]
    // address: &'a str,
    #[serde(rename = "Label")]
    label: &'a str,
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    amount: TrezorAmount,
    #[serde(rename = "Amount unit")]
    amount_unit: &'a str,
    // #[serde(rename = "Fiat (EUR)")]
    // fiat_eur: Decimal,
    // #[serde(rename = "Other")]
    // other: &'a str,
}

impl<'a> From<TrezorTransaction<'a>> for Transaction {
    // todo: translate address?
    fn from(item: TrezorTransaction) -> Self {
        let date_time = NaiveDateTime::from_timestamp_opt(item.timestamp, 0).expect("valid timestamp");
        let amount = match item.amount {
            TrezorAmount::Quantity(quantity) => Amount::new(quantity, item.amount_unit.to_owned()),
            TrezorAmount::TokenId(token_id) => Amount::new_token(token_id, item.amount_unit.to_owned()),
        };
        let mut tx = match item.type_ {
            TrezorTransactionType::Sent => Transaction::send(date_time, amount),
            TrezorTransactionType::Received => Transaction::receive(date_time, amount),
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label.to_owned()) };
        tx.tx_hash = Some(item.id.to_owned());
        tx.blockchain = Some(item.amount_unit.to_owned());
        tx.fee = match (item.fee, item.fee_unit) {
            (None, None) |
            (None, Some(_)) => None,
            (Some(fee), None) => {
                println!("warning: ignoring fee of {}, missing fee unit", fee);
                None
            }
            (Some(fee), Some(fee_unit)) => Some(Amount::new(fee, fee_unit.to_owned())),
        };
        tx
    }
}

// loads a TREZOR Suite CSV file into a list of unified transactions
pub(crate) fn load_trezor_csv(input_path: &Path) -> Result<Vec<Transaction>> {
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
