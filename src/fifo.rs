use std::{collections::{VecDeque, HashMap}, error::Error};

use chrono::NaiveDateTime;
use chrono_tz::Europe;
use serde::Serialize;

use crate::{base::{Operation, Transaction, Amount, GainError}, time::serialize_date_time};

// Temporary bookkeeping entry for FIFO
#[derive(Debug)]
struct Entry<'a> {
    tx: &'a Transaction,
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
        if holding.tx.timestamp > timestamp {
            return Err(GainError::InvalidTransactionOrder);
        }

        // we can process up to the amount in the holding entry
        let processed_quantity = holding.remaining.min(sold_quantity);
        let cost = processed_quantity * holding.unit_price;
        let proceeds = processed_quantity * sold_unit_price;

        capital_gains.push(CapitalGain {
            bought: holding.tx.timestamp,
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

pub(crate) fn fifo(transactions: &mut Vec<Transaction>) -> Result<Vec<CapitalGain>, Box<dyn Error>> {
    // holdings represented as a map of currency -> deque
    let mut holdings: HashMap<&str, VecDeque<Entry>> = HashMap::new();
    let mut capital_gains: Vec<CapitalGain> = Vec::new();

    for transaction in transactions {
        println!("{:?}", transaction);

        match &transaction.operation {
            Operation::Noop => {},
            Operation::Buy{incoming, outgoing} | Operation::Sell{incoming, outgoing} => {
                if incoming.is_fiat() && outgoing.is_fiat() {
                    return Err("Fiat to fiat trade not supported".into());
                } else if outgoing.is_fiat() {
                    // When we're buying crypto with fiat, we just need to remember
                    // the unit price, which determines the cost base when we later
                    // sell the crypto.
                    let unit_price = outgoing.quantity / incoming.quantity;
                    let currency_holdings = holdings.entry(&incoming.currency).or_default();

                    currency_holdings.push_back(Entry {
                        tx: transaction,
                        unit_price,
                        remaining: incoming.quantity,
                    });

                    println!("  {:} holdings: {:} ({:} entries)", incoming.currency, total_holdings(&currency_holdings), currency_holdings.len());
                } else {
                    let incoming = if incoming.is_fiat() {
                        // When we're selling crypto for fiat, we just calculate
                        // the capital gain on the disposed crypto by comparing
                        // the fiat obtained by the cost base.
                        Amount {
                            quantity: incoming.quantity,
                            currency: incoming.currency.clone(),
                        }
                    } else if let Some(reference_price) = &transaction.reference_price_per_unit {
                        // When we're trading crypto for crypto, it is
                        // technically handled as if we sold one crypto for fiat
                        // and then used fiat to buy another crypto. This means
                        // we need to calculate the capital gain on the disposed
                        // crypto by comparing its current value to its cost
                        // base. To do this, we need to estimate the value of
                        // either the disposed crypto or the crypto we're
                        // buying.
                        //
                        // This should be done before running this function, and
                        // setting the reference_price_per_unit.
                        if !reference_price.is_fiat() {
                            transaction.gain = Some(Err(GainError::InvalidReferencePrice));
                            continue;
                        }
                        Amount {
                            quantity: reference_price.quantity * incoming.quantity,
                            currency: reference_price.currency.clone(),
                        }
                    } else {
                        transaction.gain = Some(Err(GainError::NoReferencePrice));
                        continue;
                    };

                    let tx_gains = gains(&mut holdings, transaction.timestamp, outgoing, incoming.quantity);
                    transaction.gain = Some(match tx_gains {
                        Ok(gains) => {
                            let gain = gains.iter().map(|f| f.proceeds - f.cost).sum();
                            capital_gains.extend(gains);
                            Ok(gain)
                        },
                        Err(e) => Err(e),
                    });
                }
            }
            Operation::FiatDeposit(_) => {},
            Operation::FiatWithdrawal(_) => {},
            Operation::Fee(_) => {
                println!("warning: calculating capital gains on fees not yet implemented!")
            },
            Operation::Receive(_) => {
                // todo: print a warning if this Receive was not matched with a Send
            },
            Operation::Send(_) => {
                // todo: print a warning if this Send was not matched with a Receive
            },
            Operation::ChainSplit(_) => todo!(),
            Operation::Expense(_) => todo!(),
            Operation::Income(_) => todo!(),
            Operation::Airdrop(_) => todo!(),
            Operation::IncomingGift(_) => todo!(),
            Operation::OutgoingGift(_) => todo!(),
            Operation::Spam(_) => todo!(),
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
