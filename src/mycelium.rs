use std::{error::Error, path::Path};

use chrono::NaiveDateTime;
use csv::Trim;
use serde::{Deserialize, Deserializer};

use crate::base::Transaction;

// serialize function for reading NaiveDateTime
pub(crate) fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(&raw, "%Y-%m-%dT%H:%MZ").unwrap())
}

#[derive(Debug, Clone, Deserialize)]
enum TransferType {
    #[serde(rename = "Sent to")]
    SentTo,
    #[serde(rename = "Received with")]
    ReceivedWith,
}

// Stores values loaded from CSV file exported by Mycelium, with the following header:
// Account, Transaction ID, Destination Address, Timestamp, Value, Currency, Transaction Label
#[derive(Debug, Deserialize)]
struct MyceliumTransaction {
    // #[serde(rename = "Account")]
    // account: String,
    #[serde(rename = "Transaction ID")]
    id: String,
    // #[serde(rename = "Destination Address")]
    // address: String,
    #[serde(rename = "Timestamp", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(rename = "Value")]
    value: f64,
    // #[serde(rename = "Currency")]
    // currency: String,
    #[serde(rename = "Transaction Label")]
    label: String,
}

impl From<MyceliumTransaction> for Transaction {
    // todo: translate address?
    fn from(item: MyceliumTransaction) -> Self {
        let mut tx = if item.value < 0.0 {
            Transaction::send(item.timestamp, -item.value, "BTC")
        } else {
            Transaction::receive(item.timestamp, item.value, "BTC")
        };
        tx.description = if item.label.is_empty() { None } else { Some(item.label) };
        tx.tx_hash = Some(item.id);
        tx
    }
}

// loads a Mycelium CSV file into a list of unified transactions
pub(crate) fn load_mycelium_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .trim(Trim::Headers)
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: MyceliumTransaction = result?;
        transactions.push(record.into());
    }

    println!("Imported {} transactions from {}", transactions.len(), input_path.display());

    Ok(transactions)
}
