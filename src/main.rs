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
use chrono::{NaiveDateTime, Duration, Datelike, Utc};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price, PricePoint};
use cryptotax_ui::*;
use esplora::{blocking_esplora_client, address_transactions};
use fifo::{FIFO, CapitalGain};
use rust_decimal_macros::dec;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use slice_group_by::GroupByMut;
use slint::{VecModel, StandardListViewItem, SharedString};
use strum::{EnumIter, IntoEnumIterator};
use std::{error::Error, rc::Rc, path::{Path, PathBuf}, cell::RefCell};

#[derive(EnumIter, Serialize, Deserialize)]
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
    ReddcoinCoreCsv,
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
            TransactionsSourceType::ReddcoinCoreCsv => "Reddcoin Core (CSV)".to_owned(),
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
    #[serde(default)]
    enabled: bool,
    #[serde(skip)]
    full_path: PathBuf,
    #[serde(skip)]
    transaction_count: usize,
}

#[derive(Default, Clone)]
struct CurrencySummary {
    currency: String,
    balance_start: Decimal,
    balance_end: Decimal,
    cost_start: Decimal,
    cost_end: Decimal,
    quantity_disposed: Decimal,
    quantity_income: Decimal,
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

struct TaxReport {
    year: i32,
    long_term_capital_gains: Decimal,
    short_term_capital_gains: Decimal,
    total_capital_losses: Decimal,
    currencies: Vec<CurrencySummary>,
    gains: Vec<CapitalGain>,
}

impl TaxReport {
    fn net_capital_gains(&self) -> Decimal {
        self.long_term_capital_gains + self.short_term_capital_gains - self.total_capital_losses
    }
}

struct App {
    sources_file: PathBuf,
    sources: Vec<TransactionSource>,
    transactions: Vec<Transaction>,
    reports: Vec<TaxReport>,
    price_history: PriceHistory,
}

impl App {
    fn new() -> Self {
        Self {
            sources_file: PathBuf::new(),
            sources: Vec::new(),
            transactions: Vec::new(),
            reports: Vec::new(),
            price_history: PriceHistory::new(),
        }
    }

    fn load_sources(&mut self, sources_file: &Path) {
        self.sources_file = sources_file.into();
        let sources_path = sources_file.parent().unwrap_or(Path::new(""));
        // todo: report sources file loading error in UI
        self.sources = serde_json::from_str(&std::fs::read_to_string(sources_file).unwrap()).unwrap();
        self.sources.iter_mut().for_each(|source| {
            match source.source_type {
                TransactionsSourceType::BitcoinAddress => {},
                _ => {
                    source.full_path = sources_path.join(&source.path).into();
                }
            }
        });

        self.refresh_transactions();
    }

    fn save_sources(&self) -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string_pretty(&self.sources)?;
        // todo: set all `path` members to relative from `sources_file`
        std::fs::write(&self.sources_file, json)?;
        Ok(())
    }

    fn refresh_transactions(&mut self) {
        self.transactions = load_transactions(&mut self.sources, &self.price_history).unwrap_or_default();
        self.reports = calculate_tax_reports(&mut self.transactions);
    }
}

pub(crate) fn save_summary_to_csv(currencies: &Vec<CurrencySummary>, output_path: &Path) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::Writer::from_path(output_path)?;

    #[derive(Serialize)]
    struct CsvSummary<'a> {
        #[serde(rename = "Currency")]
        currency: &'a str,
        #[serde(rename = "Proceeds")]
        proceeds: Decimal,
        #[serde(rename = "Cost (ex Fees)")]
        cost: Decimal,
        #[serde(rename = "Fees")]
        fees: Decimal,
        #[serde(rename = "Capital Gains")]
        capital_gains: Decimal,
        #[serde(rename = "Other Income")]
        other_income: Decimal,
        #[serde(rename = "Total Gains")]
        total_gains: Decimal,
        #[serde(rename = "Opening Balance")]
        opening_balance: Decimal,
        #[serde(rename = "Quantity Traded")]
        quantity_traded: Decimal,
        #[serde(rename = "Quantity Income")]
        quantity_income: Decimal,
        #[serde(rename = "Closing Balance")]
        closing_balance: Decimal,
    }

    for currency in currencies {
        wtr.serialize(CsvSummary {
            currency: &currency.currency,
            proceeds: currency.proceeds.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            cost: currency.cost.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            fees: currency.fees.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            capital_gains: currency.capital_profit_loss.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            other_income: currency.income.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            total_gains: currency.total_profit_loss.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            opening_balance: currency.balance_start,
            quantity_traded: currency.quantity_disposed,
            quantity_income: currency.income,
            closing_balance: currency.balance_end,
        })?;
    }

    Ok(())
}

/// Maps currencies to their CMC ID
/// todo: support more currencies and load from file
fn cmc_id(currency: &str) -> i32 {
    const CMC_ID_MAP: &[(&str, i32)] = &[
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
        ("RDD", 118),
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

fn load_transactions(sources: &mut Vec<TransactionSource>, price_history: &PriceHistory) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let esplora_client = blocking_esplora_client()?;
    let mut transactions = Vec::new();

    for (index, source) in sources.iter_mut().enumerate() {
        if !source.enabled {
            source.transaction_count = 0;
            continue
        }

        let source_txs = match source.source_type {
            TransactionsSourceType::BitcoinAddress => {
                address_transactions(&esplora_client, &source.path)
            },
            TransactionsSourceType::BitcoinCoreCsv => {
                bitcoin_core::load_bitcoin_core_csv(&source.full_path)
            },
            TransactionsSourceType::BitcoinDeCsv => {
                bitcoin_de::load_bitcoin_de_csv(&source.full_path)
            },
            TransactionsSourceType::BitonicCsv => {
                bitonic::load_bitonic_csv(&source.full_path)
            },
            TransactionsSourceType::ElectrumCsv => {
                electrum::load_electrum_csv(&source.full_path)
            },
            TransactionsSourceType::Json => {
                base::load_transactions_from_json(&source.full_path)
            },
            TransactionsSourceType::CtcImportCsv => {
                ctc::load_ctc_csv(&source.full_path)
            },
            TransactionsSourceType::MyceliumCsv => {
                mycelium::load_mycelium_csv(&source.full_path)
            },
            TransactionsSourceType::PeercoinCsv => {
                bitcoin_core::load_peercoin_csv(&source.full_path)
            }
            TransactionsSourceType::PoloniexDepositsCsv => {
                poloniex::load_poloniex_deposits_csv(&source.full_path)
            },
            TransactionsSourceType::PoloniexTradesCsv => {
                poloniex::load_poloniex_trades_csv(&source.full_path)
            },
            TransactionsSourceType::PoloniexWithdrawalsCsv => {
                poloniex::load_poloniex_withdrawals_csv(&source.full_path)
            },
            TransactionsSourceType::ReddcoinCoreCsv => {
                bitcoin_core::load_reddcoin_core_csv(&source.full_path)
            },
            TransactionsSourceType::TrezorCsv => {
                trezor::load_trezor_csv(&source.full_path)
            },
        };

        match source_txs {
            Ok(mut source_txs) => {
                for tx in source_txs.iter_mut() {
                    tx.source_index = index;
                }
                source.transaction_count = source_txs.len();
                transactions.extend(source_txs);
            },
            // todo: provide this feedback to the UI
            Err(e) => {
                source.transaction_count = 0;
                println!("Error loading source {}: {}", source.full_path.display(), e);
            }
        }
    }

    // sort transactions by date
    transactions.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    match_send_receive(&mut transactions);
    estimate_transaction_values(&mut transactions, price_history);

    Ok(transactions)
}

fn match_send_receive(transactions: &mut Vec<Transaction>) {
    // before applying FIFO, turn any unmatched Send transactions into Sell transactions
    // and unmatched Receive transactions into Buy transactions
    let mut unmatched_sends_receives = Vec::new();
    let mut matching_pairs = Vec::new();

    for (index, tx) in transactions.iter().enumerate() {
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
                    let tx: &Transaction = &transactions[**tx_index];
                    tx.timestamp >= oldest_match_time
                }).find(|(_, tx_index)| {
                    let candidate_tx: &Transaction = &transactions[**tx_index];

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
                    matching_pairs.push(if tx.operation.is_send() { (index, matching_tx_index) } else { (matching_tx_index, index) });
                } else {
                    // no match was found for this transactions, so add it to the unmatched list
                    unmatched_sends_receives.push(index);
                }
            },
            _ => {}
        }
    }

    unmatched_sends_receives.iter().for_each(|unmatched_send| {
        let tx = &mut transactions[*unmatched_send];
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

    for (send_index, receive_index) in matching_pairs {
        (&mut transactions[send_index]).matching_tx = Some(receive_index);
        (&mut transactions[receive_index]).matching_tx = Some(send_index);

        // Derive the fee based on received amount and sent amount
        let adjusted = match (&transactions[send_index].operation, &transactions[receive_index].operation) {
            (Operation::Send(sent), Operation::Receive(received)) if received.quantity < sent.quantity => {
                assert!(sent.currency == received.currency);

                let implied_fee = Amount {
                    quantity: sent.quantity - received.quantity,
                    currency: sent.currency.clone(),
                };
                match &transactions[send_index].fee {
                    Some(existing_fee) => {
                        if existing_fee.currency != implied_fee.currency {
                            println!("warning: send/receive amounts imply fee, but existing fee is set in a different currency for transaction {:?}", transactions[send_index]);
                            None
                        } else if existing_fee.quantity != implied_fee.quantity {
                            println!("warning: replacing existing fee {:?} with implied fee of {:?} and adjusting sent amount to {:?}", existing_fee, implied_fee, received);
                            Some((received.clone(), implied_fee))
                        } else {
                            println!("warning: fee {:?} appears to have been included in the sent amount {:?}, adjusting sent amount to {:?}", existing_fee, sent, received);
                            Some((received.clone(), implied_fee))
                        }
                    },
                    None => {
                        println!("warning: a fee of {:?} appears to have been included in the sent amount {:?}, adjusting sent amount to {:?} and setting fee", implied_fee, sent, received);
                        Some((received.clone(), implied_fee))
                    },
                }
            }
            _ => None,
        };

        if let Some((adjusted_send_amount, adjusted_fee)) = adjusted {
            let tx = &mut transactions[send_index];
            tx.fee = Some(adjusted_fee);
            if let Operation::Send(send_amount) = &mut tx.operation {
                *send_amount = adjusted_send_amount;
            }
        }
    }
}

struct PriceHistory {
    prices: Vec<PricePoint>,
}

impl PriceHistory {
    fn new() -> Self {
        Self {
            prices: load_btc_price_history_data().unwrap_or_default(),
        }
    }

    fn estimate_price(&self, timestamp: NaiveDateTime, currency: &str) -> Option<Decimal> {
        match currency {
            "BTC" => estimate_btc_price(timestamp, &self.prices),
            "EUR" => Some(Decimal::ONE),
            _ => {
                println!("todo: estimate price for {} at {}", currency, timestamp);
                None
            }
        }
    }

    fn estimate_value(&self, timestamp: NaiveDateTime, amount: &Amount) -> Option<Amount> {
        self.estimate_price(timestamp, &amount.currency).map(|price| Amount {
            quantity: price * amount.quantity,
            currency: "EUR".to_owned()
        })
    }
}

fn estimate_transaction_values(transactions: &mut Vec<Transaction>, price_history: &PriceHistory) {
    let estimate_transaction_value = |tx: &mut Transaction| {
        if tx.value.is_none() {
            tx.value = match &tx.operation {
                Operation::Trade { incoming, outgoing } => {
                    if incoming.is_fiat() {
                        Some(incoming.clone())
                    } else if outgoing.is_fiat() {
                        Some(outgoing.clone())
                    } else {
                        let value_incoming = price_history.estimate_value(tx.timestamp, incoming);
                        let value_outgoing = price_history.estimate_value(tx.timestamp, outgoing);
                        match (value_incoming, value_outgoing) {
                            (None, None) => None,
                            (None, Some(value_outgoing)) => Some(value_outgoing),
                            (Some(value_incoming), None) => Some(value_incoming),
                            (Some(value_incoming), Some(value_outgoing)) => {
                                println!("incoming {:?} EUR ({}), outgoing {:?} EUR ({})", value_incoming, incoming, value_outgoing, outgoing);
                                Some(Amount {
                                    quantity: (value_incoming.quantity + value_outgoing.quantity) / Decimal::TWO,
                                    currency: "EUR".to_owned()
                                })
                            }
                        }
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
                Operation::Staking(amount) |
                Operation::IncomingGift(amount) |
                Operation::OutgoingGift(amount) |
                Operation::Spam(amount) => {
                    price_history.estimate_value(tx.timestamp, amount)
                },
            };
        }

        if tx.fee_value.is_none() {
            tx.fee_value = match &tx.fee {
                Some(amount) => price_history.estimate_value(tx.timestamp, amount),
                None => None,
            };
        }
    };

    // Estimate the value for all transactions
    transactions.iter_mut().for_each(estimate_transaction_value);
}

fn calculate_tax_reports(transactions: &mut Vec<Transaction>) -> Vec<TaxReport> {
    let mut currencies = Vec::<CurrencySummary>::new();

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
    transactions.linear_group_by_key_mut(|tx| tx.timestamp.year()).map(|txs| {
        // prepare currency summary
        currencies.retain_mut(|summary| {
            summary.balance_start = summary.balance_end;
            summary.cost_start = summary.cost_end;
            summary.quantity_disposed = Decimal::ZERO;
            summary.quantity_income = Decimal::ZERO;
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

        for gain in &gains {
            let gain_or_loss = gain.profit();

            if gain_or_loss.is_sign_positive() {
                if gain.long_term() {
                    long_term_capital_gains += gain_or_loss;
                } else {
                    short_term_capital_gains += gain_or_loss;
                }
            } else {
                total_capital_losses -= gain_or_loss;
            }

            let summary = summary_for(&mut currencies, &gain.amount.currency);
            summary.quantity_disposed += gain.amount.quantity;
            // summary.quantity_income += // todo: sum up all income quantities
            summary.cost += gain.cost;
            // summary.fees = ; // todo: calculate all trade fees relevant for this currency
            summary.proceeds += gain.proceeds;
            // summary.income = ;   // todo: calculate the value of all income transactions for this currency
        }

        currencies.iter_mut().for_each(|summary| {
            summary.balance_end = fifo.currency_balance(&summary.currency);
            summary.cost_end = fifo.currency_cost_base(&summary.currency);
            summary.capital_profit_loss = summary.proceeds - summary.cost - summary.fees;
            summary.total_profit_loss = summary.capital_profit_loss + summary.income;
        });

        currencies.sort_by(|a, b| b.cost.cmp(&a.cost));

        TaxReport {
            year,
            long_term_capital_gains,
            short_term_capital_gains,
            total_capital_losses,
            currencies: currencies.clone(),
            gains,
        }
    }).collect()
}

fn initialize_ui() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    let facade = ui.global::<Facade>();

    let source_types: Vec<SharedString> = TransactionsSourceType::iter().map(|s| SharedString::from(s.to_string())).collect();
    facade.set_source_types(Rc::new(VecModel::from(source_types)).into());

    facade.on_open_transaction(move |blockchain, tx_hash| {
        let _ = match blockchain.as_str() {
            "BCH" => open::that(format!("https://blockchair.com/bitcoin-cash/transaction/{}", tx_hash)),
            "BTC" => open::that(format!("https://blockchair.com/bitcoin/transaction/{}", tx_hash)),  // or "https://btc.com/tx/{}"
            "ETH" => open::that(format!("https://etherscan.io/tx/{}", tx_hash)),
            "LTC" => open::that(format!("https://blockchair.com/litecoin/transaction/{}", tx_hash)),
            "PPC" => open::that(format!("https://explorer.peercoin.net/tx/{}", tx_hash)),
            "RDD" => open::that(format!("https://rddblockexplorer.com/tx/{}", tx_hash)),
            "ZEC" => open::that(format!("https://blockchair.com/zcash/transaction/{}", tx_hash)),
            _ => Ok(()),
        };
    });

    Ok(ui)
}

fn ui_set_sources(ui: &AppWindow, sources: &Vec<TransactionSource>) {
    let ui_sources: Vec<UiTransactionSource> = sources.iter().map(|source| {
        UiTransactionSource {
            source_type: source.source_type.to_string().into(),
            name: source.name.clone().into(),
            path: source.path.clone().into(),
            enabled: source.enabled,
            transaction_count: source.transaction_count as i32,
        }
    }).collect();
    ui.global::<Facade>().set_sources(Rc::new(VecModel::from(ui_sources)).into());
}

fn ui_set_transactions(ui: &AppWindow, app: &App) {
    let (sources, transactions) = (&app.sources, &app.transactions);
    let mut ui_transactions = Vec::new();
    for transaction in transactions {
        let source = sources.get(transaction.source_index);
        let source_name: Option<SharedString> = source.map(|source| source.name.clone().into());

        let mut value = transaction.value.as_ref();
        let mut description = transaction.description.clone();
        let mut tx_hash = transaction.tx_hash.as_ref();
        let mut blockchain = transaction.blockchain.as_ref();

        let (tx_type, sent, received, from, to) = match &transaction.operation {
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
                    blockchain = blockchain.or(matching_receive.blockchain.as_ref());
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
            Operation::Staking(amount) => {
                (UiTransactionType::Staking, None, Some(amount), None, source_name)
            },
            Operation::IncomingGift(amount) |
            Operation::OutgoingGift(amount) => {
                (UiTransactionType::Gift, None, Some(amount), None, source_name)
            },
            Operation::Spam(amount) => {
                (UiTransactionType::Spam, None, Some(amount), None, source_name)
            }
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
            tx_hash: tx_hash.map(|s| s.to_owned()).unwrap_or_default().into(),
            blockchain: blockchain.map(|s| s.to_owned()).unwrap_or_default().into(),
        });
    }

    ui.global::<Facade>().set_transactions(Rc::new(VecModel::from(ui_transactions)).into());
}

fn ui_set_reports(ui: &AppWindow, app: &App) {
    let facade = ui.global::<Facade>();

    let report_years: Vec<StandardListViewItem> = app.reports.iter().map(|report| StandardListViewItem::from(report.year.to_string().as_str())).collect();
    facade.set_report_years(Rc::new(VecModel::from(report_years)).into());

    let ui_reports: Vec<UiTaxReport> = app.reports.iter().map(|report| {
        let ui_gains: Vec<UiCapitalGain> = report.gains.iter().map(|gain| {
            let bought = gain.bought.and_utc().with_timezone(&Europe::Berlin).naive_local();
            let sold = gain.sold.and_utc().with_timezone(&Europe::Berlin).naive_local();

            UiCapitalGain {
                currency_cmc_id: cmc_id_for_amount(&gain.amount),
                bought_date: bought.date().to_string().into(),
                bought_time: bought.time().to_string().into(),
                sold_date: sold.date().to_string().into(),
                sold_time: sold.time().to_string().into(),
                amount: gain.amount.to_string().into(),
                // todo: something else than unwrap()?
                cost: gain.cost.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                proceeds: gain.proceeds.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                gain_or_loss: gain.profit().round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                long_term: gain.long_term(),
            }
        }).collect();
        let ui_gains = Rc::new(VecModel::from(ui_gains));

        let ui_currencies: Vec<UiCurrencySummary> = report.currencies.iter().map(|currency| {
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
        let ui_currencies = Rc::new(VecModel::from(ui_currencies));

        UiTaxReport {
            currencies: ui_currencies.into(),
            gains: ui_gains.into(),
            long_term_capital_gains: format!("{:.2}", report.long_term_capital_gains).into(),
            short_term_capital_gains: format!("{:.2}", report.short_term_capital_gains).into(),
            net_capital_gains: format!("{:.2}", report.net_capital_gains()).into(),
            total_capital_losses: format!("{:.2}", report.total_capital_losses).into(),
            year: report.year,
        }
    }).collect();

    facade.set_reports(Rc::new(VecModel::from(ui_reports)).into());
}

fn ui_set_portfolio(ui: &AppWindow, app: &App) {
    let facade = ui.global::<Facade>();
    if let Some(report) = app.reports.last() {
        let now = Utc::now().naive_utc();
        let mut balance = Decimal::ZERO;
        let mut cost_base = Decimal::ZERO;

        let mut ui_holdings: Vec<UiCurrencyHoldings> = report.currencies.iter().filter_map(|currency| {
            if currency.balance_end.is_zero() {
                return None
            }

            let current_price = app.price_history.estimate_price(now, &currency.currency);
            let current_value = current_price.map(|price| currency.balance_end * price).unwrap_or(Decimal::ZERO);
            let unrealized_gain = current_value - currency.cost_end;
            let roi = if currency.cost_end > Decimal::ZERO { Some(unrealized_gain / currency.cost_end * Decimal::ONE_HUNDRED) } else { None };

            balance += current_value;
            cost_base += currency.cost_end;

            Some(UiCurrencyHoldings {
                currency_cmc_id: cmc_id(&currency.currency),
                currency: currency.currency.clone().into(),
                quantity: currency.balance_end.normalize().to_string().into(),
                cost: currency.cost_end.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                value: current_value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                roi: roi.map(|roi| format!("{:.2}%", roi)).unwrap_or_else(|| { "-".to_owned() }).into(),
                is_profit: roi.map_or(false, |roi| roi > Decimal::ZERO),
                unrealized_gain: unrealized_gain.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
                percentage_of_portfolio: 0.0,
            })
        }).collect();

        // set the percentage of portfolio for each currency
        if balance > Decimal::ZERO {
            ui_holdings.iter_mut().for_each(|currency| {
                let balance: f32 = balance.try_into().unwrap();
                currency.percentage_of_portfolio = (currency.value / balance) * 100.0;
            });
        }

        facade.set_portfolio(UiPortfolio {
            balance: balance.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
            cost_base: cost_base.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
            unrealized_gains: (balance - cost_base).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero).try_into().unwrap(),
            holdings: Rc::new(VecModel::from(ui_holdings)).into(),
        });
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // todo: use command-line parameter
    let sources_file = Path::new("/home/bjorn/ledger/sources.json");

    let mut app = App::new();
    app.load_sources(sources_file);

    let ui = initialize_ui()?;

    ui_set_sources(&ui, &app.sources);
    ui_set_transactions(&ui, &app);
    ui_set_reports(&ui, &app);
    ui_set_portfolio(&ui, &app);

    let app = Rc::new(RefCell::new(app));
    let facade = ui.global::<Facade>();
    let ui_weak = ui.as_weak();

    let app_for_set_source_enabled = app.clone();
    facade.on_set_source_enabled(move |index, enabled| {
        let mut app = app_for_set_source_enabled.borrow_mut();
        let source = app.sources.get_mut(index as usize);
        if let Some(source) = source {
            source.enabled = enabled;
        }
        app.refresh_transactions();

        // refresh the UI
        let ui = ui_weak.unwrap();
        ui_set_sources(&ui, &app.sources);
        ui_set_transactions(&ui, &app);
        ui_set_reports(&ui, &app);
        ui_set_portfolio(&ui, &app);

        // save the sources file
        match app.save_sources() {
            Ok(_) => { println!("Saved sources to {}", sources_file.display()); },
            Err(_) => { println!("Error saving sources to {}", sources_file.display()); },
        }
    });

    fn save_csv_file(title: &str, starting_file_name: &str) -> Option<PathBuf> {
        let dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(starting_file_name)
            .add_filter("CSV", &["csv"]);
        dialog.save_file()
    }

    let app_for_export_summary = app.clone();
    facade.on_export_summary(move |index| {
        let app = app_for_export_summary.borrow();
        let report = app.reports.get(index as usize).expect("report index should be valid");
        let file_name = format!("report_summary_{}.csv", report.year);

        match save_csv_file("Export Report Summary (CSV)", &file_name) {
            Some(path) => {
                // todo: provide this feedback in the UI
                match save_summary_to_csv(&report.currencies, &path) {
                    Ok(_) => {
                        println!("Saved summary to {}", path.display());
                    },
                    Err(e) => {
                        println!("Error saving summary to {}: {}", path.display(), e);
                    }
                }
            },
            _ => {},
        }
    });

    let app_for_export_capital_gains = app.clone();
    facade.on_export_capital_gains(move |index| {
        let app = app_for_export_capital_gains.borrow();
        let report = app.reports.get(index as usize).expect("report index should be valid");
        let file_name = format!("capital_gains_{}.csv", report.year);

        match save_csv_file("Export Capital Gains (CSV)", &file_name) {
            Some(path) => {
                // todo: provide this feedback in the UI
                match fifo::save_gains_to_csv(&report.gains, &path) {
                    Ok(_) => {
                        println!("Saved gains to {}", path.display());
                    },
                    Err(e) => {
                        println!("Error saving gains to {}: {}", path.display(), e);
                    }
                }
            },
            _ => {},
        }
    });

    ui.run()?;

    Ok(())
}
