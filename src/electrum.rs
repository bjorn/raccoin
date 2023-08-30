use std::error::Error;

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time};

#[derive(Debug, Deserialize)]
struct ElectrumHistoryItem {
    transaction_hash: String,
    label: String,
    // confirmations: u64,
    value: f64,
    // fiat_value: f64,
    fee: Option<f64>,
    // fiat_fee: Option<f64>,
    #[serde(deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
}

pub(crate) fn convert_electrum_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: ElectrumHistoryItem = result?;
        let utc_time = Berlin.from_local_datetime(&record.timestamp).unwrap().naive_utc();

        wtr.serialize(CtcTx {
            id: Some(&record.transaction_hash),
            description: Some(&record.label),
            fee_amount: record.fee,
            fee_currency: if record.fee.is_none() { None } else { Some("BTC") },
            blockchain: Some("BTC"),
            ..CtcTx::new(
                utc_time,
                if record.value < 0.0 { CtcTxType::Send } else { CtcTxType::Receive },
                "BTC",
                record.value.abs() - record.fee.unwrap_or(0.0))
        })?;
    }

    Ok(())
}
