use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::{
    base::{deserialize_amount, Amount, Operation, Transaction},
    CsvSpec, TransactionSource,
};
use linkme::distributed_slice;

#[derive(Debug, Deserialize)]
enum BitstampTransactionType {
    Market,
    Withdrawal,
    Deposit,
    #[serde(rename = "Inter Account Transfer")]
    InterAccountTransfer,
    // A previously credited deposit that Bitstamp reversed. The amount is
    // negative, since the funds leave the account again.
    #[serde(rename = "Deposit reverted")]
    DepositReverted,
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

// deserialize function for reading the RFC 3339 / ISO 8601 datetime used by
// the "RFC 4180 (neu)" format, e.g. "2017-01-27T15:28:14Z". The timestamps are
// in UTC, so we drop the offset and store the naive UTC datetime.
fn deserialize_date_time_rfc3339<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.naive_utc())
        .map_err(|e| serde::de::Error::custom(format!(
            "Failed to parse datetime '{}': {} (expected RFC 3339, e.g. 2017-01-27T15:28:14Z)", raw, e
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
    #[serde(rename = "Datetime", deserialize_with = "deserialize_date_time_rfc3339")]
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
            BitstampTransactionType::DepositReverted => {
                // The reverted deposit leaves the account again, so the funds
                // move out just like a withdrawal.
                let amount = amount.abs();
                Ok(if amount.is_fiat() {
                    Transaction::fiat_withdrawal(item.datetime, amount)
                } else {
                    Transaction::send(item.datetime, amount)
                })
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
    let rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    read_bitstamp_csv(rdr)
}

fn read_bitstamp_csv<R: std::io::Read>(mut rdr: csv::Reader<R>) -> Result<Vec<Transaction>> {
    let mut converter = BitstampTransactionsConverter::new();

    for result in rdr.deserialize() {
        let record: BitstampTransaction = result?;
        converter.convert(record);
    }

    Ok(converter.finish())
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITSTAMP_CSV: TransactionSource = TransactionSource {
    id: "BitstampCsv",
    label: "Bitstamp Old (CSV)",
    csv: &[CsvSpec::new(&[
        "Type", "Datetime", "Account", "Amount", "Value", "Rate", "Fee", "Sub Type",
    ])],
    detect: None,
    load_sync: Some(load_bitstamp_old_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITSTAMP_CSV_NEW: TransactionSource = TransactionSource {
    id: "BitstampCsvNew",
    label: "Bitstamp RFC 4180 (CSV)",
    csv: &[CsvSpec::new(&[
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
    ])],
    detect: None,
    load_sync: Some(load_bitstamp_csv),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_date_time;
    use rust_decimal_macros::dec;

    fn load(csv: &str) -> Vec<Transaction> {
        let rdr = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        read_bitstamp_csv(rdr).unwrap()
    }

    // Synthetic "RFC 4180 (new)" export, covering the RFC 3339 datetimes (with
    // the trailing "Z") and a reverted deposit.
    const NEW_FORMAT_CSV: &str = "\
ID,Account,Type,Subtype,Datetime,Amount,Amount currency,Value,Value currency,Rate,Rate currency,Fee,Fee currency,Order ID
10000001,Main Account,Deposit,,2020-03-15T09:00:00Z,1500.00,EUR,,,,,,,
10000002,Main Account,Market,Buy,2020-03-15T09:05:00Z,0.10000000,BTC,1500.00,EUR,15000.00,EUR,,,100000001
10000003,Main Account,Market,Sell,2021-07-01T12:00:00Z,0.10000000,BTC,2500.00,EUR,25000.00,EUR,7.50,EUR,100000002
10000004,Main Account,Withdrawal,,2021-08-01T08:00:00Z,0.05000000,BTC,,,,,0.00010000,BTC,
10000005,Main Account,Deposit reverted,,2022-02-10T14:30:00Z,-0.00200000,XLM,,,,,,,
";

    #[test]
    fn parses_rfc3339_datetime() {
        let txs = load(NEW_FORMAT_CSV);
        // The trailing "Z" used to break parsing with a "trailing input" error.
        assert_eq!(
            txs[0].timestamp,
            parse_date_time("2020-03-15 09:00:00").unwrap()
        );
        assert!(matches!(
            &txs[0].operation,
            Operation::FiatDeposit(amount) if amount.quantity == dec!(1500.00) && amount.currency == "EUR"
        ));
    }

    #[test]
    fn parses_market_buy_and_sell() {
        let txs = load(NEW_FORMAT_CSV);

        match &txs[1].operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.10000000));
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(1500.00));
                assert_eq!(outgoing.currency, "EUR");
            }
            op => panic!("expected a Trade for the Buy, got {:?}", op),
        }

        match &txs[2].operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "EUR");
                assert_eq!(outgoing.currency, "BTC");
            }
            op => panic!("expected a Trade for the Sell, got {:?}", op),
        }
        assert_eq!(txs[2].fee.as_ref().unwrap().quantity, dec!(7.50));
    }

    #[test]
    fn reverted_deposit_becomes_a_send() {
        let txs = load(NEW_FORMAT_CSV);
        let reverted = txs.last().unwrap();
        // The funds leave the account again, so the negative amount maps to a
        // Send of its absolute value (matched against the Receive in another
        // wallet, when present).
        assert!(matches!(
            &reverted.operation,
            Operation::Send(amount) if amount.quantity == dec!(0.00200000) && amount.currency == "XLM"
        ));
        assert_eq!(
            reverted.timestamp,
            parse_date_time("2022-02-10 14:30:00").unwrap()
        );
    }
}
