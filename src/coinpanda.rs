use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::{ctc::{CtcTxType, CtcTx}, time::serialize_date_time};

#[derive(Debug, Serialize)]
enum CoinpandaTxType {
    /// Use this if your transaction involves both a buy and sell side - typically whenever you have exchanged one currency for another.
    Trade,
    /// Use this if you have received crypto or fiat - i.e., an oncoming transaction. Only the columns Received Amount and Received Currency should be filled for Receive transactions.
    Receive,
    /// Use this if you have sent crypto or fiat - i.e., an outgoing transaction. Only the columns Sent Amount and Sent Currency should be filled for Send transactions.
    Send,
}

#[derive(Debug, Serialize)]
struct CoinpandaTx<'a> {
    /// All dates should be UTC timezone
    #[serde(rename = "Timestamp (UTC)", serialize_with = "serialize_date_time")]
    timestamp: NaiveDateTime,

    /// Can be added to make reading the file easier
    #[serde(rename = "Type")]
    type_: CoinpandaTxType,

    /// The amount sold/withdrawn/sent (outgoing)
    #[serde(rename = "Sent Amount")]
    sent_amount: Option<Decimal>,

    /// Currency sold/withdrawn/sent (outgoing)
    #[serde(rename = "Sent Currency")]
    sent_currency: Option<&'a str>,

    /// The amount bought/deposited/received (incoming)
    #[serde(rename = "Received Amount")]
    received_amount: Option<Decimal>,

    /// Currency bought/deposited/received (incoming)
    #[serde(rename = "Received Currency")]
    received_currency: Option<&'a str>,

    /// Any associated fee amount
    #[serde(rename = "Fee Amount")]
    fee_amount: Option<Decimal>,

    /// Fee currency if you paid any fee
    #[serde(rename = "Fee Currency")]
    fee_currency: Option<&'a str>,

    /// The value of the transaction in a fiat currency
    #[serde(rename = "Net Worth Amount")]
    net_worth_amount: Option<Decimal>,

    /// The fiat currency used to value the transaction
    #[serde(rename = "Net Worth Currency")]
    net_worth_currency: Option<&'a str>,

    /// You can specify labels for both Trade, Receive and Send types. A list of supported labels is shown below.
    #[serde(rename = "Label")]
    label: Option<&'a str>,

    /// Description of the transaction, eg. Sent bitcoin to mom
    #[serde(rename = "Description")]
    description: Option<&'a str>,

    /// Transaction Hash
    #[serde(rename = "TxHash")]
    tx_hash: Option<&'a str>,
}

impl<'a> CoinpandaTx<'a> {
    fn trade(timestamp: NaiveDateTime, received_amount: Decimal, received_currency: &'a str, sent_amount: Decimal, sent_currency: &'a str) -> Self {
        Self {
            timestamp,
            type_: CoinpandaTxType::Trade,
            sent_amount: Some(sent_amount),
            sent_currency: Some(sent_currency),
            received_amount: Some(received_amount),
            received_currency: Some(received_currency),
            fee_amount: None,
            fee_currency: None,
            net_worth_amount: None,
            net_worth_currency: None,
            label: None,
            description: None,
            tx_hash: None,
        }
    }

    fn send(timestamp: NaiveDateTime, sent_amount: Decimal, sent_currency: &'a str) -> Self {
        Self {
            timestamp,
            type_: CoinpandaTxType::Send,
            sent_amount: Some(sent_amount),
            sent_currency: Some(sent_currency),
            received_amount: None,
            received_currency: None,
            fee_amount: None,
            fee_currency: None,
            net_worth_amount: None,
            net_worth_currency: None,
            label: None,
            description: None,
            tx_hash: None,
        }
    }

    fn receive(timestamp: NaiveDateTime, received_amount: Decimal, received_currency: &'a str) -> Self {
        Self {
            timestamp,
            type_: CoinpandaTxType::Receive,
            sent_amount: None,
            sent_currency: None,
            received_amount: Some(received_amount),
            received_currency: Some(received_currency),
            fee_amount: None,
            fee_currency: None,
            net_worth_amount: None,
            net_worth_currency: None,
            label: None,
            description: None,
            tx_hash: None,
        }
    }
}

fn convert_ctc_to_coinpanda<'a>(ctc: &'a CtcTx) -> CoinpandaTx<'a> {
    let mut tx = match ctc.operation {
        CtcTxType::Buy => CoinpandaTx::trade(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency,
            ctc.quote_amount.expect("quote amount"),
            ctc.quote_currency.expect("quote currency")
        ),
        CtcTxType::Sell => CoinpandaTx::trade(
            ctc.timestamp,
            ctc.quote_amount.expect("quote amount"),
            ctc.quote_currency.expect("quote currency"),
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::FiatDeposit => CoinpandaTx::receive(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::FiatWithdrawal => CoinpandaTx::send(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::Fee => CoinpandaTx::send(
            ctc.timestamp,
            ctc.fee_amount.expect("fee amount"),
            ctc.fee_currency.expect("fee currency")
        ),
        CtcTxType::Approval => todo!(),
        CtcTxType::Receive => CoinpandaTx::receive(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::Send => CoinpandaTx::send(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::ChainSplit => todo!(),
        CtcTxType::Expense => todo!(),
        CtcTxType::Stolen => todo!(),
        CtcTxType::Lost => todo!(),
        CtcTxType::Burn => todo!(),
        CtcTxType::Income => todo!(),
        CtcTxType::Interest => todo!(),
        CtcTxType::Mining => todo!(),
        CtcTxType::Airdrop => todo!(),
        CtcTxType::Staking => todo!(),
        CtcTxType::StakingDeposit => todo!(),
        CtcTxType::StakingWithdrawal => todo!(),
        CtcTxType::Cashback => todo!(),
        CtcTxType::Royalties => todo!(),
        CtcTxType::PersonalUse => todo!(),
        CtcTxType::IncomingGift => CoinpandaTx::receive(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::OutgoingGift => CoinpandaTx::send(
            ctc.timestamp,
            ctc.base_amount,
            ctc.base_currency
        ),
        CtcTxType::Borrow => todo!(),
        CtcTxType::LoanRepayment => todo!(),
        CtcTxType::Liquidate => todo!(),
        CtcTxType::RealizedProfit => todo!(),
        CtcTxType::RealizedLoss => todo!(),
        CtcTxType::MarginFee => todo!(),
        CtcTxType::BridgeIn => todo!(),
        CtcTxType::BridgeOut => todo!(),
        CtcTxType::Mint => todo!(),
        CtcTxType::CollateralWithdrawal => todo!(),
        CtcTxType::CollateralDeposit => todo!(),
        CtcTxType::AddLiquidity => todo!(),
        CtcTxType::ReceiveLpToken => todo!(),
        CtcTxType::RemoveLiquidity => todo!(),
        CtcTxType::ReturnLpToken => todo!(),
        CtcTxType::FailedIn => todo!(),
        CtcTxType::FailedOut => todo!(),
        CtcTxType::Spam => todo!(),
    };
    tx.description = ctc.description;
    // tx.fee_amount = ctc.fee_amount;
    // tx.fee_currency = ctc.fee_currency;
    tx
}

#[allow(dead_code)] // todo: change to load into list of base::Transaction
pub(crate) fn convert_ctc_csv_to_coinpanda_csv(input_path: &str, output_path: &str) -> Result<()> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;
    let mut raw_record = csv::StringRecord::new();
    let headers = rdr.headers()?.clone();

    let mut wtr = csv::Writer::from_path(output_path)?;

    while rdr.read_record(&mut raw_record)? {
        let record: CtcTx = raw_record.deserialize(Some(&headers))?;
        wtr.serialize(convert_ctc_to_coinpanda(&record))?;
    }

    Ok(())
}
