use std::{collections::{VecDeque, HashMap}, error::Error};

use chrono::NaiveDateTime;
use chrono_tz::Europe;
use serde::Serialize;

use crate::{base::{Operation, Transaction, Amount, GainError}, time::serialize_date_time};

// Temporary bookkeeping entry for FIFO
#[derive(Debug)]
struct Entry {
    timestamp: NaiveDateTime,
    unit_price: f64,
    remaining: f64,
}

#[derive(Debug)]
pub(crate) struct CapitalGain {
    pub bought: NaiveDateTime,
    pub sold: NaiveDateTime,
    pub amount: Amount,
    pub cost: f64,
    pub proceeds: f64,
}

/// Determines the capital gains made with this sale based on the oldest
/// holdings and the current price. Consumes the holdings in the process.
fn gains<'a>(holdings: &mut HashMap<&'a str, VecDeque<Entry>>, timestamp: NaiveDateTime, outgoing: &'a Amount, incoming_fiat: f64) -> Result<Vec<CapitalGain>, GainError> {
    // todo: find a way to not insert an empty deque?
    let currency_holdings = holdings.entry(&outgoing.currency).or_default();

    let mut capital_gains: Vec<CapitalGain> = Vec::new();
    let mut sold_quantity = outgoing.quantity;
    let sold_unit_price = incoming_fiat / sold_quantity;

    while let Some(holding) = currency_holdings.front_mut() {
        if holding.timestamp > timestamp {
            return Err(GainError::InvalidTransactionOrder);
        }

        // we can process up to the amount in the holding entry
        let processed_quantity = holding.remaining.min(sold_quantity);
        let cost = processed_quantity * holding.unit_price;
        let proceeds = processed_quantity * sold_unit_price;

        capital_gains.push(CapitalGain {
            bought: holding.timestamp,
            sold: timestamp,
            amount: Amount {
                quantity: processed_quantity,
                currency: outgoing.currency.clone(),
            },
            cost,
            proceeds,
        });

        println!("    {:?}", capital_gains.last().unwrap());

        sold_quantity -= processed_quantity;

        if holding.remaining == processed_quantity {
            // consume the holding and keep processing the remaining quantity
            currency_holdings.pop_front();
        } else {
            // we finished processing the sale
            holding.remaining -= processed_quantity;
            break;
        }
    }

    println!("  {:} holdings: {:} ({:} entries)", outgoing.currency, total_holdings(&currency_holdings), currency_holdings.len());

    if sold_quantity > 0.0 {
        return Err(GainError::InsufficientBalance);
    }

    Ok(capital_gains)
}

fn fiat_value(amount: &Option<Amount>) -> Result<f64, GainError> {
    match amount {
        Some(amount) => {
            if amount.is_fiat() {
                Ok(amount.quantity)
            } else {
                Err(GainError::InvalidTransactionValue)
            }
        },
        None => Err(GainError::MissingTransactionValue),
    }
}

fn add_holdings<'a>(holdings: &mut HashMap<&'a str, VecDeque<Entry>>, tx: &Transaction, amount: &'a Amount, value: &Option<Amount>) -> Result<f64, GainError> {
    holdings.entry(&amount.currency).or_default().push_back(Entry {
        timestamp: tx.timestamp,
        unit_price: fiat_value(value)? / amount.quantity,
        remaining: amount.quantity,
    });
    Ok(0.0)
}

fn dispose_holdings<'a>(holdings: &mut HashMap<&'a str, VecDeque<Entry>>, capital_gains: &mut Vec<CapitalGain>, timestamp: NaiveDateTime, outgoing: &'a Amount, value: &Option<Amount>) -> Result<f64, GainError> {
    let tx_gains = gains(holdings, timestamp, outgoing, fiat_value(value)?);
    match tx_gains {
        Ok(gains) => {
            let gain: f64 = gains.iter().map(|f| f.proceeds - f.cost).sum();
            capital_gains.extend(gains);
            Ok(gain)
        },
        Err(e) => Err(e),
    }
}

pub(crate) fn fifo(transactions: &mut Vec<Transaction>) -> Result<Vec<CapitalGain>, Box<dyn Error>> {
    // holdings represented as a map of currency -> deque
    let mut holdings: HashMap<&str, VecDeque<Entry>> = HashMap::new();
    let mut capital_gains: Vec<CapitalGain> = Vec::new();

    for transaction in transactions {
        println!("{:?}", transaction);

        match &transaction.operation {
            Operation::IncomingGift(amount) |
            Operation::Airdrop(amount) |
            Operation::Buy(amount) |
            Operation::Income(amount) => {
                transaction.gain = Some(add_holdings(&mut holdings, transaction, amount, &transaction.value));
            },
            Operation::Trade{incoming, outgoing} => {
                // When we're trading crypto for crypto, it is
                // technically handled as if we sold one crypto for fiat
                // and then used fiat to buy another crypto.
                if !outgoing.is_fiat() {
                    transaction.gain = Some(dispose_holdings(&mut holdings, &mut capital_gains, transaction.timestamp, outgoing, &transaction.value));
                }

                if !incoming.is_fiat() {
                    let result = add_holdings(&mut holdings, transaction, incoming, &transaction.value);
                    if result.is_err() && transaction.gain.is_none() {
                        transaction.gain = Some(result);
                    }
                }
            }
            Operation::Fee(amount) |
            Operation::Expense(amount) |
            Operation::Sell(amount) |
            Operation::OutgoingGift(amount) => {
                transaction.gain = Some(dispose_holdings(&mut holdings, &mut capital_gains, transaction.timestamp, amount, &transaction.value));
            },
            Operation::Noop |
            Operation::FiatDeposit(_) |
            Operation::FiatWithdrawal(_) |
            Operation::Receive(_) |
            Operation::Send(_) |
            Operation::Spam(_) => {
                // Non-taxable events
            },
            Operation::ChainSplit(amount) => {
                // Chain split is special in that it adds holdings with 0 cost base
                transaction.gain = Some(add_holdings(&mut holdings, transaction, amount, &Some(Amount { quantity: 0.0, currency: "EUR".to_owned() })));
            },
        }
    }

    Ok(capital_gains)
}

fn total_holdings(holdings: &VecDeque<Entry>) -> f64 {
    let mut total_amount = 0.0;
    for h in holdings {
        total_amount += h.remaining;
    }
    total_amount
}

pub(crate) fn save_gains_to_csv(gains: &Vec<CapitalGain>, output_path: &str) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::Writer::from_path(output_path)?;

    #[derive(Serialize)]
    struct CsvGain<'a> {
        #[serde(rename = "Currency")]
        currency: &'a str,
        #[serde(rename = "Bought", serialize_with = "serialize_date_time")]
        bought: NaiveDateTime,
        #[serde(rename = "Sold", serialize_with = "serialize_date_time")]
        sold: NaiveDateTime,
        #[serde(rename = "Quantity")]
        quantity: f64,
        #[serde(rename = "Cost")]
        cost: f64,
        #[serde(rename = "Proceeds")]
        proceeds: f64,
        #[serde(rename = "Gain or Loss")]
        gain_or_loss: f64,
        #[serde(rename = "Long Term")]
        long_term: bool,
    }

    for gain in gains {
        wtr.serialize(CsvGain {
            currency: &gain.amount.currency,
            bought: gain.bought.and_utc().with_timezone(&Europe::Berlin).naive_local(),
            sold: gain.sold.and_utc().with_timezone(&Europe::Berlin).naive_local(),
            quantity: gain.amount.quantity,
            cost: gain.cost,
            proceeds: gain.proceeds,
            gain_or_loss: gain.proceeds - gain.cost,
            long_term: (gain.sold - gain.bought) > chrono::Duration::days(365),
        })?;
    }

    Ok(())
}
