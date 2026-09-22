use std::path::Path;

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    base::{self, deserialize_amount, Amount, Transaction},
    time::deserialize_date_time,
    CsvSpec, TransactionSource,
};
use linkme::distributed_slice;

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
    #[serde(rename = "transfer_in", alias = "Transfer In")]
    TransferIn,
    #[serde(rename = "transfer_out", alias = "Transfer Out")]
    TransferOut,
    #[serde(rename = "Binance Convert")]
    Convert,
    #[serde(
        rename = "Small Assets Exchange BNB",
        alias = "Small assets exchange BNB"
    )]
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
    #[serde(rename = "Pool Distribution")]
    PoolDistribution,
    #[serde(rename = "Referrer rebates", alias = "Referrer Rebate")]
    ReferrerRebate,
    #[serde(rename = "Commission History")]
    CommissionHistory,
    #[serde(rename = "Commission Rebate")]
    CommissionRebate,
    #[serde(rename = "ETH 2.0 Staking Rewards")]
    EthStakingRewards,
    #[serde(rename = "Savings Interest")]
    SavingsInterest,
    #[serde(
        rename = "Savings Principal redemption",
        alias = "POS savings redemption"
    )]
    SavingsPrincipalRedemption,
    #[serde(rename = "Savings purchase", alias = "POS savings purchase")]
    SavingsPurchase,
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
    #[serde(rename = "Transaction Related")]
    TransactionRelated,
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
struct BinanceConvert {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
    #[serde(rename = "Coin")]
    coin: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
    #[serde(rename = "Fee", deserialize_with = "deserialize_amount")]
    fee: Amount,
    #[serde(rename = "Converted To", deserialize_with = "deserialize_amount")]
    converted_to: Amount,
}

// Binance reported BCH as BCC
fn normalize_currency(timestamp: NaiveDateTime, currency: String) -> String {
    match currency.as_str() {
        "BCC" => "BCH".to_owned(),
        "MANA" => "MANA (Decentraland)".to_owned(),
        "NANO" => "XNO".to_owned(),
        // rename LUNA to LUNC if it is mentioned before the rename that happened between 2022-05-26 and 2022-05-30
        // https://www.binance.com/en/support/announcement/binance-will-list-terra-2-0-luna-in-the-innovation-zone-luna-old-renamed-as-lunc-d044a6742e484b77a170111460b0eed3
        "LUNA"
            if timestamp
                < NaiveDate::from_ymd_opt(2022, 5, 27)
                    .unwrap()
                    .and_time(NaiveTime::MIN) =>
        {
            "LUNC".to_owned()
        }
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
    // we do keep track of ignore reason, but don't use it for now since it is too verbose
    #[allow(dead_code)]
    IgnoreReason(&'static str),
    InvalidValue(Operation, Decimal),
}

impl TryFrom<BinanceTransactionRecord> for Transaction {
    type Error = ConversionError;

    fn try_from(item: BinanceTransactionRecord) -> Result<Self, Self::Error> {
        // https://www.binance.com/en/support/announcement/binance-will-update-the-ticker-of-nano-to-xno-3dc8f6de281f4781a246a1658a21cb80
        if item.operation == Operation::Distribution && (item.coin == "NANO" || item.coin == "XNO")
        {
            return Err(ConversionError::IgnoreReason(
                "Ignored NANO -> XNO conversion",
            ));
        }

        let currency = normalize_currency(item.timestamp, item.coin);

        // Depending on the operation we expect a negative or positive amount,
        // for others we can expect either. Raise an error otherwise.
        let amount = match item.operation {
            Operation::Distribution
            | Operation::Deposit
            | Operation::FiatDeposit
            | Operation::CardCashback
            | Operation::Airdrop
            | Operation::PoolDistribution
            | Operation::ReferrerRebate
            | Operation::CommissionHistory
            | Operation::CommissionRebate
            | Operation::EthStakingRewards
            | Operation::SavingsInterest
            | Operation::TransactionBuy
            | Operation::TransactionRevenue => {
                if item.change > Decimal::ZERO {
                    Ok(Amount::new(item.change, currency))
                } else {
                    Err(ConversionError::InvalidValue(item.operation, item.change))
                }
            }

            Operation::Withdraw
            | Operation::FiatWithdrawal
            | Operation::TransactionFee
            | Operation::TransactionSpend
            | Operation::TransactionSold => {
                if item.change < Decimal::ZERO {
                    Ok(Amount::new(-item.change, currency))
                } else {
                    Err(ConversionError::InvalidValue(item.operation, item.change))
                }
            }

            Operation::Transfer
            | Operation::TransferIn
            | Operation::TransferOut
            | Operation::Convert
            | Operation::SmallAssetsExchange
            | Operation::CardSpending
            | Operation::SavingsPrincipalRedemption
            | Operation::SavingsPurchase
            | Operation::TransactionRelated => Ok(Amount::new(item.change.abs(), currency)),
        }?;

        let operation = match item.operation {
            Operation::Distribution => Ok(base::Operation::ChainSplit(amount)),
            Operation::Airdrop => Ok(base::Operation::Airdrop(amount)),
            Operation::PoolDistribution
            | Operation::EthStakingRewards
            | Operation::SavingsInterest => Ok(base::Operation::Staking(amount)),
            Operation::ReferrerRebate | Operation::CommissionHistory => {
                Ok(base::Operation::Income(amount))
            }
            Operation::CommissionRebate => Ok(base::Operation::Cashback(amount)),
            Operation::Deposit => Ok(base::Operation::Receive(amount)),
            Operation::Withdraw => Ok(base::Operation::Send(amount)),
            Operation::Transfer
            | Operation::TransferIn
            | Operation::TransferOut
            | Operation::SavingsPrincipalRedemption
            | Operation::SavingsPurchase => Err(ConversionError::IgnoreReason(
                "Internal Binance account movement ignored",
            )),
            Operation::Convert => {
                if item.change > Decimal::ZERO {
                    Err(ConversionError::IncompleteConvert(
                        base::Operation::Receive(amount),
                    ))
                } else {
                    Err(ConversionError::IncompleteConvert(base::Operation::Send(
                        amount,
                    )))
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
                if item.change > Decimal::ZERO {
                    // pay-in, likely a refund
                    if amount.is_fiat() {
                        Ok(base::Operation::FiatDeposit(amount))
                    } else {
                        Ok(base::Operation::Receive(amount))
                    }
                } else {
                    Ok(base::Operation::Expense(amount))
                }
            }
            Operation::TransactionFee
            | Operation::TransactionBuy
            | Operation::TransactionSpend
            | Operation::TransactionSold
            | Operation::TransactionRevenue
            | Operation::TransactionRelated => Err(ConversionError::IgnoreReason(
                "Trade related entries are loaded from trade export",
            )),
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

impl From<BinanceConvert> for Transaction {
    fn from(item: BinanceConvert) -> Self {
        let incoming = item.converted_to;
        let outgoing = Amount::new(item.amount, normalize_currency(item.timestamp, item.coin));
        let mut tx = Transaction::trade(item.timestamp, incoming, outgoing);
        tx.fee = Some(item.fee);
        tx
    }
}

fn load_binance_transaction_records_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;

    let mut incomplete_convert: Option<base::Operation> = None;

    for result in rdr.deserialize() {
        let record: BinanceTransactionRecord = result?;
        let timestamp = record.timestamp;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(err) => match err {
                ConversionError::IncompleteConvert(operation) => {
                    match (&mut incomplete_convert, operation) {
                        (
                            Some(base::Operation::Receive(incoming)),
                            base::Operation::Send(outgoing),
                        ) => {
                            transactions.push(Transaction::trade(
                                timestamp,
                                incoming.clone(),
                                outgoing,
                            ));
                            incomplete_convert = None;
                        }
                        (
                            Some(base::Operation::Send(outgoing)),
                            base::Operation::Receive(incoming),
                        ) => {
                            transactions.push(Transaction::trade(
                                timestamp,
                                incoming,
                                outgoing.clone(),
                            ));
                            incomplete_convert = None;
                        }
                        (None, operation) if operation.is_send() || operation.is_receive() => {
                            incomplete_convert = Some(operation);
                        }
                        (_, operation) => {
                            println!(
                                "Error handling incomplete convert with operation: {:?}",
                                operation
                            );
                        }
                    }
                }
                ConversionError::IgnoreReason(_) => {}
                ConversionError::InvalidValue(operation, change) => {
                    println!("Unexpected 'change' value for {:?}: {:}", operation, change);
                }
            },
        }
    }

    if let Some(operation) = incomplete_convert {
        println!(
            "Error: remaining incomplete convert with operation: {:?}",
            operation
        );
    }

    Ok(transactions)
}

fn load_binance_spot_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BinanceSpotTrade = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// todo: document custom format
fn load_binance_convert_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BinanceConvert = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BINANCE_CONVERT_CSV: TransactionSource = TransactionSource {
    id: "BinanceConvertCsv",
    label: "Binance Convert (CSV)",
    csv: &[CsvSpec::new(&[
        "Date",
        "Coin",
        "Amount",
        "Fee",
        "Converted To",
    ])],
    detect: None,
    load_sync: Some(load_binance_convert_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BINANCE_SPOT_TRADE_HISTORY_CSV: TransactionSource = TransactionSource {
    id: "BinanceSpotTradeHistoryCsv",
    label: "Binance Spot Trade History (CSV)",
    csv: &[CsvSpec::new(&[
        "Date(UTC)",
        "Pair",
        "Side",
        "Price",
        "Executed",
        "Amount",
        "Fee",
    ])],
    detect: None,
    load_sync: Some(load_binance_spot_trades_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BINANCE_TRANSACTION_HISTORY_CSV: TransactionSource = TransactionSource {
    id: "BinanceTransactionHistoryCsv",
    label: "Binance Transaction History (CSV)",
    csv: &[CsvSpec::new(&[
        "User_ID",
        "UTC_Time",
        "Account",
        "Operation",
        "Coin",
        "Change",
        "Remark",
    ])],
    detect: None,
    load_sync: Some(load_binance_transaction_records_csv),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use rust_decimal_macros::dec;

    use super::*;
    use crate::base::Operation as BaseOperation;

    fn temp_csv(name: &str, contents: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "raccoin-binance-test-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn loads_additional_binance_statement_operations() {
        let csv_path = temp_csv(
            "binance-statement.csv",
            "User_ID,UTC_Time,Account,Operation,Coin,Change,Remark\n\
             1,2024-03-04 05:06:07,USDT-Futures,Referrer rebates,USDT,1.5,referral\n\
             1,2024-03-05 05:06:07,Spot,Commission Rebate,BNB,0.02,rebate\n\
             1,2024-03-06 05:06:07,Spot,ETH 2.0 Staking Rewards,ETH,0.03,staking\n\
             1,2024-03-07 05:06:07,Spot,Savings Interest,USDT,0.04,savings interest\n\
             1,2024-03-08 05:06:07,Pool,Pool Distribution,BUSD,0.05,pool distribution\n\
             1,2024-03-09 05:06:07,Spot,transfer_in,BTC,1.0,internal in\n\
             1,2024-03-10 05:06:07,Spot,Savings purchase,USDT,-25,internal savings\n\
             1,2024-03-11 05:06:07,Spot,Transaction Related,USDT,-10,trade ledger line\n",
        );

        let txs = load_binance_transaction_records_csv(&csv_path).unwrap();

        assert_eq!(txs.len(), 5);
        match &txs[0].operation {
            BaseOperation::Income(amount) => {
                assert_eq!(amount.quantity, dec!(1.5));
                assert_eq!(amount.currency, "USDT");
            }
            operation => panic!("Unexpected operation {operation:?}"),
        }
        match &txs[1].operation {
            BaseOperation::Cashback(amount) => {
                assert_eq!(amount.quantity, dec!(0.02));
                assert_eq!(amount.currency, "BNB");
            }
            operation => panic!("Unexpected operation {operation:?}"),
        }
        for (tx, expected_quantity, expected_currency) in [
            (&txs[2], dec!(0.03), "ETH"),
            (&txs[3], dec!(0.04), "USDT"),
            (&txs[4], dec!(0.05), "BUSD"),
        ] {
            match &tx.operation {
                BaseOperation::Staking(amount) => {
                    assert_eq!(amount.quantity, expected_quantity);
                    assert_eq!(amount.currency, expected_currency);
                }
                operation => panic!("Unexpected operation {operation:?}"),
            }
        }
    }
}
