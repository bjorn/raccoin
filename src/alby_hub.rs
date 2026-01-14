use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    base::{Amount, Transaction},
    CsvSpec, TransactionSourceType,
};
use linkme::distributed_slice;

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "lowercase")]
enum RecordType {
    Incoming,
    Outgoing,
}

// CSV header:
// type,state,invoice,description,descriptionHash,preimage,paymentHash,amount,feesPaid,updatedAt,createdAt,settledAt,appId,metadata,failureReason
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbyHubRecord {
    #[serde(rename = "type")]
    type_: RecordType,
    state: String,
    // invoice: String,
    description: String,
    // description_hash: String,
    // preimage: String,
    payment_hash: String,
    amount: i64,
    fees_paid: i64,
    updated_at: Option<DateTime<FixedOffset>>,
    created_at: DateTime<FixedOffset>,
    settled_at: Option<DateTime<FixedOffset>>,
    app_id: String,
    // metadata: String,
    failure_reason: String,
}

impl AlbyHubRecord {
    fn into_transaction(self) -> Result<Option<Transaction>> {
        if !self.state.eq_ignore_ascii_case("settled") {
            return Ok(None);
        }

        if let Some(reason) = non_empty(&self.failure_reason) {
            // Skip failed entries even if they slipped through as "settled".
            println!(
                "Skipping Alby Hub record with failure reason '{}', payment hash {}",
                reason, self.payment_hash
            );
            return Ok(None);
        }

        let timestamp = self.timestamp()?;
        let amount_btc = msat_to_btc_amount(self.amount);

        let mut tx = match self.type_ {
            RecordType::Incoming => Transaction::receive(timestamp, amount_btc),
            RecordType::Outgoing => Transaction::send(timestamp, amount_btc),
        };

        tx.fee = self.fee();
        tx.description = self.compose_description();

        Ok(Some(tx))
    }

    fn fee(&self) -> Option<Amount> {
        if self.fees_paid == 0 {
            None
        } else {
            Some(msat_to_btc_amount(self.fees_paid))
        }
    }

    fn timestamp(&self) -> Result<NaiveDateTime> {
        Ok(self
            .settled_at
            .or(self.updated_at)
            .unwrap_or(self.created_at)
            .naive_utc())
    }

    fn compose_description(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(desc) = non_empty(&self.description) {
            parts.push(desc.to_owned());
        }

        if let Some(app_id) = non_empty(&self.app_id) {
            parts.push(format!("App ID: {}", app_id));
        }

        if !parts.is_empty() {
            return Some(parts.join(" | "));
        }

        None
    }
}

fn load_alby_hub_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for record in rdr.deserialize() {
        let row: AlbyHubRecord = record?;
        if let Some(tx) = row.into_transaction()? {
            transactions.push(tx);
        }
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static ALBY_HUB_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "AlbyHubCsv",
    label: "Alby Hub (CSV)",
    csv: &[CsvSpec::new(&[
        "type",
        "state",
        "invoice",
        "description",
        "descriptionHash",
        "preimage",
        "paymentHash",
        "amount",
        "feesPaid",
        "updatedAt",
        "createdAt",
        "settledAt",
        "appId",
        "metadata",
        "failureReason",
    ])],
    detect: None,
    load_sync: Some(load_alby_hub_csv),
    load_async: None,
};

const MSATS_SCALE: u32 = 11;

fn msat_to_btc_amount(msat: i64) -> Amount {
    Amount::new(Decimal::new(msat, MSATS_SCALE), "BTC".to_owned())
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
    use chrono::DateTime;

    #[test]
    fn updated_timestamp_used_when_settled_missing() {
        let updated = DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z").unwrap();
        let expected = updated.naive_utc();
        let created = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap();
        let record = AlbyHubRecord {
            type_: RecordType::Incoming,
            state: "settled".to_owned(),
            description: String::new(),
            payment_hash: String::new(),
            amount: 0,
            fees_paid: 0,
            updated_at: Some(updated),
            created_at: created,
            settled_at: None,
            app_id: String::new(),
            failure_reason: String::new(),
        };

        assert_eq!(record.timestamp().unwrap(), expected);
    }

    #[test]
    fn record_into_transaction_creates_receive() {
        let updated = DateTime::parse_from_rfc3339("2024-05-06T07:08:09Z").unwrap();
        let expected = updated.naive_utc();
        let created = DateTime::parse_from_rfc3339("2024-05-05T06:00:00Z").unwrap();
        let record = AlbyHubRecord {
            type_: RecordType::Incoming,
            state: "settled".to_owned(),
            description: "Test payment".to_owned(),
            payment_hash: String::new(),
            amount: 2_000,
            fees_paid: 0,
            updated_at: Some(updated),
            created_at: created,
            settled_at: None,
            app_id: "app-1".to_owned(),
            failure_reason: String::new(),
        };

        let tx = record
            .into_transaction()
            .expect("row parsed")
            .expect("transaction created");

        assert_eq!(tx.timestamp, expected);
        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, Decimal::new(2000, MSATS_SCALE));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("unexpected operation: {:?}", other),
        }
        assert!(tx.fee.is_none());
        assert_eq!(
            tx.description.as_deref(),
            Some("Test payment | App ID: app-1")
        );
    }
}
