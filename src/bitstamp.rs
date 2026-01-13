use std::path::Path;

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{
    base::{deserialize_amount, Amount, Operation, Transaction},
    CsvSpec, TransactionSourceType,
};
use linkme::distributed_slice;

#[derive(Debug, Deserialize)]
enum BitstampTransactionType {
    Market,
    Withdrawal,
    Deposit,
    #[serde(rename = "Inter Account Transfer")]
    InterAccountTransfer,
}

#[derive(Debug, Deserialize)]
enum SubType {
    Buy,
    Sell,
}

// deserialize function for reading NaiveDateTime in the format "Jan. 27, 2017, 03:28 PM"
fn deserialize_date_time<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    NaiveDateTime::parse_from_str(raw, "%b. %d, %Y, %I:%M %p")
        .map_err(|e| serde::de::Error::custom(format!(
            "Failed to parse datetime '{}': {} (expected format: %b. %d, %Y, %I:%M %p)", raw, e
        )))
}

fn deserialize_amount_opt<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Amount>, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(Amount::try_from(raw).unwrap()))
}

// struct for storing the following CSV columns (Old format):
// Type,Datetime,Account,Amount,Value,Rate,Fee,Sub Type
#[derive(Debug, Deserialize)]
struct BitstampTransactionOld {
    #[serde(rename = "Type")]
    pub type_: BitstampTransactionType,
    #[serde(rename = "Datetime", deserialize_with = "deserialize_date_time")]
    pub datetime: NaiveDateTime,
    // #[serde(rename = "Account")]
    // pub account: String,
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    pub amount: Amount,
    #[serde(rename = "Value", deserialize_with = "deserialize_amount_opt")]
    pub value: Option<Amount>,
    // #[serde(rename = "Rate", deserialize_with = "deserialize_amount_opt")]
    // pub rate: Option<Amount>,
    #[serde(rename = "Fee", deserialize_with = "deserialize_amount_opt")]
    pub fee: Option<Amount>,
    #[serde(rename = "Sub Type")]
    pub sub_type: Option<SubType>,
}

// struct for storing the following CSV columns ("RFC 4180 (neu)" format):
// ID,Account,Type,Subtype,Datetime,Amount,Amount currency,Value,Value currency,Rate,Rate currency,Fee,Fee currency,Order ID
#[derive(Debug, Deserialize)]
struct BitstampTransaction {
    // #[serde(rename = "ID")]
    // pub id: String,
    // #[serde(rename = "Account")]
    // pub account: String,
    #[serde(rename = "Type")]
    pub type_: BitstampTransactionType,
    #[serde(rename = "Subtype")]
    pub sub_type: Option<SubType>,
    #[serde(rename = "Datetime")]
    pub datetime: NaiveDateTime,
    #[serde(rename = "Amount")]
    pub amount: Decimal,
    #[serde(rename = "Amount currency")]
    pub amount_currency: String,
    #[serde(rename = "Value")]
    pub value: Option<Decimal>,
    #[serde(rename = "Value currency")]
    pub value_currency: Option<String>,
    // #[serde(rename = "Rate")]
    // pub rate: Option<Decimal>,
    // #[serde(rename = "Rate currency")]
    // pub rate_currency: Option<String>,
    #[serde(rename = "Fee")]
    pub fee: Option<Decimal>,
    #[serde(rename = "Fee currency")]
    pub fee_currency: Option<String>,
    #[serde(rename = "Order ID")]
    pub order_id: Option<String>,
}

impl From<BitstampTransactionOld> for BitstampTransaction {
    fn from(item: BitstampTransactionOld) -> Self {
        BitstampTransaction {
            type_: item.type_,
            sub_type: item.sub_type,
            datetime: item.datetime,
            amount: item.amount.quantity,
            amount_currency: item.amount.currency,
            value: item.value.as_ref().map(|v| v.quantity),
            value_currency: item.value.map(|v| v.currency),
            fee: item.fee.as_ref().map(|f| f.quantity),
            fee_currency: item.fee.map(|f| f.currency),
            order_id: None,
        }
    }
}

#[derive(Debug)]
enum ConversionError {
    MissingFields,
    InterAccountTransfer(Transaction),
}

impl TryFrom<BitstampTransaction> for Transaction {
    type Error = ConversionError;

    fn try_from(item: BitstampTransaction) -> Result<Self, Self::Error> {
        let amount = Amount::new(item.amount, item.amount_currency);
        let mut tx = match item.type_ {
            BitstampTransactionType::Market => {
                let value = match (item.value, item.value_currency) {
                    (Some(value), Some(currency)) => Some(Amount::new(value, currency)),
                    _ => None,
                };

                match (item.sub_type, value) {
                    (Some(SubType::Buy), Some(value)) => {
                        Ok(Transaction::trade(item.datetime, amount, value))
                    }
                    (Some(SubType::Sell), Some(value)) => {
                        Ok(Transaction::trade(item.datetime, value, amount))
                    }
                    _ => Err(ConversionError::MissingFields), // Missing Sub Type and/or Value for Market transaction
                }
            }
            BitstampTransactionType::Withdrawal => Ok(if amount.is_fiat() {
                Transaction::fiat_withdrawal(item.datetime, amount)
            } else {
                Transaction::send(item.datetime, amount)
            }),
            BitstampTransactionType::Deposit => Ok(if amount.is_fiat() {
                Transaction::fiat_deposit(item.datetime, amount)
            } else {
                Transaction::receive(item.datetime, amount)
            }),
            BitstampTransactionType::InterAccountTransfer => {
                // Create a temporary transaction that we can later try to match
                let tx = if amount.quantity > Decimal::ZERO {
                    Transaction::receive(item.datetime, amount)
                } else {
                    Transaction::send(item.datetime, amount.abs())
                };
                Err(ConversionError::InterAccountTransfer(tx))
            }
        }?;

        tx.fee = match (item.fee, item.fee_currency) {
            (Some(fee), Some(currency)) => Some(Amount::new(fee, currency)),
            _ => None,
        };

        tx.description = item
            .order_id
            .map(|order_id| format!("Order ID: {}", order_id));

        Ok(tx)
    }
}

struct Airdrop {
    date: NaiveDate,
    currency: String,
}

struct BitstampTransactionsConverter {
    transactions: Vec<Transaction>,
    inter_account_transfers: Vec<Transaction>,
    airdrops: Vec<Airdrop>,
}

impl BitstampTransactionsConverter {
    fn new() -> Self {


        BitstampTransactionsConverter {
            transactions: Vec::new(),
            inter_account_transfers: Vec::new(),

            // Known airdrops on Bitstamp
            airdrops: vec![
                Airdrop {
                    date: NaiveDate::from_ymd_opt(2021, 12, 8).unwrap(),
                    currency: "SGB".to_string(),
                },
                Airdrop {
                    date: NaiveDate::from_ymd_opt(2023, 1, 6).unwrap(),
                    currency: "FLR".to_string(),
                },
            ],
        }
    }

    fn convert(&mut self, bitstamp_tx: BitstampTransaction) {
        match Transaction::try_from(bitstamp_tx) {
            Ok(tx) => {
                let tx = self.check_airdrop(tx);
                self.transactions.push(tx);
            }
            Err(ConversionError::InterAccountTransfer(tx)) => {
                match &tx.operation {
                    Operation::Receive(incoming) if incoming.is_fiat() => {
                        // If we receive fiat, match it with the oldest pending crypto send
                        if let Some(send_idx) = self.inter_account_transfers.iter().position(|pending| {
                            matches!(&pending.operation, Operation::Send(amount) if !amount.is_fiat())
                        }) {
                            let send_tx = self.inter_account_transfers.remove(send_idx);
                            if let Operation::Send(outgoing) = send_tx.operation {
                                // Create a trade: received fiat in exchange for sent crypto
                                println!("Bitstamp: Merging Inter Account Transfer at {} as {} traded for {}", tx.timestamp, outgoing, incoming);
                                let mut trade = Transaction::trade(send_tx.timestamp, incoming.clone(), outgoing);
                                trade.description = Some(format!("Merge of Inter Account Transfers at {} and {}", send_tx.timestamp, tx.timestamp));
                                self.transactions.push(trade);
                                return;
                            }
                        }
                    }
                    _ => (),
                }

                // Remember unmatched inter-account transfers
                self.inter_account_transfers.push(tx);
            }
            Err(_) => return,
        };
    }

    fn check_airdrop(&mut self, mut tx: Transaction) -> Transaction {
        let tx_date = tx.timestamp.date();
        tx.operation = match tx.operation {
            Operation::Receive(receive) if !receive.is_fiat() => {
                if let Some(airdrop_idx) = self.airdrops.iter().position(|airdrop| {
                    airdrop.currency == receive.currency && airdrop.date == tx_date
                }) {
                    println!(
                        "Bitstamp: Detected airdrop of {} at {}",
                        receive, tx.timestamp
                    );
                    self.airdrops.remove(airdrop_idx);  // Only one airdrop expected per currency
                    Operation::Airdrop(receive)
                } else {
                    Operation::Receive(receive)
                }
            }
            op => op,
        };
        tx
    }

    fn finish(mut self) -> Vec<Transaction> {
        // Add unmatched inter-account transfers to transactions
        self.transactions.extend(self.inter_account_transfers);
        self.transactions
    }
}

fn load_bitstamp_old_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut converter = BitstampTransactionsConverter::new();
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitstampTransactionOld = result?;
        converter.convert(BitstampTransaction::from(record));
    }

    Ok(converter.finish())
}

fn load_bitstamp_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut converter = BitstampTransactionsConverter::new();
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitstampTransaction = result?;
        converter.convert(record);
    }

    Ok(converter.finish())
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITSTAMP_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "BitstampCsv",
    label: "Bitstamp Old (CSV)",
    csv: Some(CsvSpec::new(&[
        "Type", "Datetime", "Account", "Amount", "Value", "Rate", "Fee", "Sub Type",
    ])),
    detect: None,
    load_sync: Some(load_bitstamp_old_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITSTAMP_CSV_NEW_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "BitstampCsvNew",
    label: "Bitstamp RFC 4180 (CSV)",
    csv: Some(CsvSpec::new(&[
        "ID",
        "Account",
        "Type",
        "Subtype",
        "Datetime",
        "Amount",
        "Amount currency",
        "Value",
        "Value currency",
        "Rate",
        "Rate currency",
        "Fee",
        "Fee currency",
        "Order ID",
    ])),
    detect: None,
    load_sync: Some(load_bitstamp_csv),
    load_async: None,
};
