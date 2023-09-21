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
use chrono::{NaiveDateTime, Duration, Datelike};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use cryptotax_ui::*;
use esplora::{blocking_esplora_client, address_transactions};
use fifo::FIFO;
use rust_decimal_macros::dec;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use slice_group_by::GroupByMut;
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

#[derive(Default)]
struct CurrencySummary {
    currency: String,
    balance_start: Decimal,
    balance_end: Decimal,
    quantity_disposed: Decimal,
    cost: Decimal,
    fees: Decimal,
    proceeds: Decimal,
    capital_profit_loss: Decimal,
    income: Decimal,
    total_profit_loss: Decimal,
}

impl CurrencySummary {
    fn new(currency: &str) -> Self {
        Self {
            currency: currency.to_owned(),
            ..Default::default()
        }
    }
}

/// Maps currencies to their CMC ID
/// todo: support more currencies and load from file
fn cmc_id(currency: &str) -> i32 {
    const CMC_ID_MAP: [(&str, i32); 15] = [
        ("BCH", 1831),
        ("BNB", 1839),
        ("BTC", 1),
        ("DASH", 131),
        ("ETH", 1027),
        ("FTC", 8),
        ("LTC", 2),
        ("MANA", 1966),
        ("MIOTA", 1720),
        ("PPC", 5),
        ("XEM", 873),
        ("XLM", 512),
        ("XMR", 328),
        ("XRP", 52),
        ("ZEC", 1437),
    ];
    match CMC_ID_MAP.binary_search_by(|(cur, _)| (*cur).cmp(currency)) {
        Ok(index) => CMC_ID_MAP[index].1,
        Err(_) => -1
    }
}

fn cmc_id_for_amount(amount: &Amount) -> i32 {
    cmc_id(&amount.currency)
}

fn load_transactions() -> Result<(Vec<TransactionSource>, Vec<Transaction>), Box<dyn Error>> {
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

    for (index, tx) in txs.iter().enumerate() {
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
                            if receive_amount.quantity > send_amount.quantity || receive_amount.quantity < send_amount.quantity * dec!(0.95) {
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
            "EUR" => Some(Decimal::ONE),
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

    Ok((sources, txs))
}

fn main() -> Result<(), Box<dyn Error>> {
    let (sources, mut transactions) = load_transactions()?;

    let ui = AppWindow::new()?;
    let mut currencies: Vec<CurrencySummary> = Vec::new();

    fn summary_for<'a>(currencies: &'a mut Vec<CurrencySummary>, currency: &str) -> &'a mut CurrencySummary {
        match currencies.iter().position(|s| s.currency == currency) {
            Some(index) => currencies.get_mut(index).unwrap(),
            None => {
                currencies.push(CurrencySummary::new(currency));
                currencies.last_mut().unwrap()
            }
        }
    }

    // Process transactions per-year
    let mut fifo = FIFO::new();
    let reports: Vec<UiTaxReport> = transactions.linear_group_by_key_mut(|tx| tx.timestamp.year()).map(|txs| {
        // prepare currency summary
        currencies.retain_mut(|summary| {
            summary.balance_start = summary.balance_end;
            summary.quantity_disposed = Decimal::ZERO;
            summary.cost = Decimal::ZERO;
            summary.fees = Decimal::ZERO;
            summary.proceeds = Decimal::ZERO;
            summary.income = Decimal::ZERO;

            summary.balance_start > Decimal::ZERO
        });

        let year = txs.first().unwrap().timestamp.year();
        let gains = fifo.process(txs);

        let mut long_term_capital_gains = Decimal::ZERO;
        let mut short_term_capital_gains = Decimal::ZERO;
        let mut total_capital_losses = Decimal::ZERO;
        let mut ui_gains: Vec<UiCapitalGain> = Vec::new();

        for gain in gains {
            let long_term = (gain.sold - gain.bought) > chrono::Duration::days(365);
            let gain_or_loss = gain.proceeds - gain.cost;

            if gain_or_loss.is_sign_positive() {
                if long_term {
                    long_term_capital_gains += gain_or_loss;
                } else {
                    short_term_capital_gains += gain_or_loss;
                }
            } else {
                total_capital_losses -= gain_or_loss;
            }

            let summary = summary_for(&mut currencies, &gain.amount.currency);
            summary.quantity_disposed += gain.amount.quantity;
            summary.cost += gain.cost;
            // summary.fees = ; // todo: calculate all trade fees relevant for this currency
            summary.proceeds += gain.proceeds;
            // summary.income = ;   // todo: calculate the value of all income transactions for this currency

            ui_gains.push(UiCapitalGain {
                currency_cmc_id: cmc_id_for_amount(&gain.amount),
                currency: gain.amount.currency.into(),
                bought: gain.bought.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
                sold: gain.sold.and_utc().with_timezone(&Europe::Berlin).naive_local().to_string().into(),
                // todo: something else than unwrap()?
                quantity: gain.amount.quantity.try_into().unwrap(),
                cost: gain.cost.try_into().unwrap(),
                proceeds: gain.proceeds.try_into().unwrap(),
                gain_or_loss: gain_or_loss.try_into().unwrap(),
                long_term,
            });
        }

        currencies.iter_mut().for_each(|summary| {
            summary.balance_end = fifo.currency_balance(&summary.currency);
            summary.capital_profit_loss = summary.proceeds - summary.cost - summary.fees;
            summary.total_profit_loss = summary.capital_profit_loss + summary.income;
        });

        currencies.sort_by(|a, b| b.cost.cmp(&a.cost));

        let ui_currencies: Vec<UiCurrencySummary> = currencies.iter().map(|currency| {
            UiCurrencySummary {
                currency_cmc_id: cmc_id(&currency.currency),
                currency: currency.currency.clone().into(),
                balance_start: currency.balance_start.to_string().into(),
                balance_end: currency.balance_end.to_string().into(),
                quantity_disposed: currency.quantity_disposed.to_string().into(),
                cost: format!("{:.2}", currency.cost).into(),
                fees: format!("{:.2}", currency.fees).into(),
                proceeds: format!("{:.2}", currency.proceeds).into(),
                capital_profit_loss: format!("{:.2}", currency.capital_profit_loss).into(),
                income: format!("{:.2}", currency.income).into(),
                total_profit_loss: format!("{:.2}", currency.total_profit_loss).into(),
            }
        }).collect();

        let gain_entries: Vec<ModelRc<StandardListViewItem>> = ui_gains.into_iter().map(|entry| {
            VecModel::from_slice(&[
                StandardListViewItem::from(entry.currency),
                StandardListViewItem::from(entry.bought.to_string().as_str()),
                StandardListViewItem::from(entry.sold),
                StandardListViewItem::from(entry.quantity.to_string().as_str()),
                StandardListViewItem::from(entry.cost.to_string().as_str()),
                StandardListViewItem::from(entry.proceeds.to_string().as_str()),
                StandardListViewItem::from(entry.gain_or_loss.to_string().as_str()),
                StandardListViewItem::from(if entry.long_term { "true" } else { "false" }),
            ])
        }).collect();

        let currencies_model = Rc::new(VecModel::from(ui_currencies));
        let entries_model = Rc::new(VecModel::from(gain_entries));
        let net_capital_gains = short_term_capital_gains + long_term_capital_gains - total_capital_losses;

        UiTaxReport {
            currencies: currencies_model.into(),
            gains: entries_model.into(),
            long_term_capital_gains: format!("{:.2}", long_term_capital_gains).into(),
            short_term_capital_gains: format!("{:.2}", short_term_capital_gains).into(),
            net_capital_gains: format!("{:.2}", net_capital_gains).into(),
            total_capital_losses: format!("{:.2}", total_capital_losses).into(),
            year,
        }
    }).collect();

    let report_years: Vec<StandardListViewItem> = reports.iter().map(|report| StandardListViewItem::from(report.year.to_string().as_str())).collect();
    ui.set_report_years(Rc::new(VecModel::from(report_years)).into());
    ui.set_reports(Rc::new(VecModel::from(reports)).into());

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
            Some(Err(e)) => (Decimal::ZERO, Some(e.to_string())),
            None => (Decimal::ZERO, None),
        };

        ui_transactions.push(UiTransaction {
            from: from.unwrap_or_default(),
            to: to.unwrap_or_default(),
            date: transaction.timestamp.date().to_string().into(),
            time: transaction.timestamp.time().to_string().into(),
            tx_type,
            received_cmc_id: received.map(cmc_id_for_amount).unwrap_or(-1),
            received: received.map_or_else(String::default, Amount::to_string).into(),
            sent_cmc_id: sent.map(cmc_id_for_amount).unwrap_or(-1),
            sent: sent.map_or_else(String::default, Amount::to_string).into(),
            fee: transaction.fee.as_ref().map_or_else(String::default, Amount::to_string).into(),
            value: value.map_or_else(String::default, Amount::to_string).into(),
            gain: gain.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
            gain_error: gain_error.unwrap_or_default().into(),
            description: description.unwrap_or_default().into(),
            tx_hash: tx_hash.map(|s| s.as_str()).unwrap_or_default().to_owned().into(),
        });
    }

    ui.set_transactions(Rc::new(VecModel::from(ui_transactions)).into());

    ui.on_open_transaction(move |tx_hash| {
        let _ = open::that(format!("http://blockchair.com/bitcoin/transaction/{}", tx_hash));
    });

    ui.run()?;

    Ok(())
}
