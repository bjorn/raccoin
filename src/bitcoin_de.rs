use std::{path::Path, fs::File, io::BufReader};

use anyhow::Result;
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{time::deserialize_date_time, base::{Transaction, Amount, Operation}};

#[derive(Debug, Deserialize)]
enum BitcoinDeActionType {
    #[serde(alias = "Registrierung")]
    Registration,
    #[serde(alias = "Kauf")]
    Purchase,
    #[serde(alias = "Auszahlung")]
    Disbursement,
    #[serde(alias = "Einzahlung")]
    Deposit,
    #[serde(alias = "Verkauf")]
    Sale,
    #[serde(rename = "Network fee", alias = "Netzwerk-Gebühr")]
    NetworkFee,
    #[serde(rename = "Partner programme", alias = "Partnerprogramm")]
    PartnerProgramme,
}

// struct for storing CSV columns from various Bitcoin.de export formats:
// 
// Original format:
// Date;Type;Currency;Reference;BTC-address;Price;"unit (rate)";"BTC incl. fee";"amount before fee";"unit (amount before fee)";"BTC excl. Bitcoin.de fee";"amount after Bitcoin.de-fee";"unit (amount after Bitcoin.de-fee)";"Incoming / Outgoing";"Account balance"
//
// New English format (2025):
// date;"Booking type";currency;reference;BTC-Address;Rate;"unit (rate)";"BTC before fee";"amount before fee";"unit (amount before fee)";"BTC excl. Bitcoin.de fee";"amount after Bitcoin.de-fee";"unit (amount after Bitcoin.de-fee)";"incoming / outgoing";balance
//
// New German format (2025):
// Datum;Typ;Währung;Referenz;BTC-Adresse;Kurs;"Einheit (Kurs)";"BTC vor Gebühr";"Menge vor Gebühr";"Einheit (Menge vor Gebühr)";"BTC nach Bitcoin.de-Gebühr";"Menge nach Bitcoin.de-Gebühr";"Einheit (Menge nach Bitcoin.de-Gebühr)";"Zu- / Abgang";Kontostand
#[derive(Debug, Deserialize)]
struct BitcoinDeAction {
    // Date field - supports multiple header variations
    #[serde(rename = "Date", alias = "date", alias = "Datum", deserialize_with = "deserialize_date_time")]
    pub date: NaiveDateTime,
    
    // Type field - supports multiple header variations
    #[serde(rename = "Type", alias = "Booking type", alias = "Typ")]
    pub type_: BitcoinDeActionType,
    
    // Currency field - supports multiple header variations
    #[serde(rename = "Currency", alias = "currency", alias = "Währung")]
    pub currency: String,
    
    // Reference field - supports multiple header variations
    #[serde(rename = "Reference", alias = "reference", alias = "Referenz")]
    pub reference: String,
    
    // Amount after fee - supports multiple header variations
    #[serde(rename = "amount after Bitcoin.de-fee", alias = "Menge nach Bitcoin.de-Gebühr")]
    pub amount_after_bitcoin_de_fee: Option<Decimal>,
    
    // Unit for amount after fee - supports multiple header variations
    #[serde(rename = "unit (amount after Bitcoin.de-fee)", alias = "Einheit (Menge nach Bitcoin.de-Gebühr)")]
    pub unit_amount_after_bitcoin_de_fee: String,
    
    // Incoming/Outgoing field - supports multiple header variations
    #[serde(rename = "Incoming / Outgoing", alias = "incoming / outgoing", alias = "Zu- / Abgang")]
    pub incoming_outgoing: Decimal,
}

impl TryFrom<BitcoinDeAction> for Transaction {
    type Error = &'static str;

    // todo: take trading fee into account?
    // todo: translate btc_address?
    fn try_from(item: BitcoinDeAction) -> Result<Self, Self::Error> {
        let utc_time = Berlin.from_local_datetime(&item.date).unwrap().naive_utc();
        let currency = item.currency.clone();
        let mut tx = match item.type_ {
            BitcoinDeActionType::Registration => {
                Err("Registration is not a transaction")
            }
            BitcoinDeActionType::Purchase => {
                Ok(Transaction::trade(
                    utc_time,
                    Amount::new(item.incoming_outgoing, currency),
                    Amount::new(item.amount_after_bitcoin_de_fee.expect("Purchase should have an amount"), item.unit_amount_after_bitcoin_de_fee),
                ))
            }
            BitcoinDeActionType::Disbursement => {
                Ok(Transaction::send(utc_time, Amount::new(-item.incoming_outgoing, currency)))
            }
            BitcoinDeActionType::Deposit => {
                Ok(Transaction::receive(utc_time, Amount::new(item.incoming_outgoing, currency)))
            }
            BitcoinDeActionType::Sale => {
                Ok(Transaction::trade(
                    utc_time,
                    Amount::new(item.amount_after_bitcoin_de_fee.expect("Sale should have an amount"), item.unit_amount_after_bitcoin_de_fee),
                    Amount::new(-item.incoming_outgoing, currency,),
                ))
            }
            BitcoinDeActionType::NetworkFee => {
                Ok(Transaction::fee(utc_time, Amount::new(-item.incoming_outgoing, currency)))
            }
            BitcoinDeActionType::PartnerProgramme => {
                // Partner programme transactions are treated as income (free coins received)
                // This is typically a referral bonus or similar promotional reward
                Ok(Transaction::new(utc_time, Operation::Income(Amount::new(item.incoming_outgoing, currency))))
            }
        }?;
        match item.type_ {
            BitcoinDeActionType::Registration => unreachable!(),
            BitcoinDeActionType::Purchase |
            BitcoinDeActionType::Sale => {
                tx.description = Some(item.reference);
            }
            BitcoinDeActionType::Disbursement |
            BitcoinDeActionType::Deposit |
            BitcoinDeActionType::NetworkFee => {
                tx.tx_hash = Some(item.reference);
                tx.blockchain = Some(item.currency);
            }
            BitcoinDeActionType::PartnerProgramme => {
                tx.description = Some(format!("Partner programme: {}", item.reference));
            }
        };
        Ok(tx)
    }
}

// Detects if a CSV file is a Bitcoin.de export by checking for known header formats
// Supports multiple CSV formats:
// - Original format (Date;Type;Currency;...)
// - New English format (date;"Booking type";currency;...)  
// - New German format (Datum;Typ;Währung;...)
pub(crate) fn is_bitcoin_de_csv(path: &Path) -> Result<bool> {
    let file = File::open(path)?;
    let mut buf_reader = BufReader::new(file);

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_reader(buf_reader);

    if let Ok(headers) = rdr.headers() {
        // Check for original format
        let original_headers: &[&str] = &["Date", "Type", "Currency", "Reference", "BTC-address", "Price", "unit (rate)", "BTC incl. fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "Incoming / Outgoing", "Account balance"];
        if headers == original_headers {
            return Ok(true);
        }

        // Check for new English format
        let english_headers: &[&str] = &["date", "Booking type", "currency", "reference", "BTC-Address", "Rate", "unit (rate)", "BTC before fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "incoming / outgoing", "balance"];
        if headers == english_headers {
            return Ok(true);
        }

        // Check for new German format
        let german_headers: &[&str] = &["Datum", "Typ", "Währung", "Referenz", "BTC-Adresse", "Kurs", "Einheit (Kurs)", "BTC vor Gebühr", "Menge vor Gebühr", "Einheit (Menge vor Gebühr)", "BTC nach Bitcoin.de-Gebühr", "Menge nach Bitcoin.de-Gebühr", "Einheit (Menge nach Bitcoin.de-Gebühr)", "Zu- / Abgang", "Kontostand"];
        if headers == german_headers {
            return Ok(true);
        }
    }

    Ok(false)
}

// loads a bitcoin.de CSV file into a list of unified transactions
// Supports multiple CSV formats:
// - Original format (Date;Type;Currency;...)
// - New English format (date;"Booking type";currency;...)  
// - New German format (Datum;Typ;Währung;...)
pub(crate) fn load_bitcoin_de_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .flexible(true)  // Allow records with varying number of fields
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(_) => continue,  // Skip non-transaction records like Registration
        };
    }

    // bitcoin.de reports disbursement fees separately. Merge them where possible.
    let mut index = 1;
    while index < transactions.len() {
        let (a, b) = transactions.split_at_mut(index);
        let (a, b) = (a.last_mut().unwrap(), &b[0]);

        let fee = match (&a.operation, &b.operation) {
            (Operation::Send(_), Operation::Fee(fee)) if a.fee.is_none() && b.fee.is_none() && a.tx_hash == b.tx_hash => {
                Some(fee.clone())
            }
            _ => None,
        };

        if fee.is_some() {
            a.fee = fee;
            transactions.remove(index);
        }

        index += 1;
    }

    Ok(transactions)
}
