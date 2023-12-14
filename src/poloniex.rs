use std::path::Path;

use anyhow::Result;
use chrono::{NaiveDateTime, FixedOffset, DateTime};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Deserializer};

use crate::{
    time::deserialize_date_time,
    base::{Transaction, Amount}
};

// deserialize function for trying a number of date-time formats, all of which
// have been seen in Poloniex trade CSV formats
pub(crate) fn deserialize_poloniex_timestamp<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    let date_time: NaiveDateTime = DateTime::<FixedOffset>::parse_from_rfc3339(raw)
        .and_then(|dt| Ok(dt.naive_utc()))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y/%m/%d %H:%M"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .map_err(serde::de::Error::custom)?;
    Ok(date_time)
}

// ,timestamp,currency,amount,address,status
#[derive(Debug, Deserialize)]
struct PoloniexDeposit {
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "Address")]
    address: String,
    #[serde(alias = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    // #[serde(alias = "Status")]
    // status: String,
}

// ,timestamp,currency,amount,fee_deducted,status
#[derive(Debug, Deserialize)]
struct PoloniexWithdrawal {
    #[serde(alias = "Fee Deducted")]
    fee_deducted: Decimal,
    #[serde(alias = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "Amount")]
    amount: Decimal,
    // #[serde(rename = "Amount-Fee")]
    // amount_minus_fee: Decimal,
    #[serde(rename = "Address")]
    address: Option<String>,
    #[serde(alias = "Status")]
    status: String,  // Can be "COMPLETED" or "COMPLETE: tx_hash"
}

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// Format I got from the website somehow:
// csv columns: Date,Market,Type,Side,Price,Amount,Total,Fee,Order Number,Fee Currency,Fee Total
//
// Format I got when asking Poloniex support for an export:
// csv columns: ,timestamp,trade_id,market,wallet,side,price,amount,fee,fee_currency,fee_total
#[derive(Debug, Deserialize)]
struct PoloniexTrade {
    #[serde(alias = "Date", deserialize_with = "deserialize_poloniex_timestamp")]
    timestamp: NaiveDateTime,
    #[serde(alias = "Market")]
    market: String,
    // #[serde(rename = "Type")]
    // type_: String,   // always LIMIT
    #[serde(alias = "Side")]
    side: Operation,
    #[serde(alias = "Price")]
    price: Decimal,
    #[serde(alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "Total")]
    total: Option<Decimal>,
    // #[serde(alias = "Fee")]
    // fee: Decimal,
    #[serde(rename = "Order Number", alias = "trade_id")]
    order_number: String,
    #[serde(alias = "Fee Currency")]
    fee_currency: String,
    #[serde(alias = "Fee Total")]
    fee_total: Decimal,
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
        let mut tx = Transaction::receive(item.timestamp, Amount::new(item.amount, currency.to_owned()));
        tx.description = Some(item.address);
        tx
    }
}

impl From<PoloniexWithdrawal> for Transaction {
    fn from(item: PoloniexWithdrawal) -> Self {
        let currency = normalize_currency(item.currency.as_str());
        let mut tx = Transaction::send(item.timestamp, Amount::new(item.amount - item.fee_deducted, currency.to_owned()));
        tx.fee = Some(Amount::new(item.fee_deducted, currency.to_owned()));
        tx.description = item.address;
        tx.tx_hash = if item.status.starts_with("COMPLETE: ") { Some(item.status.trim_start_matches("COMPLETE: ").to_owned()) } else { None };
        tx.blockchain = Some(currency.to_owned());
        tx
    }
}

impl TryFrom<PoloniexTrade> for Transaction {
    type Error = &'static str;

    fn try_from(item: PoloniexTrade) -> Result<Self, Self::Error> {
        // split record.market at the underscore or dash to obtain the base_currency and the quote_currency
        let mut split = item.market.split('_');
        let (base_currency, quote_currency) = match (split.next(), split.next()) {
            (Some(base_currency), Some(quote_currency)) => Ok::<(&str, &str), &'static str>((base_currency, quote_currency)),
            _ => {
                let mut split = item.market.split('-');
                match (split.next(), split.next()) {
                    (Some(quote_currency), Some(base_currency)) => Ok((base_currency, quote_currency)),
                    _ => return Err("Invalid Poloniex market")
                }
            }
        }?;

        let quote_currency = normalize_currency(quote_currency);
        let base_currency = normalize_currency(base_currency);
        let fee_currency = normalize_currency(&item.fee_currency);

        let total = item.total.unwrap_or_else(|| (item.price * item.amount).round_dp_with_strategy(8, RoundingStrategy::ToZero));

        let mut tx = match item.side {
            Operation::Buy => Transaction::trade(
                item.timestamp,
                Amount::new(item.amount, base_currency.to_owned()),
                Amount::new(total, quote_currency.to_owned()),
            ),
            Operation::Sell => Transaction::trade(
                item.timestamp,
                Amount::new(total, quote_currency.to_owned()),
                Amount::new(item.amount, base_currency.to_owned()),
            ),
        };

        // Some Poloniex export formats report a more precise fee than the one
        // that is actually calculated, judging by balance errors.
        let actual_fee = item.fee_total.round_dp_with_strategy(8, RoundingStrategy::ToZero);
        tx.fee = Some(Amount::new(actual_fee, fee_currency.to_owned()));
        tx.description = Some(format!("Order #{}", item.order_number));

        Ok(tx)
    }
}

// loads a Poloniex Deposits CSV file into a list of unified transactions
pub(crate) fn load_poloniex_deposits_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexDeposit = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Withdrawals CSV file into a list of unified transactions
pub(crate) fn load_poloniex_withdrawals_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexWithdrawal = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Trades CSV file into a list of unified transactions
pub(crate) fn load_poloniex_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexTrade = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(err) => {
                println!("Error: {:?}", err);
                continue;
            }
        };
    }

    Ok(transactions)
}
