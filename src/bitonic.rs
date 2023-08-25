use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time};

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
struct BitonicAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Action")]
    operation: Operation,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Price")]
    price: f64,
}

// converts a custom Bitonic CSV file to one for CryptoTaxCalculator
pub(crate) fn convert_bitonic_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitonicAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        // Since Bitonic does not hold any fiat or crypto, we add dummy deposit and send transactions
        // for each buy/sell transaction.
        match record.operation {
            Operation::Buy => {
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
            Operation::Sell => {
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
