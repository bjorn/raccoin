//! Importer for wave.space transaction registry CSV exports.

use std::{collections::HashMap, convert::TryFrom, path::Path};

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    base::{Amount, Operation, Transaction},
    time::deserialize_date_time,
    CsvSpec, TransactionSource,
};
use linkme::distributed_slice;

const WAVE_SPACE_HEADERS: [&str; 9] = [
    "Type Category",
    "Executes At",
    "Transaction ID",
    "Transaction Type",
    "From Currency",
    "From Amount",
    "To Currency",
    "To Amount",
    "Memo",
];

const FIAT_CURRENCY: &str = "EUR";

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
enum TypeCategory {
    Deposit,
    Fee,
    Transaction,
}

// Transaction Type values (from wave.space):
//
// INTRA_LEDGER_SEND_MONEY
// INTER_LEDGER_SEND_MONEY
// SEPA_PAYOUT
// CURRENCY_SWAP
// SEPA_PAYIN_DEPOSIT
// ON_CHAIN_DEPOSIT
// LIGHTNING_DEPOSIT
// ON_CHAIN_WITHDRAW
// LIGHTNING_WITHDRAW
// APPLICATION_FEE
// NETWORK_FEE
// BUY_TO_WALLET
// SELL_TO_BANK
// CARD_AUTHORIZATION
// CARD_AUTHORIZATION_DECLINED
// CARD_AUTHORIZATION_REVERSAL
// CARD_AUTHORIZATION_RELEASE
// CARD_AUTHORIZATION_SETTLEMENT_CONFIRMED
// CARD_TRANSACTION_REFUND_PROCESSED
// CARD_TO_CARD_CREDIT
// FX_PADDING
// REWARD
//
// However, without additional information, it is unclear if any of these need special handling
// while loading transactions.

#[derive(Debug, Deserialize)]
struct WaveSpaceRecord {
    #[serde(rename = "Type Category")]
    type_category: TypeCategory,
    #[serde(rename = "Executes At", deserialize_with = "deserialize_date_time")]
    executes_at: NaiveDateTime,
    #[serde(rename = "Transaction ID")]
    transaction_id: String,
    #[serde(rename = "Transaction Type")]
    transaction_type: String,
    #[serde(rename = "From Currency")]
    from_currency: String,
    #[serde(rename = "From Amount")]
    from_amount: Decimal,
    #[serde(rename = "To Currency")]
    to_currency: String,
    #[serde(rename = "To Amount")]
    to_amount: Decimal,
    #[serde(rename = "Memo")]
    memo: String,
}

fn is_fiat(currency: &str) -> bool {
    currency == FIAT_CURRENCY
}

impl TryFrom<WaveSpaceRecord> for Transaction {
    type Error = anyhow::Error;

    fn try_from(record: WaveSpaceRecord) -> Result<Self> {
        let timestamp = record.executes_at;
        let from_amount = record.from_amount;
        let to_amount = record.to_amount;
        let from_currency = record.from_currency;
        let to_currency = record.to_currency;

        let mut tx = match record.type_category {
            TypeCategory::Deposit => {
                let amount = Amount::new(to_amount, to_currency);

                if amount.is_fiat() {
                    Transaction::fiat_deposit(timestamp, amount)
                } else {
                    Transaction::receive(timestamp, amount)
                }
            }
            TypeCategory::Fee => {
                Transaction::fee(timestamp, Amount::new(from_amount, from_currency))
            }
            TypeCategory::Transaction => {
                let outgoing = Amount::new(from_amount, from_currency);
                let mut tx = Transaction::new(timestamp, Operation::Expense(outgoing));
                if is_fiat(&to_currency) {
                    tx.value = Some(Amount::new(to_amount, to_currency));
                }
                tx
            }
        };

        let mut description_parts = Vec::new();
        if !record.transaction_type.is_empty() {
            description_parts.push(record.transaction_type);
        }
        if !record.memo.is_empty() {
            description_parts.push(record.memo);
        }
        if !description_parts.is_empty() {
            tx.description = Some(description_parts.join(" | "));
        }

        Ok(tx)
    }
}

fn records_to_transactions(records: Vec<WaveSpaceRecord>) -> Result<Vec<Transaction>> {
    let mut fee_by_id: HashMap<String, WaveSpaceRecord> = HashMap::new();
    let mut other_records = Vec::new();

    // Remember fee records by their transaction ID
    for record in records {
        if matches!(record.type_category, TypeCategory::Fee) {
            if let Some(existing) = fee_by_id.insert(record.transaction_id.clone(), record) {
                // If we run into multiple fee records with the same transaction ID, we only try to
                // merge the last one, the rest become dedicated fee transactions.
                other_records.push(existing);
            }
        } else {
            other_records.push(record);
        }
    }

    let mut transactions = Vec::new();

    for record in other_records {
        let transaction_id = record.transaction_id.clone();
        let mut tx = Transaction::try_from(record)?;

        if !matches!(tx.operation, Operation::Fee(_)) {
            // Check whether the transaction has a fee record associated with it
            if let Some(fee_record) = fee_by_id.remove(&transaction_id) {
                tx.fee = Some(Amount::new(
                    fee_record.from_amount,
                    fee_record.from_currency,
                ));
                if tx.description.is_none() && !fee_record.memo.is_empty() {
                    tx.description = Some(fee_record.memo);
                }
            }
        }

        transactions.push(tx);
    }

    // Create dedicated fee transactions for any remaining fee records
    for (_, record) in fee_by_id {
        transactions.push(Transaction::try_from(record)?);
    }

    Ok(transactions)
}

fn load_wave_space_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::Fields)
        .from_path(input_path)?;
    let mut records = Vec::new();

    for result in reader.deserialize() {
        records.push(result?);
    }

    records_to_transactions(records)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static WAVE_SPACE_CSV: TransactionSource = TransactionSource {
    id: "WaveSpaceCsv",
    label: "wave.space (CSV)",
    csv: &[CsvSpec::new(&WAVE_SPACE_HEADERS)],
    detect: None,
    load_sync: Some(load_wave_space_csv),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use csv::StringRecord;
    use rust_decimal_macros::dec;

    fn parse_csv_rows(csv: &str) -> Vec<Transaction> {
        let header = StringRecord::from(&WAVE_SPACE_HEADERS[..]);
        let mut reader = csv::ReaderBuilder::new()
            .trim(csv::Trim::Fields)
            .from_reader(csv.as_bytes());
        reader.set_headers(header);
        let records: Vec<WaveSpaceRecord> = reader
            .deserialize()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        records_to_transactions(records).unwrap()
    }

    fn parse_csv_row(csv: &str) -> Transaction {
        parse_csv_rows(csv).into_iter().next().unwrap()
    }

    #[test]
    fn deposit_becomes_receive() {
        let csv = "DEPOSIT,2025-11-25 12:09:09,TX-ID,LIGHTNING_DEPOSIT,BTC,0,BTC,0.0009,wavecard topup";
        let tx = parse_csv_row(csv);

        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.0009));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected receive, got {:?}", other),
        }
    }

    #[test]
    fn fee_becomes_fee_transaction() {
        let csv = "FEE,2025-12-20 11:03:25,TX-ID,APPLICATION_FEE,BTC,0.00000586,BTC,0,payWaveLowValuePurchase Card Authorization at X application fee of  0.00000586 BTC";
        let tx = parse_csv_row(csv);

        match tx.operation {
            Operation::Fee(amount) => {
                assert_eq!(amount.quantity, dec!(0.00000586));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected fee, got {:?}", other),
        }
    }

    #[test]
    fn card_authorization_becomes_expense_with_value() {
        let csv = "TRANSACTION,2025-12-20 11:03:27,TX-ID,CARD_AUTHORIZATION,BTC,0.00058554,EUR,43.85,";
        let tx = parse_csv_row(csv);

        match tx.operation {
            Operation::Expense(amount) => {
                assert_eq!(amount.quantity, dec!(0.00058554));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected expense, got {:?}", other),
        }

        let value = tx.value.expect("value should be set");
        assert_eq!(value.quantity, dec!(43.85));
        assert_eq!(value.currency, "EUR");
    }

    #[test]
    fn fee_is_attached_when_transaction_id_matches() {
        let csv = concat!(
            "FEE,2025-12-20 11:03:25,TX-ID,APPLICATION_FEE,BTC,0.00000586,BTC,0,fee memo\n",
            "TRANSACTION,2025-12-20 11:03:27,TX-ID,CARD_AUTHORIZATION,BTC,0.00058554,EUR,43.85,\n",
        );
        let transactions = parse_csv_rows(csv);
        assert_eq!(transactions.len(), 1);
        let tx = &transactions[0];
        let fee = tx.fee.as_ref().expect("fee should be attached");
        assert_eq!(fee.quantity, dec!(0.00000586));
        assert_eq!(fee.currency, "BTC");
    }
}
