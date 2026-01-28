use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount}, CsvSpec, TransactionSource};
use linkme::distributed_slice;

/// Normalize Kraken currency codes to standard format.
///
/// Kraken uses special prefixes for currencies:
/// - `X` prefix for crypto (XXBT = BTC, XETH = ETH, XDOGE = DOGE)
/// - `Z` prefix for fiat (ZEUR = EUR, ZUSD = USD, ZCHF = CHF)
/// - Some newer assets have no prefix (DOT, SOL, MATIC)
///
/// This function handles known mappings explicitly, then falls back to
/// stripping X/Z prefixes for unknown 4+ character codes.
fn normalize_currency(currency: &str) -> String {
    match currency {
        // Bitcoin special cases (XBT is ISO code, BTC is common)
        "XXBT" | "XBT" => "BTC".to_owned(),
        // Known crypto with X prefix
        "XETH" => "ETH".to_owned(),
        "XXRP" => "XRP".to_owned(),
        "XLTC" => "LTC".to_owned(),
        "XXLM" => "XLM".to_owned(),
        "XXMR" => "XMR".to_owned(),
        "XXDG" => "DOGE".to_owned(),
        // Known fiat with Z prefix
        "ZEUR" => "EUR".to_owned(),
        "ZUSD" => "USD".to_owned(),
        "ZGBP" => "GBP".to_owned(),
        "ZJPY" => "JPY".to_owned(),
        "ZCAD" => "CAD".to_owned(),
        "ZAUD" => "AUD".to_owned(),
        "ZCHF" => "CHF".to_owned(),
        other => {
            // For unknown codes, try stripping X/Z prefix if 4+ chars
            // e.g., XDOGE -> DOGE, XADA -> ADA, ZSEK -> SEK
            if other.len() >= 4 {
                let first_char = other.chars().next().unwrap();
                if first_char == 'X' || first_char == 'Z' {
                    return other[1..].to_owned();
                }
            }
            other.to_owned()
        }
    }
}

/// Parse Kraken trading pair into (base, quote) currencies.
///
/// Kraken uses concatenated currency codes for pairs, e.g.:
/// - "XXBTZEUR" -> ("BTC", "EUR") - Bitcoin/Euro
/// - "XETHXXBT" -> ("ETH", "BTC") - Ethereum/Bitcoin
/// - "DOTUSD" -> ("DOT", "USD") - Polkadot/USD (newer format)
///
/// The parsing strategy:
/// 1. Try matching known base currencies at the start
/// 2. Try matching known quote currencies at the end
/// 3. Fallback: split at position 3-4 for unknown pairs
fn parse_pair(pair: &str) -> (String, String) {
    // Known Kraken currency codes for matching
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

        // Kraken reports fees in the quote currency for spot trades.
        // The trades CSV doesn't include a separate fee currency field,
        // so we assume the fee is denominated in the quote currency.
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
        // Skip trade-related entries (Trade, Spend, Receive) as they should be
        // imported via the separate trades CSV export. The ledger CSV contains
        // duplicate information for trades as paired spend/receive entries,
        // and importing from both sources would result in double-counting.
        // Users should import both trades.csv (for trades) and ledger.csv
        // (for deposits, withdrawals, staking) to get complete history.
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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_normalize_currency() {
        // Known mappings
        assert_eq!(normalize_currency("XXBT"), "BTC");
        assert_eq!(normalize_currency("XBT"), "BTC");
        assert_eq!(normalize_currency("XETH"), "ETH");
        assert_eq!(normalize_currency("ZEUR"), "EUR");
        assert_eq!(normalize_currency("ZUSD"), "USD");
        assert_eq!(normalize_currency("XXDG"), "DOGE");
        assert_eq!(normalize_currency("ZCHF"), "CHF");
        // Passthrough for non-prefixed
        assert_eq!(normalize_currency("DOT"), "DOT");
        assert_eq!(normalize_currency("SOL"), "SOL");
        // Generic X/Z prefix stripping for unknown codes
        assert_eq!(normalize_currency("XADA"), "ADA");
        assert_eq!(normalize_currency("XDOGE"), "DOGE");
        assert_eq!(normalize_currency("ZSEK"), "SEK");
        // Short codes should not be stripped
        assert_eq!(normalize_currency("XRP"), "XRP");
    }

    #[test]
    fn test_parse_pair() {
        // Classic Kraken pairs with X/Z prefixes
        assert_eq!(parse_pair("XXBTZEUR"), ("BTC".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("XETHXXBT"), ("ETH".to_owned(), "BTC".to_owned()));
        assert_eq!(parse_pair("XXBTZUSD"), ("BTC".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("XETHZEUR"), ("ETH".to_owned(), "EUR".to_owned()));
        // Newer altcoin pairs without X prefix
        assert_eq!(parse_pair("DOTUSD"), ("DOT".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("DOTEUR"), ("DOT".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("SOLUSD"), ("SOL".to_owned(), "USD".to_owned()));
    }

    #[test]
    fn test_parse_trade_buy() {
        let csv_data = r#"txid,ordertxid,pair,time,type,ordertype,price,cost,fee,vol,margin,misc,ledgers
"ABC123","ORD456","XXBTZEUR","2024-01-15 10:30:45.1234","buy","limit","40000.0","400.00","0.10","0.01","0.0","",""
"#;
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            crate::base::Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(incoming.quantity, dec!(0.01));
                assert_eq!(outgoing.currency, "EUR");
                assert_eq!(outgoing.quantity, dec!(400.00));
            }
            _ => panic!("Expected Trade operation"),
        }
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(0.10));
    }

    #[test]
    fn test_parse_trade_sell() {
        let csv_data = r#"txid,ordertxid,pair,time,type,ordertype,price,cost,fee,vol,margin,misc,ledgers
"DEF789","ORD012","XXBTZEUR","2024-01-16 14:20:00.0000","sell","market","41000.0","410.00","0.15","0.01","0.0","",""
"#;
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            crate::base::Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "EUR");
                assert_eq!(incoming.quantity, dec!(410.00));
                assert_eq!(outgoing.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(0.01));
            }
            _ => panic!("Expected Trade operation"),
        }
    }

    #[test]
    fn test_parse_ledger_deposit() {
        let csv_data = r#"txid,refid,time,type,subtype,aclass,asset,wallet,amount,fee,balance
"LED123","REF456","2024-01-10 08:00:00.0000","deposit","","currency","XXBT","","0.5","0.0","0.5"
"#;
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        assert!(tx.operation.is_receive());
    }

    #[test]
    fn test_parse_ledger_withdrawal() {
        let csv_data = r#"txid,refid,time,type,subtype,aclass,asset,wallet,amount,fee,balance
"LED789","REF012","2024-01-20 16:00:00.0000","withdrawal","","currency","XETH","","-1.0","0.005","0.0"
"#;
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        assert!(tx.operation.is_send());
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(0.005));
    }

    #[test]
    fn test_parse_ledger_staking() {
        let csv_data = r#"txid,refid,time,type,subtype,aclass,asset,wallet,amount,fee,balance
"STK123","REF789","2024-02-01 00:00:00.0000","staking","","currency","DOT","","0.1","0.0","10.1"
"#;
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            crate::base::Operation::Staking(amount) => {
                assert_eq!(amount.currency, "DOT");
                assert_eq!(amount.quantity, dec!(0.1));
            }
            _ => panic!("Expected Staking operation"),
        }
    }
}
