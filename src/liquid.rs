use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount, deserialize_amount}, CsvSpec, TransactionSourceType};
use linkme::distributed_slice;

// function for reading NaiveDateTime in the format "07/04/2019 07:48:17"
pub(crate) fn deserialize_date_time_mdy<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(raw, "%m/%d/%Y %H:%M:%S").unwrap())
}

// function for reading NaiveDateTime in the format "19/06/25 22:39:11"
pub(crate) fn deserialize_date_time_ymd<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(raw, "%y/%m/%d %H:%M:%S").unwrap())
}

/// Liquid Deposit
//
// Not actually a format exported from Liquid, but rather than data you'd get by
// copy-pasting the code from the Liquid website.
//
// ID,Type,Amount,Status,Created (YY/MM/DD),Hash
#[derive(Debug, Deserialize)]
struct LiquidDeposit {
    #[serde(rename = "ID")]
    id: String,
    // #[serde(rename = "Type")]
    // type_: String,   // assuming "Funding"
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    amount: Amount,
    // #[serde(rename = "Status")]
    // status: String,  // assuming "Succeeded"
    #[serde(rename = "Created (YY/MM/DD)", deserialize_with = "deserialize_date_time_ymd")]
    created: NaiveDateTime,
    #[serde(rename = "Hash")]
    hash: String,
}

impl From<LiquidDeposit> for Transaction {
    fn from(deposit: LiquidDeposit) -> Self {
        let blockchain = deposit.amount.currency.clone();
        let mut tx = Transaction::receive(deposit.created, deposit.amount);
        tx.tx_hash = Some(deposit.hash);
        tx.blockchain = Some(blockchain);
        tx.description = Some(deposit.id);
        tx
    }
}

// Liquid Withdrawal
//
// Not actually a format exported from Liquid, but rather than data you'd get by
// copy-pasting the code from the Liquid website.
//
// ID,Wallet label,Amount,Created On,Transfer network,Status,Address,Liquid Fee,Network Fee,Broadcasted At,Hash
#[derive(Debug, Deserialize)]
struct LiquidWithdrawal {
    #[serde(rename = "ID")]
    id: String,
    // #[serde(rename = "Wallet label")]
    // wallet_label: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
    #[serde(rename = "Created On", deserialize_with = "deserialize_date_time_ymd")]
    created_on: NaiveDateTime,
    #[serde(rename = "Transfer network")]
    transfer_network: String,
    // #[serde(rename = "Status")]
    // status: String,
    // #[serde(rename = "Address")]
    // address: String,
    #[serde(rename = "Liquid Fee")]
    liquid_fee: Decimal,
    #[serde(rename = "Network Fee")]
    network_fee: Decimal,
    // #[serde(rename = "Broadcasted At")]
    // broadcasted_at: NaiveDateTime,
    #[serde(rename = "Hash")]
    hash: String,
}

impl From<LiquidWithdrawal> for Transaction {
    // todo: add target address?
    fn from(withdrawal: LiquidWithdrawal) -> Self {
        let currency = match withdrawal.transfer_network.as_str() {
            "Bitcoin" => "BTC",
            other => other,
        };
        let mut tx = Transaction::send(withdrawal.created_on, Amount::new(withdrawal.amount, currency.to_owned()));
        tx.fee = Some(Amount::new(withdrawal.liquid_fee + withdrawal.network_fee, currency.to_owned()));
        tx.tx_hash = Some(withdrawal.hash);
        tx.blockchain = Some(currency.to_owned());
        tx.description = Some(withdrawal.id);
        tx
    }
}

#[derive(Debug, Deserialize)]
enum TradeType {
    Bought,
    Sold,
}

// #[derive(Debug, Deserialize)]
// enum TradeSide {
//     Maker,
//     Taker,
// }

// Spot Trade
//
// As observed as part of the "Overview Report" which can be sent by email from
// the Transactions page.
//
// Quoted currency,Base currency,Qex/liquid,Execution,Type,Date,Open qty,Price,Fee,Fee currency,Amount,Trade side
#[derive(Debug, Deserialize)]
struct LiquidSpotTrade<'a> {
    #[serde(rename = "Quoted currency")]
    quoted_currency: &'a str,
    #[serde(rename = "Base currency")]
    base_currency: &'a str,
    // #[serde(rename = "Qex/liquid")]
    // qex_liquid: &'a str,
    // #[serde(rename = "Execution")]
    // execution: &'a str,
    #[serde(rename = "Type")]
    trade_type: TradeType,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time_mdy")]
    date: NaiveDateTime,
    #[serde(rename = "Open qty")]
    open_qty: Decimal,
    // #[serde(rename = "Price")]
    // price: Decimal,
    #[serde(rename = "Fee")]
    fee: Decimal,
    #[serde(rename = "Fee currency")]
    fee_currency: &'a str,
    #[serde(rename = "Amount")]
    amount: Decimal,
    // #[serde(rename = "Trade side")]
    // trade_side: TradeSide,
}

impl<'a> From<LiquidSpotTrade<'a>> for Transaction {
    fn from(trade: LiquidSpotTrade) -> Self {
        let quote_amount = Amount::new(trade.amount, trade.quoted_currency.to_owned());
        let base_amount = Amount::new(trade.open_qty, trade.base_currency.to_owned());
        let mut tx = match trade.trade_type {
            TradeType::Bought => Transaction::trade(trade.date, base_amount, quote_amount),
            TradeType::Sold => Transaction::trade(trade.date, quote_amount, base_amount),
        };
        tx.fee = Some(Amount::new(trade.fee, trade.fee_currency.to_owned()));
        tx
    }
}

fn load_liquid_deposits_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: LiquidDeposit = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

fn load_liquid_withdrawals_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: LiquidWithdrawal = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static LIQUID_DEPOSITS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "LiquidDepositsCsv",
    label: "Liquid Deposits (CSV)",
    csv: Some(CsvSpec::new(&[
        "ID",
        "Type",
        "Amount",
        "Status",
        "Created (YY/MM/DD)",
        "Hash",
    ])),
    detect: None,
    load_sync: Some(load_liquid_deposits_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static LIQUID_TRADES_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "LiquidTradesCsv",
    label: "Liquid Trades (CSV)",
    csv: Some(CsvSpec {
        headers: &[
            "Quoted currency",
            "Base currency",
            "Qex/liquid",
            "Execution",
            "Type",
            "Date",
            "Open qty",
            "Price",
            "Fee",
            "Fee currency",
            "Amount",
            "Trade side",
        ],
        delimiters: &[b','],
        skip_lines: 2,
    }),
    detect: None,
    load_sync: Some(load_liquid_trades_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static LIQUID_WITHDRAWALS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "LiquidWithdrawalsCsv",
    label: "Liquid Withdrawals (CSV)",
    csv: Some(CsvSpec::new(&[
        "ID",
        "Wallet label",
        "Amount",
        "Created On",
        "Transfer network",
        "Status",
        "Address",
        "Liquid Fee",
        "Network Fee",
        "Broadcasted At",
        "Hash",
    ])),
    detect: None,
    load_sync: Some(load_liquid_withdrawals_csv),
    load_async: None,
};

fn load_liquid_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(input_path)?;

    // Skip the first three rows
    let mut raw_record = csv::StringRecord::new();
    for _ in 0..3 {
        rdr.read_record(&mut raw_record)?;
    }

    let headers = csv::StringRecord::from(vec!["Quoted currency", "Base currency", "Qex/liquid", "Execution", "Type", "Date", "Open qty", "Price", "Fee", "Fee currency", "Amount", "Trade side"]);
    let mut transactions = Vec::new();

    while rdr.read_record(&mut raw_record)? {
        // Probably reached the end of the list of spot trades
        if raw_record.len() < headers.len() {
            break;
        }

        let record: LiquidSpotTrade = raw_record.deserialize(Some(&headers))?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
