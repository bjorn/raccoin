use std::{error::Error, path::Path};

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{base::{Transaction, Operation}, time::deserialize_date_time};

#[derive(Debug, Clone, Deserialize)]
pub(crate) enum BitonicActionType {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BitonicAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    pub date: NaiveDateTime,
    #[serde(rename = "Action")]
    pub action: BitonicActionType,
    #[serde(rename = "Amount")]
    pub amount: f64,
    #[serde(rename = "Price")]
    pub price: f64,
}

impl From<BitonicAction> for Transaction {
    fn from(item: BitonicAction) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        match item.action {
            BitonicActionType::Buy => {
                Transaction::buy(
                    utc_time,
                    item.amount,
                    "BTC",
                    -item.price,
                    "EUR")
            },
            BitonicActionType::Sell => {
                Transaction::sell(
                    utc_time,
                    -item.amount,
                    "BTC",
                    item.price,
                    "EUR")
            },
        }
    }
}

// loads a bitonic CSV file into a list of unified transactions
pub(crate) fn load_bitonic_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitonicAction = result?;
        let transaction: Transaction = record.into();

        // Since Bitonic does not hold any fiat or crypto, we add dummy deposit and send transactions
        // for each buy/sell transaction.
        match &transaction.operation {
            Operation::Buy { incoming, outgoing } => {
                transactions.push(Transaction::fiat_deposit(
                    transaction.timestamp - chrono::Duration::minutes(1),
                    outgoing.quantity,
                    &outgoing.currency));

                transactions.push(Transaction::send(
                    transaction.timestamp + chrono::Duration::minutes(1),
                    incoming.quantity,
                    &incoming.currency));
            },
            Operation::Sell { incoming, outgoing } => {
                transactions.push(Transaction::receive(
                    transaction.timestamp - chrono::Duration::minutes(1),
                    outgoing.quantity,
                    &outgoing.currency));

                transactions.push(Transaction::fiat_withdrawal(
                    transaction.timestamp + chrono::Duration::minutes(1),
                    incoming.quantity,
                    &incoming.currency));
            },
            _ => {}
        }

        transactions.push(transaction)
    }

    println!("Imported {} transactions from {}", transactions.len(), input_path.display());

    Ok(transactions)
}
