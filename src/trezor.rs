use std::{io::Seek, path::Path};

use anyhow::{Context, Result};
use chrono::DateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};

use crate::base::{Amount, Operation, Transaction};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TrezorTransactionType {
    #[serde(alias = "SENT")]
    Sent,
    #[serde(alias = "RECV", alias = "recv")]
    Received,
    #[serde(alias = "FAILED")]
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
enum TrezorAmount {
    Quantity(Decimal),
    TokenId(String),
}

fn deserialize_amount<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<TrezorAmount, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    match Decimal::try_from(raw) {
        Ok(quantity) => Ok(TrezorAmount::Quantity(quantity)),
        Err(_) => Ok(TrezorAmount::TokenId(
            raw.trim_start_matches("ID ").to_owned(),
        )),
    }
}

// Stores values loaded from CSV file exported by Trezor Suite, with the following header:
// Timestamp;Date;Time;Type;Transaction ID;Fee;Fee unit;Address;Label;Amount;Amount unit;Fiat (EUR);Other
#[derive(Debug, Deserialize)]
struct TrezorTransactionCsv<'a> {
    #[serde(rename = "Timestamp")]
    timestamp: i64,
    // #[serde(rename = "Date")]
    // date: &'a str,
    // #[serde(rename = "Time")]
    // time: &'a str,
    #[serde(rename = "Type")]
    type_: TrezorTransactionType,
    #[serde(rename = "Transaction ID")]
    id: &'a str,
    #[serde(rename = "Fee")]
    fee: Option<Decimal>,
    #[serde(rename = "Fee unit")]
    fee_unit: Option<&'a str>,
    // #[serde(rename = "Address")]
    // address: &'a str,
    #[serde(rename = "Label")]
    label: &'a str,
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    amount: TrezorAmount,
    #[serde(rename = "Amount unit")]
    amount_unit: &'a str,
    // #[serde(rename = "Fiat (EUR)")]
    // fiat_eur: Decimal,
    // #[serde(rename = "Other")]
    // other: &'a str,
}

impl<'a> TryFrom<TrezorTransactionCsv<'a>> for Transaction {
    type Error = anyhow::Error;

    // todo: translate address?
    fn try_from(item: TrezorTransactionCsv<'a>) -> std::result::Result<Self, Self::Error> {
        let date_time = DateTime::from_timestamp(item.timestamp, 0)
            .expect("valid timestamp")
            .naive_utc();
        let amount = match item.amount {
            TrezorAmount::Quantity(quantity) => Amount::new(quantity, item.amount_unit.to_owned()),
            TrezorAmount::TokenId(token_id) => {
                Amount::new_token(token_id, item.amount_unit.to_owned())
            }
        };
        let mut tx = match item.type_ {
            TrezorTransactionType::Sent => Transaction::send(date_time, amount),
            TrezorTransactionType::Received => Transaction::receive(date_time, amount),
            TrezorTransactionType::Failed => {
                anyhow::bail!("skipping failed transaction {}", item.id);
            }
        };
        tx.description = if item.label.is_empty() {
            None
        } else {
            Some(item.label.to_owned())
        };
        tx.tx_hash = Some(item.id.to_owned());
        tx.blockchain = Some(item.amount_unit.to_owned());
        tx.fee = match (item.fee, item.fee_unit) {
            (None, None) | (None, Some(_)) => None,
            (Some(fee), None) => {
                println!("warning: ignoring fee of {}, missing fee unit", fee);
                None
            }
            (Some(fee), Some(fee_unit)) => Some(Amount::new(fee, fee_unit.to_owned())),
        };
        Ok(tx)
    }
}

// loads a TREZOR Suite CSV file into a list of unified transactions
pub(crate) fn load_trezor_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let mut transactions = Vec::new();

    let mut buf_reader = BufReader::new(File::open(input_path)?);

    // Trezor Suite used to export with ';' delimiter, but now uses ','
    let mut first_line = String::new();
    buf_reader.read_line(&mut first_line)?;

    let mut reader_builder = csv::ReaderBuilder::new();
    if first_line.starts_with("Timestamp;") {
        reader_builder.delimiter(b';');
    }

    buf_reader.rewind()?;
    let mut rdr = reader_builder.from_reader(buf_reader);

    let mut raw_record = csv::StringRecord::new();
    let headers = rdr.headers()?.clone();

    while rdr.read_record(&mut raw_record)? {
        let record: TrezorTransactionCsv = raw_record.deserialize(Some(&headers))?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(_) => continue,
        }
    }

    Ok(transactions)
}

#[derive(Deserialize)]
struct InternalTransfer {
    #[serde(rename = "type")]
    type_: TrezorTransactionType,
    amount: Decimal,
    // from: String,
    // to: String,
}

// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// struct Target {
//     n: usize,
//     addresses: Vec<String>,
//     is_address: bool,
//     amount: Decimal,
//     #[serde(default)]
//     is_account_target: bool,
// }

#[derive(Deserialize)]
struct TokenTransfer {
    #[serde(rename = "type")]
    type_: TrezorTransactionType,
    // from: String,
    // to: String,
    contract: String,
    name: String,
    symbol: String,
    // decimals: u8,
    #[serde(deserialize_with = "deserialize_amount")]
    amount: TrezorAmount,
    // standard: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrezorTransaction {
    // descriptor: String,
    // device_state: String,
    symbol: String,
    type_: TrezorTransactionType,
    txid: String,
    block_time: u64,
    // block_height: u64,
    // block_hash: String,
    amount: Decimal,
    fee: Decimal,
    // targets: Vec<Target>,
    tokens: Vec<TokenTransfer>,
    internal_transfers: Vec<InternalTransfer>,
}

#[derive(Deserialize)]
struct TrezorWallet {
    // coin: String,
    transactions: Vec<TrezorTransaction>,
}

impl TrezorTransaction {
    fn extract_transactions(self, transactions: &mut Vec<Transaction>) -> Result<()> {
        let currency = self.symbol.to_uppercase();
        let date_time = DateTime::from_timestamp(self.block_time as i64, 0)
            .context("invalid timestamp")?
            .naive_utc();

        // Fee is only paid for "sent" transactions
        let mut fee = if matches!(self.type_, TrezorTransactionType::Sent) && !self.fee.is_zero() {
            Some(Amount::new(self.fee, currency.to_owned()))
        } else {
            None
        };

        let mut push_transaction = |operation, fee: &mut Option<Amount>| {
            let mut tx = Transaction::new(date_time, operation);
            tx.tx_hash = Some(self.txid.clone());
            tx.blockchain = Some(currency.clone());
            tx.fee = fee.take();
            transactions.push(tx);
        };

        // Determine the main transaction type
        let mut op = match self.type_ {
            TrezorTransactionType::Sent if self.amount.is_zero() && fee.is_some() => {
                Operation::Fee(fee.take().unwrap())
            }
            TrezorTransactionType::Sent => {
                Operation::Send(Amount::new(self.amount, currency.clone()))
            }
            TrezorTransactionType::Received => {
                Operation::Receive(Amount::new(self.amount, currency.clone()))
            }
            TrezorTransactionType::Failed => {
                return Ok(());
            }
        };

        // Adjust the operation based on internal transfers
        for internal_transfer in self.internal_transfers {
            let internal_amount = Amount::new(internal_transfer.amount, currency.clone());
            let internal_op = match internal_transfer.type_ {
                TrezorTransactionType::Sent => Operation::Send(internal_amount),
                TrezorTransactionType::Received => Operation::Receive(internal_amount),
                TrezorTransactionType::Failed => continue,
            };
            op = match (op, internal_op) {
                (Operation::Fee(fee_amount), internal_op) => {
                    fee = Some(fee_amount);
                    internal_op
                }
                (Operation::Receive(amount_a), Operation::Receive(amount_b)) => {
                    Operation::Receive(amount_a.try_add(&amount_b).unwrap())
                }
                (Operation::Send(amount_a), Operation::Send(amount_b)) => {
                    Operation::Send(amount_a.try_add(&amount_b).unwrap())
                }
                (Operation::Send(amount_a), Operation::Receive(amount_b)) => {
                    let change = amount_b.quantity - amount_a.quantity;
                    if change > Decimal::ZERO {
                        Operation::Receive(Amount::new(change, currency.clone()))
                    } else {
                        Operation::Send(Amount::new(-change, currency.clone()))
                    }
                }
                (Operation::Receive(amount_a), Operation::Send(amount_b)) => {
                    let change = amount_a.quantity - amount_b.quantity;
                    if change > Decimal::ZERO {
                        Operation::Receive(Amount::new(change, currency.clone()))
                    } else {
                        Operation::Send(Amount::new(-change, currency.clone()))
                    }
                }
                _ => unreachable!(),
            }
        }

        // If we sent or received some token, change the operation to a trade when applicable
        for token in self.tokens {
            let currency = match token.symbol.as_str() {
                "" => token.contract,
                _ => format!("{} ({})", token.symbol, token.name),
            };
            let token_amount = match token.amount {
                TrezorAmount::Quantity(quantity) => Amount::new(quantity, currency),
                TrezorAmount::TokenId(token_id) => Amount::new_token(token_id, currency),
            };
            let token_op = match token.type_ {
                TrezorTransactionType::Sent => Operation::Send(token_amount),
                TrezorTransactionType::Received => Operation::Receive(token_amount),
                TrezorTransactionType::Failed => continue,
            };

            op = match (op, token_op) {
                (Operation::Receive(amount_a), Operation::Receive(amount_b))
                    if amount_a.is_zero() =>
                {
                    Operation::Receive(amount_b)
                }
                (Operation::Receive(receive_amount), Operation::Send(send_amount))
                | (Operation::Send(send_amount), Operation::Receive(receive_amount)) => {
                    Operation::Trade {
                        incoming: receive_amount,
                        outgoing: send_amount,
                    }
                }
                (Operation::Fee(fee_amount), token_op) => {
                    fee = Some(fee_amount);
                    token_op
                }
                (op_orig, token_op) => {
                    // When we can't merge, create multiple transactions
                    push_transaction(op_orig, &mut fee);
                    token_op
                }
            }
        }

        push_transaction(op, &mut fee);
        Ok(())
    }
}

// loads a TREZOR Suite JSON file into a list of unified transactions
pub(crate) fn load_trezor_json(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let json: TrezorWallet = serde_json::from_str(&std::fs::read_to_string(input_path)?)?;
    for tx in json.transactions {
        tx.extract_transactions(&mut transactions)?;
    }

    Ok(transactions)
}
