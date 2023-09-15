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

use base::{Operation, Amount, Transaction};
use chrono_tz::Europe;
use chrono::{NaiveDateTime, Duration};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use cryptotax_ui::*;
use esplora::{blocking_esplora_client, address_transactions};
use fifo::{fifo, save_gains_to_csv};
use serde::{Deserialize, Serialize};
use slint::{VecModel, StandardListViewItem, ModelRc, SharedString};
use std::{error::Error, rc::Rc, path::Path};

#[derive(Serialize, Deserialize)]
enum TransactionsSourceType {
    BitcoinAddress,
    BitcoinCoreCsv,
    BitcoinDeCsv,
    BitonicCsv,     // todo: remove custom format
    CtcImportCsv,
    ElectrumCsv,
    Json,
    MyceliumCsv,
    PeercoinCsv,
    PoloniexDepositsCsv,
    PoloniexTradesCsv,
    PoloniexWithdrawalsCsv,
    TrezorCsv,
}

impl ToString for TransactionsSourceType {
    fn to_string(&self) -> String {
        match self {
            TransactionsSourceType::BitcoinAddress => "Bitcoin Address".to_owned(),
            TransactionsSourceType::BitcoinCoreCsv => "Bitcoin Core (CSV)".to_owned(),
            TransactionsSourceType::BitcoinDeCsv => "bitcoin.de (CSV)".to_owned(),
            TransactionsSourceType::BitonicCsv => "Bitonic (CSV)".to_owned(),
            TransactionsSourceType::ElectrumCsv => "Electrum (CSV)".to_owned(),
            TransactionsSourceType::Json => "JSON".to_owned(),
            TransactionsSourceType::CtcImportCsv => "CryptoTaxCalculator import (CSV)".to_owned(),
            TransactionsSourceType::MyceliumCsv => "Mycelium (CSV)".to_owned(),
            TransactionsSourceType::PeercoinCsv => "Peercoin Qt (CSV)".to_owned(),
            TransactionsSourceType::PoloniexDepositsCsv => "Poloniex Deposits (CSV)".to_owned(),
            TransactionsSourceType::PoloniexTradesCsv => "Poloniex Trades (CSV)".to_owned(),
            TransactionsSourceType::PoloniexWithdrawalsCsv => "Poloniex Withdrawals (CSV)".to_owned(),
            TransactionsSourceType::TrezorCsv => "Trezor (CSV)".to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TransactionSource {
    source_type: TransactionsSourceType,
    path: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    name: String,
    #[serde(skip)]
    transaction_count: usize,
}

fn run() -> Result<(Vec<TransactionSource>, Vec<Transaction>, Vec<UiCapitalGain>), Box<dyn Error>> {
    let sources_file = Path::new("sources.json");
    let sources_path = sources_file.parent().unwrap_or(Path::new(""));
    let mut sources: Vec<TransactionSource> = serde_json::from_str(&std::fs::read_to_string(sources_file)?)?;

    let esplora_client = blocking_esplora_client()?;
    let mut txs = Vec::new();

    for (index, source) in sources.iter_mut().enumerate() {
        let full_path = sources_path.join(&source.path);
        let source_txs = match source.source_type {
            TransactionsSourceType::BitcoinAddress => {
                address_transactions(&esplora_client, &source.path)
            },
            TransactionsSourceType::BitcoinCoreCsv => {
                bitcoin_core::load_bitcoin_core_csv(&full_path)
            },
            TransactionsSourceType::BitcoinDeCsv => {
                bitcoin_de::load_bitcoin_de_csv(&full_path)
            },
            TransactionsSourceType::BitonicCsv => {
                bitonic::load_bitonic_csv(&full_path)
            },
            TransactionsSourceType::ElectrumCsv => {
                electrum::load_electrum_csv(&full_path)
            },
            TransactionsSourceType::Json => {
                base::load_transactions_from_json(&full_path)
            },
            TransactionsSourceType::CtcImportCsv => {
                ctc::load_ctc_csv(&full_path)
            },
            TransactionsSourceType::MyceliumCsv => {
                mycelium::load_mycelium_csv(&full_path)
            },
            TransactionsSourceType::PeercoinCsv => {
                bitcoin_core::load_peercoin_csv(&full_path)
            }
            TransactionsSourceType::PoloniexDepositsCsv => {
                poloniex::load_poloniex_deposits_csv(&full_path)
            },
            TransactionsSourceType::PoloniexTradesCsv => {
                poloniex::load_poloniex_trades_csv(&full_path)
            },
            TransactionsSourceType::PoloniexWithdrawalsCsv => {
                poloniex::load_poloniex_withdrawals_csv(&full_path)
            },
            TransactionsSourceType::TrezorCsv => {
                trezor::load_trezor_csv(&full_path)
            },
        };

        match source_txs {
            Ok(mut source_txs) => {
                for tx in source_txs.iter_mut() {
                    tx.source_index = index;
                }
                source.transaction_count = source_txs.len();
                txs.extend(source_txs);
            },
            Err(e) => println!("Error loading source {}: {}", full_path.display(), e),
        }
    }

    // let poloniex_path = Path::new("archive/poloniex");
    // let poloniex_ctc_csv_file = Path::new("poloniex-for-ctc.csv");
    // convert_poloniex_to_ctc(poloniex_path, poloniex_ctc_csv_file)?;

    // sort transactions by date
    txs.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    let prices = load_btc_price_history_data().unwrap_or_default();

    // before applying FIFO, turn any unmatched Send transactions into Sell transactions
    // and unmatched Receive transactions into Buy transactions
    let mut unmatched_sends_receives = Vec::new();
    let mut matching_pairs = Vec::new();

    for (index, tx) in &mut txs.iter().enumerate() {
        match &tx.operation {
            Operation::Send(_) | Operation::Receive(_) => {
                // try to find a matching transaction, by reverse iterating, but no further than one day ago (for receive) or one hour ago (for send)
                let oldest_match_time = tx.timestamp - if tx.operation.is_send() {
                    Duration::hours(1)
                } else {
                    Duration::days(1)
                };

                let matching_index = unmatched_sends_receives.iter().enumerate().rev().take_while(|(_, tx_index)| -> bool {
                    // the unmatched send may not be too old
                    let tx: &Transaction = &txs[**tx_index];
                    tx.timestamp >= oldest_match_time
                }).find(|(_, tx_index)| {
                    let candidate_tx: &Transaction = &txs[**tx_index];

                    match (&candidate_tx.operation, &tx.operation) {
                        (Operation::Send(send_amount), Operation::Receive(receive_amount)) |
                        (Operation::Receive(receive_amount), Operation::Send(send_amount)) => {
                            // the send and receive transactions must have the same currency
                            if receive_amount.currency != send_amount.currency {
                                return false;
                            }

                            // if both transactions have a tx_hash set, it must be equal
                            if let (Some(candidate_tx_hash), Some(tx_hash)) = (&candidate_tx.tx_hash, &tx.tx_hash) {
                                if candidate_tx_hash != tx_hash {
                                    return false;
                                }
                            }

                            // check whether the price roughly matches (sent amount can't be lower than received amount, but can be 5% higher)
                            if receive_amount.quantity > send_amount.quantity || receive_amount.quantity < send_amount.quantity * 0.95 {
                                return false;
                            }

                            true
                        },
                        _ => false,
                    }
                }).map(|(i, _)| i);

                if let Some(matching_index) = matching_index {
                    // this send is now matched, so remove it from the list of unmatched sends
                    let matching_tx_index = unmatched_sends_receives.remove(matching_index);
                    matching_pairs.push((index, matching_tx_index));
                } else {
                    // no match was found for this transactions, so add it to the unmatched list
                    unmatched_sends_receives.push(index);
                }
            },
            _ => {}
        }
    }

    unmatched_sends_receives.iter().for_each(|unmatched_send| {
        let tx = &mut txs[*unmatched_send];
        match &tx.operation {
            // Turn unmatched Sends into Sells
            Operation::Send(amount) => {
                tx.operation = Operation::Sell(amount.clone());
            },
            // Turn unmatched Receives into Buys
            Operation::Receive(amount) => {
                tx.operation = Operation::Buy(amount.clone());
            },
            _ => unreachable!("only Send and Receive transactions can be unmatched"),
        }
    });

    for (a, b) in matching_pairs {
        (&mut txs[a]).matching_tx = Some(b);
        (&mut txs[b]).matching_tx = Some(a);
    }

    let estimate_value = |timestamp: NaiveDateTime, amount: &Amount| -> Option<Amount> {
        match amount.currency.as_str() {
            "BTC" => estimate_btc_price(timestamp, &prices),
            "EUR" => Some(1.),
            _ => {
                println!("todo: estimate value for {} at {}", amount.currency, timestamp);
                None
            }
        }.map(|price| Amount {
            quantity: price * amount.quantity,
            currency: "EUR".to_owned()
        })
    };

    let estimate_transaction_value = |tx: &mut Transaction| {
        if tx.value.is_none() {
            tx.value = match &tx.operation {
                Operation::Noop => None,
                Operation::Trade { incoming, outgoing } => {
                    if incoming.is_fiat() {
                        Some(incoming.clone())
                    } else if outgoing.is_fiat() {
                        Some(outgoing.clone())
                    } else {
                        let value_incoming = estimate_value(tx.timestamp, incoming);
                        let value_outgoing = estimate_value(tx.timestamp, outgoing);
                        println!("incoming {:?} EUR ({}), outgoing {:?} EUR ({})", value_incoming, incoming, value_outgoing, outgoing);
                        value_incoming.or(value_outgoing)
                    }
                },
                Operation::Buy(amount) |
                Operation::Sell(amount) |
                Operation::FiatDeposit(amount) |
                Operation::FiatWithdrawal(amount) |
                Operation::Fee(amount) |
                Operation::Receive(amount) |
                Operation::Send(amount) |
                Operation::ChainSplit(amount) |
                Operation::Expense(amount) |
                Operation::Income(amount) |
                Operation::Airdrop(amount) |
                Operation::IncomingGift(amount) |
                Operation::OutgoingGift(amount) |
                Operation::Spam(amount) => {
                    estimate_value(tx.timestamp, amount)
                },
            };
        }

        if tx.fee_value.is_none() {
            tx.fee_value = match &tx.fee {
                Some(amount) => estimate_value(tx.timestamp, amount),
                None => None,
            };
        }
    };

    // Estimate the value for all transactions
    txs.iter_mut().for_each(estimate_transaction_value);

    let gains = fifo(&mut txs)?;

    // output gains as CSV
    let filename = format!("gains-{}.csv", 2013);
    save_gains_to_csv(&gains, &filename)?;

    let mut entries: Vec<UiCapitalGain> = Vec::new();

    // add entries from result to ui
    for gain in gains {
        entries.push(UiCapitalGain {
            currency: gain.amount.currency.into(),
            bought: gain.bought.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
            sold: gain.sold.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
            quantity: gain.amount.quantity as f32,
            cost: gain.cost as f32,
            proceeds: gain.proceeds as f32,
            gain_or_loss: (gain.proceeds - gain.cost) as f32,
            long_term: (gain.sold - gain.bought) > chrono::Duration::days(365),
        });
    }

    Ok((sources, txs, entries))
}

fn main() -> Result<(), Box<dyn Error>> {
    let result = run()?;

    let ui = AppWindow::new()?;
    let (sources, transactions, entries) = result;

    let source_types: Vec<SharedString> = vec![
        TransactionsSourceType::BitcoinAddress,
        TransactionsSourceType::BitcoinCoreCsv,
        TransactionsSourceType::BitcoinDeCsv,
        TransactionsSourceType::BitonicCsv,
        TransactionsSourceType::ElectrumCsv,
        TransactionsSourceType::Json,
        TransactionsSourceType::MyceliumCsv,
        TransactionsSourceType::PoloniexDepositsCsv,
        TransactionsSourceType::PoloniexTradesCsv,
        TransactionsSourceType::PoloniexWithdrawalsCsv,
        TransactionsSourceType::TrezorCsv,
    ].iter().map(|s| SharedString::from(s.to_string())).collect();
    ui.set_source_types(Rc::new(VecModel::from(source_types)).into());

    let mut ui_sources = Vec::new();
    for source in &sources {
        ui_sources.push(UiTransactionSource {
            source_type: source.source_type.to_string().into(),
            name: source.name.clone().into(),
            path: source.path.clone().into(),
            transaction_count: source.transaction_count as i32,
        });
    }
    ui.set_sources(Rc::new(VecModel::from(ui_sources)).into());

    let mut ui_transactions = Vec::new();
    for transaction in &transactions {
        let source = sources.get(transaction.source_index);
        let source_name: Option<SharedString> = source.map(|source| source.name.clone().into());

        let mut value = transaction.value.as_ref();
        let mut description = transaction.description.clone();
        let mut tx_hash = transaction.tx_hash.as_ref();

        let (tx_type, sent, received, from, to) = match &transaction.operation {
            Operation::Noop => {
                // ignore Noop transactions
                continue;
            }
            Operation::Buy(amount) => (UiTransactionType::Buy, None, Some(amount), None, source_name),
            Operation::Sell(amount) => (UiTransactionType::Sell, Some(amount), None, source_name, None),
            Operation::Trade { incoming, outgoing } => {
                (UiTransactionType::Trade, Some(outgoing), Some(incoming), source_name.clone(), source_name)
            }
            Operation::FiatDeposit(amount) => {
                (UiTransactionType::Deposit, None, Some(amount), None, source_name)
            }
            Operation::FiatWithdrawal(amount) => {
                (UiTransactionType::Withdrawal, Some(amount), None, source_name, None)
            }
            Operation::Send(send_amount) => {
                // matching_tx has to be set at this point, otherwise it should have been a Sell
                let matching_receive = &transactions[transaction.matching_tx.expect("Send should have matched a Receive transaction")];
                if let Operation::Receive(receive_amount) = &matching_receive.operation {
                    let receive_source = sources.get(matching_receive.source_index);
                    let receive_source_name = receive_source.map(|source| source.name.clone().into());

                    value = value.or(matching_receive.value.as_ref());
                    tx_hash = tx_hash.or(matching_receive.tx_hash.as_ref());
                    description = match (description, &matching_receive.description) {
                        (Some(s), Some(r)) => Some(s + ", " + r),
                        (Some(s), None) => Some(s),
                        (None, Some(r)) => Some(r.to_owned()),
                        (None, None) => None,
                    };

                    (UiTransactionType::Transfer, Some(send_amount), Some(receive_amount), source_name, receive_source_name)
                } else {
                    unreachable!("Send was matched with a non-Receive transaction");
                }
            }
            Operation::Receive(_) => {
                assert!(transaction.matching_tx.is_some(), "Unmatched Receive should have been changed to Buy");
                continue;   // added as a Transfer when handling the Send
            }
            Operation::Fee(amount) => {
                (UiTransactionType::Fee, Some(amount), None, source_name, None)
            }
            Operation::ChainSplit(amount) => {
                (UiTransactionType::ChainSplit, Some(amount), None, source_name, None)
            }
            Operation::Expense(amount) => {
                (UiTransactionType::Expense, Some(amount), None, source_name, None)
            }
            Operation::Income(amount) => {
                (UiTransactionType::Income, None, Some(amount), None, source_name)
            }
            Operation::Airdrop(amount) => {
                (UiTransactionType::Airdrop, None, Some(amount), None, source_name)
            }
            Operation::Spam(amount) => {
                (UiTransactionType::Spam, None, Some(amount), None, source_name)
            }
            _ => todo!("unsupported operation: {:?}", transaction.operation),
        };

        let (gain, gain_error) = match &transaction.gain {
            Some(Ok(gain)) => (*gain, None),
            Some(Err(e)) => (0.0, Some(e.to_string())),
            None => (0.0, None),
        };

        ui_transactions.push(UiTransaction {
            from: from.unwrap_or_default(),
            to: to.unwrap_or_default(),
            date: transaction.timestamp.date().to_string().into(),
            time: transaction.timestamp.time().to_string().into(),
            tx_type,
            received: received.map_or_else(String::default, Amount::to_string).into(),
            sent: sent.map_or_else(String::default, Amount::to_string).into(),
            fee: transaction.fee.as_ref().map_or_else(String::default, Amount::to_string).into(),
            value: value.map_or_else(String::default, Amount::to_string).into(),
            gain: ((gain * 100.0).round() / 100.0) as f32,
            gain_error: gain_error.unwrap_or_default().into(),
            description: description.unwrap_or_default().into(),
            tx_hash: tx_hash.map(|s| s.as_str()).unwrap_or_default().to_owned().into(),
        });
    }

    ui.set_transactions(Rc::new(VecModel::from(ui_transactions)).into());

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

    ui.on_open_transaction(move |tx_hash| {
        let _ = open::that(format!("http://blockchair.com/bitcoin/transaction/{}", tx_hash));
    });

    ui.run()?;

    Ok(())
}
