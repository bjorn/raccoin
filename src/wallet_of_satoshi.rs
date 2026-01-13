use std::{convert::TryFrom, path::Path};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    base::{Amount, Transaction},
    CsvSpec, TransactionSourceType,
};
use linkme::distributed_slice;

const BTC_CURRENCY: &str = "BTC";
const LIGHTNING_CURRENCY: &str = "LIGHTNING";

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
enum EntryType {
    Credit,
    Debit,
}

// CSV header: utcDate,type,currency,amount,fees,address,description,pointOfSale
// Non-custodial: "utcDate","type","currency","amount","fees","status","address","description","transactionId","pointOfSale"
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WalletOfSatoshiRecord {
    #[serde(rename = "type")]
    entry_type: EntryType,
    utc_date: DateTime<FixedOffset>,
    currency: String,
    amount: Decimal,
    fees: Decimal,
    // status: Option<String>, // generally "PAID", absent in custodial CSV export
    // address: String,
    description: String,
    transaction_id: Option<String>,
    point_of_sale: bool,
}

impl WalletOfSatoshiRecord {
    fn ensure_supported_currency(&self) -> Result<()> {
        let currency = &self.currency;
        if currency.eq_ignore_ascii_case(BTC_CURRENCY) || currency.eq_ignore_ascii_case(LIGHTNING_CURRENCY)
        {
            Ok(())
        } else {
            Err(anyhow!(
                "Wallet of Satoshi CSV contains unsupported currency '{}'",
                self.currency
            ))
        }
    }

    fn btc_amount(&self) -> Amount {
        Amount::new(self.amount, BTC_CURRENCY.to_owned())
    }

    fn fee_amount(&self) -> Option<Amount> {
        if self.fees.is_zero() {
            None
        } else {
            Some(Amount::new(self.fees, BTC_CURRENCY.to_owned()))
        }
    }

    fn compose_description(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(text) = non_empty(&self.description) {
            parts.push(text.to_owned());
        }

        if self.point_of_sale {
            parts.push("Point of sale".to_owned());
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }
}

impl TryFrom<WalletOfSatoshiRecord> for Transaction {
    type Error = anyhow::Error;

    fn try_from(record: WalletOfSatoshiRecord) -> Result<Self> {
        record.ensure_supported_currency()?;

        let timestamp = record.utc_date.naive_utc();
        let amount = if matches!(record.entry_type, EntryType::Credit)
        {
            let mut gross = record.btc_amount();
            gross.quantity += record.fees;
            gross
        } else {
            record.btc_amount()
        };

        let mut tx = match record.entry_type {
            EntryType::Credit => Transaction::receive(timestamp, amount),
            EntryType::Debit => Transaction::send(timestamp, amount),
        };

        tx.fee = record.fee_amount();
        tx.description = record.compose_description();
        tx.tx_hash = record.transaction_id;

        Ok(tx)
    }
}

pub(crate) fn load_wallet_of_satoshi_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for record in reader.deserialize() {
        let row: WalletOfSatoshiRecord = record?;
        let utc_date = row.utc_date.clone();
        let tx = Transaction::try_from(row).with_context(|| {
            format!("Failed to convert Wallet of Satoshi row dated {}", utc_date)
        })?;
        transactions.push(tx);
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
pub(crate) static WALLET_OF_SATOSHI_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "WalletOfSatoshiCsv",
    label: "Wallet of Satoshi (CSV)",
    csv: Some(CsvSpec {
        headers: &[
            "utcDate",
            "type",
            "currency",
            "amount",
            "fees",
            "address",
            "description",
            "pointOfSale",
        ],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_wallet_of_satoshi_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
pub(crate) static WALLET_OF_SATOSHI_NON_CUSTODIAL_CSV_SOURCE: TransactionSourceType =
    TransactionSourceType {
        id: "WalletOfSatoshiNonCustodialCsv",
        label: "Wallet of Satoshi Self-Custody (CSV)",
        csv: Some(CsvSpec {
            headers: &[
                "utcDate",
                "type",
                "currency",
                "amount",
                "fees",
                "status",
                "address",
                "description",
                "transactionId",
                "pointOfSale",
            ],
            delimiters: &[b','],
            skip_lines: 0,
        }),
        detect: None,
        load_sync: Some(load_wallet_of_satoshi_csv),
        load_async: None,
    };

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Operation;
    use csv::StringRecord;
    use rust_decimal_macros::dec;

    #[test]
    fn credit_row_converts_to_receive() {
        let csv =
            "2025-12-17T09:32:39.872Z,CREDIT,LIGHTNING,0.00001,0,,addr,Test sending to WoS,,false";
        let tx = parse_csv_row(csv).unwrap();

        let expected_ts = DateTime::parse_from_rfc3339("2025-12-17T09:32:39.872Z")
            .unwrap()
            .naive_utc();
        assert_eq!(tx.timestamp, expected_ts);
        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.00001));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }
        assert!(tx.fee.is_none());
        assert!(tx.tx_hash.is_none());
        assert_eq!(tx.description.as_deref(), Some("Test sending to WoS"));
    }

    #[test]
    fn debit_row_converts_to_send_with_fee() {
        let csv = "2025-12-17T11:51:23.628Z,DEBIT,LIGHTNING,0.00000979,0.00000010,,addr,\"Thanks, sats received!\",,true";
        let tx = parse_csv_row(csv).unwrap();

        let expected_ts = DateTime::parse_from_rfc3339("2025-12-17T11:51:23.628Z")
            .unwrap()
            .naive_utc();
        assert_eq!(tx.timestamp, expected_ts);
        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.00000979));
            }
            other => panic!("expected send, got {:?}", other),
        }
        let fee = tx.fee.expect("fee set");
        assert_eq!(fee.quantity, dec!(0.00000010));
        assert!(tx.tx_hash.is_none());
        assert_eq!(
            tx.description.as_deref(),
            Some("Thanks, sats received! | Point of sale")
        );
    }

    #[test]
    fn unsupported_currency_is_rejected() {
        let csv = "2025-01-01T00:00:00Z,CREDIT,EUR,0.1,0,,addr,,,false";
        assert!(parse_csv_row(csv).is_err());
    }

    // For incoming BTC on-chain, the CSV amount is net of the fee
    #[test]
    fn credit_btc_adds_fee_to_amount() {
        let csv =
            "2025-02-03T04:05:06Z,CREDIT,BTC,0.00042,0.00001,,addr,On-chain receive,abc123,false";
        let tx = parse_csv_row(csv).unwrap();
        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.00043));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }
        let fee = tx.fee.expect("fee set");
        assert_eq!(fee.quantity, dec!(0.00001));
    }

    fn parse_csv_row(csv: &str) -> Result<Transaction> {
        let header = StringRecord::from(vec![
            "utcDate","type","currency","amount","fees","status","address","description","transactionId","pointOfSale",
        ]);
        let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        reader.set_headers(header);
        let record: WalletOfSatoshiRecord = reader.deserialize().next().unwrap().unwrap();
        Transaction::try_from(record)
    }
}
