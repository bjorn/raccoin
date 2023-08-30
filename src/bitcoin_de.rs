use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::{ctc::{CtcTx, CtcTxType}, time::deserialize_date_time, base::Transaction};

#[derive(Debug, Deserialize)]
pub(crate) enum BitcoinDeActionType {
    Registration,
    Purchase,
    Disbursement,
    Deposit,
    Sale,
    #[serde(rename = "Network fee")]
    NetworkFee,
}

// struct for storing the following CSV columns:
// Date;Type;Currency;Reference;BTC-address;Price;"unit (rate)";"BTC incl. fee";"amount before fee";"unit (amount before fee)";"BTC excl. Bitcoin.de fee";"amount after Bitcoin.de-fee";"unit (amount after Bitcoin.de-fee)";"Incoming / Outgoing";"Account balance"
#[derive(Debug, Deserialize)]
pub(crate) struct BitcoinDeAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    pub date: NaiveDateTime,
    #[serde(rename = "Type")]
    pub type_: BitcoinDeActionType,
    #[serde(rename = "Currency")]
    pub currency: String,
    #[serde(rename = "Reference")]
    pub reference: String,
    #[serde(rename = "BTC-address")]
    pub btc_address: String,
    // #[serde(rename = "Price")]
    // pub price: Option<f64>,
    // #[serde(rename = "unit (rate)")]
    // pub unit_rate: String,
    // #[serde(rename = "BTC incl. fee")]
    // pub btc_incl_fee: Option<f64>,
    // #[serde(rename = "amount before fee")]
    // pub amount_before_fee: Option<f64>,
    // #[serde(rename = "unit (amount before fee)")]
    // pub unit_amount_before_fee: String,
    // #[serde(rename = "BTC excl. Bitcoin.de fee")]
    // pub btc_excl_bitcoin_de_fee: Option<f64>,
    #[serde(rename = "amount after Bitcoin.de-fee")]
    pub amount_after_bitcoin_de_fee: Option<f64>,
    #[serde(rename = "unit (amount after Bitcoin.de-fee)")]
    pub unit_amount_after_bitcoin_de_fee: String,
    #[serde(rename = "Incoming / Outgoing")]
    pub incoming_outgoing: f64,
    // #[serde(rename = "Account balance")]
    // pub account_balance: f64,
}

impl From<BitcoinDeAction> for Transaction {
    // todo: take trading fee into account?
    // todo: translate btc_address?
    fn from(item: BitcoinDeAction) -> Self {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        let mut tx = match item.type_ {
            BitcoinDeActionType::Registration => Transaction::noop(utc_time),
            BitcoinDeActionType::Purchase => {
                Transaction::buy(
                    utc_time,
                    item.incoming_outgoing,
                    &item.currency,
                    item.amount_after_bitcoin_de_fee.expect("Purchase should have an amount"),
                    &item.unit_amount_after_bitcoin_de_fee)
            },
            BitcoinDeActionType::Disbursement => Transaction::send(utc_time, -item.incoming_outgoing, &item.currency),
            BitcoinDeActionType::Deposit => Transaction::receive(utc_time, item.incoming_outgoing, &item.currency),
            BitcoinDeActionType::Sale => {
                Transaction::sell(
                    utc_time,
                    -item.incoming_outgoing,
                    &item.currency,
                    item.amount_after_bitcoin_de_fee.expect("Sale should have an amount"),
                    &item.unit_amount_after_bitcoin_de_fee)
            },
            BitcoinDeActionType::NetworkFee => Transaction::fee(utc_time, item.incoming_outgoing, &item.currency),
        };
        tx.description = item.reference;
        tx
    }
}

// loads a bitcoin.de CSV file into a list of unified transactions
pub(crate) fn load_bitcoin_de_csv(input_path: &str) -> Result<Vec<Transaction>, Box<dyn std::error::Error>> {
    let mut transactions = Vec::new();

    println!("Loading {}", input_path);

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// converts the bitcoin.de csv file to one for CryptoTaxCalculator
pub(crate) fn convert_bitcoin_de_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        // handle various record type
        match record.type_ {
            BitcoinDeActionType::Registration => {},
            BitcoinDeActionType::Purchase => {
                // When purchasing on Bitcoin.de, the EUR amount is actually sent directly to the seller.
                // To avoid building up a negative EUR balance, we add a fiat deposit.
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::FiatDeposit,
                    &record.unit_amount_after_bitcoin_de_fee,
                    record.amount_after_bitcoin_de_fee.expect("Purchase should have an amount")))?;

                wtr.serialize(CtcTx {
                    id: Some(&record.reference),
                    quote_currency: Some(&record.unit_amount_after_bitcoin_de_fee),
                    quote_amount: record.amount_after_bitcoin_de_fee,
                    // reference_price_per_unit: record.price,
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Buy,
                        &record.currency,
                        record.incoming_outgoing
                    )
                })?;
            },
            BitcoinDeActionType::Disbursement => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Send,
                        &record.currency,
                        -record.incoming_outgoing)
                })?;
            },
            BitcoinDeActionType::Deposit => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Receive,
                        &record.currency,
                        record.incoming_outgoing)
                })?;
            },
            BitcoinDeActionType::Sale => {
                // When selling on Bitcoin.de, the EUR amount is actually sent directly to the buyer.
                // To avoid building up a positive EUR balance, we add a fiat withdrawal.
                wtr.serialize(CtcTx {
                    id: Some(&record.reference),
                    quote_currency: Some(&record.unit_amount_after_bitcoin_de_fee),
                    quote_amount: record.amount_after_bitcoin_de_fee,
                    // reference_price_per_unit: record.price,
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Sell,
                        &record.currency,
                        -record.incoming_outgoing
                    )
                })?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::FiatWithdrawal,
                    &record.unit_amount_after_bitcoin_de_fee,
                    record.amount_after_bitcoin_de_fee.expect("Sale should have an amount")))?;
            },
            BitcoinDeActionType::NetworkFee => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Fee,
                        &record.currency,
                        -record.incoming_outgoing)
                })?;
            },
        }
    }

    Ok(())
}
