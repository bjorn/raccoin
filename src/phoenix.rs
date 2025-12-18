//! Phoenix mobile wallet CSV importer.

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::base::{Amount, Transaction};

const BTC_CURRENCY: &str = "BTC";

/// Scale for converting millisatoshis to BTC:
/// 1 BTC = 100_000_000 sats = 100_000_000_000 msat => scale = 11
const MSAT_SCALE: u32 = 11;

#[derive(Debug, Deserialize, Copy, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum RecordType {
    LightningReceived,
    LightningSent,
    SwapIn,
    SwapOut,
}

/// CSV record mapping for Phoenix export
/// date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description
#[derive(Debug, Deserialize)]
struct PhoenixRecord {
    date: DateTime<FixedOffset>,
    // id: String,
    #[serde(rename = "type")]
    type_: RecordType,
    amount_msat: i64,
    // #[serde(default)]
    // amount_fiat: String,
    // #[serde(default)]
    // fee_credit_msat: i64,
    #[serde(default)]
    mining_fee_sat: i64,
    // #[serde(default)]
    // mining_fee_fiat: String,
    #[serde(default)]
    service_fee_msat: i64,
    // #[serde(default)]
    // service_fee_fiat: String,
    // #[serde(default)]
    // payment_hash: String,
    tx_id: Option<String>,
    // #[serde(default)]
    // destination: String,
    #[serde(default)]
    description: String,
}

impl PhoenixRecord {
    /// Compute total fee in msat using precise msat arithmetic:
    /// mining_fee_sat * 1000 + service_fee_msat
    fn total_fee_msat(&self) -> i64 {
        // Use saturating_mul to be defensive against overflow (very large values)
        self.mining_fee_sat
            .saturating_mul(1000)
            .saturating_add(self.service_fee_msat)
    }
}

impl From<PhoenixRecord> for Transaction {
    fn from(record: PhoenixRecord) -> Self {
        let timestamp = record.date.naive_utc();
        let total_fee_msat = record.total_fee_msat();

        // For receive records, Phoenix reports the incoming amount AFTER fees were subtracted. We
        // need to add fees back to the incoming amount so Transaction::receive holds the gross
        // amount.
        //
        // For sending records, Phoenix reports the outgoing amount including fees, but the
        // value is negative, so in this case the fees need to be added as well.
        let amount = msat_to_btc_amount(record.amount_msat.saturating_add(total_fee_msat).abs());

        let mut tx = match record.type_ {
            RecordType::LightningReceived | RecordType::SwapIn => {
                Transaction::receive(timestamp, amount)
            }
            RecordType::LightningSent | RecordType::SwapOut => {
                Transaction::send(timestamp, amount)
            }
        };

        if total_fee_msat != 0 {
            tx.fee = Some(msat_to_btc_amount(total_fee_msat));
        }

        tx.description = non_empty(&record.description).map(|s| s.to_owned());

        match record.type_ {
            RecordType::SwapIn | RecordType::SwapOut => {
                tx.tx_hash = record.tx_id;
            }
            _ => {}
        }

        tx
    }
}

pub(crate) fn load_phoenix_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in reader.deserialize() {
        let record: PhoenixRecord = result?;
        let tx = Transaction::from(record);
        transactions.push(tx);
    }

    Ok(transactions)
}

/// Helpers

fn msat_to_btc_amount(msat: i64) -> Amount {
    Amount::new(Decimal::new(msat, MSAT_SCALE), BTC_CURRENCY.to_owned())
}

fn non_empty(value: &str) -> Option<&str> {
    let t = value.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Operation;
    use rust_decimal_macros::dec;

    /// Helper which builds a CSV with header and returns a parsed Transaction (via TryFrom)
    fn parse_csv_row(csv: &str) -> Transaction {
        let csv_with_header = format!(
            "date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description\n{}",
            csv
        );
        let mut reader = csv::ReaderBuilder::new().from_reader(csv_with_header.as_bytes());
        let record: PhoenixRecord = reader.deserialize().next().unwrap().unwrap();
        Transaction::from(record)
    }

    #[test]
    fn incoming_adds_fees_back() {
        // incoming amount (net) 8558000 msat, mining 342 sat => 342000 msat, service 1100000 msat
        // total fees = 342000 + 1100000 = 1442000 msat
        // gross msat = 8558000 + 1442000 = 10000000 msat = 0.00010000 BTC
        let csv = "2025-12-18T08:28:56.852Z,cb6b958c-1245-4880-af21-e75323c2a02f,lightning_received,8558000,7.4267 USD,0,342,0.2967 USD,1100000,0.9545 USD,cb6b958c124598802f21e75323c2a02fef57193b89d827986a558c19959924e6,2732313c92187075d5e92bdd0336f89b03cfb86b5c78dc62065c807a722b2d15,,Received from Alby Hub";
        let tx = parse_csv_row(csv);

        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.0001));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }

        // fee should equal 1442000 msat -> 0.00001442 BTC
        let fee = tx.fee.expect("fee should be set");
        assert_eq!(fee.quantity, dec!(0.00001442));
    }

    #[test]
    fn outgoing_keeps_fee_and_amount() {
        // outgoing -1008000 msat -> 0.00001 BTC
        let csv = "2025-12-18T08:40:46.437Z,73567ac5-b860-4fa9-8602-020bdcdcafba,lightning_sent,-1008000,0.7472 EUR,0,0,0.0000 EUR,8000,0.0059 EUR,72b38c974d4605b69b52e817b4448539c9b49e9a8943bb3459230591fac844be,,02ed6712779fcdb483e4be5f9249aa5b788c59ce57b711971ac656af5594ef4b4b,\"Sending some sats back\"";
        let tx = parse_csv_row(csv);

        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.00001));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected send, got {:?}", other),
        }

        let fee = tx.fee.expect("fee should be set");
        // fee: service_fee_msat = 8000 msat -> 0.00000008 BTC
        assert_eq!(fee.quantity, dec!(0.00000008));
    }

    #[test]
    fn swap_in_is_receive() {
        let csv = "2025-04-10T14:41:04.523Z,2be0d870-d64d-471d-9678-eaba3d854bd8,swap_in,3499790000,2555.4602 EUR,0,210,0.1533 EUR,0,0.0000 EUR,,8c4edae8ef920bf536631aa7c13e8280a4fc0e94e2c231939478630cf075f1aa,,";
        let tx = parse_csv_row(csv);
        match tx.operation {
            Operation::Receive(amount) => {
                // amount 3499790000 msat + 210 sat fee = 0.03500000000 BTC
                assert_eq!(amount.quantity, dec!(0.03500000000));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }
    }
}
