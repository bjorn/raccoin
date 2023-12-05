use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use serde::{Deserialize, Deserializer};

use crate::base::{Transaction, Amount, deserialize_amount};

#[derive(Debug, Deserialize)]
enum BitstampTransactionType {
    Market,
    Withdrawal,
    Deposit,
}

#[derive(Debug, Deserialize)]
enum SubType {
    Buy,
    Sell,
}

// deserialize function for reading NaiveDateTime in the format "Jan. 27, 2017, 03:28 PM"
fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(raw, "%b. %d, %Y, %I:%M %p").unwrap())
}

fn deserialize_amount_opt<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<Amount>, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(Amount::try_from(raw).unwrap()))
}

// struct for storing the following CSV columns:
// Type,Datetime,Account,Amount,Value,Rate,Fee,Sub Type
#[derive(Debug, Deserialize)]
struct BitstampTransaction {
    #[serde(rename = "Type")]
    pub type_: BitstampTransactionType,
    #[serde(rename = "Datetime", deserialize_with = "deserialize_date_time")]
    pub datetime: NaiveDateTime,
    // #[serde(rename = "Account")]
    // pub account: String,
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    pub amount: Amount,
    #[serde(rename = "Value", deserialize_with = "deserialize_amount_opt")]
    pub value: Option<Amount>,
    // #[serde(rename = "Rate", deserialize_with = "deserialize_amount_opt")]
    // pub rate: Option<Amount>,
    #[serde(rename = "Fee", deserialize_with = "deserialize_amount_opt")]
    pub fee: Option<Amount>,
    #[serde(rename = "Sub Type")]
    pub sub_type: Option<SubType>,
}

impl TryFrom<BitstampTransaction> for Transaction {
    type Error = &'static str;

    fn try_from(item: BitstampTransaction) -> Result<Self, Self::Error> {
        let mut tx = match item.type_ {
            BitstampTransactionType::Market => {
                match (item.sub_type, item.value) {
                    (Some(SubType::Buy), Some(value)) => Ok(Transaction::trade(item.datetime, item.amount, value)),
                    (Some(SubType::Sell), Some(value)) => Ok(Transaction::trade(item.datetime, value, item.amount)),
                    _ => Err("Missing Sub Type and/or Value for Market transaction"),
                }
            }
            BitstampTransactionType::Withdrawal => {
                Ok(if item.amount.is_fiat() {
                    Transaction::fiat_withdrawal(item.datetime, item.amount)
                } else {
                    Transaction::send(item.datetime, item.amount)
                })
            }
            BitstampTransactionType::Deposit => {
                Ok(if item.amount.is_fiat() {
                    Transaction::fiat_deposit(item.datetime, item.amount)
                } else {
                    Transaction::receive(item.datetime, item.amount)
                })
            }
        }?;

        tx.fee = item.fee;
        Ok(tx)
    }
}

// loads a Bitstamp CSV file into a list of unified transactions
pub(crate) fn load_bitstamp_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitstampTransaction = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(_) => continue,
        };
    }

    Ok(transactions)
}
