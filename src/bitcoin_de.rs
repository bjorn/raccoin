use std::{path::Path, fs::File, io::BufReader};

use anyhow::Result;
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Berlin;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{time::deserialize_date_time, base::{Transaction, Amount, Operation}, TransactionSource};
use linkme::distributed_slice;

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
    #[serde(alias = "Date", alias = "Datum", deserialize_with = "deserialize_date_time")]
    pub date: NaiveDateTime,

    #[serde(rename = "Type", alias = "Booking type", alias = "Typ")]
    pub type_: BitcoinDeActionType,

    #[serde(alias = "Currency", alias = "Währung")]
    pub currency: String,

    #[serde(alias = "Reference", alias = "Referenz")]
    pub reference: String,

    #[serde(rename = "amount after Bitcoin.de-fee", alias = "Menge nach Bitcoin.de-Gebühr")]
    pub amount_after_bitcoin_de_fee: Option<Decimal>,

    #[serde(rename = "unit (amount after Bitcoin.de-fee)", alias = "Einheit (Menge nach Bitcoin.de-Gebühr)")]
    pub unit_amount_after_bitcoin_de_fee: String,

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
pub(crate) fn is_bitcoin_de_csv(path: &Path) -> Result<bool> {
    let file = File::open(path)?;
    let buf_reader = BufReader::new(file);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_reader(buf_reader);

    const KNOWN_FORMATS: [&[&str]; 4] = [
        // Original format
        &["Date", "Type", "Currency", "Reference", "BTC-address", "Price", "unit (rate)", "BTC incl. fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "Incoming / Outgoing", "Account balance"],
        // Original format (extended)
        &["Date", "Type", "Currency", "Reference", "BTC-address", "Price", "unit (rate)", "BTC incl. fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "EUR excl. Bitcoin.de and Fidor fee", "Incoming / Outgoing", "Account balance"],
        // New English format
        &["date", "Booking type", "currency", "reference", "BTC-Address", "Rate", "unit (rate)", "BTC before fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "incoming / outgoing", "balance"],
        // New German format
        &["Datum", "Typ", "Währung", "Referenz", "BTC-Adresse", "Kurs", "Einheit (Kurs)", "BTC vor Gebühr", "Menge vor Gebühr", "Einheit (Menge vor Gebühr)", "BTC nach Bitcoin.de-Gebühr", "Menge nach Bitcoin.de-Gebühr", "Einheit (Menge nach Bitcoin.de-Gebühr)", "Zu- / Abgang", "Kontostand"],
    ];

    let headers = rdr.headers()?;
    Ok(KNOWN_FORMATS.iter().any(|format_headers| headers == *format_headers))
}

// loads a bitcoin.de CSV file into a list of unified transactions
fn load_bitcoin_de_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(_) => continue,  // Skip non-transaction records like Registration
        };
    }

    // reverse the transactions when the last transaction happened before the first
    // (new CSV exports appear to be ordered in reverse)
    if let Some(last) = transactions.last() {
        if last.timestamp < transactions.first().unwrap().timestamp {
            transactions.reverse();
        }
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

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITCOIN_DE_CSV: TransactionSource = TransactionSource {
    id: "BitcoinDeCsv",
    label: "bitcoin.de (CSV)",
    csv: &[],
    detect: Some(is_bitcoin_de_csv),
    load_sync: Some(load_bitcoin_de_csv),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Operation;
    use rust_decimal::Decimal;

    #[test]
    fn parses_all_header_formats() {
        let test_files = [
            ("tests/data/bitcoin_de_original.csv", "original"),
            ("tests/data/bitcoin_de_english.csv", "english"),
            ("tests/data/bitcoin_de_german.csv", "german"),
        ];

        for (path, format_name) in test_files {
            let path = Path::new(path);

            // Verify header detection works
            assert!(is_bitcoin_de_csv(path).unwrap(), "Failed to detect {} format", format_name);

            let transactions = load_bitcoin_de_csv(path).unwrap();
            assert_eq!(transactions.len(), 5, "Wrong transaction count for {} format", format_name); // 6 records - 1 merged fee = 5 (Registration filtered out)

            // Check the purchase trade (transaction 0)
            match &transactions[0].operation {
                Operation::Trade { incoming, outgoing } => {
                    assert_eq!(incoming.currency, "BTC", "Purchase incoming currency wrong in {} format", format_name);
                    assert_eq!(incoming.quantity, Decimal::new(99, 2), "Purchase incoming quantity wrong in {} format", format_name); // 0.99
                    assert_eq!(outgoing.currency, "EUR", "Purchase outgoing currency wrong in {} format", format_name);
                    assert_eq!(outgoing.quantity, Decimal::new(9950, 2), "Purchase outgoing quantity wrong in {} format", format_name); // 99.50
                }
                _ => panic!("Expected Trade operation for purchase in {} format", format_name),
            }

            // Check disbursement with merged fee (transaction 1)
            match &transactions[1].operation {
                Operation::Send(amount) => {
                    assert_eq!(amount.currency, "BTC", "Disbursement currency wrong in {} format", format_name);
                    assert_eq!(amount.quantity, Decimal::new(9899, 4), "Disbursement quantity wrong in {} format", format_name); // 0.9899
                }
                _ => panic!("Expected Send operation for disbursement in {} format", format_name),
            }

            // Verify that disbursement fee was merged
            assert!(transactions[1].fee.is_some(), "Fee not merged for disbursement in {} format", format_name);
            if let Some(fee) = &transactions[1].fee {
                assert_eq!(fee.quantity, Decimal::new(1, 4), "Fee amount wrong in {} format", format_name); // 0.0001
            }

            // Check deposit (transaction 2)
            match &transactions[2].operation {
                Operation::Receive(amount) => {
                    assert_eq!(amount.currency, "BTC", "Deposit currency wrong in {} format", format_name);
                    assert_eq!(amount.quantity, Decimal::new(5, 1), "Deposit quantity wrong in {} format", format_name); // 0.5
                }
                _ => panic!("Expected Receive operation for deposit in {} format", format_name),
            }

            // Check partner programme (transaction 3)
            match &transactions[3].operation {
                Operation::Income(amount) => {
                    assert_eq!(amount.currency, "BTC", "Partner programme currency wrong in {} format", format_name);
                    assert_eq!(amount.quantity, Decimal::new(1, 3), "Partner programme quantity wrong in {} format", format_name); // 0.001
                }
                _ => panic!("Expected Income operation for partner programme in {} format", format_name),
            }

            // Check sale trade (transaction 4)
            match &transactions[4].operation {
                Operation::Trade { incoming, outgoing } => {
                    assert_eq!(incoming.currency, "EUR", "Sale incoming currency wrong in {} format", format_name);
                    assert_eq!(outgoing.currency, "BTC", "Sale outgoing currency wrong in {} format", format_name);
                    assert_eq!(outgoing.quantity, Decimal::new(5, 1), "Sale outgoing quantity wrong in {} format", format_name); // 0.5
                }
                _ => panic!("Expected Trade operation for sale in {} format", format_name),
            }
        }
    }

    #[test]
    fn rejects_non_bitcoin_de_csv() {
        // Test non-Bitcoin.de format
        assert!(!is_bitcoin_de_csv(Path::new("tests/data/bitcoin_core_transactions.csv")).unwrap());
    }
}
