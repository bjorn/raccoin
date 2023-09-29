use std::path::Path;

use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{time::deserialize_date_time, base::{Transaction, Amount}};

#[derive(Debug, Deserialize)]
enum BitcoinDeActionType {
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
struct BitcoinDeAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    pub date: NaiveDateTime,
    #[serde(rename = "Type")]
    pub type_: BitcoinDeActionType,
    #[serde(rename = "Currency")]
    pub currency: String,
    #[serde(rename = "Reference")]
    pub reference: String,
    // #[serde(rename = "BTC-address")]
    // pub btc_address: String,
    // #[serde(rename = "Price")]
    // pub price: Option<Decimal>,
    // #[serde(rename = "unit (rate)")]
    // pub unit_rate: String,
    // #[serde(rename = "BTC incl. fee")]
    // pub btc_incl_fee: Option<Decimal>,
    // #[serde(rename = "amount before fee")]
    // pub amount_before_fee: Option<Decimal>,
    // #[serde(rename = "unit (amount before fee)")]
    // pub unit_amount_before_fee: String,
    // #[serde(rename = "BTC excl. Bitcoin.de fee")]
    // pub btc_excl_bitcoin_de_fee: Option<Decimal>,
    #[serde(rename = "amount after Bitcoin.de-fee")]
    pub amount_after_bitcoin_de_fee: Option<Decimal>,
    #[serde(rename = "unit (amount after Bitcoin.de-fee)")]
    pub unit_amount_after_bitcoin_de_fee: String,
    #[serde(rename = "Incoming / Outgoing")]
    pub incoming_outgoing: Decimal,
    // #[serde(rename = "Account balance")]
    // pub account_balance: Decimal,
}

impl TryFrom<BitcoinDeAction> for Transaction {
    type Error = &'static str;

    // todo: take trading fee into account?
    // todo: translate btc_address?
    fn try_from(item: BitcoinDeAction) -> Result<Self, Self::Error> {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        let currency = item.currency.clone();
        let mut tx = match item.type_ {
            BitcoinDeActionType::Registration => Err("Registration is not a transaction"),
            BitcoinDeActionType::Purchase => {
                Ok(Transaction::trade(
                    utc_time,
                    Amount {
                        quantity: item.incoming_outgoing,
                        currency: currency,
                    },
                    Amount {
                        quantity: item.amount_after_bitcoin_de_fee.expect("Purchase should have an amount"),
                        currency: item.unit_amount_after_bitcoin_de_fee
                    },
                ))
            },
            BitcoinDeActionType::Disbursement => Ok(Transaction::send(utc_time, Amount::new(-item.incoming_outgoing, currency))),
            BitcoinDeActionType::Deposit => Ok(Transaction::receive(utc_time, Amount::new(item.incoming_outgoing, currency))),
            BitcoinDeActionType::Sale => {
                Ok(Transaction::trade(
                    utc_time,
                    Amount {
                        quantity: item.amount_after_bitcoin_de_fee.expect("Sale should have an amount"),
                        currency: item.unit_amount_after_bitcoin_de_fee
                    },
                    Amount {
                        quantity: -item.incoming_outgoing,
                        currency: currency,
                    },
                ))
            },
            BitcoinDeActionType::NetworkFee => Ok(Transaction::fee(utc_time, Amount::new(-item.incoming_outgoing, currency))),
        }?;
        match item.type_ {
            BitcoinDeActionType::Registration => unreachable!(),
            BitcoinDeActionType::Purchase |
            BitcoinDeActionType::Sale => {
                tx.description = Some(item.reference);
            },
            BitcoinDeActionType::Disbursement |
            BitcoinDeActionType::Deposit |
            BitcoinDeActionType::NetworkFee => {
                tx.tx_hash = Some(item.reference);
                tx.blockchain = Some(item.currency);
            },
        };
        Ok(tx)
    }
}

// loads a bitcoin.de CSV file into a list of unified transactions
pub(crate) fn load_bitcoin_de_csv(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn std::error::Error>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(_) => continue,
        };
    }

    Ok(transactions)
}
