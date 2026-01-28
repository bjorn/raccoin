//! Kraken CSV import support for trades and ledger history.
//!
//! This module provides two transaction sources:
//! - **Kraken Trades CSV**: Import buy/sell trades with proper pair parsing
//! - **Kraken Ledger CSV**: Import deposits, withdrawals, staking rewards, dividends
//!
//! # Recommended Usage
//!
//! Export both files from Kraken and import them:
//! 1. Trades CSV for trading history (buy/sell)
//! 2. Ledger CSV for deposits, withdrawals, staking, dividends
//!
//! Note: The ledger CSV contains trade entries (spend/receive pairs) that are
//! automatically skipped to avoid double-counting with the trades CSV.
//!
//! # Margin Trading
//!
//! Margin trades are partially supported. The trades CSV captures margin trades,
//! but margin-specific ledger entries (rollover fees, liquidations) may require
//! manual review. For full margin support, consider using Kraken's API directly.
//!
//! # Known Limitations
//!
//! - **Fee currency**: Fees are assumed to be in the quote currency, which is
//!   Kraken's default. If you've configured fees to be paid in the base currency,
//!   the fee amounts will still be correct but attributed to the wrong currency.
//! - **Timezone**: All timestamps are in UTC as exported by Kraken.
//! - **Ledger-only import**: If you import only the ledger CSV without the trades
//!   CSV, you will miss all trading activity. Always import both files.

use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{base::{Transaction, Amount, Operation}, CsvSpec, TransactionSource};
use linkme::distributed_slice;

// ============================================================================
// Currency Normalization
// ============================================================================

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
        // Bitcoin special cases (XBT is ISO 4217 code, BTC is common usage)
        "XXBT" | "XBT" => "BTC".to_owned(),
        // Known crypto with X prefix
        "XETH" => "ETH".to_owned(),
        "XXRP" => "XRP".to_owned(),
        "XLTC" => "LTC".to_owned(),
        "XXLM" => "XLM".to_owned(),
        "XXMR" => "XMR".to_owned(),
        "XXDG" => "DOGE".to_owned(),
        "XZEC" => "ZEC".to_owned(),
        "XETC" => "ETC".to_owned(),
        "XREP" => "REP".to_owned(),
        "XMLN" => "MLN".to_owned(),
        // Known fiat with Z prefix
        "ZEUR" => "EUR".to_owned(),
        "ZUSD" => "USD".to_owned(),
        "ZGBP" => "GBP".to_owned(),
        "ZJPY" => "JPY".to_owned(),
        "ZCAD" => "CAD".to_owned(),
        "ZAUD" => "AUD".to_owned(),
        "ZCHF" => "CHF".to_owned(),
        "ZKRW" => "KRW".to_owned(),
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

// ============================================================================
// Pair Parsing
// ============================================================================

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
    // Known Kraken currency codes for matching (ordered by length, longest first)
    let known_bases = [
        "XXBT", "XETH", "XXRP", "XLTC", "XXLM", "XXMR", "XZEC", "XETC", "XREP", "XMLN",
        "XBT", "ETH",
    ];
    let known_quotes = [
        "ZEUR", "ZUSD", "ZGBP", "ZCAD", "ZAUD", "ZJPY", "ZCHF",
        "XXBT", "XETH",
        "XBT", "ETH", "EUR", "USD", "GBP", "CAD", "AUD", "JPY", "CHF",
    ];

    // Strategy 1: Match known base at start
    for base in known_bases {
        if pair.starts_with(base) {
            let quote = &pair[base.len()..];
            if !quote.is_empty() {
                return (normalize_currency(base), normalize_currency(quote));
            }
        }
    }

    // Strategy 2: Match known quote at end
    for quote in known_quotes {
        if pair.ends_with(quote) && pair.len() > quote.len() {
            let base = &pair[..pair.len() - quote.len()];
            if !base.is_empty() {
                return (normalize_currency(base), normalize_currency(quote));
            }
        }
    }

    // Strategy 3: Fallback - split at position based on length
    // Most pairs are 6-8 chars, assume 3-4 char base
    if pair.len() >= 6 {
        let mid = if pair.len() > 6 { 4 } else { 3 };
        (normalize_currency(&pair[..mid]), normalize_currency(&pair[mid..]))
    } else {
        // Very short pair, just return as-is
        (pair.to_owned(), String::new())
    }
}

// ============================================================================
// DateTime Parsing
// ============================================================================

/// Deserialize Kraken's datetime format.
///
/// Supports multiple formats found in Kraken exports:
/// - "2024-01-15 10:30:45.1234" (with subseconds)
/// - "2024-01-15 10:30:45" (without subseconds)
/// - "2024-01-15T10:30:45Z" (ISO 8601)
/// - "2024-01-15T10:30:45.123Z" (ISO 8601 with subseconds)
fn deserialize_kraken_datetime<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;

    // Try formats in order of likelihood
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.fZ"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%SZ"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|e| serde::de::Error::custom(format!("Invalid datetime '{}': {}", raw, e)))
}

// ============================================================================
// Trades CSV
// ============================================================================

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum KrakenTradeType {
    Buy,
    Sell,
}

/// Kraken trades CSV record.
///
/// Headers: txid,ordertxid,pair,aclass,subclass,time,type,ordertype,price,cost,fee,vol,margin,misc,ledgers,posttxid,poststatuscode,cprice,ccost,cfee,cvol,cmargin,net,trades
///
/// Note: The `price` field is not used as we can derive it from cost/vol.
/// The `margin` field is present but margin-specific handling is limited.
/// Fields after `ledgers` are for closed position info and are optional.
#[derive(Debug, Deserialize)]
struct KrakenTrade {
    #[serde(rename = "txid")]
    tx_id: String,
    #[serde(rename = "ordertxid")]
    _order_tx_id: String,
    pair: String,
    #[serde(rename = "aclass")]
    _aclass: String,
    #[serde(rename = "subclass")]
    _subclass: String,
    #[serde(rename = "time", deserialize_with = "deserialize_kraken_datetime")]
    time: NaiveDateTime,
    #[serde(rename = "type")]
    trade_type: KrakenTradeType,
    #[serde(rename = "ordertype")]
    _order_type: String,
    #[serde(rename = "price")]
    _price: Decimal,
    cost: Decimal,
    fee: Decimal,
    vol: Decimal,
    margin: Decimal,
    #[serde(rename = "misc")]
    _misc: String,
    #[serde(rename = "ledgers")]
    _ledgers: String,
    // Additional fields for closed positions (optional, handled by flexible parsing)
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

        // Add description with margin warning if applicable
        if trade.margin.is_zero() {
            tx.description = Some(format!("Kraken trade {}", trade.tx_id));
        } else {
            // Margin trade: include warning in description for manual review
            tx.description = Some(format!(
                "Kraken trade {} [MARGIN: {} {} used - review for tax implications]",
                trade.tx_id, trade.margin, quote
            ));
        }

        // Kraken fees are in quote currency by default, but users can configure
        // fees to be paid in base currency for some pairs. Since the CSV doesn't
        // include a fee currency field, we assume quote currency (the default).
        // See: https://support.kraken.com/hc/en-us/articles/360000526126
        if !trade.fee.is_zero() {
            tx.fee = Some(Amount::new(trade.fee, quote));
        }
        tx
    }
}

fn load_kraken_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true) // Allow varying number of fields (some exports have extra columns)
        .from_path(input_path)?;
    let mut transactions = Vec::new();

    for (line_num, result) in rdr.deserialize().enumerate() {
        let record: KrakenTrade = result.map_err(|e| {
            anyhow::anyhow!("Failed to parse trade at line {}: {}", line_num + 2, e)
        })?;
        transactions.push(record.into());
    }

    // Reverse to get chronological order (Kraken exports newest first)
    transactions.reverse();
    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static KRAKEN_TRADES_CSV: TransactionSource = TransactionSource {
    id: "KrakenTradesCsv",
    label: "Kraken Trades (CSV)",
    csv: &[CsvSpec::new(&[
        "txid", "ordertxid", "pair", "aclass", "subclass", "time", "type", "ordertype",
        "price", "cost", "fee", "vol", "margin", "misc", "ledgers",
    ])],
    detect: None,
    load_sync: Some(load_kraken_trades_csv),
    load_async: None,
};

// ============================================================================
// Ledger CSV
// ============================================================================

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
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
    Margin,
    Rollover,
    Adjustment,
    #[serde(other)]
    Unknown,
}

/// Kraken ledger CSV record.
///
/// Headers: txid,refid,time,type,subtype,aclass,subclass,asset,wallet,amount,fee,balance
///
/// The `refid` field links related entries (e.g., spend/receive pairs for trades).
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
    #[serde(rename = "subtype")]
    _subtype: String,
    #[serde(rename = "aclass")]
    _aclass: String,
    #[serde(rename = "subclass")]
    _subclass: String,
    asset: String,
    #[serde(rename = "wallet")]
    _wallet: String,
    amount: Decimal,
    fee: Decimal,
    #[serde(rename = "balance")]
    _balance: Decimal,
}

impl KrakenLedger {
    /// Check if this ledger entry should be skipped when importing.
    ///
    /// Trade-related entries (Trade, Spend, Receive) are skipped because:
    /// 1. They duplicate information from the trades CSV
    /// 2. Importing both would result in double-counting
    fn should_skip(&self) -> bool {
        matches!(
            self.ledger_type,
            KrakenLedgerType::Trade | KrakenLedgerType::Spend | KrakenLedgerType::Receive
        )
    }
}

/// Check if a currency is fiat (for FiatDeposit/FiatWithdrawal operations)
fn is_fiat_currency(currency: &str) -> bool {
    matches!(currency, "EUR" | "USD" | "GBP" | "JPY" | "CAD" | "AUD" | "CHF" | "KRW")
}

impl From<KrakenLedger> for Transaction {
    fn from(ledger: KrakenLedger) -> Self {
        let currency = normalize_currency(&ledger.asset);
        let amount = Amount::new(ledger.amount.abs(), currency.clone());
        let is_fiat = is_fiat_currency(&currency);

        let mut tx = match ledger.ledger_type {
            // Deposits: External funds coming in
            KrakenLedgerType::Deposit => {
                if is_fiat {
                    Transaction::fiat_deposit(ledger.time, amount)
                } else {
                    Transaction::receive(ledger.time, amount)
                }
            }

            // Withdrawals: Funds leaving the exchange
            KrakenLedgerType::Withdrawal => {
                if is_fiat {
                    Transaction::fiat_withdrawal(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }

            // Staking rewards and general rewards
            KrakenLedgerType::Staking | KrakenLedgerType::Reward => {
                Transaction::new(ledger.time, Operation::Staking(amount))
            }

            // Dividends from holding certain assets
            KrakenLedgerType::Dividend => {
                Transaction::new(ledger.time, Operation::Income(amount))
            }

            // Transfers between wallets (internal)
            KrakenLedgerType::Transfer => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }

            // Margin-related entries
            KrakenLedgerType::Margin | KrakenLedgerType::Rollover | KrakenLedgerType::Settled => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }

            // Adjustments (corrections, airdrops, etc.)
            KrakenLedgerType::Adjustment => {
                if ledger.amount.is_sign_positive() {
                    Transaction::new(ledger.time, Operation::Income(amount))
                } else {
                    Transaction::new(ledger.time, Operation::Expense(amount))
                }
            }

            // Sale (fiat conversion)
            KrakenLedgerType::Sale => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }

            // Trade/Spend/Receive are typically skipped, but handle them just in case
            KrakenLedgerType::Trade | KrakenLedgerType::Spend => {
                Transaction::send(ledger.time, amount)
            }
            KrakenLedgerType::Receive => {
                Transaction::receive(ledger.time, amount)
            }

            // Unknown types: use sign to determine direction
            KrakenLedgerType::Unknown => {
                if ledger.amount.is_sign_positive() {
                    Transaction::receive(ledger.time, amount)
                } else {
                    Transaction::send(ledger.time, amount)
                }
            }
        };

        // Add fee if present (fees are always positive in the CSV)
        if !ledger.fee.is_zero() {
            tx.fee = Some(Amount::new(ledger.fee.abs(), currency.clone()));
        }

        // Set blockchain for deposit/withdrawal to enable transaction matching
        // across different wallets (e.g., matching a Kraken withdrawal with a
        // hardware wallet deposit)
        if matches!(ledger.ledger_type, KrakenLedgerType::Deposit | KrakenLedgerType::Withdrawal) {
            tx.blockchain = Some(currency);
        }

        tx.description = Some(format!("Kraken {} (ref: {})", ledger.tx_id, ledger.ref_id));
        tx
    }
}

fn load_kraken_ledger_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(input_path)?;
    let mut transactions = Vec::new();

    for (line_num, result) in rdr.deserialize().enumerate() {
        let record: KrakenLedger = result.map_err(|e| {
            anyhow::anyhow!("Failed to parse ledger at line {}: {}", line_num + 2, e)
        })?;

        // Skip trade-related entries to avoid double-counting with trades CSV
        if record.should_skip() {
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
        "txid", "refid", "time", "type", "subtype", "aclass", "subclass",
        "asset", "wallet", "amount", "fee", "balance",
    ])],
    detect: None,
    load_sync: Some(load_kraken_ledger_csv),
    load_async: None,
};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Kraken CSV headers - minimum required fields for parsing
    // Note: Actual exports may have additional fields (posttxid, poststatuscode, etc.)
    // but flexible parsing handles this
    const TRADES_HEADER: &str = "txid,ordertxid,pair,aclass,subclass,time,type,ordertype,price,cost,fee,vol,margin,misc,ledgers";
    const LEDGER_HEADER: &str = "txid,refid,time,type,subtype,aclass,subclass,asset,wallet,amount,fee,balance";

    // ------------------------------------------------------------------------
    // Currency Normalization Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_normalize_currency_known_crypto() {
        assert_eq!(normalize_currency("XXBT"), "BTC");
        assert_eq!(normalize_currency("XBT"), "BTC");
        assert_eq!(normalize_currency("XETH"), "ETH");
        assert_eq!(normalize_currency("XXRP"), "XRP");
        assert_eq!(normalize_currency("XLTC"), "LTC");
        assert_eq!(normalize_currency("XXLM"), "XLM");
        assert_eq!(normalize_currency("XXMR"), "XMR");
        assert_eq!(normalize_currency("XXDG"), "DOGE");
        assert_eq!(normalize_currency("XZEC"), "ZEC");
        assert_eq!(normalize_currency("XETC"), "ETC");
    }

    #[test]
    fn test_normalize_currency_known_fiat() {
        assert_eq!(normalize_currency("ZEUR"), "EUR");
        assert_eq!(normalize_currency("ZUSD"), "USD");
        assert_eq!(normalize_currency("ZGBP"), "GBP");
        assert_eq!(normalize_currency("ZJPY"), "JPY");
        assert_eq!(normalize_currency("ZCAD"), "CAD");
        assert_eq!(normalize_currency("ZAUD"), "AUD");
        assert_eq!(normalize_currency("ZCHF"), "CHF");
        assert_eq!(normalize_currency("ZKRW"), "KRW");
    }

    #[test]
    fn test_normalize_currency_passthrough() {
        // Non-prefixed currencies should pass through unchanged
        assert_eq!(normalize_currency("DOT"), "DOT");
        assert_eq!(normalize_currency("SOL"), "SOL");
        assert_eq!(normalize_currency("MATIC"), "MATIC");
        assert_eq!(normalize_currency("ATOM"), "ATOM");
        assert_eq!(normalize_currency("LINK"), "LINK");
    }

    #[test]
    fn test_normalize_currency_generic_stripping() {
        // Unknown 4+ char codes with X/Z prefix should be stripped
        assert_eq!(normalize_currency("XADA"), "ADA");
        assert_eq!(normalize_currency("XDOGE"), "DOGE");
        assert_eq!(normalize_currency("XMATIC"), "MATIC");
        assert_eq!(normalize_currency("ZSEK"), "SEK");
        assert_eq!(normalize_currency("ZNOK"), "NOK");
    }

    #[test]
    fn test_normalize_currency_short_codes_not_stripped() {
        // Short codes (< 4 chars) should not be stripped even with X/Z
        assert_eq!(normalize_currency("XRP"), "XRP");
        assert_eq!(normalize_currency("XMR"), "XMR");
        assert_eq!(normalize_currency("ZEC"), "ZEC");
    }

    #[test]
    fn test_is_fiat_currency() {
        // Fiat currencies
        assert!(is_fiat_currency("EUR"));
        assert!(is_fiat_currency("USD"));
        assert!(is_fiat_currency("GBP"));
        assert!(is_fiat_currency("JPY"));
        assert!(is_fiat_currency("CAD"));
        assert!(is_fiat_currency("AUD"));
        assert!(is_fiat_currency("CHF"));
        assert!(is_fiat_currency("KRW"));
        // Crypto currencies
        assert!(!is_fiat_currency("BTC"));
        assert!(!is_fiat_currency("ETH"));
        assert!(!is_fiat_currency("DOT"));
        assert!(!is_fiat_currency("USDT")); // Stablecoin is not fiat
        assert!(!is_fiat_currency("USDC")); // Stablecoin is not fiat
    }

    // ------------------------------------------------------------------------
    // Pair Parsing Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_parse_pair_classic_kraken() {
        assert_eq!(parse_pair("XXBTZEUR"), ("BTC".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("XETHXXBT"), ("ETH".to_owned(), "BTC".to_owned()));
        assert_eq!(parse_pair("XXBTZUSD"), ("BTC".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("XETHZEUR"), ("ETH".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("XXRPZEUR"), ("XRP".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("XLTCZUSD"), ("LTC".to_owned(), "USD".to_owned()));
    }

    #[test]
    fn test_parse_pair_altcoins() {
        // Newer altcoin pairs without X prefix
        assert_eq!(parse_pair("DOTUSD"), ("DOT".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("DOTEUR"), ("DOT".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("SOLUSD"), ("SOL".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("SOLEUR"), ("SOL".to_owned(), "EUR".to_owned()));
        assert_eq!(parse_pair("MATICUSD"), ("MATIC".to_owned(), "USD".to_owned()));
        assert_eq!(parse_pair("ATOMUSD"), ("ATOM".to_owned(), "USD".to_owned()));
    }

    #[test]
    fn test_parse_pair_crypto_to_crypto() {
        assert_eq!(parse_pair("XETHXXBT"), ("ETH".to_owned(), "BTC".to_owned()));
        assert_eq!(parse_pair("XXRPXXBT"), ("XRP".to_owned(), "BTC".to_owned()));
        assert_eq!(parse_pair("DOTXBT"), ("DOT".to_owned(), "BTC".to_owned()));
        assert_eq!(parse_pair("SOLETH"), ("SOL".to_owned(), "ETH".to_owned()));
    }

    #[test]
    fn test_parse_pair_fallback() {
        // Test fallback logic for unusual pairs
        assert_eq!(parse_pair("ABCDEF"), ("ABC".to_owned(), "DEF".to_owned()));
        assert_eq!(parse_pair("ABCDEFGH"), ("ABCD".to_owned(), "EFGH".to_owned()));
    }

    // ------------------------------------------------------------------------
    // DateTime Parsing Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_datetime_with_subseconds() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45.1234\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.time.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:30:45");
    }

    #[test]
    fn test_datetime_without_subseconds() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.time.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:30:45");
    }

    #[test]
    fn test_datetime_iso8601() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15T10:30:45Z\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.time.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:30:45");
    }

    // ------------------------------------------------------------------------
    // Trade CSV Parsing Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_parse_trade_buy() {
        let csv_data = format!("{}\n\"ABC123\",\"ORD456\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45.1234\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.tx_id, "ABC123");
        assert_eq!(record.trade_type, KrakenTradeType::Buy);
        assert_eq!(record.vol, dec!(0.01));
        assert_eq!(record.cost, dec!(400.00));
        assert_eq!(record.fee, dec!(0.10));

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(incoming.quantity, dec!(0.01));
                assert_eq!(outgoing.currency, "EUR");
                assert_eq!(outgoing.quantity, dec!(400.00));
            }
            _ => panic!("Expected Trade operation"),
        }
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(0.10));
        assert_eq!(tx.fee.as_ref().unwrap().currency, "EUR");
        assert!(tx.description.unwrap().contains("ABC123"));
    }

    #[test]
    fn test_parse_trade_sell() {
        let csv_data = format!("{}\n\"DEF789\",\"ORD012\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-16 14:20:00.0000\",\"sell\",\"market\",\"41000.0\",\"410.00\",\"0.15\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "EUR");
                assert_eq!(incoming.quantity, dec!(410.00));
                assert_eq!(outgoing.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(0.01));
            }
            _ => panic!("Expected Trade operation"),
        }
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(0.15));
    }

    #[test]
    fn test_parse_trade_zero_fee() {
        let csv_data = format!("{}\n\"TX123\",\"ORD456\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.0\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        assert!(tx.fee.is_none());
    }

    #[test]
    fn test_parse_trade_margin() {
        // Margin trade with 100 EUR margin used
        let csv_data = format!("{}\n\"TX123\",\"ORD456\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"100.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.margin, dec!(100.0));

        let tx: Transaction = record.into();
        // Description should contain MARGIN warning
        let desc = tx.description.unwrap();
        assert!(desc.contains("MARGIN"));
        assert!(desc.contains("100"));
        assert!(desc.contains("review for tax implications"));
    }

    #[test]
    fn test_parse_trade_altcoin_pair() {
        let csv_data = format!("{}\n\"TX123\",\"ORD456\",\"DOTUSD\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"7.5\",\"75.00\",\"0.05\",\"10.0\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "DOT");
                assert_eq!(incoming.quantity, dec!(10.0));
                assert_eq!(outgoing.currency, "USD");
                assert_eq!(outgoing.quantity, dec!(75.00));
            }
            _ => panic!("Expected Trade operation"),
        }
    }

    #[test]
    fn test_parse_trade_with_all_fields() {
        // Test with all fields including the extended position fields
        let csv_data = format!("{}\n\"TX123\",\"ORD456\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"L1,L2\",\"POST1\",\"ok\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"0.0\",\"1\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        // Should still parse correctly with extra fields
        assert_eq!(record.tx_id, "TX123");
        assert_eq!(record.vol, dec!(0.01));
    }

    // ------------------------------------------------------------------------
    // Ledger CSV Parsing Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_parse_ledger_crypto_deposit() {
        let csv_data = format!("{}\n\"LED123\",\"REF456\",\"2024-01-10 08:00:00.0000\",\"deposit\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.5\",\"0.0\",\"0.5\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Deposit);
        assert!(!record.should_skip());

        let tx: Transaction = record.into();
        // Crypto deposits should use Receive operation
        assert!(tx.operation.is_receive());
        match &tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.currency, "BTC");
                assert_eq!(amount.quantity, dec!(0.5));
            }
            _ => panic!("Expected Receive operation for crypto deposit"),
        }
        // Blockchain should be set for transaction matching
        assert_eq!(tx.blockchain, Some("BTC".to_owned()));
    }

    #[test]
    fn test_parse_ledger_fiat_deposit() {
        let csv_data = format!("{}\n\"LED123\",\"REF456\",\"2024-01-10 08:00:00.0000\",\"deposit\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"1000.0\",\"0.0\",\"1000.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        // Fiat deposits should use FiatDeposit operation
        match &tx.operation {
            Operation::FiatDeposit(amount) => {
                assert_eq!(amount.currency, "EUR");
                assert_eq!(amount.quantity, dec!(1000.0));
            }
            _ => panic!("Expected FiatDeposit operation for EUR deposit"),
        }
        // Blockchain should be set even for fiat (for consistency)
        assert_eq!(tx.blockchain, Some("EUR".to_owned()));
    }

    #[test]
    fn test_parse_ledger_crypto_withdrawal() {
        let csv_data = format!("{}\n\"LED789\",\"REF012\",\"2024-01-20 16:00:00.0000\",\"withdrawal\",\"\",\"currency\",\"\",\"XETH\",\"\",\"-1.0\",\"0.005\",\"0.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Withdrawal);

        let tx: Transaction = record.into();
        // Crypto withdrawals should use Send operation
        assert!(tx.operation.is_send());
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(0.005));
        assert_eq!(tx.fee.as_ref().unwrap().currency, "ETH");
        // Blockchain should be set for transaction matching
        assert_eq!(tx.blockchain, Some("ETH".to_owned()));
    }

    #[test]
    fn test_parse_ledger_fiat_withdrawal() {
        let csv_data = format!("{}\n\"LED789\",\"REF012\",\"2024-01-20 16:00:00.0000\",\"withdrawal\",\"\",\"currency\",\"\",\"ZUSD\",\"\",\"-500.0\",\"5.0\",\"0.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        // Fiat withdrawals should use FiatWithdrawal operation
        match &tx.operation {
            Operation::FiatWithdrawal(amount) => {
                assert_eq!(amount.currency, "USD");
                assert_eq!(amount.quantity, dec!(500.0));
            }
            _ => panic!("Expected FiatWithdrawal operation for USD withdrawal"),
        }
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(5.0));
        assert_eq!(tx.blockchain, Some("USD".to_owned()));
    }

    #[test]
    fn test_parse_ledger_staking() {
        let csv_data = format!("{}\n\"STK123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"staking\",\"\",\"currency\",\"\",\"DOT\",\"\",\"0.1\",\"0.0\",\"10.1\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Staking);

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Staking(amount) => {
                assert_eq!(amount.currency, "DOT");
                assert_eq!(amount.quantity, dec!(0.1));
            }
            _ => panic!("Expected Staking operation"),
        }
    }

    #[test]
    fn test_parse_ledger_reward() {
        let csv_data = format!("{}\n\"RWD123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"reward\",\"\",\"currency\",\"\",\"ETH\",\"\",\"0.001\",\"0.0\",\"1.001\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Reward);

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Staking(amount) => {
                assert_eq!(amount.currency, "ETH");
                assert_eq!(amount.quantity, dec!(0.001));
            }
            _ => panic!("Expected Staking operation for reward"),
        }
    }

    #[test]
    fn test_parse_ledger_dividend() {
        let csv_data = format!("{}\n\"DIV123\",\"REF789\",\"2024-03-15 00:00:00.0000\",\"dividend\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.0001\",\"0.0\",\"1.0001\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Dividend);

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Income(amount) => {
                assert_eq!(amount.currency, "BTC");
                assert_eq!(amount.quantity, dec!(0.0001));
            }
            _ => panic!("Expected Income operation for dividend"),
        }
    }

    #[test]
    fn test_parse_ledger_transfer() {
        // Positive transfer (receiving)
        let csv_data = format!("{}\n\"TRF123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"transfer\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"100.0\",\"0.0\",\"1100.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        let tx: Transaction = record.into();
        assert!(tx.operation.is_receive());

        // Negative transfer (sending)
        let csv_data = format!("{}\n\"TRF124\",\"REF790\",\"2024-02-01 00:00:00.0000\",\"transfer\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"-100.0\",\"0.0\",\"900.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        let tx: Transaction = record.into();
        assert!(tx.operation.is_send());
    }

    #[test]
    fn test_parse_ledger_adjustment() {
        // Positive adjustment (e.g., airdrop, correction)
        let csv_data = format!("{}\n\"ADJ123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"adjustment\",\"\",\"currency\",\"\",\"DOT\",\"\",\"5.0\",\"0.0\",\"105.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Income(amount) => {
                assert_eq!(amount.quantity, dec!(5.0));
            }
            _ => panic!("Expected Income operation for positive adjustment"),
        }

        // Negative adjustment
        let csv_data = format!("{}\n\"ADJ124\",\"REF790\",\"2024-02-01 00:00:00.0000\",\"adjustment\",\"\",\"currency\",\"\",\"DOT\",\"\",\"-5.0\",\"0.0\",\"95.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Expense(amount) => {
                assert_eq!(amount.quantity, dec!(5.0));
            }
            _ => panic!("Expected Expense operation for negative adjustment"),
        }
    }

    #[test]
    fn test_parse_ledger_margin() {
        // Positive margin (receiving collateral back)
        let csv_data = format!("{}\n\"MRG123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"margin\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"500.0\",\"0.0\",\"1500.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.ledger_type, KrakenLedgerType::Margin);
        let tx: Transaction = record.into();
        assert!(tx.operation.is_receive());

        // Negative margin (posting collateral)
        let csv_data = format!("{}\n\"MRG124\",\"REF790\",\"2024-02-01 00:00:00.0000\",\"margin\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"-500.0\",\"0.0\",\"500.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        let tx: Transaction = record.into();
        assert!(tx.operation.is_send());
    }

    #[test]
    fn test_parse_ledger_rollover() {
        // Rollover fee (negative)
        let csv_data = format!("{}\n\"ROL123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"rollover\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"-0.50\",\"0.0\",\"999.50\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.ledger_type, KrakenLedgerType::Rollover);
        let tx: Transaction = record.into();
        assert!(tx.operation.is_send());
    }

    #[test]
    fn test_parse_ledger_settled() {
        // Settled position (positive = profit)
        let csv_data = format!("{}\n\"STL123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"settled\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"100.0\",\"0.0\",\"1100.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.ledger_type, KrakenLedgerType::Settled);
        let tx: Transaction = record.into();
        assert!(tx.operation.is_receive());
    }

    #[test]
    fn test_parse_ledger_sale() {
        // Sale (fiat conversion, negative = selling)
        let csv_data = format!("{}\n\"SAL123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"sale\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"-0.1\",\"0.0\",\"0.9\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(record.ledger_type, KrakenLedgerType::Sale);
        let tx: Transaction = record.into();
        assert!(tx.operation.is_send());
    }

    #[test]
    fn test_ledger_trade_entries_are_skipped() {
        let csv_data = format!("{}\n\"TRD1\",\"REF1\",\"2024-01-15 10:30:45\",\"trade\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.01\",\"0.0\",\"0.01\"\n\"TRD2\",\"REF1\",\"2024-01-15 10:30:45\",\"spend\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"-400.0\",\"0.0\",\"600.0\"\n\"TRD3\",\"REF1\",\"2024-01-15 10:30:45\",\"receive\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.01\",\"0.0\",\"0.01\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());

        for result in rdr.deserialize() {
            let record: KrakenLedger = result.unwrap();
            assert!(record.should_skip(), "Trade/Spend/Receive entries should be skipped");
        }
    }

    #[test]
    fn test_parse_ledger_unknown_type() {
        let csv_data = format!("{}\n\"UNK123\",\"REF789\",\"2024-02-01 00:00:00.0000\",\"newtype\",\"\",\"currency\",\"\",\"DOT\",\"\",\"1.0\",\"0.0\",\"11.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        assert_eq!(record.ledger_type, KrakenLedgerType::Unknown);

        let tx: Transaction = record.into();
        // Unknown positive amount should become receive
        assert!(tx.operation.is_receive());
    }

    // ------------------------------------------------------------------------
    // Load Function Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_load_kraken_trades_csv() {
        // Kraken exports newest first, so TX2 (newer) comes before TX1 (older) in the file
        let csv_content = format!("{}\n\"TX2\",\"ORD2\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-16 14:20:00\",\"sell\",\"market\",\"41000.0\",\"410.00\",\"0.15\",\"0.01\",\"0.0\",\"\",\"\"\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(csv_content.as_bytes()).unwrap();

        let transactions = load_kraken_trades_csv(temp_file.path()).unwrap();

        assert_eq!(transactions.len(), 2);
        // After reverse, oldest (TX1) should be first
        assert!(transactions[0].description.as_ref().unwrap().contains("TX1"));
        assert!(transactions[1].description.as_ref().unwrap().contains("TX2"));
    }

    #[test]
    fn test_load_kraken_ledger_csv_skips_trades() {
        // Kraken exports newest first, so WTH1 (newest) comes first, DEP1 (oldest) comes last
        let csv_content = format!("{}\n\"WTH1\",\"REF3\",\"2024-01-20 16:00:00\",\"withdrawal\",\"\",\"currency\",\"\",\"XETH\",\"\",\"-1.0\",\"0.005\",\"0.0\"\n\"TRD1\",\"REF2\",\"2024-01-15 10:30:45\",\"trade\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.01\",\"0.0\",\"0.51\"\n\"TRD2\",\"REF2\",\"2024-01-15 10:30:45\",\"spend\",\"\",\"currency\",\"\",\"ZEUR\",\"\",\"-400.0\",\"0.0\",\"600.0\"\n\"DEP1\",\"REF1\",\"2024-01-10 08:00:00\",\"deposit\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.5\",\"0.0\",\"0.5\"\n", LEDGER_HEADER);
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(csv_content.as_bytes()).unwrap();

        let transactions = load_kraken_ledger_csv(temp_file.path()).unwrap();

        // Should only have 2 transactions (deposit and withdrawal), not 4
        // After reverse, oldest (DEP1) should be first
        assert_eq!(transactions.len(), 2);
        assert!(transactions[0].description.as_ref().unwrap().contains("DEP1"));
        assert!(transactions[1].description.as_ref().unwrap().contains("WTH1"));
    }

    #[test]
    fn test_load_empty_csv() {
        let csv_content = format!("{}\n", TRADES_HEADER);
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(csv_content.as_bytes()).unwrap();

        let transactions = load_kraken_trades_csv(temp_file.path()).unwrap();
        assert!(transactions.is_empty());
    }

    // ------------------------------------------------------------------------
    // Edge Case Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_very_small_amounts() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"0.0001\",\"0.00000001\",\"0.0000000025\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.0000000025));
                assert_eq!(outgoing.quantity, dec!(0.0001));
            }
            _ => panic!("Expected Trade operation"),
        }
    }

    #[test]
    fn test_large_amounts() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"1000000.00\",\"100.00\",\"25.0\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(25.0));
                assert_eq!(outgoing.quantity, dec!(1000000.00));
            }
            _ => panic!("Expected Trade operation"),
        }
        assert_eq!(tx.fee.as_ref().unwrap().quantity, dec!(100.00));
    }

    // ------------------------------------------------------------------------
    // Error Handling Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_invalid_datetime_format() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"invalid-date\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let result: Result<KrakenTrade, _> = rdr.deserialize().next().unwrap();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid datetime"));
    }

    #[test]
    fn test_invalid_trade_type() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"invalid\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let result: Result<KrakenTrade, _> = rdr.deserialize().next().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_decimal_format() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"not-a-number\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let result: Result<KrakenTrade, _> = rdr.deserialize().next().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_field() {
        // Missing the 'vol' field (truncated row)
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let result: Result<KrakenTrade, _> = rdr.deserialize().next().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = load_kraken_trades_csv(Path::new("/nonexistent/path/trades.csv"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_csv_with_parse_error() {
        // Create a CSV with invalid data in the middle
        let csv_content = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"400.00\",\"0.10\",\"0.01\",\"0.0\",\"\",\"\"\n\"TX2\",\"ORD2\",\"XXBTZEUR\",\"currency\",\"\",\"invalid-date\",\"sell\",\"market\",\"41000.0\",\"410.00\",\"0.15\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(csv_content.as_bytes()).unwrap();

        let result = load_kraken_trades_csv(temp_file.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("line 3")); // Error should indicate the problematic line
    }

    #[test]
    fn test_ledger_invalid_amount() {
        let csv_data = format!("{}\n\"LED123\",\"REF456\",\"2024-01-10 08:00:00\",\"deposit\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"not-a-number\",\"0.0\",\"0.5\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let result: Result<KrakenLedger, _> = rdr.deserialize().next().unwrap();
        assert!(result.is_err());
    }

    // ------------------------------------------------------------------------
    // Edge Case Tests - Pair Parsing
    // ------------------------------------------------------------------------

    #[test]
    fn test_parse_pair_empty_string() {
        let (base, quote) = parse_pair("");
        assert_eq!(base, "");
        assert_eq!(quote, "");
    }

    #[test]
    fn test_parse_pair_very_short() {
        let (base, quote) = parse_pair("AB");
        assert_eq!(base, "AB");
        assert_eq!(quote, "");
    }

    #[test]
    fn test_parse_pair_exactly_six_chars() {
        // Should split at position 3
        let (base, quote) = parse_pair("ABCDEF");
        assert_eq!(base, "ABC");
        assert_eq!(quote, "DEF");
    }

    #[test]
    fn test_normalize_currency_empty_string() {
        assert_eq!(normalize_currency(""), "");
    }

    #[test]
    fn test_normalize_currency_single_char() {
        assert_eq!(normalize_currency("X"), "X");
        assert_eq!(normalize_currency("Z"), "Z");
    }

    // ------------------------------------------------------------------------
    // Zero Amount Edge Cases
    // ------------------------------------------------------------------------

    #[test]
    fn test_trade_with_zero_volume() {
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"0.0\",\"0.0\",\"0.0\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.0));
                assert_eq!(outgoing.quantity, dec!(0.0));
            }
            _ => panic!("Expected Trade operation"),
        }
        assert!(tx.fee.is_none()); // Zero fee should not be set
    }

    #[test]
    fn test_ledger_with_zero_amount() {
        let csv_data = format!("{}\n\"LED123\",\"REF456\",\"2024-01-10 08:00:00\",\"deposit\",\"\",\"currency\",\"\",\"XXBT\",\"\",\"0.0\",\"0.0\",\"0.0\"\n", LEDGER_HEADER);
        let mut rdr = csv::Reader::from_reader(csv_data.as_bytes());
        let record: KrakenLedger = rdr.deserialize().next().unwrap().unwrap();

        let tx: Transaction = record.into();
        match &tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.0));
            }
            _ => panic!("Expected Receive operation"),
        }
    }

    // ------------------------------------------------------------------------
    // Negative Amount Edge Cases
    // ------------------------------------------------------------------------

    #[test]
    fn test_trade_with_negative_values() {
        // Negative cost and fee values (shouldn't happen in real data, but test handling)
        let csv_data = format!("{}\n\"TX1\",\"ORD1\",\"XXBTZEUR\",\"currency\",\"\",\"2024-01-15 10:30:45\",\"buy\",\"limit\",\"40000.0\",\"-400.00\",\"-0.10\",\"0.01\",\"0.0\",\"\",\"\"\n", TRADES_HEADER);
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(csv_data.as_bytes());
        let record: KrakenTrade = rdr.deserialize().next().unwrap().unwrap();

        // Should parse without error (negative values are passed through)
        let tx: Transaction = record.into();
        assert!(tx.description.is_some());
    }
}
