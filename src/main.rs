slint::include_modules!();

mod base;
mod bitcoin_core;
mod bitcoin_de;
mod bitonic;
mod coinmarketcap;
mod coinpanda;
mod ctc;
mod electrum;
mod esplora;
mod fifo;
mod mycelium;
mod poloniex;
mod time;
mod trezor;

use base::{Operation, Amount};
use bitcoin_core::load_bitcoin_core_csv;
use bitcoin_de::load_bitcoin_de_csv;
use bitonic::load_bitonic_csv;
use chrono::{NaiveDateTime, Duration};
use chrono_tz::Europe;
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use esplora::{blocking_esplora_client, address_transactions};
use fifo::{fifo, save_gains_to_csv};
use std::{error::Error, rc::Rc};
use slint::{Model, VecModel, StandardListViewItem, ModelRc};

use crate::{electrum::load_electrum_csv, base::{save_transactions_to_json, load_transactions_from_json}};

fn run() -> Result<Vec<CapitalGainUi>, Box<dyn Error>> {
    let mut txs = Vec::new();

    let bitcoin_de_csv_file = "bitcoin.de/btc_account_statement_20120831-20230831.csv";
    txs.extend(load_bitcoin_de_csv(bitcoin_de_csv_file)?);

    let bitcoin_core_csv_file = "bitcoin-core-transactions.csv";
    txs.extend(load_bitcoin_core_csv(bitcoin_core_csv_file)?);

    let bitonic_csv_file = "bitonic.csv";
    txs.extend(load_bitonic_csv(bitonic_csv_file)?);

    let electrum_csv_file = "electrum-history.csv";
    txs.extend(load_electrum_csv(electrum_csv_file)?);

    // let poloniex_path = "poloniex";
    // let poloniex_ctc_csv_file = "poloniex-for-ctc.csv";
    // convert_poloniex_to_ctc(poloniex_path, poloniex_ctc_csv_file)?;

    let esplora_client = blocking_esplora_client()?;

    for address in [
        "1APN7z3TjGTr4TZHFnjmXcHc78TopGs48f",
    ] {
        let filename = format!("bitcoin/{}.json", address);
        // save_transactions_to_json(
        //     &address_transactions(&esplora_client, address)?,
        //     &filename)?;
        txs.extend(load_transactions_from_json(&filename)?);
    }

    // sort transactions by date
    txs.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    let prices = load_btc_price_history_data()?;

    // before applying FIFO, turn any unmatched Send transactions into Sell transactions
    // and unmatched Receive transactions into Buy transactions
    let mut unmatched_sends = Vec::new();
    let mut unmatched_receives = Vec::new();
    for (index, tx) in &mut txs.iter().enumerate() {
        match &tx.operation {
            Operation::Send(_) => {
                unmatched_sends.push(index);
            },
            Operation::Receive(amount) => {
                // try to find a matching send transaction, by reverse iterating, but no further than one day ago
                let oldest_send_time = tx.timestamp - Duration::days(1);
                let unmatched_send_pos = unmatched_sends.iter().rposition(|unmatched_send| {
                    let send = txs.get(*unmatched_send).unwrap();

                    // the unmatched send may not be more than one day older than the receive
                    if send.timestamp < oldest_send_time {
                        return false;
                    }

                    if let Operation::Send(send_amount) = &send.operation {
                        // the send and receive transactions must have the same currency
                        if amount.currency != send_amount.currency {
                            return false;
                        }
                        // check whether the price roughly matches (sent amount can't be lower than received amount, but can be 5% higher)
                        if amount.quantity > send_amount.quantity || amount.quantity < send_amount.quantity * 0.95 {
                            return false;
                        }
                    } else {
                        return false;
                    }

                    true
                });

                if let Some(unmatched_send_pos) = unmatched_send_pos {
                    // this send is now matched, so remove it from the list of unmatched sends
                    // println!("send pos {} is send index {}", unmatched_send_pos, unmatched_sends[unmatched_send_pos]);
                    let send_index = unmatched_sends.remove(unmatched_send_pos);
                    println!("matched receive {} with send {}", index, send_index);
                    println!(" send {:?}", txs[send_index]);
                    println!(" receive {:?}", tx);
                } else {
                    // no send was found for this receive, so add it to the list of unmatched receives
                    println!("unmatched receive {}: {:?}", index, tx);
                    unmatched_receives.push(index);
                }
            },
            _ => {}
        }
    }

    // Turn all unmatched Sends into Sells based on an estimated price
    unmatched_sends.iter().for_each(|unmatched_send| {
        let tx = &mut txs[*unmatched_send];
        if let Operation::Send(send_amount) = &mut tx.operation {
            let price = if send_amount.currency == "BTC" {
                estimate_btc_price(tx.timestamp, &prices).unwrap()
            } else {
                println!("can't estimate price for {}", send_amount.currency);
                0.0
            };

            tx.operation = Operation::Sell {
                incoming: Amount {
                    quantity: send_amount.quantity * price,
                    currency: "EUR".into(),
                },
                outgoing: Amount {
                    quantity: send_amount.quantity,
                    currency: send_amount.currency.clone(),
                },
            };
        }
    });

    // Turn all unmatched Receives into Buys based on an estimated price
    unmatched_receives.iter().for_each(|unmatched_receive| {
        let tx = &mut txs[*unmatched_receive];
        if let Operation::Receive(receive_amount) = &mut tx.operation {
            let price = if receive_amount.currency == "BTC" {
                estimate_btc_price(tx.timestamp, &prices).unwrap()
            } else {
                println!("can't estimate price for {}", receive_amount.currency);
                0.0
            };

            tx.operation = Operation::Buy {
                incoming: Amount {
                    quantity: receive_amount.quantity,
                    currency: receive_amount.currency.clone(),
                },
                outgoing: Amount {
                    quantity: receive_amount.quantity * price,
                    currency: "EUR".into(),
                },
            }
        }
    });

    // filter out all transactions before 2020
    txs.retain(|tx| tx.timestamp < NaiveDateTime::parse_from_str("2020-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S").unwrap());

    let gains = fifo(&txs)?;

    // output gains as CSV
    let filename = format!("gains-{}.csv", 2013);
    save_gains_to_csv(&gains, &filename)?;

    let mut entries: Vec<CapitalGainUi> = Vec::new();

    // add entries from result to ui
    for gain in gains {
        entries.push(CapitalGainUi {
            currency: gain.amount.currency.into(),
            bought: gain.bought.timestamp.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
            sold: gain.sold.timestamp.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
            quantity: gain.amount.quantity as f32,
            cost: gain.cost as f32,
            proceeds: gain.proceeds as f32,
            gain_or_loss: (gain.proceeds - gain.cost) as f32,
            long_term: (gain.sold.timestamp - gain.bought.timestamp) > chrono::Duration::days(365),
        });
    }

    // price estimate for testing purposes
    println!("BTC price estimate for 2014-01-01T12:00:00: {}", estimate_btc_price(NaiveDateTime::parse_from_str("2014-01-01T12:00:00", "%Y-%m-%dT%H:%M:%S").unwrap(), &prices).unwrap());

    Ok(entries)
}

fn main() -> Result<(), slint::PlatformError> {
    let result = run();
    if let Err(err) = &result {
        println!("{}", err);
    }

    let ui = AppWindow::new()?;
    let entries = result.unwrap();

    let mut gain_entries: Vec<ModelRc<StandardListViewItem>> = Vec::new();
    for entry in entries {
        gain_entries.push(VecModel::from_slice(&[
            StandardListViewItem::from(entry.currency),
            StandardListViewItem::from(entry.bought.to_string().as_str()),
            StandardListViewItem::from(entry.sold),
            StandardListViewItem::from(entry.quantity.to_string().as_str()),
            StandardListViewItem::from(entry.cost.to_string().as_str()),
            StandardListViewItem::from(entry.proceeds.to_string().as_str()),
            StandardListViewItem::from(entry.gain_or_loss.to_string().as_str()),
            StandardListViewItem::from(if entry.long_term { "true" } else { "false" }),
        ]));
    }

    let entries_model = Rc::new(VecModel::from(gain_entries));
    ui.set_gain_entries(entries_model.into());

    ui.run()
}
