use std::error::Error;

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time, base::{Transaction, Operation}};

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
pub(crate) fn load_bitonic_csv(input_path: &str) -> Result<Vec<Transaction>, Box<dyn Error>> {
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

    println!("Imported {} transactions from {}", transactions.len(), input_path);

    Ok(transactions)
}

// converts a custom Bitonic CSV file to one for CryptoTaxCalculator
pub(crate) fn convert_bitonic_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitonicAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        // Since Bitonic does not hold any fiat or crypto, we add dummy deposit and send transactions
        // for each buy/sell transaction.
        match record.action {
            BitonicActionType::Buy => {
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::FiatDeposit,
                    "EUR",
                    -record.price
                ))?;
                wtr.serialize(CtcTx {
                    quote_currency: Some("EUR"),
                    quote_amount: Some(-record.price),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Buy,
                        "BTC",
                        record.amount
                )})?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::Send,
                    "BTC",
                    record.amount
                ))?;
            }
            BitonicActionType::Sell => {
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::Receive,
                    "BTC",
                    -record.amount
                ))?;
                wtr.serialize(CtcTx {
                    quote_currency: Some("EUR"),
                    quote_amount: Some(record.price),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Sell,
                        "BTC",
                        -record.amount)
                })?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::FiatWithdrawal,
                    "EUR",
                    record.price
                ))?;
            }
        }
    }

    Ok(())
}
