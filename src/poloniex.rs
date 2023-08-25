use std::error::Error;

use chrono::{NaiveDateTime, FixedOffset, DateTime};
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time};

#[derive(Debug, Deserialize)]
pub(crate) struct PoloniexDeposit {
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Address")]
    address: String,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    // #[serde(rename = "Status")]
    // status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PoloniexWithdrawal {
    #[serde(rename = "Fee Deducted")]
    fee_deducted: f64,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Currency")]
    currency: String,
    // #[serde(rename = "Amount")]
    // amount: f64,
    #[serde(rename = "Amount-Fee")]
    amount_fee: f64,
    #[serde(rename = "Address")]
    address: String,
    // #[serde(rename = "Status")]
    // status: String,  // always COMPLETED
}

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// csv columns: Date,Market,Type,Side,Price,Amount,Total,Fee,Order Number,Fee Currency,Fee Total
#[derive(Debug, Deserialize)]
pub(crate) struct PoloniexTrade {
    #[serde(rename = "Date")]
    date: DateTime<FixedOffset>,
    #[serde(rename = "Market")]
    market: String,
    // #[serde(rename = "Type")]
    // type_: String,   // always LIMIT
    #[serde(rename = "Side")]
    side: Operation,
    // #[serde(rename = "Price")]
    // price: f64,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Total")]
    total: f64,
    #[serde(rename = "Fee")]
    fee: f64,
    #[serde(rename = "Order Number")]
    order_number: String,
    #[serde(rename = "Fee Currency")]
    fee_currency: String,
    // #[serde(rename = "Fee Total")]
    // fee_total: f64,  // always same as fee
}

pub(crate) fn convert_poloniex_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::Writer::from_path(output_path)?;

    // deposits
    let deposits_file = input_path.to_owned() + "/deposit.csv";
    println!("Converting {} to {}", deposits_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(deposits_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexDeposit = result?;
        // let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();
        wtr.serialize(CtcTx {
            description: Some(&record.address),
            ..CtcTx::new(
                record.date,
                CtcTxType::Receive,
                &record.currency,
                record.amount)
        })?;
    }

    // withdrawals
    let withdrawals_file = input_path.to_owned() + "/withdrawal.csv";
    println!("Converting {} to {}", withdrawals_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(withdrawals_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexWithdrawal = result?;
        // let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();
        wtr.serialize(CtcTx {
            description: Some(&record.address),
            fee_amount: Some(record.fee_deducted),
            fee_currency: Some(&record.currency),
            ..CtcTx::new(
                record.date,
                CtcTxType::Send,
                &record.currency,
                record.amount_fee)
        })?;
    }

    // trades
    let trades_file = input_path.to_owned() + "/all-trades.csv";
    println!("Converting {} to {}", trades_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(trades_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexTrade = result?;

        // split record.market at the underscore to obtain the base_currency and the quote_currency
        let collect = record.market.split("_").collect::<Vec<&str>>();
        let base_currency = collect[0];
        let quote_currency = collect[1];

        wtr.serialize(CtcTx {
            description: Some(&record.order_number),
            quote_amount: Some(record.total),
            quote_currency: Some(quote_currency),
            fee_amount: Some(record.fee),
            fee_currency: Some(&record.fee_currency),
            ..CtcTx::new(
                record.date.naive_utc(),
                match record.side {
                    Operation::Buy => CtcTxType::Buy,
                    Operation::Sell => CtcTxType::Sell,
                },
                base_currency,
                record.amount)
        })?;
    }

    Ok(())
}
