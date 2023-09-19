use std::{error::Error, path::Path};

use chrono::{NaiveDateTime, FixedOffset, DateTime};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    ctc::save_transactions_to_ctc_csv,
    time::deserialize_date_time,
    base::{Transaction, Amount}
};

#[derive(Debug, Deserialize)]
pub(crate) struct PoloniexDeposit {
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
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
    fee_deducted: Decimal,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Currency")]
    currency: String,
    // #[serde(rename = "Amount")]
    // amount: Decimal,
    #[serde(rename = "Amount-Fee")]
    amount_minus_fee: Decimal,
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
    // price: Decimal,
    #[serde(rename = "Amount")]
    amount: Decimal,
    #[serde(rename = "Total")]
    total: Decimal,
    #[serde(rename = "Fee")]
    fee: Decimal,
    #[serde(rename = "Order Number")]
    order_number: String,
    #[serde(rename = "Fee Currency")]
    fee_currency: String,
    // #[serde(rename = "Fee Total")]
    // fee_total: Decimal,  // always same as fee
}

// Poloniex reported XLM as STR
fn normalize_currency(currency: &str) -> &str {
    match currency {
        "STR" => "XLM",
        _ => currency,
    }
}

impl From<PoloniexDeposit> for Transaction {
    fn from(item: PoloniexDeposit) -> Self {
        let currency = normalize_currency(item.currency.as_str());
        let mut tx = Transaction::receive(item.date, Amount::new(item.amount, currency.to_owned()));
        tx.description = Some(item.address);
        tx
    }
}

impl From<PoloniexWithdrawal> for Transaction {
    fn from(item: PoloniexWithdrawal) -> Self {
        let currency = normalize_currency(item.currency.as_str());
        let mut tx = Transaction::send(item.date, Amount::new(item.amount_minus_fee, currency.to_owned()));
        tx.fee = Some(Amount { quantity: item.fee_deducted, currency: currency.to_owned() });
        tx.description = Some(item.address);
        tx
    }
}

impl From<PoloniexTrade> for Transaction {
    fn from(item: PoloniexTrade) -> Self {
        // split record.market at the underscore to obtain the base_currency and the quote_currency
        let collect = item.market.split("_").collect::<Vec<&str>>();
        let base_currency = normalize_currency(collect[0]);
        let quote_currency = normalize_currency(collect[1]);

        let timestamp = item.date.naive_utc();

        let mut tx = match item.side {
            Operation::Buy => Transaction::trade(
                timestamp,
                Amount {
                    quantity: item.amount,
                    currency: base_currency.to_owned(),
                },
                Amount {
                    quantity: item.total,
                    currency: quote_currency.to_owned(),
                }
            ),
            Operation::Sell => Transaction::trade(
                timestamp,
                Amount {
                    quantity: item.total,
                    currency: quote_currency.to_owned(),
                },
                Amount {
                    quantity: item.amount,
                    currency: base_currency.to_owned(),
                }
            ),
        };
        tx.fee = Some(Amount { quantity: item.fee, currency: item.fee_currency });
        tx.description = Some(format!("Order #{}", item.order_number));
        tx
    }
}

// loads a Poloniex Deposits CSV file into a list of unified transactions
pub(crate) fn load_poloniex_deposits_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexDeposit = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Withdrawals CSV file into a list of unified transactions
pub(crate) fn load_poloniex_withdrawals_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexWithdrawal = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Trades CSV file into a list of unified transactions
pub(crate) fn load_poloniex_trades_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexTrade = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

pub(crate) fn convert_poloniex_to_ctc(input_path: &Path, output_path: &Path) -> Result<(), Box<dyn Error>> {
    let mut txs = Vec::new();

    // deposits
    let deposits_file = input_path.join("deposit.csv");
    txs.extend(load_poloniex_deposits_csv(&deposits_file)?);

    // withdrawals
    let withdrawals_file = input_path.join("withdrawal.csv");
    txs.extend(load_poloniex_withdrawals_csv(&withdrawals_file)?);

    // trades
    let trades_file = input_path.join("all-trades.csv");
    txs.extend(load_poloniex_trades_csv(&trades_file)?);

    save_transactions_to_ctc_csv(&txs, output_path)?;

    Ok(())
}
