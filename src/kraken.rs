use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount}, CsvSpec, TransactionSource};
use linkme::distributed_slice;

/// Normalize Kraken currency codes to standard format
/// XXBT -> BTC, XETH -> ETH, ZEUR -> EUR, etc.
fn normalize_currency(currency: &str) -> String {
    match currency {
        "XXBT" | "XBT" => "BTC".to_owned(),
        "XETH" => "ETH".to_owned(),
        "XXRP" => "XRP".to_owned(),
        "XLTC" => "LTC".to_owned(),
        "XXLM" => "XLM".to_owned(),
        "XXMR" => "XMR".to_owned(),
        "ZEUR" => "EUR".to_owned(),
        "ZUSD" => "USD".to_owned(),
        "ZGBP" => "GBP".to_owned(),
        "ZJPY" => "JPY".to_owned(),
        "ZCAD" => "CAD".to_owned(),
        "ZAUD" => "AUD".to_owned(),
        other => other.to_owned(),
    }
}

/// Parse Kraken trading pair into (base, quote) currencies
fn parse_pair(pair: &str) -> (String, String) {
    // Common Kraken pairs: XXBTZEUR, XETHXXBT, XXBTZUSD, etc.
    // Try known patterns first
    let known_bases = ["XXBT", "XETH", "XXRP", "XLTC", "XXLM", "XXMR", "XBT", "ETH"];
    let known_quotes = ["ZEUR", "ZUSD", "ZGBP", "ZCAD", "ZAUD", "ZJPY", "XXBT", "XBT", "EUR", "USD"];

    for base in known_bases {
        if pair.starts_with(base) {
            let quote = &pair[base.len()..];
            return (normalize_currency(base), normalize_currency(quote));
        }
    }

    // Fallback: try splitting at common quote currencies
    for quote in known_quotes {
        if pair.ends_with(quote) {
            let base = &pair[..pair.len() - quote.len()];
            return (normalize_currency(base), normalize_currency(quote));
        }
    }

    // Last resort: assume 3-4 char base + rest is quote
    let mid = if pair.len() > 6 { 4 } else { 3 };
    (normalize_currency(&pair[..mid]), normalize_currency(&pair[mid..]))
}

/// Deserialize Kraken's datetime format
fn deserialize_kraken_datetime<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    // Kraken format: "2024-01-15 10:30:45.1234" or "2024-01-15T10:30:45Z"
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%SZ"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KrakenTradeType {
    Buy,
    Sell,
}

/// Kraken trades CSV record
/// Headers: txid,ordertxid,pair,time,type,ordertype,price,cost,fee,vol,margin,misc,ledgers
#[derive(Debug, Deserialize)]
struct KrakenTrade {
    #[serde(rename = "txid")]
    tx_id: String,
    // ordertxid: String,  // Not needed
    pair: String,
    #[serde(rename = "time", deserialize_with = "deserialize_kraken_datetime")]
    time: NaiveDateTime,
    #[serde(rename = "type")]
    trade_type: KrakenTradeType,
    // ordertype: String,  // Not needed
    // price: Decimal,     // Not needed (can calculate from cost/vol)
    cost: Decimal,
    fee: Decimal,
    vol: Decimal,
    // margin: Decimal,    // Not needed for spot trades
    // misc: String,       // Not needed
    // ledgers: String,    // Not needed
}

impl From<KrakenTrade> for Transaction {
    fn from(trade: KrakenTrade) -> Self {
        let (base, quote) = parse_pair(&trade.pair);
        let base_amount = Amount::new(trade.vol, base);
        let quote_amount = Amount::new(trade.cost, quote.clone());

        let mut tx = match trade.trade_type {
            KrakenTradeType::Buy => Transaction::trade(trade.time, base_amount, quote_amount),
            KrakenTradeType::Sell => Transaction::trade(trade.time, quote_amount, base_amount),
        };

        if !trade.fee.is_zero() {
            tx.fee = Some(Amount::new(trade.fee, quote));
        }

        tx.description = Some(format!("Kraken trade {}", trade.tx_id));
        tx
    }
}

fn load_kraken_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: KrakenTrade = result?;
        transactions.push(record.into());
    }

    transactions.reverse();
    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static KRAKEN_TRADES_CSV: TransactionSource = TransactionSource {
    id: "KrakenTradesCsv",
    label: "Kraken Trades (CSV)",
    csv: &[CsvSpec::new(&[
        "txid", "ordertxid", "pair", "time", "type", "ordertype",
        "price", "cost", "fee", "vol", "margin", "misc", "ledgers",
    ])],
    detect: None,
    load_sync: Some(load_kraken_trades_csv),
    load_async: None,
};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum KrakenLedgerType {
    Deposit,
    Withdrawal,
    Trade,
    Spend,
    Receive,
    Transfer,
    Staking,
    Dividend,
    Reward,
    Settled,
    Sale,
    #[serde(other)]
    Unknown,
}

/// Kraken ledger CSV record
/// Headers: txid,refid,time,type,subtype,aclass,asset,wallet,amount,fee,balance
#[derive(Debug, Deserialize)]
struct KrakenLedger {
    #[serde(rename = "txid")]
    tx_id: String,
    #[serde(rename = "refid")]
    ref_id: String,
    #[serde(rename = "time", deserialize_with = "deserialize_kraken_datetime")]
    time: NaiveDateTime,
    #[serde(rename = "type")]
    ledger_type: KrakenLedgerType,
    // subtype: String,   // Not needed
    // aclass: String,    // Not needed
    asset: String,
    // wallet: String,    // Not needed
    amount: Decimal,
    fee: Decimal,
    // balance: Decimal,  // Not needed
}

impl From<KrakenLedger> for Transaction {
    fn from(ledger: KrakenLedger) -> Self {
        let currency = normalize_currency(&ledger.asset);
        let amount = Amount::new(ledger.amount.abs(), currency.clone());

        let mut tx = match ledger.ledger_type {
            KrakenLedgerType::Deposit => Transaction::receive(ledger.time, amount),
            KrakenLedgerType::Withdrawal => Transaction::send(ledger.time, amount),
            KrakenLedgerType::Staking | KrakenLedgerType::Reward => {
                Transaction::new(ledger.time, crate::base::Operation::Staking(amount))
            }
            KrakenLedgerType::Dividend => {
                Transaction::new(ledger.time, crate::base::Operation::Income(amount))
            }
            KrakenLedgerType::Spend => {
                Transaction::send(ledger.time, amount)
            }
            KrakenLedgerType::Receive => {
                Transaction::receive(ledger.time, amount)
            }
            KrakenLedgerType::Trade => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }
            KrakenLedgerType::Transfer => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }
            KrakenLedgerType::Settled | KrakenLedgerType::Sale | KrakenLedgerType::Unknown => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }
        };

        if !ledger.fee.is_zero() {
            tx.fee = Some(Amount::new(ledger.fee.abs(), currency));
        }

        tx.description = Some(format!("Kraken {} (ref: {})", ledger.tx_id, ledger.ref_id));
        tx
    }
}

fn load_kraken_ledger_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: KrakenLedger = result?;
        // Skip trade entries as they should be imported via trades CSV
        if record.ledger_type == KrakenLedgerType::Trade
            || record.ledger_type == KrakenLedgerType::Spend
            || record.ledger_type == KrakenLedgerType::Receive {
            continue;
        }
        transactions.push(record.into());
    }

    transactions.reverse();
    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static KRAKEN_LEDGER_CSV: TransactionSource = TransactionSource {
    id: "KrakenLedgerCsv",
    label: "Kraken Ledger (CSV)",
    csv: &[CsvSpec::new(&[
        "txid", "refid", "time", "type", "subtype", "aclass",
        "asset", "wallet", "amount", "fee", "balance",
    ])],
    detect: None,
    load_sync: Some(load_kraken_ledger_csv),
    load_async: None,
};