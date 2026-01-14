use std::{collections::{HashMap, HashSet}, path::Path};

use anyhow::{anyhow, Result};
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{
    base::{Amount, Operation, Transaction},
    CsvSpec, TransactionSourceType,
};
use linkme::distributed_slice;

const BTC_CURRENCY: &str = "BTC";
const USD_CURRENCY: &str = "USD";
const SATS_SCALE: u32 = 8;
const USD_SCALE: u32 = 2;

pub(crate) const BLINK_HEADERS: &[&str] = &[
    "id",
    "walletId",
    "type",
    "credit",
    "debit",
    "fee",
    "currency",
    "timestamp",
    "pendingConfirmation",
    "journalId",
    "lnMemo",
    "usd",
    "feeUsd",
    "recipientWalletId",
    "username",
    "memoFromPayer",
    "paymentHash",
    "pubkey",
    "feeKnownInAdvance",
    "address",
    "txHash",
    "displayAmount",
    "displayFee",
    "displayCurrency",
];

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BLINK_CSV: TransactionSourceType = TransactionSourceType {
    id: "BlinkCsv",
    label: "Blink (CSV)",
    csv: &[CsvSpec::new(BLINK_HEADERS)],
    detect: None,
    load_sync: Some(load_blink_csv),
    load_async: None,
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlinkType {
    Invoice,
    OnchainReceipt,
    FeeReimbursement,
    Payment,
    OnchainPayment,
    OnUs,
    LnOnUs,
    OnchainOnUs,
    SelfTrade,
    LnSelfTrade,
    OnchainSelfTrade,
    Fee,
    Escrow,
    Reconciliation,
}

impl BlinkType {
    fn is_self_trade(&self) -> bool {
        matches!(
            self,
            BlinkType::SelfTrade | BlinkType::LnSelfTrade | BlinkType::OnchainSelfTrade
        )
    }

    fn is_onchain(&self) -> bool {
        matches!(
            self,
            BlinkType::OnchainReceipt
                | BlinkType::OnchainPayment
                | BlinkType::OnchainOnUs
                | BlinkType::OnchainSelfTrade
        )
    }

    fn label(&self) -> Option<&'static str> {
        match self {
            BlinkType::Fee => Some("Blink fee"),
            BlinkType::FeeReimbursement => Some("Fee reimbursement"),
            BlinkType::Reconciliation => Some("Reconciliation"),
            BlinkType::SelfTrade | BlinkType::LnSelfTrade | BlinkType::OnchainSelfTrade => {
                Some("Self trade")
            }
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlinkRecord {
    id: String,
    // wallet_id: String,
    journal_id: String,
    #[serde(rename = "type")]
    type_: BlinkType,
    credit: i64,
    debit: i64,
    fee: Option<i64>,
    currency: String,
    #[serde(deserialize_with = "deserialize_blink_timestamp")]
    timestamp: DateTime<FixedOffset>,
    pending_confirmation: bool,
    ln_memo: String,
    memo_from_payer: String,
    // display_amount: i64,
    // display_fee: i64,
    // display_currency: String,
    // #[serde(default)]
    // usd: String,
    // #[serde(default)]
    // fee_usd: String,
    recipient_wallet_id: String,
    username: String,
    // payment_hash: String,
    // #[serde(default)]
    // pubkey: String,
    fee_known_in_advance: bool,
    address: String,
    tx_hash: String,
}

impl BlinkRecord {
    fn into_transaction(self) -> Result<Option<Transaction>> {
        let timestamp = self.timestamp.naive_utc();
        let (direction, amount) = match self.amount_direction()? {
            Some(value) => value,
            None => {
                if matches!(self.type_, BlinkType::Fee) {
                    return self.fee_only_transaction(timestamp);
                }
                println!("Skipping Blink transaction {} with no amount", self.id);
                return Ok(None);
            }
        };

        let mut tx = match self.type_ {
            BlinkType::Fee => return self.fee_only_transaction(timestamp),
            BlinkType::Reconciliation => {
                let operation = match direction {
                    Direction::Incoming => Operation::Income(amount),
                    Direction::Outgoing => Operation::Expense(amount),
                };
                Transaction::new(timestamp, operation)
            }
            _ => match direction {
                Direction::Incoming => Transaction::receive(timestamp, amount),
                Direction::Outgoing => Transaction::send(timestamp, amount),
            },
        };

        tx.fee = self.fee_amount()?;
        tx.description = self.compose_description();
        if self.type_.is_onchain() {
            tx.tx_hash = self.primary_tx_hash();
            tx.blockchain = Some(BTC_CURRENCY.to_owned());
        }

        Ok(Some(tx))
    }

    fn fee_only_transaction(self, timestamp: chrono::NaiveDateTime) -> Result<Option<Transaction>> {
        let fee_amount = if self.debit != 0 {
            self.amount_from_minor_units(self.debit)
        } else if let Some(fee) = self.fee {
            if fee == 0 {
                return Ok(None);
            }
            self.amount_from_minor_units(fee)
        } else if self.credit != 0 {
            self.amount_from_minor_units(self.credit)
        } else {
            return Ok(None);
        }?;

        let mut tx = Transaction::fee(timestamp, fee_amount);
        tx.description = self.compose_description();
        if self.type_.is_onchain() {
            tx.tx_hash = self.primary_tx_hash();
            tx.blockchain = Some(BTC_CURRENCY.to_owned());
        }

        Ok(Some(tx))
    }

    fn amount_direction(&self) -> Result<Option<(Direction, Amount)>> {
        if self.credit != 0 {
            Ok(Some((
                Direction::Incoming,
                self.amount_from_minor_units(self.credit)?,
            )))
        } else if self.debit != 0 {
            Ok(Some((
                Direction::Outgoing,
                self.amount_from_minor_units(self.debit)?,
            )))
        } else {
            Ok(None)
        }
    }

    fn amount_from_minor_units(&self, value: i64) -> Result<Amount> {
        let currency = self.currency.trim().to_uppercase();
        let (scale, normalized) = match currency.as_str() {
            BTC_CURRENCY => (SATS_SCALE, BTC_CURRENCY),
            USD_CURRENCY => (USD_SCALE, USD_CURRENCY),
            _ => {
                return Err(anyhow!(
                    "Blink CSV contains unsupported currency '{}'",
                    self.currency
                ))
            }
        };

        Ok(Amount::new(Decimal::new(value, scale), normalized.to_owned()))
    }

    fn fee_amount(&self) -> Result<Option<Amount>> {
        match self.fee {
            Some(fee) if fee != 0 => self.amount_from_minor_units(fee).map(Some),
            _ => Ok(None),
        }
    }

    fn compose_description(&self) -> Option<String> {
        let mut parts = Vec::new();
        let mut seen = HashSet::new();

        if let Some(label) = self.type_.label() {
            if seen.insert(label.to_owned()) {
                parts.push(label.to_owned());
            }
        }

        for text in [&self.ln_memo, &self.memo_from_payer] {
            if let Some(trimmed) = non_empty(text) {
                if seen.insert(trimmed.to_owned()) {
                    parts.push(trimmed.to_owned());
                }
            }
        }

        if let Some(username) = non_empty(&self.username) {
            parts.push(format!("User: {}", username));
        }

        if let Some(address) = non_empty(&self.address) {
            parts.push(format!("Address: {}", address));
        }

        if let Some(recipient) = non_empty(&self.recipient_wallet_id) {
            parts.push(format!("Recipient wallet: {}", recipient));
        }

        if self.fee_known_in_advance {
            parts.push("Fee known in advance".to_owned());
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }

    fn primary_tx_hash(&self) -> Option<String> {
        non_empty(&self.tx_hash)
            .map(|value| value.to_owned())
    }
}

#[derive(Debug, Copy, Clone)]
enum Direction {
    Incoming,
    Outgoing,
}

fn load_blink_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut records = Vec::new();

    for result in reader.deserialize() {
        let record: BlinkRecord = result?;
        records.push(record);
    }

    records_to_transactions(records)
}

fn records_to_transactions(records: Vec<BlinkRecord>) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();
    let mut self_trades: HashMap<String, Vec<BlinkRecord>> = HashMap::new();

    for record in records {
        if record.pending_confirmation {
            println!("Skipping Blink pending transaction {}", record.id);
            continue;
        }

        if matches!(record.type_, BlinkType::Escrow) {
            println!("Skipping Blink escrow transaction {}", record.id);
            continue;
        }

        if record.type_.is_self_trade() {
            self_trades
                .entry(record.journal_id.clone())
                .or_default()
                .push(record);
        } else if let Some(tx) = record.into_transaction()? {
            transactions.push(tx);
        }
    }

    for (journal_id, group) in self_trades {
        transactions.extend(convert_self_trade_group(&journal_id, group)?);
    }

    Ok(transactions)
}

fn convert_self_trade_group(journal_id: &str, records: Vec<BlinkRecord>) -> Result<Vec<Transaction>> {
    let to_individual_transfers = |records: Vec<BlinkRecord>| -> Result<Vec<Transaction>> {
        let mut transactions = Vec::new();
        for record in records {
            if let Some(tx) = record.into_transaction()? {
                transactions.push(tx);
            }
        }
        Ok(transactions)
    };

    if records.len() != 2 {
        println!(
            "Blink self trade group {} has {} records; keeping as individual transfers",
            journal_id,
            records.len()
        );
        return to_individual_transfers(records);
    }

    let mut incoming = None;
    let mut outgoing = None;
    let mut fee: Option<Amount> = None;
    let mut fee_currency_mismatch = false;
    let mut tx_hash = None;
    let mut onchain = false;
    let mut timestamp = records[0].timestamp.naive_utc();
    let mut description_parts = Vec::new();
    let mut description_seen = HashSet::new();

    for record in &records {
        timestamp = timestamp.max(record.timestamp.naive_utc());

        if let Some((direction, amount)) = record.amount_direction()? {
            match direction {
                Direction::Incoming => incoming = Some(amount),
                Direction::Outgoing => outgoing = Some(amount),
            }
        }

        if let Some(record_fee) = record.fee_amount()? {
            fee = match fee {
                None => Some(record_fee),
                Some(existing) => {
                    if existing.currency == record_fee.currency {
                        Some(Amount::new(
                            existing.quantity + record_fee.quantity,
                            existing.currency,
                        ))
                    } else {
                        println!(
                            "Blink self trade group {} has fees in multiple currencies ({} and {}), keeping individual transfers",
                            journal_id,
                            existing.currency,
                            record_fee.currency
                        );
                        fee_currency_mismatch = true;
                        Some(existing)
                    }
                }
            };
        }

        if tx_hash.is_none() {
            tx_hash = record.primary_tx_hash();
        }

        if record.type_.is_onchain() {
            onchain = true;
        }

        if let Some(description) = record.compose_description() {
            if description_seen.insert(description.clone()) {
                description_parts.push(description);
            }
        }
    }

    if fee_currency_mismatch {
        return to_individual_transfers(records);
    }

    let (incoming, outgoing) = match (incoming, outgoing) {
        (Some(incoming), Some(outgoing)) => (incoming, outgoing),
        _ => {
            println!(
                "Blink self trade group {} missing incoming or outgoing amount; keeping individual transfers",
                journal_id
            );
            return to_individual_transfers(records);
        }
    };

    let mut tx = Transaction::trade(timestamp, incoming, outgoing);
    tx.fee = fee;
    if description_parts.is_empty() {
        tx.description = Some("Self trade".to_owned());
    } else {
        tx.description = Some(description_parts.join(" | "));
    }
    if onchain && tx_hash.is_some() {
        tx.tx_hash = tx_hash;
        tx.blockchain = Some(BTC_CURRENCY.to_owned());
    }

    Ok(vec![tx])
}

fn deserialize_blink_timestamp<'de, D>(deserializer: D) -> Result<DateTime<FixedOffset>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: String = Deserialize::deserialize(deserializer)?;
    let trimmed = raw.trim();
    let cleaned = trimmed.split(" (").next().unwrap_or(trimmed).trim();
    DateTime::parse_from_str(cleaned, "%a %b %d %Y %H:%M:%S GMT%z")
        .or_else(|_| DateTime::parse_from_str(cleaned, "%a %b %e %Y %H:%M:%S GMT%z"))
        .map_err(|e| {
            serde::de::Error::custom(format!(
                "Failed to parse Blink timestamp '{}': {}",
                raw, e
            ))
        })
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
    use csv::StringRecord;
    use rust_decimal_macros::dec;

    fn parse_csv_row(csv: &str) -> BlinkRecord {
        let header = StringRecord::from(BLINK_HEADERS);
        let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        reader.set_headers(header);
        reader.deserialize().next().unwrap().unwrap()
    }

    #[test]
    fn invoice_credit_becomes_receive() {
        let csv = "1,w1,invoice,1000,0,0,BTC,Wed Dec 10 2025 23:17:28 GMT+0000,false,j1,Coffee,,,,,Thanks,,,false,,,0,0,EUR";
        let record = parse_csv_row(csv);
        let tx = record.into_transaction().unwrap().unwrap();

        match tx.operation {
            Operation::Receive(amount) => {
                assert_eq!(amount.quantity, dec!(0.00001000));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected receive, got {:?}", other),
        }
    }

    #[test]
    fn payment_debit_becomes_send_with_fee() {
        let csv = "2,w1,payment,0,2500,10,BTC,Wed Dec 10 2025 23:17:28 GMT+0000,false,j2,,,,,,,,,false,,,0,0,EUR";
        let record = parse_csv_row(csv);
        let tx = record.into_transaction().unwrap().unwrap();

        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.00002500));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected send, got {:?}", other),
        }

        let fee = tx.fee.expect("fee set");
        assert_eq!(fee.quantity, dec!(0.00000010));
    }

    #[test]
    fn self_trade_group_becomes_trade() {
        let incoming = parse_csv_row("3,w1,self_trade,0,150000,0,BTC,Wed Dec 10 2025 23:17:28 GMT+0000,false,j3,,,,,,,,,false,,,0,0,EUR");
        let outgoing = parse_csv_row("4,w2,self_trade,2500,0,0,USD,Wed Dec 10 2025 23:17:28 GMT+0000,false,j3,,,,,,,,,false,,,0,0,EUR");

        let mut txs = convert_self_trade_group("j3", vec![incoming, outgoing]).unwrap();
        assert_eq!(txs.len(), 1);
        let tx = txs.pop().unwrap();

        match tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, USD_CURRENCY);
                assert_eq!(incoming.quantity, dec!(25.00));
                assert_eq!(outgoing.currency, BTC_CURRENCY);
                assert_eq!(outgoing.quantity, dec!(0.00150000));
            }
            other => panic!("expected trade, got {:?}", other),
        }
    }

    #[test]
    fn fee_type_becomes_fee_transaction() {
        let csv = "5,w1,fee,0,0,50,BTC,Wed Dec 10 2025 23:17:28 GMT+0000,false,j4,,,,,,,,,false,,,0,0,EUR";
        let record = parse_csv_row(csv);
        let tx = record.into_transaction().unwrap().unwrap();

        match tx.operation {
            Operation::Fee(amount) => {
                assert_eq!(amount.quantity, dec!(0.00000050));
                assert_eq!(amount.currency, BTC_CURRENCY);
            }
            other => panic!("expected fee, got {:?}", other),
        }
    }

    #[test]
    fn pending_confirmation_is_skipped() {
        let csv = "6,w1,invoice,1000,0,0,BTC,Wed Dec 10 2025 23:17:28 GMT+0000,true,j5,,,,,,,,,false,,,0,0,EUR";
        let record = parse_csv_row(csv);
        assert!(record.into_transaction().unwrap().is_none());
    }
}
