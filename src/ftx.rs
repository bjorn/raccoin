use std::path::Path;

use anyhow::{Result, bail};
use chrono::{NaiveDateTime, FixedOffset, DateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount}, CsvSpec, TransactionSourceType};
use linkme::distributed_slice;

// function for reading NaiveDateTime in the format "2/25/2021, 2:24:46 PM"
pub(crate) fn deserialize_date_time_mdy<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(raw, "%m/%d/%Y, %I:%M:%S %p").unwrap())
}

/// FTX Deposit
// " ","Time","Coin","Amount","Status","Additional info","Transaction ID"
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FtxDeposit {
    // #[serde(rename = " ")]
    // id: String,
    time: DateTime<FixedOffset>,
    coin: String,
    amount: Decimal,
    // status: String,  // "complete" or "confirmed"
    #[serde(rename = "Additional info")]
    additional_info: String,
    #[serde(rename = "Transaction ID")]
    transaction_id: String,
}

impl From<FtxDeposit> for Transaction {
    fn from(deposit: FtxDeposit) -> Self {
        let blockchain = deposit.coin.clone();
        let mut tx = Transaction::receive(deposit.time.naive_utc(), Amount::new(deposit.amount, deposit.coin));
        tx.tx_hash = Some(deposit.transaction_id);
        tx.blockchain = Some(blockchain);
        tx.description = Some(deposit.additional_info);
        tx
    }
}

/// FTX Withdrawal
// " ","Time","Coin","Amount","Destination","Status","Transaction ID","fee"
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FtxWithdrawal {
    #[serde(rename = " ")]
    id: String,
    time: DateTime<FixedOffset>,
    coin: String,
    amount: Decimal,
    // destination: String,
    // status: String,
    #[serde(rename = "Transaction ID")]
    transaction_id: String,
    #[serde(rename = "fee")]
    fee: Decimal,
}

impl From<FtxWithdrawal> for Transaction {
    // todo: add destination?
    fn from(withdrawal: FtxWithdrawal) -> Self {
        let fee_currency = withdrawal.coin.clone();
        let blockchain = withdrawal.coin.clone();
        let mut tx = Transaction::send(withdrawal.time.naive_utc(), Amount::new(withdrawal.amount, withdrawal.coin));
        tx.fee = Some(Amount::new(withdrawal.fee, fee_currency));
        tx.tx_hash = Some(withdrawal.transaction_id);
        tx.blockchain = Some(blockchain);
        tx.description = Some(withdrawal.id);
        tx
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Side {
    Buy,
    Sell,
}

/// FTX Trade
// "ID","Time","Market","Side","Order Type","Size","Price","Total","Fee","Fee Currency","TWAP"
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FtxTrade {
    #[serde(rename = "ID")]
    id: String,
    #[serde(deserialize_with = "deserialize_date_time_mdy")]
    time: NaiveDateTime,
    market: String,
    side: Side,
    // #[serde(rename = "Order Type")]
    // order_type: String, // "OTC" or "Order" have been observed
    size: Decimal,
    // price: Decimal,
    total: Decimal,
    fee: Option<Decimal>,
    #[serde(rename = "Fee Currency")]
    fee_currency: String,
    // /// Whether the trade was executed using Time-Weighted Average Price.
    // #[serde(rename = "TWAP")]
    // twap: bool,
}

// FTX trades report in USD even if USDC was deposited. This is likely not the
// right way to resolve this issue...
fn normalize_currency(currency: &str) -> &str {
    match currency {
        "USD" => "USDC",
        _ => currency,
    }
}

impl TryFrom<FtxTrade> for Transaction {
    type Error = anyhow::Error;

    fn try_from(trade: FtxTrade) -> Result<Self, Self::Error> {
        let mut split = trade.market.split('/');
        match (split.next(), split.next()) {
            (Some(base_currency), Some(quote_currency)) => {
                let base = Amount::new(trade.size, normalize_currency(base_currency).to_owned());
                let quote = Amount::new(trade.total, normalize_currency(quote_currency).to_owned());
                let mut tx = match trade.side {
                    Side::Buy => Transaction::trade(trade.time, base, quote),
                    Side::Sell => Transaction::trade(trade.time, quote, base),
                };
                tx.fee = trade.fee.map(|fee| Amount::new(fee, normalize_currency(&trade.fee_currency).to_owned()));
                tx.description = Some(trade.id);
                Ok(tx)
            }
            _ => bail!("Invalid market value, expected: '<base_currency>/<quote_currency>'"),
        }
    }
}

fn load_ftx_deposits_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: FtxDeposit = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

fn load_ftx_withdrawals_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: FtxWithdrawal = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

fn load_ftx_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: FtxTrade = result?;
        transactions.push(record.try_into()?);
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static FTX_DEPOSITS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "FtxDepositsCsv",
    label: "FTX Deposits (CSV)",
    csv: Some(CsvSpec {
        headers: &[" ", "Time", "Coin", "Amount", "Status", "Additional info", "Transaction ID"],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_ftx_deposits_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static FTX_WITHDRAWALS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "FtxWithdrawalsCsv",
    label: "FTX Withdrawal (CSV)",
    csv: Some(CsvSpec {
        headers: &[" ", "Time", "Coin", "Amount", "Destination", "Status", "Transaction ID", "fee"],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_ftx_withdrawals_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static FTX_TRADES_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "FtxTradesCsv",
    label: "FTX Trades (CSV)",
    csv: Some(CsvSpec {
        headers: &["ID", "Time", "Market", "Side", "Order Type", "Size", "Price", "Total", "Fee", "Fee Currency", "TWAP"],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_ftx_trades_csv),
    load_async: None,
};
