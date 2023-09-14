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
use bitcoin_core::load_bitcoin_core_csv;
use bitcoin_de::load_bitcoin_de_csv;
use bitonic::load_bitonic_csv;
use chrono_tz::Europe;
use chrono::{NaiveDateTime, Duration};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use cryptotax_ui::*;
use esplora::{blocking_esplora_client, address_transactions};
use fifo::{fifo, save_gains_to_csv};
use serde::{Deserialize, Serialize};
use slint::{VecModel, StandardListViewItem, ModelRc, SharedString};
use std::{error::Error, rc::Rc, path::Path};

use crate::{
    electrum::load_electrum_csv,
    base::{
        save_transactions_to_json,
        load_transactions_from_json
    },
    mycelium::load_mycelium_csv,
    trezor::load_trezor_csv,
    poloniex::{
        load_poloniex_deposits_csv,
        load_poloniex_trades_csv,
        load_poloniex_withdrawals_csv
    }
};

#[derive(Serialize, Deserialize)]
enum TransactionsSourceType {
    BitcoinAddress,
    BitcoinCoreCsv,
    BitcoinDeCsv,
    BitonicCsv,     // todo: remove custom format
    ElectrumCsv,
    Json,
    MyceliumCsv,
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
            TransactionsSourceType::MyceliumCsv => "Mycelium (CSV)".to_owned(),
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
                load_bitcoin_core_csv(&full_path)
            },
            TransactionsSourceType::BitcoinDeCsv => {
                load_bitcoin_de_csv(&full_path)
            },
            TransactionsSourceType::BitonicCsv => {
                load_bitonic_csv(&full_path)
            },
            TransactionsSourceType::ElectrumCsv => {
                load_electrum_csv(&full_path)
            },
            TransactionsSourceType::Json => {
                load_transactions_from_json(&full_path)
            },
            TransactionsSourceType::MyceliumCsv => {
                load_mycelium_csv(&full_path)
            },
            TransactionsSourceType::PoloniexDepositsCsv => {
                load_poloniex_deposits_csv(&full_path)
            },
            TransactionsSourceType::PoloniexTradesCsv => {
                load_poloniex_trades_csv(&full_path)
            },
            TransactionsSourceType::PoloniexWithdrawalsCsv => {
                load_poloniex_withdrawals_csv(&full_path)
            },
            TransactionsSourceType::TrezorCsv => {
                load_trezor_csv(&full_path)
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

    // Turn all unmatched Sends into Sells
    unmatched_sends.iter().for_each(|unmatched_send| {
        let tx = &mut txs[*unmatched_send];
        if let Operation::Send(amount) = &tx.operation {
            tx.operation = Operation::Sell(amount.clone());
        }
    });

    // Turn all unmatched Receives into Buys
    unmatched_receives.iter().for_each(|unmatched_receive| {
        let tx = &mut txs[*unmatched_receive];
        if let Operation::Receive(amount) = &tx.operation {
            tx.operation = Operation::Buy(amount.clone());
        }
    });

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
        if tx.value.is_some() {
            return;
        }

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
    for transaction in transactions {
        let (tx_type, sent, received) = match &transaction.operation {
            Operation::Noop => {
                // ignore Noop transactions
                continue;
            }
            Operation::Buy(amount) => (UiTransactionType::Buy, "".to_owned(), amount.to_string()),
            Operation::Sell(amount) => (UiTransactionType::Sell, amount.to_string(), "".to_owned()),
            Operation::Trade { incoming, outgoing } => {
                // Categorize as buy or sell depending on whether fiat is involved
                let tx_type = if outgoing.is_fiat() {
                    UiTransactionType::Buy
                } else if incoming.is_fiat() {
                    UiTransactionType::Sell
                } else {
                    UiTransactionType::Trade
                };
                (tx_type, outgoing.to_string(), incoming.to_string())
            }
            Operation::FiatDeposit(amount) => {
                (UiTransactionType::Deposit, "".to_owned(), amount.to_string())
            }
            Operation::FiatWithdrawal(amount) => {
                (UiTransactionType::Withdrawal, amount.to_string(), "".to_owned())
            }
            Operation::Send(amount) => {
                (UiTransactionType::Send, amount.to_string(), "".to_owned())
            }
            Operation::Receive(amount) => {
                (UiTransactionType::Receive, "".to_owned(), amount.to_string())
            }
            Operation::Fee(amount) => {
                (UiTransactionType::Fee, amount.to_string(), "".to_owned())
            }
            Operation::ChainSplit(amount) => {
                (UiTransactionType::ChainSplit, amount.to_string(), "".to_owned())
            }
            Operation::Expense(amount) => {
                (UiTransactionType::Expense, amount.to_string(), "".to_owned())
            }
            Operation::Income(amount) => {
                (UiTransactionType::Income, "".to_owned(), amount.to_string())
            }
            Operation::Airdrop(amount) => {
                (UiTransactionType::Airdrop, "".to_owned(), amount.to_string())
            }
            Operation::Spam(amount) => {
                (UiTransactionType::Spam, "".to_owned(), amount.to_string())
            }
            _ => todo!("unsupported operation: {:?}", transaction.operation),
        };

        let source = sources.get(transaction.source_index);
        let (gain, gain_error) = match transaction.gain {
            Some(Ok(gain)) => (gain, None),
            Some(Err(e)) => (0.0, Some(e.to_string())),
            None => (0.0, None),
        };

        ui_transactions.push(UiTransaction {
            source: source.and_then(|source| Some(source.name.clone())).unwrap_or_default().into(),
            date: transaction.timestamp.date().to_string().into(),
            time: transaction.timestamp.time().to_string().into(),
            tx_type,
            received: received.into(),
            sent: sent.into(),
            fee: if let Some(fee) = transaction.fee { fee.to_string() } else { "".to_owned() }.into(),
            value: if let Some(value) = transaction.value { value.to_string() } else { "".to_owned() }.into(),
            gain: ((gain * 100.0).round() / 100.0) as f32,
            gain_error: gain_error.unwrap_or_default().into(),
            description: if let Some(description) = transaction.description { description } else { "".to_owned() }.into(),
            tx_hash: if let Some(tx_hash) = transaction.tx_hash { tx_hash } else { "".to_owned() }.into(),
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
