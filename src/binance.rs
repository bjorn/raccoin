use std::path::Path;

use anyhow::Result;
use chrono::{NaiveDateTime, NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{time::deserialize_date_time, base::{Amount, Transaction, self, deserialize_amount}};

// #[derive(Debug, Deserialize)]
// enum Account {
//     Funding,
//     Spot,
// }

#[derive(Debug, Deserialize, Copy, Clone, PartialEq)]
enum Operation {
    Distribution,
    Deposit,
    Withdraw,
    #[serde(rename = "Transfer Between Main and Funding Wallet")]
    Transfer,
    #[serde(rename = "Binance Convert")]
    Convert,
    #[serde(rename = "Small Assets Exchange BNB")]
    SmallAssetsExchange,
    #[serde(rename = "Fiat Deposit")]
    FiatDeposit,
    #[serde(rename = "Fiat Withdrawal", alias = "Fiat Withdraw")]
    FiatWithdrawal,
    #[serde(rename = "Binance Card Cashback")]
    CardCashback,
    #[serde(rename = "Binance Card Spending")]
    CardSpending,
    #[serde(rename = "Airdrop Assets")]
    Airdrop,
    #[serde(rename = "Transaction Fee", alias = "Fee")]
    TransactionFee,
    #[serde(rename = "Transaction Buy", alias = "Buy")]
    TransactionBuy,
    #[serde(rename = "Transaction Spend")]
    TransactionSpend,
    #[serde(rename = "Transaction Sold", alias = "Sell")]
    TransactionSold,
    #[serde(rename = "Transaction Revenue")]
    TransactionRevenue,
}

// struct for storing the following CSV columns:
// "User_ID","UTC_Time","Account","Operation","Coin","Change","Remark"
#[derive(Debug, Deserialize)]
struct BinanceTransactionRecord {
    // #[serde(rename = "User_ID")]
    // user_id: String,
    #[serde(rename = "UTC_Time", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    // #[serde(rename = "Account")]
    // account: Account,
    #[serde(rename = "Operation")]
    operation: Operation,
    #[serde(rename = "Coin")]
    coin: String,
    #[serde(rename = "Change")]
    change: Decimal,
    #[serde(rename = "Remark")]
    remark: String,
}

#[derive(Debug, Deserialize)]
enum Side {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// struct for storing the following CSV columns:
// Date(UTC),Pair,Side,Price,Executed,Amount,Fee
#[derive(Debug, Deserialize)]
struct BinanceSpotTrade {
    #[serde(rename = "Date(UTC)", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    // #[serde(rename = "Pair")]
    // pair: String,
    #[serde(rename = "Side")]
    side: Side,
    // #[serde(rename = "Price")]
    // price: Decimal,
    #[serde(rename = "Executed", deserialize_with = "deserialize_amount")]
    executed: Amount,
    #[serde(rename = "Amount", deserialize_with = "deserialize_amount")]
    amount: Amount,
    #[serde(rename = "Fee", deserialize_with = "deserialize_amount")]
    fee: Amount,
}

// struct for storing the following CSV columns:
// Date,Coin,Amount,Fee (BNB),Converted BNB
#[derive(Debug, Deserialize)]
struct BinanceBnbConvert {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(rename = "Coin")]
    coin: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
    #[serde(rename = "Fee (BNB)")]
    fee_bnb: Decimal,
    #[serde(rename = "Converted BNB")]
    converted_bnb: Decimal,
}

// Binance reported BCH as BCC
fn normalize_currency(timestamp: NaiveDateTime, currency: String) -> String {
    match currency.as_str() {
        "BCC" => "BCH".to_owned(),
        "MANA" => "MANA (Decentraland)".to_owned(),
        "NANO" => "XNO".to_owned(),
        // rename LUNA to LUNC if it is mentioned before the rename that happened between 2022-05-26 and 2022-05-30
        // https://www.binance.com/en/support/announcement/binance-will-list-terra-2-0-luna-in-the-innovation-zone-luna-old-renamed-as-lunc-d044a6742e484b77a170111460b0eed3
        "LUNA" if timestamp < NaiveDate::from_ymd_opt(2022, 5, 27).unwrap().and_time(NaiveTime::MIN) => "LUNC".to_owned(),
        _ => currency,
    }
}
fn normalize_currency_for_amount(timestamp: NaiveDateTime, amount: Amount) -> Amount {
    Amount {
        quantity: amount.quantity,
        currency: normalize_currency(timestamp, amount.currency),
        token_id: amount.token_id,
    }
}

#[derive(Debug)]
enum ConversionError {
    IncompleteConvert(base::Operation),
    IgnoreReason(&'static str),
    InvalidValue(Operation, Decimal),
}

impl TryFrom<BinanceTransactionRecord> for Transaction {
    type Error = ConversionError;

    fn try_from(item: BinanceTransactionRecord) -> Result<Self, Self::Error> {
        // https://www.binance.com/en/support/announcement/binance-will-update-the-ticker-of-nano-to-xno-3dc8f6de281f4781a246a1658a21cb80
        if item.operation == Operation::Distribution && (item.coin == "NANO" || item.coin == "XNO") {
            return Err(ConversionError::IgnoreReason("Ignored NANO -> XNO conversion"));
        }

        let currency = normalize_currency(item.timestamp, item.coin);

        // Depending on the operation we expect a negative or positive amount,
        // for others we can expect either. Raise an error otherwise.
        let amount = match item.operation {
            Operation::Distribution |
            Operation::Deposit |
            Operation::FiatDeposit |
            Operation::CardCashback |
            Operation::Airdrop |
            Operation::TransactionBuy |
            Operation::TransactionRevenue => {
                if item.change > Decimal::ZERO {
                    Ok(Amount::new(item.change, currency))
                } else {
                    Err(ConversionError::InvalidValue(item.operation, item.change))
                }
            }

            Operation::Withdraw |
            Operation::FiatWithdrawal |
            Operation::TransactionFee |
            Operation::TransactionSpend |
            Operation::TransactionSold => {
                if item.change < Decimal::ZERO {
                    Ok(Amount::new(-item.change, currency))
                } else {
                    Err(ConversionError::InvalidValue(item.operation, item.change))
                }
            }

            Operation::Transfer |
            Operation::Convert |
            Operation::SmallAssetsExchange |
            Operation::CardSpending => {
                Ok(Amount::new(item.change.abs(), currency))
            }
        }?;

        let operation = match item.operation {
            Operation::Distribution => Ok(base::Operation::ChainSplit(amount)),
            Operation::Airdrop => Ok(base::Operation::Airdrop(amount)),
            Operation::Deposit => Ok(base::Operation::Receive(amount)),
            Operation::Withdraw => Ok(base::Operation::Send(amount)),
            Operation::Transfer => Err(ConversionError::IgnoreReason("'Transfer Between Main and Funding Wallet' ignored")),
            Operation::Convert => {
                if item.change > Decimal::ZERO {
                    Err(ConversionError::IncompleteConvert(base::Operation::Receive(amount)))
                } else {
                    Err(ConversionError::IncompleteConvert(base::Operation::Send(amount)))
                }
            }
            Operation::SmallAssetsExchange => {
                // These exchanges can't be reliably loaded from these
                // transaction records, since it's not possible to match each
                // incoming BNB amount to the correct small amount of assets
                // they were exchanged for. Instead, manually copy them from
                // https://www.binance.com/en/my/wallet/history/bnbconvert.
                Err(ConversionError::IgnoreReason("Export BNB Convert from https://www.binance.com/en/my/wallet/history/bnbconvert instead"))
            }
            Operation::FiatDeposit => Ok(base::Operation::FiatDeposit(amount)),
            Operation::FiatWithdrawal => Ok(base::Operation::FiatWithdrawal(amount)),
            Operation::CardCashback => Ok(base::Operation::Cashback(amount)),
            Operation::CardSpending => {
                if item.change > Decimal::ZERO {    // pay-in, likely a refund
                    if amount.is_fiat() {
                        Ok(base::Operation::FiatDeposit(amount))
                    } else {
                        Ok(base::Operation::Receive(amount))
                    }
                } else {
                    Ok(base::Operation::Expense(amount))
                }
            }
            Operation::TransactionFee |
            Operation::TransactionBuy |
            Operation::TransactionSpend |
            Operation::TransactionSold |
            Operation::TransactionRevenue => Err(ConversionError::IgnoreReason("Trade related entries are loaded from trade export")),
        }?;

        let mut tx = Transaction::new(item.timestamp, operation);
        tx.description = Some(item.remark);

        Ok(tx)
    }
}

impl From<BinanceSpotTrade> for Transaction {
    fn from(item: BinanceSpotTrade) -> Self {
        let executed = normalize_currency_for_amount(item.timestamp, item.executed);
        let amount = normalize_currency_for_amount(item.timestamp, item.amount);

        let mut tx = match item.side {
            Side::Buy => Transaction::trade(item.timestamp, executed, amount),
            Side::Sell => Transaction::trade(item.timestamp, amount, executed),
        };

        tx.fee = Some(normalize_currency_for_amount(item.timestamp, item.fee));
        tx
    }
}

impl From<BinanceBnbConvert> for Transaction {
    fn from(item: BinanceBnbConvert) -> Self {
        let incoming = Amount::new(item.converted_bnb, "BNB".to_owned());
        let outgoing = Amount::new(item.amount, normalize_currency(item.timestamp, item.coin));
        let mut tx = Transaction::trade(item.timestamp, incoming, outgoing);
        tx.fee = Some(Amount::new(item.fee_bnb, "BNB".to_owned()));
        tx
    }
}

pub(crate) fn load_binance_transaction_records_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut incomplete_convert: Option<base::Operation> = None;

    for result in rdr.deserialize() {
        let record: BinanceTransactionRecord = result?;
        let timestamp = record.timestamp;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(err) => match err {
                ConversionError::IncompleteConvert(operation) => {
                    match (&mut incomplete_convert, operation) {
                        (Some(base::Operation::Receive(incoming)), base::Operation::Send(outgoing)) => {
                            transactions.push(Transaction::trade(timestamp, incoming.clone(), outgoing));
                            incomplete_convert = None;
                        },
                        (Some(base::Operation::Send(outgoing)), base::Operation::Receive(incoming)) => {
                            transactions.push(Transaction::trade(timestamp, incoming, outgoing.clone()));
                            incomplete_convert = None;
                        },
                        (None, operation) if operation.is_send() || operation.is_receive() => {
                            incomplete_convert = Some(operation);
                        }
                        (_, operation) => {
                            println!("Error handling incomplete convert with operation: {:?}", operation);
                        }
                    }
                }
                ConversionError::IgnoreReason(_) => {}
                ConversionError::InvalidValue(operation, change) => {
                    println!("Unexpected 'change' value for {:?}: {:}", operation, change);
                }
            }
        }
    }

    if let Some(operation) = incomplete_convert {
        println!("Error: remaining incomplete convert with operation: {:?}", operation);
    }

    Ok(transactions)
}

pub(crate) fn load_binance_spot_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BinanceSpotTrade = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

pub(crate) fn load_binance_bnb_convert_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BinanceBnbConvert = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
