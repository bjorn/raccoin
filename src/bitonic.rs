use std::{error::Error, path::Path};

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{base::{Transaction, Operation, Amount}, time::deserialize_date_time};

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
    pub amount: Decimal,
    #[serde(rename = "Price")]
    pub price: Decimal,
}

impl From<BitonicAction> for Transaction {
    fn from(item: BitonicAction) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        match item.action {
            BitonicActionType::Buy => {
                Transaction::trade(
                    utc_time,
                    Amount { quantity: item.amount, currency: "BTC".to_owned() },
                    Amount { quantity: -item.price, currency: "EUR".to_owned() }
                )
            },
            BitonicActionType::Sell => {
                Transaction::trade(
                    utc_time,
                    Amount { quantity: item.price, currency: "EUR".to_owned() },
                    Amount { quantity: -item.amount, currency: "BTC".to_owned() },
                )
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
        if let Operation::Trade { incoming, outgoing } = &transaction.operation {
            if incoming.is_fiat() {
                transactions.push(Transaction::receive(
                    transaction.timestamp - chrono::Duration::minutes(1),
                    Amount::new(outgoing.quantity, outgoing.currency.clone())));

                transactions.push(Transaction::fiat_withdrawal(
                    transaction.timestamp + chrono::Duration::minutes(1),
                    Amount::new(incoming.quantity, incoming.currency.clone())));
            }
            else if outgoing.is_fiat() {
                transactions.push(Transaction::fiat_deposit(
                    transaction.timestamp - chrono::Duration::minutes(1),
                    Amount::new(outgoing.quantity, outgoing.currency.clone())));

                transactions.push(Transaction::send(
                    transaction.timestamp + chrono::Duration::minutes(1),
                    Amount::new(incoming.quantity, incoming.currency.clone())));
            }
        }

        transactions.push(transaction)
    }

    Ok(transactions)
}
