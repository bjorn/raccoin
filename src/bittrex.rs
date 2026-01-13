use std::path::Path;

use anyhow::{Result, bail};
use chrono::{NaiveDateTime, NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount}, CsvSpec, TransactionSourceType};
use linkme::distributed_slice;

// deserialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    match NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M") {
        Ok(date_and_time) => Ok(date_and_time),
        Err(_) => Ok(NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap().and_time(NaiveTime::MIN)),
    }
}

#[derive(Debug, Deserialize)]
enum MarketSide {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// Date,Market,Side,Type,Price,Quantity,Total
#[derive(Debug, Deserialize)]
struct BittrexOrder {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Market")]
    market: String,
    #[serde(rename = "Side")]
    side: MarketSide,
    // #[serde(rename = "Type")]
    // type_: String,
    // #[serde(rename = "Price")]
    // price: Decimal,
    #[serde(rename = "Quantity")]
    quantity: Decimal,
    #[serde(rename = "Total")]
    total: Decimal,
}

#[derive(Debug, Deserialize)]
enum BittrexTransactionType {
    #[serde(alias = "WITHDRAWAL")]
    Withdrawal,
    #[serde(alias = "DEPOSIT")]
    Deposit,
}

// Date,Currency,Type,Address,Memo/Tag,TxId,Amount
#[derive(Debug, Deserialize)]
struct BittrexTransaction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Type")]
    type_: BittrexTransactionType,
    // #[serde(rename = "Address")]
    // address: String,
    // #[serde(rename = "Memo/Tag")]
    // memo_tag: String,
    #[serde(rename = "TxId")]
    tx_id: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
}

impl TryFrom<BittrexOrder> for Transaction {
    type Error = anyhow::Error;

    fn try_from(item: BittrexOrder) -> Result<Self, Self::Error> {
        let mut split = item.market.split('/');
        match (split.next(), split.next()) {
            (Some(base_currency), Some(quote_currency)) => {
                let base = Amount::new(item.quantity, base_currency.to_owned());
                let quote = Amount::new(item.total, quote_currency.to_owned());
                match item.side {
                    MarketSide::Buy => Ok(Transaction::trade(item.date, base, quote)),
                    MarketSide::Sell => Ok(Transaction::trade(item.date, quote, base)),
                }
            }
            _ => bail!("Invalid market value, expected: '<base_currency>/<quote_currency>'"),
        }
    }
}

impl From<BittrexTransaction> for Transaction {
    fn from(item: BittrexTransaction) -> Self {
        let blockchain = item.currency.clone();
        let mut tx = match item.type_ {
            BittrexTransactionType::Withdrawal => Transaction::send(item.date, Amount::new(-item.amount, item.currency)),
            BittrexTransactionType::Deposit => Transaction::receive(item.date, Amount::new(item.amount, item.currency)),
        };

        // For ETH deposits on Bittrex, the reported tx_id can be the Sweep from
        // the deposit address, rather than the transaction that was used to
        // send the ETH to the deposit address. To still allow the transaction
        // to be recognized as "transfer", we don't set the tx_hash in that
        // case.
        if !(tx.operation.is_receive() && blockchain == "ETH") {
            tx.tx_hash = Some(item.tx_id);
        }

        // For BTC and BCH withdrawals from Bittrex, it appears a 0.001 BTC/BCH
        // fee was charged, but the CSV does unfortunately not report withdrawal
        // fees.
        if tx.operation.is_send() && (blockchain == "BTC" || blockchain == "BCH") {
            tx.fee = Some(Amount::new(Decimal::new(1, 3), blockchain.clone()));
        }

        tx.blockchain = Some(blockchain);
        tx
    }
}

// loads a Bittrex Order History CSV file into a list of unified transactions
pub(crate) fn load_bittrex_order_history_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: BittrexOrder = result?;
        transactions.push(record.try_into()?);
    }

    transactions.reverse();
    Ok(transactions)
}

// loads a Bittrex Transaction History CSV file into a list of unified transactions
pub(crate) fn load_bittrex_transaction_history_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: BittrexTransaction = result?;
        transactions.push(record.into());
    }

    transactions.reverse();
    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
pub(crate) static BITTREX_ORDER_HISTORY_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "BittrexOrderHistoryCsv",
    label: "Bittrex Order History (CSV)",
    csv: Some(CsvSpec {
        headers: &["Date", "Market", "Side", "Type", "Price", "Quantity", "Total"],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_bittrex_order_history_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
pub(crate) static BITTREX_TRANSACTION_HISTORY_CSV_SOURCE: TransactionSourceType =
    TransactionSourceType {
        id: "BittrexTransactionHistoryCsv",
        label: "Bittrex Transaction History (CSV)",
        csv: Some(CsvSpec {
            headers: &["Date", "Currency", "Type", "Address", "Memo/Tag", "TxId", "Amount"],
            delimiters: &[b','],
            skip_lines: 0,
        }),
        detect: None,
        load_sync: Some(load_bittrex_transaction_history_csv),
        load_async: None,
    };
