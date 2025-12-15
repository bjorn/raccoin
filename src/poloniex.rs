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

// Exported from https://www.poloniex.com/activity/wallet/deposit:
// Currency,Amount,Address,Date,Status
//
// Export requested through Support Ticket (2023):
// ,timestamp,currency,amount,address,status
//
// Export requested through Support Ticket (2025):
// f_created_at,currency,f_amount,f_address,f_status
#[derive(Debug, Deserialize)]
struct PoloniexDeposit {
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "f_amount", alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "f_address", alias = "Address")]
    address: String,
    #[serde(alias = "f_created_at", alias = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    // #[serde(alias = "f_status", alias = "Status")]
    // status: String,
}

// Exported from https://www.poloniex.com/activity/wallet/withdraw:
// Fee Deducted,Date,Currency,Amount,Amount-Fee,Address,Status
//
// Export requested through Support Ticket (2023):
// ,timestamp,currency,amount,fee_deducted,status
//
// Export requested through Support Ticket (2025):
// f_date,currency,f_amount,f_feededucted,f_status
#[derive(Debug, Deserialize)]
struct PoloniexWithdrawal {
    #[serde(alias = "f_feededucted", alias = "Fee Deducted")]
    fee_deducted: Decimal,
    #[serde(alias = "f_date", alias = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "f_amount", alias = "Amount")]
    amount: Decimal,
    // #[serde(rename = "Amount-Fee")]
    // amount_minus_fee: Decimal,
    #[serde(rename = "Address")]
    address: Option<String>,
    #[serde(alias = "f_status", alias = "Status")]
    status: String,  // Can be "COMPLETED" or "COMPLETE: tx_hash"
}

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// Exported from https://www.poloniex.com/activity/spot/trades:
// Date,Market,Type,Side,Price,Amount,Total,Fee,Order Number,Fee Currency,Fee Total
//
// Export requested through Support Ticket (2023):
// ,timestamp,trade_id,market,wallet,side,price,amount,fee,fee_currency,fee_total
//
// Export requested through Support Ticket (2025):
// order_id,activity,order_role,order_type,base_currency_name,quote_currency_name,fee_currency_name,price,amount,fee_amount,usd_amount,usd_fee_amount,utc_time
#[derive(Debug, Deserialize)]
struct PoloniexTrade {
    #[serde(alias = "utc_time", alias = "Date", deserialize_with = "deserialize_poloniex_timestamp")]
    timestamp: NaiveDateTime,

    // Some formats have a "market" column while others have separate "base_currency_name" and
    // "quote_currency_name" columns
    #[serde(alias = "Market")]
    market: Option<String>,

    base_currency_name: Option<String>,
    quote_currency_name: Option<String>,

    // #[serde(rename = "Type")]
    // type_: String,   // always LIMIT
    #[serde(alias = "activity", alias = "Side")]
    side: Operation,
    #[serde(alias = "Price")]
    price: Decimal,
    #[serde(alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "Total")]
    total: Option<Decimal>,
    // #[serde(alias = "Fee")]
    // fee: Decimal,
    #[serde(rename = "Order Number", alias = "trade_id", alias = "order_id")]
    order_number: String,
    #[serde(alias = "fee_currency_name", alias = "Fee Currency")]
    fee_currency: String,
    #[serde(alias = "fee_amount", alias = "Fee Total")]
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
        let (base_currency, quote_currency) = match (&item.market, &item.base_currency_name, &item.quote_currency_name) {
            (Some(market), _, _) => {
                // split record.market at the underscore or dash to obtain the base_currency and the quote_currency
                let mut split = market.split('_');
                match (split.next(), split.next()) {
                    (Some(base_currency), Some(quote_currency)) => Ok::<(&str, &str), &'static str>((base_currency, quote_currency)),
                    _ => {
                        let mut split = market.split('-');
                        match (split.next(), split.next()) {
                            (Some(quote_currency), Some(base_currency)) => Ok((base_currency, quote_currency)),
                            _ => return Err("Invalid Poloniex market")
                        }
                    }
                }
            }
            (None, Some(base_currency), Some(quote_currency)) => Ok((base_currency.as_str(), quote_currency.as_str())),
            _ => return Err("Could not determine base_currency and quote_currency")
        }?;

        let quote_currency = normalize_currency(quote_currency);
        let base_currency = normalize_currency(base_currency);
        let fee_currency = normalize_currency(&item.fee_currency);

        // Poloniex does not provide the total amount, so we need to calculate it based on the price
        // and amount. We truncate the result to 8 decimal places and hope it's accurate enough.
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
