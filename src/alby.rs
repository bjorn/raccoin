//! Importer for Alby (getalby.com) transaction CSV exports.
//!
//! This is distinct from the Alby Hub format - the getalby.com web wallet
//! exports a different CSV structure.

use std::{convert::TryFrom, path::Path};

use anyhow::{anyhow, Result};
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    base::{Amount, Transaction},
    CsvSpec, TransactionSourceType,
};

const BTC_CURRENCY: &str = "BTC";

/// Scale for converting satoshis to BTC (1 BTC = 100,000,000 sats)
const SATS_SCALE: u32 = 8;

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "lowercase")]
enum InvoiceType {
    Incoming,
    Outgoing,
}

/// CSV header:
/// Invoice Type,Amount,Fee,Creation Date,Settled Date,Memo,Comment,Message,Payer Name,Payer Pubkey,Payment Hash,Preimage,Fiat In Cents,Currency,USD In Cents,Is Boostagram,Is Zap
#[derive(Debug, Deserialize)]
struct AlbyRecord {
    #[serde(rename = "Invoice Type")]
    invoice_type: InvoiceType,
    #[serde(rename = "Amount")]
    amount: i64,
    #[serde(rename = "Fee")]
    fee: Option<i64>,
    #[serde(rename = "Creation Date", deserialize_with = "deserialize_creation_date")]
    creation_date: NaiveDateTime,
    #[serde(rename = "Settled Date")]
    settled_date: i64,
    #[serde(rename = "Memo")]
    memo: String,
    #[serde(rename = "Comment")]
    comment: String,
    #[serde(rename = "Message")]
    message: String,
    #[serde(rename = "Payer Name")]
    payer_name: String,
    // #[serde(rename = "Payer Pubkey")]
    // payer_pubkey: String,
    // #[serde(rename = "Payment Hash")]
    // payment_hash: String,
    // #[serde(rename = "Preimage")]
    // preimage: String,
    // #[serde(rename = "Fiat In Cents")]
    // fiat_in_cents: i64,
    #[serde(rename = "Currency")]
    currency: String,
    // #[serde(rename = "USD In Cents")]
    // usd_in_cents: i64,
    #[serde(rename = "Is Boostagram")]
    is_boostagram: bool,
    #[serde(rename = "Is Zap")]
    is_zap: bool,
}

/// Deserialize creation date in "YYYY-MM-DD HH:MM:SS UTC" format
fn deserialize_creation_date<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S UTC").map_err(serde::de::Error::custom)
}

impl AlbyRecord {
    fn fee_amount(&self) -> Option<Amount> {
        self.fee
            .filter(|&f| f != 0)
            .map(sats_to_btc_amount)
    }

    fn timestamp(&self) -> NaiveDateTime {
        // Prefer settled date (Unix timestamp) when available, fall back to creation date
        if self.settled_date > 0 {
            chrono::DateTime::from_timestamp(self.settled_date, 0)
                .map(|dt| dt.naive_utc())
                .unwrap_or(self.creation_date)
        } else {
            self.creation_date
        }
    }

    fn compose_description(&self) -> Option<String> {
        let mut parts = Vec::new();

        // Memo, Comment, and Message are often identical - deduplicate them
        let mut seen = std::collections::HashSet::new();

        for text in [&self.memo, &self.comment, &self.message] {
            if let Some(trimmed) = non_empty(text) {
                if seen.insert(trimmed) {
                    parts.push(trimmed.to_owned());
                }
            }
        }

        // Add payer info for incoming payments
        if let Some(payer) = non_empty(&self.payer_name) {
            parts.push(format!("From: {}", payer));
        }

        // Add boostagram/zap indicators
        if self.is_boostagram {
            parts.push("Boostagram".to_owned());
        }
        if self.is_zap {
            parts.push("Zap".to_owned());
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }
}

impl TryFrom<AlbyRecord> for Transaction {
    type Error = anyhow::Error;

    fn try_from(record: AlbyRecord) -> Result<Self> {
        // Internal transfers show up as "incoming" with memo and message both set to "transfer" but
        // there's no corresponding "outgoing" entry, so we need to skip these to avoid
        // double-counting and balance errors
        if record.memo == "transfer" && record.message == "transfer" {
            return Err(anyhow!("Internal transfer"));
        }

        let timestamp = record.timestamp();
        let amount = if record.currency.eq_ignore_ascii_case(BTC_CURRENCY) {
            sats_to_btc_amount(record.amount)
        } else {
            return Err(anyhow!("Unsupported currency: {}", record.currency));
        };

        let mut tx = match record.invoice_type {
            InvoiceType::Incoming => Transaction::receive(timestamp, amount),
            InvoiceType::Outgoing => Transaction::send(timestamp, amount),
        };

        tx.fee = record.fee_amount();
        tx.description = record.compose_description();

        Ok(tx)
    }
}

pub(crate) fn load_alby_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in reader.deserialize() {
        let record: AlbyRecord = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(e) => {
                // Skip records that should be ignored (like internal transfers)
                eprintln!("Skipping Alby record: {}", e);
            }
        }
    }

    Ok(transactions)
}

pub(crate) static ALBY_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "AlbyCsv",
    label: "Alby (CSV)",
    csv: Some(CsvSpec {
        headers: &[
            "Invoice Type",
            "Amount",
            "Fee",
            "Creation Date",
            "Settled Date",
            "Memo",
            "Comment",
            "Message",
            "Payer Name",
            "Payer Pubkey",
            "Payment Hash",
            "Preimage",
            "Fiat In Cents",
            "Currency",
            "USD In Cents",
            "Is Boostagram",
            "Is Zap",
        ],
        delimiters: &[b','],
        skip_lines: 0,
    }),
    detect: None,
    load_sync: Some(load_alby_csv),
    load_async: None,
};

fn sats_to_btc_amount(sats: i64) -> Amount {
    Amount::new(Decimal::new(sats, SATS_SCALE), BTC_CURRENCY.to_owned())
}

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
    use rust_decimal_macros::dec;

    fn parse_csv_row(csv: &str) -> Result<Transaction> {
        let csv_with_header = format!(
            "Invoice Type,Amount,Fee,Creation Date,Settled Date,Memo,Comment,Message,Payer Name,Payer Pubkey,Payment Hash,Preimage,Fiat In Cents,Currency,USD In Cents,Is Boostagram,Is Zap\n{}",
            csv
        );
        let mut reader = csv::ReaderBuilder::new().from_reader(csv_with_header.as_bytes());
        let record: AlbyRecord = reader.deserialize().next().unwrap().unwrap();
        Transaction::try_from(record)
    }

    #[test]
    fn incoming_record_converts_to_receive() {
        let csv = "incoming,979,0,2025-12-17 11:51:23 UTC,1765972284,Sending the sats back,Sending the sats back,Sending the sats back,,,,,0,BTC,0,false,false";
        let tx = parse_csv_row(csv).unwrap();

        // Should use settled_date timestamp
        let expected_ts =
            chrono::DateTime::from_timestamp(1765972284, 0).unwrap().naive_utc();
        assert_eq!(tx.timestamp, expected_ts);

        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.00000979));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }

        assert!(tx.fee.is_none());
        // Duplicate memo/comment/message should be deduplicated
        assert_eq!(tx.description.as_deref(), Some("Sending the sats back"));
    }

    #[test]
    fn outgoing_record_converts_to_send_with_fee() {
        let csv = "outgoing,1000,2,2025-12-17 09:32:41 UTC,1765963961,,,,,,,,,BTC,0,false,false";
        let tx = parse_csv_row(csv).unwrap();

        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.00001000));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected send, got {:?}", other),
        }

        let fee = tx.fee.expect("fee should be set");
        assert_eq!(fee.quantity, dec!(0.00000002));
        assert_eq!(fee.currency, BTC_CURRENCY);
    }

    #[test]
    fn boostagram_and_zap_flags_in_description() {
        let csv = "incoming,25000,0,2025-12-16 23:18:37 UTC,1765927143,Support message,,,Alice,,,,0,BTC,0,true,true";
        let tx = parse_csv_row(csv).unwrap();
        assert_eq!(
            tx.description.as_deref(),
            Some("Support message | From: Alice | Boostagram | Zap")
        );
    }

    #[test]
    fn creation_date_used_when_settled_missing() {
        let csv = "incoming,500,0,2025-01-15 10:30:00 UTC,0,,,,,,,,,BTC,0,false,false";
        let tx = parse_csv_row(csv).unwrap();
        let expected_creation = NaiveDateTime::parse_from_str("2025-01-15 10:30:00", "%Y-%m-%d %H:%M:%S")
            .unwrap();
        assert_eq!(tx.timestamp, expected_creation);
    }

    #[test]
    fn different_memo_comment_message_all_included() {
        let csv = "incoming,100000,0,2025-11-24 20:01:02 UTC,1764014464,bounty for raccoin #29,Great work!,bounty for raccoin #29,,,,,0,BTC,0,false,false";
        let tx = parse_csv_row(csv).unwrap();
        // "bounty for raccoin #29" appears twice (memo and message) but should be deduplicated
        assert_eq!(
            tx.description.as_deref(),
            Some("bounty for raccoin #29 | Great work!")
        );
    }

    #[test]
    fn internal_transfer_is_rejected() {
        let csv = "incoming,5000,0,2025-01-20 14:30:00 UTC,1737383400,transfer,,transfer,,,,,0,BTC,0,false,false";
        let result = parse_csv_row(csv);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Internal transfer"));
    }

    #[test]
    fn missing_fee_is_handled() {
        let csv = "incoming,1500,,2025-01-22 10:00:00 UTC,1737540000,Payment without fee,,,,,,,,BTC,0,false,false";
        let tx = parse_csv_row(csv).unwrap();
        assert!(tx.fee.is_none());
    }
}
