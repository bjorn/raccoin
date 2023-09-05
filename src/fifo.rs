use std::{collections::{VecDeque, HashMap}, error::Error};

use chrono::NaiveDateTime;
use chrono_tz::Europe;
use serde::Serialize;

use crate::{base::{Operation, Transaction, Amount}, time::serialize_date_time};

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

pub(crate) fn fifo(transactions: &mut Vec<Transaction>) -> Result<Vec<CapitalGain>, Box<dyn Error>> {
    // holdings represented as a map of currency -> deque
    let mut holdings: HashMap<&str, VecDeque<Entry>> = HashMap::new();
    let mut capital_gains: Vec<CapitalGain> = Vec::new();

    for transaction in transactions {
        println!("{:?}", transaction);

        match &transaction.operation {
            Operation::Noop => {},
            Operation::Buy{incoming, outgoing} => {
                assert!(is_fiat(outgoing));

                let unit_price = outgoing.quantity / incoming.quantity;
                let currency_holdings = holdings.entry(&incoming.currency).or_default();

                currency_holdings.push_back(Entry {
                    tx: transaction,
                    unit_price,
                    remaining: incoming.quantity,
                });

                println!("  {:} holdings: {:} ({:} entries)", incoming.currency, total_holdings(&currency_holdings), currency_holdings.len());
            }
            Operation::Sell{incoming, outgoing} => {
                // todo: find a way to not insert an empty deque?
                let currency_holdings = holdings.entry(&outgoing.currency).or_default();

                // Determine the profit made with this sale based on the oldest holdings
                // and the current price. Consume the holdings in the process.
                let mut sold_quantity = outgoing.quantity;
                let mut tx_gain = 0.0;
                let sold_unit_price = incoming.quantity / sold_quantity;

                while let Some(holding) = currency_holdings.front_mut() {
                    if holding.tx.timestamp > transaction.timestamp {
                        panic!("Oldest holding is newer than sale");
                    }

                    // we can process up to the amount in the holding entry
                    let processed_quantity = holding.remaining.min(sold_quantity);
                    let cost = processed_quantity * holding.unit_price;
                    let proceeds = processed_quantity * sold_unit_price;
                    tx_gain += proceeds - cost;

                    capital_gains.push(CapitalGain {
                        bought: holding.tx.timestamp,
                        sold: transaction.timestamp,
                        amount: Amount {
                            quantity: processed_quantity,
                            currency: outgoing.currency.clone(),    // todo: isn't this duplicated from the bought tx reference?
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

                transaction.gain = tx_gain;

                println!("  {:} holdings: {:} ({:} entries)", outgoing.currency, total_holdings(&currency_holdings), currency_holdings.len());

                if sold_quantity > 0.0 {
                    // Not enough holdings to sell, return error
                    return Err(format!("Not enough holdings to sell {} BTC", sold_quantity).into());
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
            Operation::ChainSplit => todo!(),
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

fn is_fiat(amount: &Amount) -> bool {
    amount.currency == "EUR"
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
