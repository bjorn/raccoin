use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{
    base::{Amount, Transaction},
    time::deserialize_date_time,
    CsvSpec, TransactionSource,
};
use linkme::distributed_slice;

const EUR_CURRENCY: &str = "EUR";

#[derive(Debug, Deserialize, Clone)]
enum BisonTransactionType {
    Deposit,
    Withdraw,
    Buy,
    Sell,
}

// CSV header (semicolon-delimited with spaces after separators):
// Transaction ID; Transaction type; Currency; Asset; Eur (amount); Asset (amount); Asset (market price); Fee; Date (UTC - Coordinated Universal Time)
#[derive(Debug, Deserialize)]
struct BisonRecord {
    #[serde(rename = "Transaction ID")]
    transaction_id: String,
    #[serde(rename = "Transaction type")]
    transaction_type: BisonTransactionType,
    #[serde(rename = "Currency", deserialize_with = "deserialize_empty_string_as_none")]
    currency: Option<String>,
    #[serde(rename = "Asset", deserialize_with = "deserialize_empty_string_as_none")]
    asset: Option<String>,
    #[serde(rename = "Eur (amount)")]
    eur_amount: Option<Decimal>,
    #[serde(rename = "Asset (amount)")]
    asset_amount: Option<Decimal>,
    // #[serde(rename = "Asset (market price)")]
    // asset_market_price: Option<Decimal>,
    #[serde(rename = "Fee")]
    fee: Option<Decimal>,
    #[serde(rename = "Date (UTC - Coordinated Universal Time)", deserialize_with = "deserialize_date_time")]
    date_utc: NaiveDateTime,
}

fn deserialize_empty_string_as_none<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

impl BisonRecord {
    fn into_transaction(self) -> Result<Transaction> {
        let timestamp = self.date_utc;

        // Values like "Eur" and "Btc" are observed, so we need to uppercase
        let currency = self
            .currency
            .as_deref()
            .map(|value| value.to_uppercase());
        let asset = self
            .asset
            .as_deref()
            .map(|value| value.to_uppercase());

        let mut tx = match self.transaction_type {
            BisonTransactionType::Deposit => {
                if let Some(asset_currency) = asset.as_deref() {
                    let amount = self
                        .asset_amount
                        .ok_or_else(|| anyhow!("Bison crypto Deposit missing asset amount"))?;
                    Transaction::receive(
                        timestamp,
                        Amount::new(amount, asset_currency.to_owned()),
                    )
                } else {
                    let eur = self
                        .eur_amount
                        .ok_or_else(|| anyhow!("Bison Deposit missing EUR amount"))?;
                    Transaction::fiat_deposit(
                        timestamp,
                        Amount::new(eur, EUR_CURRENCY.to_owned()),
                    )
                }
            }
            BisonTransactionType::Withdraw => {
                if let Some(asset_currency) = asset.as_deref() {
                    let amount = self
                        .asset_amount
                        .ok_or_else(|| anyhow!("Bison crypto Withdraw missing asset amount"))?;
                    Transaction::send(
                        timestamp,
                        Amount::new(amount, asset_currency.to_owned()),
                    )
                } else {
                    let eur = self
                        .eur_amount
                        .ok_or_else(|| anyhow!("Bison fiat Withdraw missing EUR amount"))?;
                    Transaction::fiat_withdrawal(
                        timestamp,
                        Amount::new(eur, EUR_CURRENCY.to_owned()),
                    )
                }
            }
            BisonTransactionType::Buy => {
                let asset_currency = asset
                    .as_deref()
                    .ok_or_else(|| anyhow!("Bison Buy missing asset"))?;
                let eur = self
                    .eur_amount
                    .ok_or_else(|| anyhow!("Bison Buy missing EUR amount"))?;
                let crypto = self
                    .asset_amount
                    .ok_or_else(|| anyhow!("Bison Buy missing asset amount"))?;
                Transaction::trade(
                    timestamp,
                    Amount::new(crypto, asset_currency.to_owned()),
                    Amount::new(eur, EUR_CURRENCY.to_owned()),
                )
            }
            BisonTransactionType::Sell => {
                let asset_currency = asset
                    .as_deref()
                    .ok_or_else(|| anyhow!("Bison Sell missing asset"))?;
                let eur = self
                    .eur_amount
                    .ok_or_else(|| anyhow!("Bison Sell missing EUR amount"))?;
                let crypto = self
                    .asset_amount
                    .ok_or_else(|| anyhow!("Bison Sell missing asset amount"))?;
                Transaction::trade(
                    timestamp,
                    Amount::new(eur, EUR_CURRENCY.to_owned()),
                    Amount::new(crypto, asset_currency.to_owned()),
                )
            }
        };

        tx.tx_hash = if self.transaction_id.is_empty() {
            None
        } else {
            Some(self.transaction_id)
        };

        // Determine the fee currency: EUR for fiat ops and buy/sell, asset for crypto withdrawals
        if let Some(fee) = self.fee {
            if !fee.is_zero() {
                let fee_currency = match self.transaction_type {
                    BisonTransactionType::Withdraw => {
                        asset
                            .as_ref()
                            .map(|asset| asset.to_owned())
                            .or_else(|| currency.clone())
                            .ok_or_else(|| anyhow!("Bison Withdraw missing currency or asset for fee currency"))?
                    }
                    BisonTransactionType::Deposit => {
                        asset
                            .as_ref()
                            .map(|asset| asset.to_owned())
                            .or_else(|| currency.clone())
                            .ok_or_else(|| anyhow!("Bison Deposit missing currency or asset for fee currency"))?
                    }
                    _ => EUR_CURRENCY.to_owned(),
                };
                tx.fee = Some(Amount::new(fee, fee_currency));
            }
        }

        Ok(tx)
    }
}

fn load_bison_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .trim(csv::Trim::All)
        .from_path(input_path)?;

    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: BisonRecord = result?;
        let date = record.date_utc;
        let tx = record
            .into_transaction()
            .map_err(|e| anyhow!("Failed to convert Bison row dated {}: {}", date, e))?;
        transactions.push(tx);
    }

    Ok(transactions)
}

// Trimmed Bison CSV headers
const BISON_HEADERS: [&str; 9] = [
    "Transaction ID",
    "Transaction type",
    "Currency",
    "Asset",
    "Eur (amount)",
    "Asset (amount)",
    "Asset (market price)",
    "Fee",
    "Date (UTC - Coordinated Universal Time)",
];

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BISON_CSV: TransactionSource = TransactionSource {
    id: "BisonCsv",
    label: "Bison (CSV)",
    csv: &[CsvSpec {
        headers: &BISON_HEADERS,
        delimiters: &[b';'],
        skip_lines: 0,
        trim: csv::Trim::All,
    }],
    detect: None,
    load_sync: Some(load_bison_csv),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Operation;
    use crate::time::parse_date_time;
    use csv::StringRecord;
    use rust_decimal_macros::dec;

    #[test]
    fn fiat_deposit() {
        let csv = "test-tx-id-aaa; Deposit; Eur; ; 321.45; ; ; 0; 2024-08-09 10:11:12";
        let tx = parse_csv_row(csv).unwrap();
        assert_eq!(
            tx.timestamp,
            parse_date_time("2024-08-09 10:11:12").unwrap()
        );
        match tx.operation {
            Operation::FiatDeposit(amount) => {
                assert_eq!(amount.quantity, dec!(321.45));
                assert_eq!(amount.currency, "EUR");
            }
            other => panic!("expected FiatDeposit, got {:?}", other),
        }
        assert!(tx.fee.is_none());
    }

    #[test]
    fn crypto_deposit_is_receive() {
        let csv = "test-tx-id-abc; Deposit; ; Btc; ; 0.01234567; ; 0; 2024-09-10 03:04:05";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.01234567));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected Receive, got {:?}", other),
        }
    }

    #[test]
    fn fiat_withdrawal() {
        let csv = "test-tx-id-def; Withdraw; Eur; ; 77.01; ; ; 0; 2024-09-11 06:07:08";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::FiatWithdrawal(amount) => {
                assert_eq!(amount.quantity, dec!(77.01));
                assert_eq!(amount.currency, "EUR");
            }
            other => panic!("expected FiatWithdrawal, got {:?}", other),
        }
    }

    #[test]
    fn crypto_withdrawal_is_send() {
        let csv = "test-tx-id-ghi; Withdraw; ; Btc; ; 0.0025; ; 0; 2024-09-12 09:10:11";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.0025));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected Send, got {:?}", other),
        }
    }

    #[test]
    fn buy_creates_trade() {
        let csv = "test-tx-id-jkl; Buy; Eur; Btc; 123.45; 0.006789; 12345.67; 0; 2024-09-13 12:13:14";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.006789));
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(123.45));
                assert_eq!(outgoing.currency, "EUR");
            }
            other => panic!("expected Trade, got {:?}", other),
        }
    }

    #[test]
    fn sell_creates_trade() {
        let csv = "test-tx-id-mno; Sell; Eur; Btc; 210.50; 0.004321; 21000.00; 0; 2024-09-14 15:16:17";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(210.50));
                assert_eq!(incoming.currency, "EUR");
                assert_eq!(outgoing.quantity, dec!(0.004321));
                assert_eq!(outgoing.currency, "BTC");
            }
            other => panic!("expected Trade, got {:?}", other),
        }
    }

    #[test]
    fn nonzero_fee_is_set() {
        let csv = "test-tx-id-pqr; Buy; Eur; Btc; 50.00; 0.0005; ; 0.75; 2024-09-15 18:19:20";
        let tx = parse_csv_row(csv).unwrap();
        let fee = tx.fee.expect("fee should be set");
        assert_eq!(fee.quantity, dec!(0.75));
        assert_eq!(fee.currency, "EUR");
    }

    #[test]
    fn zero_fee_is_none() {
        let csv = "test-tx-id-stu; Buy; Eur; Btc; 75.00; 0.0007; ; 0; 2024-09-16 21:22:23";
        let tx = parse_csv_row(csv).unwrap();
        assert!(tx.fee.is_none());
    }

    #[test]
    fn tx_hash_is_set() {
        let csv = "test-tx-id-xyz; Deposit; Eur; ; 55.00; ; ; 0; 2024-08-01 01:02:03";
        let tx = parse_csv_row(csv).unwrap();
        assert_eq!(tx.tx_hash.as_deref(), Some("test-tx-id-xyz"));
    }

    fn parse_csv_row(csv: &str) -> Result<Transaction> {
        let header = StringRecord::from(&BISON_HEADERS[..]);
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .trim(csv::Trim::All)
            .from_reader(csv.as_bytes());
        reader.set_headers(header);
        let record: BisonRecord = reader.deserialize().next().unwrap().unwrap();
        record.into_transaction()
    }
}
