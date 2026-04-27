use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{
    base::{Amount, Transaction},
    CsvSpec, TransactionSource,
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
    #[serde(deserialize_with = "deserialize_metadata")]
    metadata: Option<AlbyHubMetadata>,
    failure_reason: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AlbyHubMetadata {
    payer_data: Option<PayerData>,
    recipient_data: Option<RecipientData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PayerData {
    name: String,
    email: String,
}

impl PayerData {
    fn to_description_part(&self) -> Option<String> {
        let name = non_empty(&self.name);
        let email = non_empty(&self.email);
        let body = match (name, email) {
            (Some(n), Some(e)) => format!("{} <{}>", n, e),
            (Some(n), None) => n.to_owned(),
            (None, Some(e)) => e.to_owned(),
            (None, None) => return None,
        };
        Some(format!("From: {}", body))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RecipientData {
    identifier: String,
}

impl RecipientData {
    fn to_description_part(&self) -> Option<String> {
        let identifier = non_empty(&self.identifier)?;
        Some(format!("To: {}", identifier))
    }
}

fn parse_metadata(raw: &str) -> Option<AlbyHubMetadata> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str(trimmed) {
        Ok(metadata) => Some(metadata),
        Err(err) => {
            println!("Skipping unparseable Alby Hub metadata: {}", err);
            None
        }
    }
}

fn deserialize_metadata<'de, D>(deserializer: D) -> Result<Option<AlbyHubMetadata>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: &str = Deserialize::deserialize(deserializer)?;
    Ok(parse_metadata(raw))
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

        if let Some(party) = self.party_label() {
            parts.push(party);
        }

        if let Some(app_id) = non_empty(&self.app_id) {
            parts.push(format!("App ID: {}", app_id));
        }

        if !parts.is_empty() {
            return Some(parts.join(" | "));
        }

        None
    }

    fn party_label(&self) -> Option<String> {
        let metadata = self.metadata.as_ref()?;
        match self.type_ {
            RecordType::Incoming => metadata.payer_data.as_ref().and_then(PayerData::to_description_part),
            RecordType::Outgoing => metadata
                .recipient_data
                .as_ref()
                .and_then(RecipientData::to_description_part),
        }
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
static ALBY_HUB_CSV: TransactionSource = TransactionSource {
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

    fn make_record(type_: RecordType) -> AlbyHubRecord {
        let created = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap();
        AlbyHubRecord {
            type_,
            state: "settled".to_owned(),
            description: String::new(),
            payment_hash: String::new(),
            amount: 0,
            fees_paid: 0,
            updated_at: None,
            created_at: created,
            settled_at: None,
            app_id: String::new(),
            metadata: None,
            failure_reason: String::new(),
        }
    }

    #[test]
    fn updated_timestamp_used_when_settled_missing() {
        let updated = DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z").unwrap();
        let expected = updated.naive_utc();
        let mut record = make_record(RecordType::Incoming);
        record.updated_at = Some(updated);

        assert_eq!(record.timestamp().unwrap(), expected);
    }

    #[test]
    fn record_into_transaction_creates_receive() {
        let updated = DateTime::parse_from_rfc3339("2024-05-06T07:08:09Z").unwrap();
        let expected = updated.naive_utc();
        let mut record = make_record(RecordType::Incoming);
        record.description = "Test payment".to_owned();
        record.amount = 2_000;
        record.updated_at = Some(updated);
        record.app_id = "app-1".to_owned();

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

    #[test]
    fn payer_data_added_to_description_for_incoming() {
        let mut record = make_record(RecordType::Incoming);
        record.description = "Coffee tip".to_owned();
        record.app_id = "demo-app".to_owned();
        record.metadata = parse_metadata(
            r#"{"comment":"Coffee tip","payer_data":{"email":"alice@example.com","name":"Alice"}}"#,
        );

        assert_eq!(
            record.compose_description().as_deref(),
            Some("Coffee tip | From: Alice <alice@example.com> | App ID: demo-app"),
        );
    }

    #[test]
    fn recipient_data_added_to_description_for_outgoing() {
        let mut record = make_record(RecordType::Outgoing);
        record.description = "Tip jar".to_owned();
        record.metadata = parse_metadata(
            r#"{"comment":"Tip jar","recipient_data":{"identifier":"bob@example.com"}}"#,
        );

        assert_eq!(
            record.compose_description().as_deref(),
            Some("Tip jar | To: bob@example.com"),
        );
    }

    #[test]
    fn payer_data_ignored_for_outgoing() {
        let mut record = make_record(RecordType::Outgoing);
        record.description = "Outgoing".to_owned();
        record.metadata = parse_metadata(r#"{"payer_data":{"name":"Wrong Direction"}}"#);

        assert_eq!(record.compose_description().as_deref(), Some("Outgoing"));
    }

    #[test]
    fn parse_metadata_returns_none_for_empty_or_invalid() {
        assert!(parse_metadata("").is_none());
        assert!(parse_metadata("   ").is_none());
        assert!(parse_metadata("not json").is_none());
    }
}
