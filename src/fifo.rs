use std::{collections::{VecDeque, HashMap}, error::Error};

use crate::base::{Operation, Transaction, Amount};

// Temporary bookkeeping entry for FIFO
#[derive(Debug)]
struct Entry<'a> {
    tx: &'a Transaction,
    unit_price: f64,
    remaining: f64,
}

#[derive(Debug)]
pub(crate) struct CapitalGain<'a> {
    bought: &'a Transaction,
    sold: &'a Transaction,
    amount: Amount,
    cost: f64,
    proceeds: f64,
}

pub(crate) fn fifo(transactions: &Vec<Transaction>) -> Result<Vec<CapitalGain>, Box<dyn Error>> {
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
                let sold_unit_price = incoming.quantity / sold_quantity;

                while let Some(holding) = currency_holdings.front_mut() {
                    if holding.tx.timestamp > transaction.timestamp {
                        panic!("Oldest holding is newer than sale");
                    }

                    // we can process up to the amount in the holding entry
                    let processed_quantity = holding.remaining.min(sold_quantity);
                    let cost = processed_quantity * holding.unit_price;
                    let proceeds = processed_quantity * sold_unit_price;

                    capital_gains.push(CapitalGain {
                        bought: holding.tx,
                        sold: transaction,
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
                println!("  {:} holdings: {:} ({:} entries)", outgoing.currency, total_holdings(&currency_holdings), currency_holdings.len());

                if sold_quantity > 0.0 {
                    panic!("Not enough holdings to sell {:?} BTC", sold_quantity);
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
