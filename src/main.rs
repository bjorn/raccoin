#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alby;
mod alby_hub;
mod base;
mod binance;
mod bison;
mod blink;
mod bitcoin_core;
mod bitcoin_de;
mod bitonic;
mod bitstamp;
mod bittrex;
mod coinmarketcap;
mod coinpanda;
mod ctc;
mod electrum;
mod esplora;
mod etherscan;
mod fifo;
mod ftx;
mod horizon;
mod kraken;
mod liquid;
mod mycelium;
mod phoenix;
mod poloniex;
mod price_history;
mod time;
mod trezor;
mod wallet_of_satoshi;
mod wave_space;

use anyhow::{anyhow, Context, Result};
use coinmarketcap::CmcInterval;
use base::{cmc_id, Amount, Operation, Transaction};
use chrono::{Datelike, Duration, Local, TimeZone, Utc};
use directories::ProjectDirs;
use fifo::{CapitalGain, CostBasisTracking, FIFO};
use raccoin_ui::*;
use price_history::{PriceHistory, PriceRequirements, split_ranges};
use regex::{Regex, RegexBuilder};
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use slice_group_by::GroupByMut;
use slint::{Model, ModelRc, SharedString, StandardListViewItem, VecModel};
use linkme::distributed_slice;
use std::{
    cell::RefCell,
    cmp::{Eq, Ordering},
    collections::HashMap,
    default::Default,
    env,
    ffi::{OsStr, OsString},
    fs::File,
    future::Future,
    hash::Hash,
    io::BufReader,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
};

fn rounded_to_cent(amount: Decimal) -> Decimal {
    amount.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

pub(crate) type LoadFuture = Pin<Box<dyn Future<Output = Result<Vec<Transaction>>> + Send>>;

pub(crate) struct CsvSpec {
    pub(crate) headers: &'static [&'static str],
    pub(crate) delimiters: &'static [u8],
    pub(crate) skip_lines: usize,
    pub(crate) trim: csv::Trim,
}

impl CsvSpec {
    pub(crate) const fn new(headers: &'static [&'static str]) -> Self {
        Self {
            headers,
            delimiters: &[b','],
            skip_lines: 0,
            trim: csv::Trim::None,
        }
    }
}

pub(crate) struct TransactionSource {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) csv: &'static [CsvSpec],
    pub(crate) detect: Option<fn(&Path) -> Result<bool>>,
    pub(crate) load_sync: Option<fn(&Path) -> Result<Vec<Transaction>>>,
    pub(crate) load_async: Option<fn(String) -> LoadFuture>,
}

impl TransactionSource {
    pub(crate) fn detect_from_file(&self, path: &Path) -> Result<bool> {
        if let Some(detect) = self.detect {
            return detect(path);
        }

        for csv in self.csv {
            if csv_matches(path, csv)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(crate) fn can_sync(&self) -> bool {
        self.load_async.is_some()
    }
}

pub(crate) fn csv_matches(path: &Path, csv: &CsvSpec) -> Result<bool> {
    for &delimiter in csv.delimiters {
        let file = File::open(path)?;
        let mut buf_reader = BufReader::new(file);

        use std::io::BufRead;

        let mut line = String::new();
        for _ in 0..csv.skip_lines {
            buf_reader.read_line(&mut line)?;
        }

        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .trim(csv.trim)
            .from_reader(buf_reader);

        if rdr
            .headers()
            .map_or(false, |s| s == csv.headers)
        {
            return Ok(true);
        }
    }

    Ok(false)
}

#[distributed_slice]
pub(crate) static TRANSACTION_SOURCES: [TransactionSource];

fn transaction_source_by_id(id: &str) -> Option<&'static TransactionSource> {
    TRANSACTION_SOURCES
        .iter()
        .find(|source| source.id == id)
}

fn detect_source_from_file(path: &Path) -> Option<&'static TransactionSource> {
    TRANSACTION_SOURCES
        .iter()
        .find(|source| source.detect_from_file(path).ok().unwrap_or(false))
}

#[derive(Serialize, Deserialize)]
struct WalletSource {
    source_type: String,
    path: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    name: String,
    /// Whether this source is enabled.
    #[serde(default)]
    enabled: bool,
    /// The resolved path of the source.
    #[serde(skip)]
    full_path: PathBuf,
    /// The number of transactions loaded from this source.
    #[serde(skip)]
    transaction_count: usize,
    /// Transactions from this source. Only used for on-demand synchronized sources.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    transactions: Vec<Transaction>,
}

#[derive(Serialize, Deserialize)]
struct Wallet {
    name: String,
    /// Whether this wallet is enabled.
    #[serde(default)]
    enabled: bool,
    /// Whether this wallet is expanded.
    #[serde(skip)]
    expanded: bool,
    sources: Vec<WalletSource>,
    /// Currency balances, as calculated based on the transactions imported from this source.
    #[serde(skip)]
    balances: HashMap<String, Decimal>,
}

impl Wallet {
    fn new(name: String) -> Self {
        Self {
            name,
            enabled: true,
            expanded: true, // Show new wallets expanded by default
            sources: Vec::new(),
            balances: HashMap::new(),
        }
    }

    fn transaction_count(&self) -> usize {
        self.sources.iter().map(|source| source.transaction_count).sum()
    }
}

#[derive(Serialize, Deserialize, Default)]
struct AppState {
    portfolio_file: Option<PathBuf>,
    last_source_directory: Option<PathBuf>,
    last_export_directory: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Portfolio {
    #[serde(default)]
    wallets: Vec<Wallet>,
    #[serde(default)]
    ignored_currencies: Vec<String>,
    #[serde(default)]
    merge_consecutive_trades: bool,
    #[serde(default)]
    cost_basis_tracking: CostBasisTracking,
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

    fn cmp(&self, other: &Self) -> Ordering {
        match other.cost.cmp(&self.cost) {
            Ordering::Equal => self.currency.cmp(&other.currency),
            cost_ordering => cost_ordering,
        }
    }
}

struct TaxReport {
    year: i32,
    short_term_cost: Decimal,
    short_term_proceeds: Decimal,
    short_term_capital_gains: Decimal,
    short_term_capital_losses: Decimal,
    long_term_capital_gains: Decimal,
    long_term_capital_losses: Decimal,
    currencies: Vec<CurrencySummary>,
    gains: Vec<CapitalGain>,
}

impl TaxReport {
    fn short_term_net_capital_gains(&self) -> Decimal {
        self.short_term_capital_gains - self.short_term_capital_losses
    }

    fn long_term_net_capital_gains(&self) -> Decimal {
        self.long_term_capital_gains - self.long_term_capital_losses
    }

    fn total_capital_gains(&self) -> Decimal {
        self.short_term_capital_gains + self.long_term_capital_gains
    }

    fn total_capital_losses(&self) -> Decimal {
        self.short_term_capital_losses + self.long_term_capital_losses
    }

    fn total_net_capital_gains(&self) -> Decimal {
        self.total_capital_gains() - self.total_capital_losses()
    }
}

enum TransactionFilter {
    WalletIndex(usize),
    Currency(String),
    Text(Regex),
    HasGainError,
}

impl TransactionFilter {
    fn matches(&self, tx: &Transaction) -> bool {
        match self {
            TransactionFilter::WalletIndex(index) => tx.wallet_index == *index,
            TransactionFilter::Currency(currency) => match tx.incoming_outgoing() {
                (None, None) => false,
                (None, Some(amount)) |
                (Some(amount), None) => {
                    amount.effective_currency().as_ref() == currency
                }
                (Some(incoming), Some(outgoing)) => {
                    incoming.effective_currency().as_ref() == currency ||
                    outgoing.effective_currency().as_ref() == currency
                }
            }
            TransactionFilter::Text(text) => {
                tx.description.as_deref().is_some_and(|description| text.is_match(description)) ||
                    tx.tx_hash.as_deref().is_some_and(|tx_hash| text.is_match(tx_hash))
            }
            TransactionFilter::HasGainError => {
                tx.gain.as_ref().is_some_and(|gain| gain.is_err())
            }
        }
    }
}

struct App {
    project_dirs: Option<ProjectDirs>,
    state: AppState,
    portfolio: Portfolio,
    transactions: Vec<Transaction>,
    reports: Vec<TaxReport>,
    price_history: PriceHistory,
    stop_update_price_history: bool,

    transaction_filters: Vec<TransactionFilter>,

    ui_weak: slint::Weak<AppWindow>,
    ui_wallets: Rc<VecModel<UiWallet>>,
    ui_transactions: Rc<VecModel<UiTransaction>>,
    ui_report_years: Rc<VecModel<StandardListViewItem>>,
    ui_reports: Rc<VecModel<UiTaxReport>>,
}

impl App {
    fn new() -> Self {
        let project_dirs = ProjectDirs::from("org", "raccoin",  "Raccoin");

        // Try to restore application state
        let state = project_dirs.as_ref().and_then(|dirs| {
            let config_dir = dirs.config_local_dir();
            std::fs::create_dir_all(config_dir).map(|_| config_dir.join("state.json")).ok()
        }).and_then(|state_file| std::fs::read_to_string(state_file).ok()).and_then(|json| {
            serde_json::from_str::<AppState>(&json).ok()
        }).unwrap_or_default();

        // Try to load available price history
        let price_history = project_dirs.as_ref().map(|dirs| {
            let data_dir = dirs.data_local_dir();
            PriceHistory::load_from_dir(&data_dir).unwrap_or_default()
        }).unwrap_or_default();

        Self {
            project_dirs,
            state,
            portfolio: Portfolio::default(),
            transactions: Vec::new(),
            reports: Vec::new(),
            price_history,
            stop_update_price_history: false,

            transaction_filters: Vec::default(),

            ui_weak: slint::Weak::default(),
            ui_wallets: Rc::new(Default::default()),
            ui_transactions: Rc::new(Default::default()),
            ui_report_years: Rc::new(Default::default()),
            ui_reports: Rc::new(Default::default()),
        }
    }

    fn load_portfolio(&mut self, file_path: &Path) -> Result<()> {
        // todo: report portfolio loading error in UI
        let mut portfolio: Portfolio = serde_json::from_str(&std::fs::read_to_string(file_path)?)?;
        let portfolio_path = file_path.parent().unwrap_or(Path::new(""));
        portfolio.wallets.iter_mut().for_each(|w| w.sources.iter_mut().for_each(|source| {
            let source_definition = transaction_source_by_id(&source.source_type);
            let is_virtual = source_definition.map(|definition| definition.load_sync.is_none()).unwrap_or(false);
            if !is_virtual {
                source.full_path = portfolio_path.join(&source.path);
            }
        }));

        self.state.portfolio_file = Some(file_path.into());
        self.portfolio = portfolio;

        self.refresh_transactions();
        Ok(())
    }

    fn save_portfolio(&mut self, portfolio_file: Option<PathBuf>) {
        fn internal_save(portfolio: &Portfolio, portfolio_file: &Path) -> Result<()> {
            let json = serde_json::to_string_pretty(&portfolio)?;
            std::fs::write(portfolio_file, json)?;
            Ok(())
        }

        if let Some(path) = portfolio_file.as_ref().or(self.state.portfolio_file.as_ref()) {
            let portfolio_path = path.parent().unwrap_or(Path::new(""));
            self.portfolio.wallets.iter_mut().for_each(|w| w.sources.iter_mut().for_each(|source| {
                let source_definition = transaction_source_by_id(&source.source_type);
                let is_virtual = source_definition.map(|definition| definition.load_sync.is_none()).unwrap_or(false);
                if !is_virtual {
                    if let Some(relative_path) = pathdiff::diff_paths(&source.full_path, portfolio_path) {
                        source.path = relative_path.to_str().unwrap_or_default().to_owned();
                    }
                }
            }));

            match internal_save(&self.portfolio, path) {
                Ok(_) => {
                    println!("Saved portfolio to {}", path.display());
                    if portfolio_file.is_some() {
                        self.state.portfolio_file = portfolio_file;
                    }
                }
                Err(_) => {
                    println!("Error saving portfolio to {}", path.display());
                }
            }
        }
    }

    fn close_portfolio(&mut self) {
        self.portfolio = Portfolio::default();
        self.state.portfolio_file = None;
        self.refresh_transactions();
    }

    fn refresh_transactions(&mut self) {
        self.transactions = load_transactions(&mut self.portfolio).unwrap_or_default();
        estimate_transaction_values(&mut self.transactions, &self.price_history);
        self.reports = calculate_tax_reports(&mut self.transactions, self.portfolio.cost_basis_tracking);
    }

    fn ui(&self) -> AppWindow {
        self.ui_weak.unwrap()
    }

    fn push_notification(&self, notification_type: UiNotificationType, message: &str) {
        let notifications_rc = self.ui().global::<Facade>().get_notifications();
        let notifications = slint::Model::as_any(&notifications_rc).downcast_ref::<VecModel<UiNotification>>().unwrap();
        notifications.push(UiNotification {
            notification_type,
            message: message.into(),
        });
        if notifications.row_count() > 10 {
            notifications.remove(0);
        }
    }

    fn report_info(&self, message: &str) {
        self.push_notification(UiNotificationType::Info, message);
    }

    fn report_warning(&self, message: &str) {
        self.push_notification(UiNotificationType::Warning, message);
    }

    fn report_error(&self, message: &str) {
        self.push_notification(UiNotificationType::Error, message);
    }

    fn remove_notification(&self, index: usize) {
        let notifications_rc = self.ui().global::<Facade>().get_notifications();
        let notifications = slint::Model::as_any(&notifications_rc).downcast_ref::<VecModel<UiNotification>>().unwrap();
        notifications.remove(index);
    }

    fn refresh_ui(&self) {
        ui_set_wallets(self);
        ui_set_transactions(self);
        ui_set_reports(self);
        ui_set_portfolio(self);
    }

    fn save_state(&self) -> Result<()> {
        let state_file = self.project_dirs.as_ref().map(|dirs| {
            dirs.config_local_dir().join("state.json")
        }).context("Missing project directories")?;

        std::fs::write(state_file, serde_json::to_string_pretty(&self.state)?)?;
        Ok(())
    }
}

pub(crate) fn save_summary_to_csv(report: &TaxReport, output_path: &Path) -> Result<()> {
    let mut wtr = csv::WriterBuilder::new()
        .flexible(true)
        .from_path(output_path)?;

    wtr.write_record(&[format!("Exported by {} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))])?;
    wtr.write_record::<&[_; 0], &&str>(&[])?;   // empty line (actually becomes line with "")
    wtr.write_record(&["", "Short Term", "Long Term", "Total"])?;
    wtr.write_record(&["Capital Gains",
        rounded_to_cent(report.short_term_capital_gains).to_string().as_str(),
        rounded_to_cent(report.long_term_capital_gains).to_string().as_str(),
        rounded_to_cent(report.total_capital_gains()).to_string().as_str()])?;
    wtr.write_record(&["Capital Losses",
        rounded_to_cent(report.short_term_capital_losses).to_string().as_str(),
        rounded_to_cent(report.long_term_capital_losses).to_string().as_str(),
        rounded_to_cent(report.total_capital_losses()).to_string().as_str()])?;
    wtr.write_record(&["Net Capital Gains",
        rounded_to_cent(report.short_term_net_capital_gains()).to_string().as_str(),
        rounded_to_cent(report.long_term_net_capital_gains()).to_string().as_str(),
        rounded_to_cent(report.total_net_capital_gains()).to_string().as_str()])?;
    wtr.write_record::<&[_; 0], &&str>(&[])?;   // empty line (actually becomes line with "")

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

    for currency in &report.currencies {
        wtr.serialize(CsvSummary {
            currency: &currency.currency,
            proceeds: rounded_to_cent(currency.proceeds),
            cost: rounded_to_cent(currency.cost),
            fees: rounded_to_cent(currency.fees),
            capital_gains: rounded_to_cent(currency.capital_profit_loss),
            other_income: rounded_to_cent(currency.income),
            total_gains: rounded_to_cent(currency.total_profit_loss),
            opening_balance: currency.balance_start,
            quantity_traded: currency.quantity_disposed,
            quantity_income: currency.quantity_income,
            closing_balance: currency.balance_end,
        })?;
    }

    Ok(())
}

/// Exports the tax reports for each year
pub(crate) fn export_all_to(app: &App, output_path: &Path) -> Result<()> {
    let path = output_path.join(format!("yearly_summary.csv"));
    let mut wtr = csv::WriterBuilder::new()
        .from_path(path)?;

    #[derive(Serialize)]
    struct YearSummary {
        #[serde(rename = "Year")]
        year: i32,
        #[serde(rename = "Proceeds")]
        proceeds: Decimal,
        #[serde(rename = "Cost")]
        cost: Decimal,
        #[serde(rename = "Gain or Loss")]
        gains: Decimal,
    }

    for report in &app.reports {
        wtr.serialize(YearSummary {
            year: report.year,
            proceeds: rounded_to_cent(report.short_term_proceeds),
            cost: rounded_to_cent(report.short_term_cost),
            gains: rounded_to_cent(report.short_term_proceeds - report.short_term_cost),
        })?;

        let year = if report.year == 0 { "all_time".to_owned() } else { report.year.to_string() };
        let path = output_path.join(format!("{}_report_summary.csv", year));
        save_summary_to_csv(report, &path)?;

        let path = output_path.join(format!("{}_capital_gains_report.csv", year));
        fifo::save_gains_to_csv(&report.gains, &path)?;
    }
    Ok(())
}

fn load_transactions(portfolio: &mut Portfolio) -> Result<Vec<Transaction>> {
    let (wallets, ignored_currencies) = (&mut portfolio.wallets, &portfolio.ignored_currencies);
    let mut transactions = Vec::new();

    for (wallet_index, wallet) in wallets.iter_mut().enumerate() {
        let mut wallet_transactions = Vec::new();

        for source in wallet.sources.iter_mut() {
            if !source.enabled || !wallet.enabled {
                source.transaction_count = 0;
                continue
            }

            let source_definition = match transaction_source_by_id(&source.source_type) {
                Some(definition) => definition,
                None => {
                    source.transaction_count = 0;
                    println!("Unknown source type {}", source.source_type);
                    continue;
                }
            };

            let source_txs = if let Some(load_sync) = source_definition.load_sync {
                load_sync(&source.full_path)
            } else {
                anyhow::Ok(source.transactions.clone())
            };

            match source_txs {
                Ok(mut source_transactions) => {
                    // sort transactions
                    source_transactions.sort_by(|a, b| a.cmp(b));

                    // merge consecutive trades that are really the same order
                    if portfolio.merge_consecutive_trades {
                        merge_consecutive_trades(&mut source_transactions);
                    }

                    let is_ignored = |currency: &str| {
                        ignored_currencies
                            .binary_search_by(|ignored| ignored.as_str().cmp(currency))
                            .is_ok()
                    };

                    // remove transactions with ignored currencies
                    source_transactions.retain_mut(|tx| {
                        let retain_tx = match tx.incoming_outgoing() {
                            (None, None) => true,
                            (None, Some(amount)) | (Some(amount), None) => {
                                !is_ignored(amount.effective_currency().as_ref())
                            }
                            (Some(incoming), Some(outgoing)) => {
                                // Trades can only be ignored, if both the incoming and outgoing currencies are ignored
                                !(is_ignored(incoming.effective_currency().as_ref())
                                    && is_ignored(outgoing.effective_currency().as_ref()))
                            }
                        };

                        retain_tx || tx.fee.take().is_some_and(|fee| {
                            if !is_ignored(&fee.currency) {
                                // We can't ignore the fee, so keep the transaction just for the fee
                                tx.operation = Operation::Fee(fee);
                                tx.value = tx.fee_value.take();

                                true
                            } else {
                                false
                            }
                        })
                    });

                    source.transaction_count = source_transactions.len();
                    wallet_transactions.extend(source_transactions);
                }
                // todo: provide this feedback to the UI
                Err(e) => {
                    source.transaction_count = 0;
                    println!("Error loading source {}: {}", source.full_path.display(), e);
                }
            }
        }

        for tx in &mut wallet_transactions {
            tx.wallet_index = wallet_index;
        }

        wallet.balances = calculate_balances(&wallet_transactions);
        transactions.extend(wallet_transactions);
    }

    // sort transactions
    transactions.sort_by(|a, b| a.cmp(b));

    // assign transaction indices
    for (index, tx) in transactions.iter_mut().enumerate() {
        tx.index = index;
    }

    // warn about duplicates
    let mut last = transactions.first();
    for tx in transactions.iter().skip(1) {
        if last.map(|last| last == tx).unwrap_or(false) {
            println!("Duplicate transaction detected: {:?}", tx);
        }
        last = Some(tx);
    }

    match_send_receive(&mut transactions);

    Ok(transactions)
}

fn merge_consecutive_trades(transactions: &mut Vec<Transaction>) {
    let mut index = 1;
    while index < transactions.len() {
        let (a, b) = transactions.split_at_mut(index);
        let (a, b) = (a.last_mut().unwrap(), &b[0]);

        if a.merge_trades(b).is_ok() {
            transactions.remove(index);
        } else {
            index += 1;
        }
    }
}

/// A helper function that returns a mutable reference to a value in a hash map,
/// without allocating a new key when the map already contains a value for that
/// key.
fn get_or_default<'a, K, V>(hash_map: &'a mut HashMap<K, V>, key: &K) -> &'a mut V
    where
        K: Eq,
        K: Hash,
        K: ToOwned<Owned = K>,
        V: Default,
{
    if hash_map.contains_key(key) {
        hash_map.get_mut(key).unwrap()
    } else {
        hash_map.entry(key.to_owned()).or_default()
    }
}

fn calculate_balances(transactions: &[Transaction]) -> HashMap<String, Decimal> {
    let mut balances = HashMap::new();

    for tx in transactions {
        let (incoming, outgoing) = tx.incoming_outgoing();
        if let Some(incoming) = incoming {
            if !incoming.is_fiat() {
                *get_or_default(&mut balances, &incoming.currency) += incoming.quantity;
            }
        }
        if let Some(outgoing) = outgoing {
            if !outgoing.is_fiat() {
                *get_or_default(&mut balances, &outgoing.currency) -= outgoing.quantity;
            }
        }
        if let Some(fee) = &tx.fee {
            if !fee.is_fiat() {
                *get_or_default(&mut balances, &fee.currency) -= fee.quantity;
            }
        }
    }

    balances
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

                let mut best_match: Option<(usize, Decimal)> = None;

                for (i, tx_index) in unmatched_sends_receives.iter().enumerate().rev().take_while(|(_, tx_index)| -> bool {
                    // the unmatched send may not be too old
                    let tx: &Transaction = &transactions[**tx_index];
                    tx.timestamp >= oldest_match_time
                }) {
                    let candidate_tx: &Transaction = &transactions[*tx_index];

                    match (&candidate_tx.operation, &tx.operation) {
                        (Operation::Send(send_amount), Operation::Receive(receive_amount)) |
                        (Operation::Receive(receive_amount), Operation::Send(send_amount)) => {
                            // the send and receive transactions must have the same currency
                            if receive_amount.currency != send_amount.currency {
                                continue;
                            }

                            // if both transactions have a tx_hash set, it must be equal
                            if let (Some(candidate_tx_hash), Some(tx_hash)) = (&candidate_tx.tx_hash, &tx.tx_hash) {
                                if candidate_tx_hash != tx_hash {
                                    continue;
                                }
                            }

                            // check whether the price roughly matches (sent amount can't be lower than received amount, but can be 5% higher)
                            if receive_amount.quantity > send_amount.quantity || receive_amount.quantity < send_amount.quantity * dec!(0.95) {
                                continue;
                            }

                            let difference = (send_amount.quantity - receive_amount.quantity).abs();
                            match best_match {
                                None => best_match = Some((i, difference)),
                                Some((_, best_difference)) => {
                                    if difference < best_difference {
                                        best_match = Some((i, difference));
                                    }
                                }
                            }

                            if difference.is_zero() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }

                if let Some((matching_index, _)) = best_match {
                    // this send is now matched, so remove it from the list of unmatched sends
                    let matching_tx_index = unmatched_sends_receives.remove(matching_index);
                    matching_pairs.push(if tx.operation.is_send() {
                        (index, matching_tx_index)
                    } else {
                        (matching_tx_index, index)
                    });
                } else {
                    // no match was found for this transactions, so add it to the unmatched list
                    unmatched_sends_receives.push(index);
                }
            }
            _ => {}
        }
    }

    for (send_index, receive_index) in matching_pairs {
        // Derive the fee based on received amount and sent amount
        enum MatchResult {
            NoAdjustment,
            AdjustSend { amount: Amount, fee: Amount },
            AdjustReceive { amount: Amount, fee: Amount },
            AbortMatch,
        }

        let match_result = match (&transactions[send_index].operation, &transactions[receive_index].operation) {
            (Operation::Send(sent), Operation::Receive(received)) if received.quantity < sent.quantity => {
                assert!(sent.currency == received.currency);

                let implied_fee = Amount::new(sent.quantity - received.quantity, sent.currency.clone());

                match &transactions[send_index].fee {
                    Some(existing_fee) => {
                        if existing_fee.currency != implied_fee.currency {
                            println!("warning: send/receive amounts imply fee, but there's already a fee in a different currency ({}) for transaction {:?}", existing_fee.currency, transactions[send_index]);
                            MatchResult::AbortMatch
                        } else if existing_fee.quantity != implied_fee.quantity {
                            // This can happen for ETH deposits to Bitstamp, since they get rounded
                            // down to 8 decimal places even though ETH tokens can have up to 18
                            // decimal places. In this case a small amount of ETH seems to get lost.
                            //
                            // We set the receive fee to the implied fee to make sure this small
                            // loss is accounted for.
                            if transactions[receive_index].fee.is_none() {
                                println!("warning: sent amount {} different from received amount {} and the fee {} doesn't match, adjusting received amount to {} and setting receive fee to {}", sent, received, existing_fee, sent, implied_fee);
                                MatchResult::AdjustReceive { amount: sent.clone(), fee: implied_fee }
                            } else {
                                println!("warning: sent amount {} different from received amount {} and the fee {} doesn't match, but there's already a receive fee as well", sent, received, existing_fee);
                                MatchResult::NoAdjustment
                            }
                        } else {
                            println!("warning: fee {} appears to have been included in the sent amount {}, adjusting sent amount to {}", existing_fee, sent, received);
                            MatchResult::AdjustSend { amount: received.clone(), fee: implied_fee }
                        }
                    }
                    None => {
                        println!("warning: a fee of {} appears to have been included in the sent amount {}, adjusting sent amount to {} and setting fee", implied_fee, sent, received);
                        MatchResult::AdjustSend { amount: received.clone(), fee: implied_fee }
                    }
                }
            }
            _ => MatchResult::NoAdjustment,
        };

        match match_result {
            MatchResult::AbortMatch => {
                // Move matched transactions back to unmatched
                unmatched_sends_receives.push(send_index);
                unmatched_sends_receives.push(receive_index);
                continue;
            }
            MatchResult::AdjustSend { amount, fee } => {
                let tx = &mut transactions[send_index];
                tx.fee = Some(fee);
                if let Operation::Send(send_amount) = &mut tx.operation {
                    *send_amount = amount;
                }
            }
            MatchResult::AdjustReceive { amount, fee } => {
                let tx = &mut transactions[receive_index];
                tx.fee = Some(fee);
                if let Operation::Receive(receive_amount) = &mut tx.operation {
                    *receive_amount = amount;
                }
            }
            MatchResult::NoAdjustment => {}
        }

        transactions[send_index].matching_tx = Some(receive_index);
        transactions[receive_index].matching_tx = Some(send_index);
    }

    unmatched_sends_receives.iter().for_each(|unmatched_tx| {
        let tx = &mut transactions[*unmatched_tx];
        match &tx.operation {
            // Turn unmatched Sends into Sells
            Operation::Send(amount) => {
                tx.operation = Operation::Sell(amount.clone());
            }
            // Turn unmatched Receives into Buys
            Operation::Receive(amount) => {
                tx.operation = Operation::Buy(amount.clone());
            }
            _ => unreachable!("only Send and Receive transactions can be unmatched"),
        }
    });
}

fn estimate_transaction_values(transactions: &mut Vec<Transaction>, price_history: &PriceHistory) {
    let estimate_transaction_value = |tx: &mut Transaction| {
        if tx.value.is_none() {
            tx.value = match tx.incoming_outgoing() {
                (Some(incoming), Some(outgoing)) => {
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
                                let min_value = value_incoming.quantity.min(value_outgoing.quantity);
                                let max_value = value_incoming.quantity.max(value_outgoing.quantity);
                                if min_value < max_value * Decimal::new(95, 2) {
                                    println!("warning: {}% value difference between incoming {} ({}) and outgoing {} ({})", (Decimal::ONE_HUNDRED * (max_value - min_value) / max_value).round(), incoming, value_incoming, outgoing, value_outgoing);
                                }
                                let average = (value_incoming.quantity + value_outgoing.quantity) / Decimal::TWO;
                                Some(Amount::new(average, "EUR".to_owned()))
                            }
                        }
                    }
                }
                (Some(amount), None) |
                (None, Some(amount)) => {
                    price_history.estimate_value(tx.timestamp, amount)
                }
                (None, None) => None,
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

async fn update_price_history(app: Rc<RefCell<App>>) {
    // Determine which price points we need to know for our transactions and clone the
    // available price history so that it can be extended in a thread.
    let (requirements, mut price_history) = {
        app.borrow_mut().stop_update_price_history = false;
        let app = app.borrow();
        app.ui().global::<Facade>().set_updating_price_history(true);

        (collect_price_requirements(&app.transactions), app.price_history.clone())
    };

    // Determine ranges of missing price points that need to be requested
    let cmc_interval = CmcInterval::Hourly;
    let tolerance = cmc_interval.duration();
    let padding = Duration::days(7);
    let mut missing_ranges = requirements.missing_ranges(&price_history, tolerance, padding);
    let mut save = false;
    // price_history.debug_dump();
    // dbg!(&missing_ranges);

    let mut total_ranges = 0;
    let mut processed_ranges = 0;

    let max_span = cmc_interval.duration() * 400;
    for (_, ranges) in missing_ranges.iter_mut() {
        split_ranges(ranges, max_span);
        total_ranges += ranges.len();
    }

    app.borrow().ui().global::<Facade>().set_updating_price_history_progress(0.0);

    // Download missing price points
    for (currency, ranges) in missing_ranges {
        for range in ranges {
            println!("Downloading price points for {:} from {:} to {:}", currency, range.start, range.end);
            let currency_for_task = currency.clone();
            let price_points = tokio::task::spawn(async move {
                coinmarketcap::download_price_points(range.start, range.end, currency_for_task.as_str(), cmc_interval).await
            }).await.unwrap();

            match price_points {
                Ok(price_points) => {
                    let count = price_points.len();
                    price_history.price_data(currency.to_owned()).add_points(price_points);
                    let mut app = app.borrow_mut();
                    app.price_history = price_history.clone();
                    app.refresh_transactions();
                    app.refresh_ui();
                    app.report_info(&format!("Price history updated for {:} from {:} to {:} ({:} points)", currency, range.start, range.end, count));
                    save = true;
                }
                Err(e) => {
                    app.borrow().report_warning(&format!("Failed to download price points for {:}: {:}", currency, e));
                }
            }

            processed_ranges += 1;
            let progress = if total_ranges > 0 {
                processed_ranges as f32 / total_ranges as f32
            } else {
                0.0
            };
            app.borrow().ui().global::<Facade>().set_updating_price_history_progress(progress);

            if app.borrow().stop_update_price_history {
                break;
            }
        }

        if app.borrow().stop_update_price_history {
            break;
        }
    }

    // On success, update the app's price history and refresh the UI
    if save {
        let mut app = app.borrow_mut();
        app.save_portfolio(None);

        if let Some(dirs) = app.project_dirs.as_ref() {
            let data_dir = dirs.data_local_dir();
            if let Err(e) = app.price_history.save_to_dir(&data_dir) {
                eprintln!("Error saving price history: {}", e);
            }
        }
    }

    app.borrow().ui().global::<Facade>().set_updating_price_history(false);
}

fn collect_price_requirements(transactions: &[Transaction]) -> PriceRequirements {
    let mut requirements = PriceRequirements::new();

    for tx in transactions.iter() {
        // For transactions to or from fiat, we know the value exactly and don't
        // need to estimate the value.
        match tx.incoming_outgoing() {
            (None, None) => {}
            (None, Some(amount)) |
            (Some(amount), None) => {
                if !amount.is_fiat() {
                    requirements.add(&amount.currency, tx.timestamp);
                }
            }
            (Some(incoming), Some(outgoing)) => {
                if !incoming.is_fiat() && !outgoing.is_fiat() {
                    // In case neither side is fiat, we want to know the price of both
                    // currencies, since it can give a better value estimate.
                    requirements.add(&incoming.currency, tx.timestamp);
                    requirements.add(&outgoing.currency, tx.timestamp);
                }
            }
        }

        // We may also need to know the price of the fee currency.
        match &tx.fee {
            Some(amount) => {
                if !amount.is_fiat() {
                    requirements.add(&amount.currency, tx.timestamp);
                }
            }
            None => {}
        };
    }

    requirements
}

fn calculate_tax_reports(transactions: &mut Vec<Transaction>, tracking: CostBasisTracking) -> Vec<TaxReport> {
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

    // While processing the transactions, we need to be able to look up the
    // wallet index for each transaction by its global index (for handling
    // matched send/receive transactions in per-wallet FIFO). Since we're
    // iterating mutable slices, Rust doesn't allow us to read from the original
    // transactions vector. Hence we're copying the necessary data here.
    //
    // Alternatively, we could adjust FIFO::process to take a non-mutable slice.
    // The only reason the transactions are mutable is to be able to assign to
    // Transaction::gain.
    let tx_meta: Vec<fifo::TxMeta> = transactions.iter().map(|tx| fifo::TxMeta {
        wallet_index: tx.wallet_index,
    }).collect();

    // Process transactions per-year
    let mut fifo = FIFO::with_tracking(tracking);
    let mut reports: Vec<TaxReport> = transactions.linear_group_by_key_mut(|tx| tx.timestamp.year()).map(|txs| {
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
        let gains = fifo.process(txs, &tx_meta);

        let mut short_term_cost = Decimal::ZERO;
        let mut short_term_proceeds = Decimal::ZERO;
        let mut short_term_capital_gains = Decimal::ZERO;
        let mut short_term_capital_losses = Decimal::ZERO;
        let mut long_term_capital_gains = Decimal::ZERO;
        let mut long_term_capital_losses = Decimal::ZERO;

        for gain in &gains {
            let gain_or_loss = gain.profit();

            if gain_or_loss.is_sign_positive() {
                if gain.long_term() {
                    long_term_capital_gains += gain_or_loss;
                } else {
                    short_term_capital_gains += gain_or_loss;
                }
            } else {
                if gain.long_term() {
                    long_term_capital_losses -= gain_or_loss;
                } else {
                    short_term_capital_losses -= gain_or_loss;
                }
            }

            if !gain.long_term() {
                short_term_cost += gain.cost;
                short_term_proceeds += gain.proceeds;
            }

            let summary = summary_for(&mut currencies, &gain.amount.currency);
            summary.quantity_disposed += gain.amount.quantity;
            // summary.quantity_income += // todo: sum up all income quantities
            summary.cost += gain.cost;
            summary.proceeds += gain.proceeds;
            // summary.income = ;   // todo: calculate the value of all income transactions for this currency
        }

        // Sum up the fees on trades in this year, when they were not merged
        // into the outgoing amount for a trade
        txs.iter().for_each(|tx| {
            match (&tx.operation, &tx.fee, &tx.fee_value) {
                (Operation::Trade { incoming: _, outgoing }, Some(fee), Some(fee_value)) => {
                    if outgoing.try_add(fee).is_none() {
                        let summary = summary_for(&mut currencies, &fee.currency);
                        summary.fees += fee_value.quantity;
                    }
                }
                _ => {}
            }
        });

        let holdings_snapshot = fifo.holdings();

        // Make sure there is an entry for each held currency, even if it didn't generate gains or losses
        holdings_snapshot.inner().iter().for_each(|(currency, lots)| {
            if !lots.is_empty() {
                let _ = summary_for(&mut currencies, currency);
            }
        });

        currencies.iter_mut().for_each(|summary| {
            summary.balance_end = holdings_snapshot.currency_balance(&summary.currency);
            summary.cost_end = holdings_snapshot.currency_cost_base(&summary.currency);
            summary.capital_profit_loss = summary.proceeds - summary.cost - summary.fees;
            summary.total_profit_loss = summary.capital_profit_loss + summary.income;

            // Count the fees as short-term loss (todo: not sure if correct)
            short_term_capital_losses += summary.fees;
            short_term_cost += summary.fees;
        });

        currencies.sort_unstable_by(CurrencySummary::cmp);

        TaxReport {
            year,
            short_term_cost,
            short_term_proceeds,
            short_term_capital_gains,
            short_term_capital_losses,
            long_term_capital_gains,
            long_term_capital_losses,
            currencies: currencies.clone(),
            gains,
        }
    }).collect();

    // add an "all time" report
    let mut all_time = TaxReport {
        year: 0,
        short_term_cost: Decimal::ZERO,
        short_term_proceeds: Decimal::ZERO,
        short_term_capital_gains: Decimal::ZERO,
        short_term_capital_losses: Decimal::ZERO,
        long_term_capital_gains: Decimal::ZERO,
        long_term_capital_losses: Decimal::ZERO,
        currencies: Vec::new(),
        gains: Vec::new(),
    };
    for report in &reports {
        all_time.short_term_cost += report.short_term_cost;
        all_time.short_term_proceeds += report.short_term_proceeds;
        all_time.short_term_capital_gains += report.short_term_capital_gains;
        all_time.short_term_capital_losses += report.short_term_capital_losses;
        all_time.long_term_capital_gains += report.long_term_capital_gains;
        all_time.long_term_capital_losses += report.long_term_capital_losses;
        for currency_summary in &report.currencies {
            let summary = summary_for(&mut all_time.currencies, &currency_summary.currency);
            summary.balance_end = currency_summary.balance_end;
            summary.cost_end = currency_summary.cost_end;
            summary.quantity_disposed += currency_summary.quantity_disposed;
            summary.quantity_income += currency_summary.quantity_income;
            summary.cost += currency_summary.cost;
            summary.fees += currency_summary.fees;
            summary.proceeds += currency_summary.proceeds;
            summary.capital_profit_loss += currency_summary.capital_profit_loss;
            summary.income += currency_summary.income;
            summary.total_profit_loss += currency_summary.total_profit_loss;
        }
        all_time.gains.extend_from_slice(&report.gains);
    }
    all_time.currencies.sort_unstable_by(CurrencySummary::cmp);
    reports.push(all_time);

    reports
}

fn initialize_ui(app: &mut App) -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    app.ui_weak = ui.as_weak();

    let facade = ui.global::<Facade>();

    let mut source_types: Vec<SharedString> = TRANSACTION_SOURCES
        .iter()
        .map(|source| SharedString::from(source.label))
        .collect();
    source_types.sort();
    facade.set_source_types(Rc::new(VecModel::from(source_types)).into());

    facade.set_wallets(app.ui_wallets.clone().into());
    facade.set_transactions(app.ui_transactions.clone().into());
    facade.set_report_years(app.ui_report_years.clone().into());
    facade.set_reports(app.ui_reports.clone().into());
    facade.set_portfolio(UiPortfolio::default());
    facade.set_notifications(ModelRc::new(VecModel::<UiNotification>::default()));

    facade.on_open_transaction(move |blockchain, tx_hash| {
        let _ = match blockchain.as_str() {
            "BCH" => open::that(format!("https://blockchair.com/bitcoin-cash/transaction/{}", tx_hash)),
            "BTC" | "" => open::that(format!("https://blockchair.com/bitcoin/transaction/{}", tx_hash)),
            // or "https://btc.com/tx/{}"
            // or "https://live.blockcypher.com/btc/tx/{}"
            "DASH" => open::that(format!("https://live.blockcypher.com/dash/tx/{}", tx_hash)),
            "ETH" => open::that(format!("https://etherscan.io/tx/{}", tx_hash)),
            "LTC" => open::that(format!("https://blockchair.com/litecoin/transaction/{}", tx_hash)),
            "PPC" => open::that(format!("https://explorer.peercoin.net/tx/{}", tx_hash)),
            "RDD" => open::that(format!("https://rddblockexplorer.com/tx/{}", tx_hash)),
            "XLM" => open::that(format!("https://stellar.expert/explorer/public/tx/{}", tx_hash)),
            "XMR" => open::that(format!("https://blockchair.com/monero/transaction/{}", tx_hash)),
            "XRP" => open::that(format!("https://xrpscan.com/tx/{}", tx_hash)),
            "ZEC" => open::that(format!("https://blockchair.com/zcash/transaction/{}", tx_hash)),
            _ => {
                println!("No explorer URL defined for blockchain: {}", blockchain);
                Ok(())
            }
        };
    });

    Ok(ui)
}

fn ui_set_wallets(app: &App) {
    let ui_wallets: Vec<UiWallet> = app.portfolio.wallets.iter().map(|wallet| {
        let ui_sources: Vec<UiWalletSource> = wallet.sources.iter().map(|source| {
            let source_definition = transaction_source_by_id(&source.source_type);
            let label = source_definition
                .map(|definition| definition.label)
                .unwrap_or(source.source_type.as_str());
            let can_sync = source_definition.map(|definition| definition.can_sync()).unwrap_or(false);

            UiWalletSource {
                source_type: label.into(),
                name: source.name.clone().into(),
                path: source.path.clone().into(),
                enabled: source.enabled,
                can_sync,
                transaction_count: source.transaction_count as i32,
            }
        }).collect();

        UiWallet {
            // source_type: source.source_type.to_string().into(),
            name: wallet.name.clone().into(),
            enabled: wallet.enabled,
            expanded: wallet.expanded,
            transaction_count: wallet.transaction_count() as i32,
            sources: Rc::new(VecModel::from(ui_sources)).into(),
        }
    }).collect();

    app.ui_wallets.set_vec(ui_wallets);
}

fn ui_set_transactions(app: &App) {
    let wallets = &app.portfolio.wallets;
    let transactions = &app.transactions;
    let filters = &app.transaction_filters;
    let mut transaction_warning_count = 0;

    let mut ui_transactions = Vec::new();

    for transaction in transactions {
        let matching_tx = transaction.matching_tx.map(|index| &transactions[index]);
        let filter_matches = |filter: &TransactionFilter| {
            filter.matches(transaction) || matching_tx.is_some_and(|tx| filter.matches(tx))
        };

        if !(filters.iter().all(filter_matches)) {
            continue;
        }

        let wallet = wallets.get(transaction.wallet_index);
        let wallet_name: Option<SharedString> = wallet.map(|source| source.name.clone().into());

        let mut value = transaction.value.as_ref();
        let mut description = transaction.description.clone();
        let mut tx_hash = transaction.tx_hash.as_ref();
        let mut blockchain = transaction.blockchain.as_ref();
        let mut fee = transaction.fee.clone();
        let mut gain = transaction.gain.clone();

        let (tx_type, sent, received, from, to) = match &transaction.operation {
            Operation::Buy(amount) => (UiTransactionType::Buy, None, Some(amount), None, wallet_name),
            Operation::Sell(amount) => (UiTransactionType::Sell, Some(amount), None, wallet_name, None),
            Operation::Trade { incoming, outgoing } => {
                (UiTransactionType::Trade, Some(outgoing), Some(incoming), wallet_name.clone(), wallet_name)
            }
            Operation::Swap { incoming, outgoing } => {
                (UiTransactionType::Swap, Some(outgoing), Some(incoming), wallet_name.clone(), wallet_name)
            }
            Operation::FiatDeposit(amount) => {
                (UiTransactionType::Deposit, None, Some(amount), None, wallet_name)
            }
            Operation::FiatWithdrawal(amount) => {
                (UiTransactionType::Withdrawal, Some(amount), None, wallet_name, None)
            }
            Operation::Send(_) => {
                assert!(transaction.matching_tx.is_some(), "Unmatched Send should have been changed to Sell");
                continue;   // added as a Transfer when handling the Receive
            }
            Operation::Receive(receive_amount) => {
                // matching_tx has to be set at this point, otherwise it should have been a Buy
                let matching_send = &transactions[transaction.matching_tx.expect("Receive should have matched a Send transaction")];
                if let Operation::Send(send_amount) = &matching_send.operation {
                    let send_wallet = wallets.get(matching_send.wallet_index);
                    let send_wallet_name = send_wallet.map(|source| source.name.clone().into());

                    value = value.or(matching_send.value.as_ref());
                    tx_hash = tx_hash.or(matching_send.tx_hash.as_ref());
                    blockchain = blockchain.or(matching_send.blockchain.as_ref());

                    // If both sides have a fee, try to add them together
                    fee = match (fee, matching_send.fee.as_ref()) {
                        (Some(receive_fee), Some(send_fee)) => {
                            receive_fee.try_add(send_fee).or(Some(receive_fee))
                        }
                        (None, Some(send_fee)) => Some(send_fee.clone()),
                        (Some(receive_fee), None) => Some(receive_fee),
                        _ => None,
                    };

                    description = match (description, &matching_send.description) {
                        (Some(receive_description), Some(send_description)) => {
                            Some(format!("{}, {}", send_description, receive_description))
                        }
                        (Some(receive_description), None) => Some(receive_description),
                        (None, Some(send_description)) => Some(send_description.clone()),
                        (None, None) => None,
                    };

                    // When either side of the transfer has an error, make sure the error is visible
                    gain = match (gain, &matching_send.gain) {
                        // Display sum of gains (can happen due to "receiving fee")
                        (Some(Ok(a)), Some(Ok(b))) => Some(Ok(a + *b)),

                        // Error overrides any gains
                        (Some(Err(e)), _) => Some(Err(e)),
                        (_, Some(Err(e))) => Some(Err(e.clone())),

                        // If one is None, return the other
                        (Some(g), None) => Some(g),
                        (None, Some(g)) => Some(g.clone()),

                        (None, None) => None,
                    };

                    (UiTransactionType::Transfer, Some(send_amount), Some(receive_amount), send_wallet_name, wallet_name)
                } else {
                    unreachable!("Receive was matched with a non-Send transaction");
                }
            }
            Operation::Fee(amount) => {
                (UiTransactionType::Fee, Some(amount), None, wallet_name, None)
            }
            Operation::ChainSplit(amount) => {
                (UiTransactionType::ChainSplit, None, Some(amount), None, wallet_name)
            }
            Operation::Expense(amount) => {
                (UiTransactionType::Expense, Some(amount), None, wallet_name, None)
            }
            Operation::Stolen(amount) => {
                (UiTransactionType::Stolen, Some(amount), None, wallet_name, None)
            }
            Operation::Lost(amount) => {
                (UiTransactionType::Lost, Some(amount), None, wallet_name, None)
            }
            Operation::Burn(amount) => {
                (UiTransactionType::Burn, Some(amount), None, wallet_name, None)
            }
            Operation::Income(amount) => {
                (UiTransactionType::Income, None, Some(amount), None, wallet_name)
            }
            Operation::Airdrop(amount) => {
                (UiTransactionType::Airdrop, None, Some(amount), None, wallet_name)
            }
            Operation::Staking(amount) => {
                (UiTransactionType::Staking, None, Some(amount), None, wallet_name)
            }
            Operation::Cashback(amount) => {
                (UiTransactionType::Cashback, None, Some(amount), None, wallet_name)
            }
            Operation::IncomingGift(amount) |
            Operation::OutgoingGift(amount) => {
                (UiTransactionType::Gift, None, Some(amount), None, wallet_name)
            }
            Operation::Spam(amount) => {
                (UiTransactionType::Spam, None, Some(amount), None, wallet_name)
            }
        };

        let (gain, gain_error) = match gain {
            Some(Ok(gain)) => (gain, None),
            Some(Err(e)) => (Decimal::ZERO, Some(e.to_string())),
            None => (Decimal::ZERO, None),
        };

        if gain_error.is_some() {
            transaction_warning_count += 1;
        }

        let timestamp = Local.from_utc_datetime(&transaction.timestamp).naive_local();

        ui_transactions.push(UiTransaction {
            id: transaction.index as i32,
            from: from.unwrap_or_default(),
            to: to.unwrap_or_default(),
            date: timestamp.date().to_string().into(),
            time: timestamp.time().format("%H:%M:%S").to_string().into(),
            tx_type,
            received_cmc_id: received.map(Amount::cmc_id).unwrap_or(-1),
            received: received.map_or_else(String::default, Amount::to_string).into(),
            sent_cmc_id: sent.map(Amount::cmc_id).unwrap_or(-1),
            sent: sent.map_or_else(String::default, Amount::to_string).into(),
            fee: fee.as_ref().map_or_else(String::default, Amount::to_string).into(),
            value: value.map_or_else(String::default, Amount::to_string).into(),
            gain: rounded_to_cent(gain).try_into().unwrap(),
            gain_error: gain_error.unwrap_or_default().into(),
            description: description.unwrap_or_default().into(),
            tx_hash: tx_hash.map(|s| s.to_owned()).unwrap_or_default().into(),
            blockchain: blockchain.map(|s| s.to_owned()).unwrap_or_default().into(),
        });
    }

    app.ui_transactions.set_vec(ui_transactions);
    app.ui().global::<Facade>().set_transaction_warning_count(transaction_warning_count);
}

fn ui_set_reports(app: &App) {
    let report_years: Vec<StandardListViewItem> = app.reports.iter().map(|report| {
        if report.year == 0 {
            StandardListViewItem::from("All Time")
        } else {
            StandardListViewItem::from(report.year.to_string().as_str())
        }
    }).collect();
    app.ui_report_years.set_vec(report_years);

    let ui_reports: Vec<UiTaxReport> = app.reports.iter().map(|report| {
        let ui_gains: Vec<UiCapitalGain> = report.gains.iter().map(|gain| {
            let bought = Local.from_utc_datetime(&gain.bought).naive_local();
            let sold = Local.from_utc_datetime(&gain.sold).naive_local();

            UiCapitalGain {
                currency_cmc_id: gain.amount.cmc_id(),
                bought_date: bought.date().to_string().into(),
                bought_time: bought.time().format("%H:%M:%S").to_string().into(),
                bought_tx_id: gain.bought_tx_index as i32,
                sold_date: sold.date().to_string().into(),
                sold_time: sold.time().format("%H:%M:%S").to_string().into(),
                sold_tx_id: gain.sold_tx_index as i32,
                amount: gain.amount.to_string().into(),
                // todo: something else than unwrap()?
                cost: rounded_to_cent(gain.cost).try_into().unwrap(),
                proceeds: rounded_to_cent(gain.proceeds).try_into().unwrap(),
                gain_or_loss: rounded_to_cent(gain.profit()).try_into().unwrap(),
                long_term: gain.long_term(),
            }
        }).collect();
        let ui_gains = Rc::new(VecModel::from(ui_gains));

        let ui_currencies: Vec<UiCurrencySummary> = report.currencies.iter().map(|currency| {
            UiCurrencySummary {
                currency_cmc_id: cmc_id(&currency.currency),
                currency: currency.currency.clone().into(),
                balance_start: currency.balance_start.normalize().to_string().into(),
                balance_end: currency.balance_end.normalize().to_string().into(),
                quantity_disposed: currency.quantity_disposed.normalize().to_string().into(),
                cost: format!("{:.2}", rounded_to_cent(currency.cost)).into(),
                fees: format!("{:.2}", rounded_to_cent(currency.fees)).into(),
                proceeds: format!("{:.2}", rounded_to_cent(currency.proceeds)).into(),
                capital_profit_loss: format!("{:.2}", rounded_to_cent(currency.capital_profit_loss)).into(),
                income: format!("{:.2}", rounded_to_cent(currency.income)).into(),
                total_profit_loss: format!("{:.2}", rounded_to_cent(currency.total_profit_loss)).into(),
            }
        }).collect();
        let ui_currencies = Rc::new(VecModel::from(ui_currencies));

        UiTaxReport {
            currencies: ui_currencies.into(),
            gains: ui_gains.into(),
            short_term_capital_gains: format!("{:.2}", rounded_to_cent(report.short_term_capital_gains)).into(),
            short_term_capital_losses: format!("{:.2}", rounded_to_cent(report.short_term_capital_losses)).into(),
            short_term_net_capital_gains: format!("{:.2}", rounded_to_cent(report.short_term_net_capital_gains())).into(),
            long_term_capital_gains: format!("{:.2}", rounded_to_cent(report.long_term_capital_gains)).into(),
            long_term_capital_losses: format!("{:.2}", rounded_to_cent(report.long_term_capital_losses)).into(),
            long_term_net_capital_gains: format!("{:.2}", rounded_to_cent(report.long_term_net_capital_gains())).into(),
            total_capital_gains: format!("{:.2}", rounded_to_cent(report.total_capital_gains())).into(),
            total_capital_losses: format!("{:.2}", rounded_to_cent(report.total_capital_losses())).into(),
            total_net_capital_gains: format!("{:.2}", rounded_to_cent(report.total_net_capital_gains())).into(),
            year: report.year,
        }
    }).collect();

    app.ui_reports.set_vec(ui_reports);
}

fn ui_set_portfolio(app: &App) {
    let ui = app.ui();
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
                cost: rounded_to_cent(currency.cost_end).try_into().unwrap(),
                value: rounded_to_cent(current_value).try_into().unwrap(),
                roi: roi.map(|roi| format!("{:.2}%", rounded_to_cent(roi))).unwrap_or_else(|| { "-".to_owned() }).into(),
                is_profit: roi.map_or(false, |roi| roi > Decimal::ZERO),
                unrealized_gain: rounded_to_cent(unrealized_gain).try_into().unwrap(),
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
            file_name: app.state.portfolio_file.as_deref().map(Path::to_string_lossy).unwrap_or_default().to_string().into(),
            balance: rounded_to_cent(balance).try_into().unwrap(),
            cost_base: rounded_to_cent(cost_base).try_into().unwrap(),
            unrealized_gains: rounded_to_cent(balance - cost_base).try_into().unwrap(),
            holdings: Rc::new(VecModel::from(ui_holdings)).into(),
            cost_basis_tracking: match app.portfolio.cost_basis_tracking {
                CostBasisTracking::Universal => UiCostBasisTracking::Universal,
                CostBasisTracking::PerWallet => UiCostBasisTracking::PerWallet,
            },
            merge_consecutive_trades: app.portfolio.merge_consecutive_trades,
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();

    let cli_arg = env::args_os().nth(1);
    if let Some(arg) = cli_arg.as_ref() {
        if arg == OsStr::new("-v") || arg == OsStr::new("--version") {
            println!("raccoin {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    // Load portfolio from command-line or from previous application state
    if let Some(portfolio_file) = cli_arg.map(OsString::into).or_else(|| app.state.portfolio_file.to_owned()) {
        if let Err(e) = app.load_portfolio(&portfolio_file) {
            println!("Error loading portfolio from {}: {}", portfolio_file.display(), e);
            return Ok(());
        }
        println!("Restored portfolio {}", portfolio_file.display());
    }

    let ui = initialize_ui(&mut app)?;
    app.refresh_ui();

    let app = Rc::new(RefCell::new(app));
    let facade = ui.global::<Facade>();

    facade.on_ui_index_for_transaction({
        let app = app.clone();

        move |tx_index| {
            // todo: This method copies each UiTransaction instance in order to
            // find one by its id. This copying could be avoided if the VecModel
            // provided an as_slice method.
            use slint::Model;
            let ui_index = app.borrow().ui_transactions.iter().position(|tx| {
                tx.id == tx_index
            }).map(|i| i as i32).unwrap_or(-1);
            ui_index
        }
    });

    facade.on_balances_for_currency({
        let app = app.clone();

        move |currency| {
            let app = app.borrow();
            let currency = currency.as_str();

            // collect the balances for the given currency from each source
            let mut balances: Vec<(String, Decimal)> = app.portfolio.wallets.iter().filter_map(|wallet| {
                wallet.balances.get(currency).and_then(|balance|
                    if *balance != Decimal::ZERO {
                        Some((wallet.name.clone(), balance.normalize()))
                    } else {
                        None
                    })
            }).collect();

            // sort descending
            balances.sort_by(|(_, a), (_, b)| b.cmp(a) );

            let balances: Vec<UiBalanceForCurrency> = balances.into_iter().map(|(source, balance)| {
                UiBalanceForCurrency {
                    source: source.into(),
                    balance: balance.to_string().into(),
                }
            }).collect();
            Rc::new(VecModel::from(balances)).into()
        }
    });

    facade.on_balances_for_wallet({
        let app = app.clone();

        move |index| {
            let app = app.borrow();
            let mut balances: Vec<UiBalanceForWallet> = Vec::new();
            if let Some(source) = app.portfolio.wallets.get(index as usize) {
                balances.extend(source.balances.iter().filter_map(|(currency, quantity)| {
                    if *quantity != Decimal::ZERO {
                        Some(UiBalanceForWallet {
                            currency_cmc_id: cmc_id(currency),
                            currency: currency.clone().into(),
                            balance: quantity.normalize().to_string().into(),
                        })
                    } else {
                        None
                    }
                }));
            }
            // todo: would be nice to sort by fiat value
            Rc::new(VecModel::from(balances)).into()
        }
    });

    facade.on_new_portfolio({
        let app = app.clone();

        move || {
            let dialog = rfd::FileDialog::new()
                .set_title("New Portfolio")
                .set_file_name("Portfolio.json")
                .add_filter("Portfolio (JSON)", &["json"]);

            match dialog.save_file() {
                Some(path) => {
                    let mut app = app.borrow_mut();
                    app.portfolio = Portfolio::default();
                    app.save_portfolio(Some(path));
                    app.refresh_transactions();
                    app.refresh_ui();
                }
                _ => {}
            }
        }
    });

    facade.on_load_portfolio({
        let app = app.clone();

        move || {
            let dialog = rfd::FileDialog::new()
                .set_title("Load Portfolio")
                .add_filter("Portfolio (JSON)", &["json"]);

            match dialog.pick_file() {
                Some(path) => {
                    let mut app = app.borrow_mut();
                    if let Err(e) = app.load_portfolio(&path) {
                        println!("Error loading portfolio from {}: {}", path.display(), e);
                    } else {
                        app.refresh_ui();
                    }
                }
                _ => {}
            }
        }
    });

    facade.on_close_portfolio({
        let app = app.clone();

        move || {
            let mut app = app.borrow_mut();
            app.close_portfolio();
            app.refresh_ui();
        }
    });

    facade.on_set_merge_consecutive_trades({
        let app = app.clone();
        move |enabled| {
            let mut app = app.borrow_mut();
            app.portfolio.merge_consecutive_trades = enabled;
            app.refresh_transactions();
            app.refresh_ui();
            app.save_portfolio(None);
        }
    });
    facade.on_set_cost_basis_tracking({
        let app = app.clone();
        move |cost_basis_tracking| {
            let mut app = app.borrow_mut();
            app.portfolio.cost_basis_tracking = match cost_basis_tracking {
                UiCostBasisTracking::Universal => CostBasisTracking::Universal,
                UiCostBasisTracking::PerWallet => CostBasisTracking::PerWallet,
            };
            app.refresh_transactions();
            app.refresh_ui();
            app.save_portfolio(None);
        }
    });

    facade.on_add_wallet({
        let app = app.clone();

        move |name| {
            let mut app = app.borrow_mut();
            let wallet = Wallet::new(name.into());
            app.portfolio.wallets.push(wallet);
            ui_set_wallets(&app);
            app.save_portfolio(None);
        }
    });

    facade.on_remove_wallet({
        let app = app.clone();

        move |index| {
            let mut app = app.borrow_mut();
            app.portfolio.wallets.remove(index as usize);
            app.refresh_transactions();
            app.refresh_ui();
            app.save_portfolio(None);
        }
    });

    facade.on_add_source_csv({
        let app = app.clone();

        move |wallet_index| {
            let mut app = app.borrow_mut();
            let mut dialog = rfd::FileDialog::new()
                .set_title("Add Transaction Source")
                .add_filter("CSV", &["csv"]);

            if let Some(last_source_directory) = &app.state.last_source_directory {
                println!("Using last source directory: {}", last_source_directory.display());
                dialog = dialog.set_directory(last_source_directory);
            }

            if let Some(wallet) = app.portfolio.wallets.get_mut(wallet_index as usize) {
                if let Some(file_name) = dialog.pick_file() {
                    if let Some(source_type) = detect_source_from_file(&file_name) {
                        let source_directory = file_name.parent().unwrap().to_owned();
                        wallet.sources.push(WalletSource {
                            source_type: source_type.id.to_owned(),
                            path: file_name.to_str().unwrap_or_default().to_owned(),
                            name: String::default(),
                            enabled: true,
                            full_path: file_name,
                            transaction_count: 0,
                            transactions: Vec::new(),
                        });
                        app.state.last_source_directory = Some(source_directory);

                        app.refresh_transactions();
                        app.refresh_ui();
                        app.save_portfolio(None);
                    } else {
                        app.report_error("Unrecognized file type. Please consider opening an issue on GitHub!");
                    }
                }
            }
        }
    });

    facade.on_add_source_address({
        let app = app.clone();

        move |wallet_index, source_type, source_path, source_name| {
            let mut app = app.borrow_mut();
            let source_type = source_type.to_string();
            let source_path = source_path.trim();
            let source_name = source_name.trim();

            if source_path.is_empty() {
                app.report_error("Source address cannot be empty.");
                return;
            }

            let source_definition = transaction_source_by_id(&source_type);
            if source_definition.is_none() {
                app.report_error("Unknown source type.");
                return;
            }

            if let Some(wallet) = app.portfolio.wallets.get_mut(wallet_index as usize) {
                wallet.sources.push(WalletSource {
                    source_type,
                    path: source_path.to_owned(),
                    name: source_name.to_owned(),
                    enabled: true,
                    full_path: PathBuf::new(),
                    transaction_count: 0,
                    transactions: Vec::new(),
                });

                app.refresh_transactions();
                app.refresh_ui();
                app.save_portfolio(None);
            }
        }
    });

    facade.on_remove_source({
        let app = app.clone();

        move |wallet_index, source_index| {
            let mut app = app.borrow_mut();
            if let Some(wallet) = app.portfolio.wallets.get_mut(wallet_index as usize) {
                wallet.sources.remove(source_index as usize);
                app.refresh_transactions();
                app.refresh_ui();
                app.save_portfolio(None);
            }
        }
    });

    facade.on_ignore_currency({
        let app = app.clone();

        move |currency| {
            let mut app = app.borrow_mut();
            let currency = currency.to_string();

            if !app.portfolio.ignored_currencies.contains(&currency) {
                app.portfolio.ignored_currencies.push(currency);
                app.portfolio.ignored_currencies.sort();
                app.refresh_transactions();
                app.refresh_ui();
                app.save_portfolio(None);
            }
        }
    });

    facade.on_set_wallet_enabled({
        let app = app.clone();

        move |index, enabled| {
            let mut app = app.borrow_mut();
            if let Some(wallet) = app.portfolio.wallets.get_mut(index as usize) {
                wallet.enabled = enabled;

                app.refresh_transactions();
                app.refresh_ui();
                app.save_portfolio(None);
            }
        }
    });

    facade.on_set_wallet_expanded({
        let app = app.clone();

        move |index, expanded| {
            let mut app = app.borrow_mut();
            if let Some(wallet) = app.portfolio.wallets.get_mut(index as usize) {
                wallet.expanded = expanded;
            }
        }
    });

    facade.on_set_source_enabled({
        let app = app.clone();

        move |wallet_index, source_index, enabled| {
            let mut app = app.borrow_mut();
            if let Some(wallet) = app.portfolio.wallets.get_mut(wallet_index as usize) {
                if let Some(source) = wallet.sources.get_mut(source_index as usize) {
                    source.enabled = enabled;

                    app.refresh_transactions();
                    app.refresh_ui();
                    app.save_portfolio(None);
                }
            }
        }
    });

    facade.on_sync_source({
        let app = app.clone();

        move |wallet_index, source_index| {
            let app_for_future = app.clone();
            let (source_type, source_path) = {
                let app_borrow = app.borrow();
                let source = app_borrow.portfolio.wallets.get(wallet_index as usize)
                    .and_then(|wallet| wallet.sources.get(source_index as usize));

                if source.is_none() {
                    return
                }
                let source = source.unwrap();

                (source.source_type.clone(), source.path.clone())
            };

            slint::spawn_local(async move {
                let transactions = tokio::task::spawn(async move {
                    let source_definition = transaction_source_by_id(&source_type)
                        .ok_or_else(|| anyhow!("Unknown source type {}", source_type))?;
                    let load_async = source_definition
                        .load_async
                        .ok_or_else(|| anyhow!("Sync not supported for this source type"))?;

                    let mut transactions = load_async(source_path).await;

                    let _ = transactions.as_mut().map(|transactions| {
                        transactions.sort_by(|a, b| a.cmp(b) );
                    });
                    transactions
                }).await.unwrap();

                match transactions {
                    Ok(transactions) => {
                        let mut app = app_for_future.borrow_mut();

                        if let Some(source) = app.portfolio.wallets.get_mut(wallet_index as usize)
                            .and_then(|wallet| wallet.sources.get_mut(source_index as usize)) {
                            source.transactions = transactions;

                            app.refresh_transactions();
                            app.refresh_ui();
                            app.save_portfolio(None);
                        }
                    }
                    Err(e) => {
                        // todo: show error in UI
                        println!("Error syncing transactions: {}", e);
                    }
                }
            }).unwrap();
        }
    });

    facade.on_update_price_history({
        let app = app.clone();

        move || {
            let app = app.clone();
            slint::spawn_local(update_price_history(app)).unwrap();
        }
    });

    facade.on_stop_update_price_history({
        let app = app.clone();

        move || {
            app.borrow_mut().stop_update_price_history = true;
        }
    });

    fn save_csv_file(title: &str, starting_file_name: &str) -> Option<PathBuf> {
        let dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(starting_file_name)
            .add_filter("CSV", &["csv"]);
        dialog.save_file()
    }

    facade.on_export_summary({
        let app = app.clone();

        move |index| {
            let app = app.borrow();
            let report = app.reports.get(index as usize).expect("report index should be valid");
            let file_name = format!("report_summary_{}.csv", report.year);

            match save_csv_file("Export Report Summary (CSV)", &file_name) {
                Some(path) => {
                    // todo: provide this feedback in the UI
                    match save_summary_to_csv(report, &path) {
                        Ok(_) => {
                            println!("Saved summary to {}", path.display());
                        }
                        Err(e) => {
                            println!("Error saving summary to {}: {}", path.display(), e);
                        }
                    }
                }
                _ => {}
            }
        }
    });

    facade.on_export_capital_gains({
        let app = app.clone();

        move |index| {
            let app = app.borrow();
            let report = app.reports.get(index as usize).expect("report index should be valid");
            let file_name = format!("capital_gains_{}.csv", report.year);

            match save_csv_file("Export Capital Gains (CSV)", &file_name) {
                Some(path) => {
                    // todo: provide this feedback in the UI
                    match fifo::save_gains_to_csv(&report.gains, &path) {
                        Ok(_) => {
                            println!("Saved gains to {}", path.display());
                        }
                        Err(e) => {
                            println!("Error saving gains to {}: {}", path.display(), e);
                        }
                    }
                }
                _ => {}
            }
        }
    });

    facade.on_export_all({
        let app = app.clone();

        move || {
            let mut app = app.borrow_mut();
            let mut dialog = rfd::FileDialog::new()
                .set_title("Target Directory");

            if let Some(last_export_directory) = &app.state.last_export_directory {
                println!("Using last export directory: {}", last_export_directory.display());
                dialog = dialog.set_directory(last_export_directory);
            }

            match dialog.pick_folder() {
                Some(path) => {
                    // todo: provide this feedback in the UI
                    match export_all_to(&app, &path) {
                        Ok(_) => {
                            println!("Exported to {}", path.display());
                        }
                        Err(e) => {
                            println!("Error exporting to {}: {}", path.display(), e);
                        }
                    }
                    app.state.last_export_directory = Some(path);
                }
                _ => {}
            }
        }
    });

    facade.on_export_transactions_csv({
        let app = app.clone();

        move || {
            match save_csv_file("Export Transactions (CSV)", "transactions.csv") {
                Some(path) => {
                    let app = app.borrow();
                    // todo: provide this feedback in the UI
                    match ctc::save_transactions_to_ctc_csv(&app.transactions, &path) {
                        Ok(_) => {
                            println!("Exported transactions to {}", path.display());
                        }
                        Err(e) => {
                            println!("Error exporting transactions to {}: {}", path.display(), e);
                        }
                    }
                }
                _ => {}
            }
        }
    });

    facade.on_export_transactions_json({
        let app = app.clone();

        move || {
            let dialog = rfd::FileDialog::new()
                .set_title("Export Transactions (JSON)")
                .set_file_name("transactions.json")
                .add_filter("JSON", &["json"]);

            match dialog.save_file() {
                Some(path) => {
                    let app = app.borrow();
                    // todo: provide this feedback in the UI
                    match base::save_transactions_to_json(&app.transactions, &path) {
                        Ok(_) => {
                            println!("Exported transactions to {}", path.display());
                        }
                        Err(e) => {
                            println!("Error exporting transactions to {}: {}", path.display(), e);
                        }
                    }
                }
                _ => {}
            }
        }
    });

    facade.on_remove_notification({
        let app = app.clone();

        move |index| {
            let app = app.borrow();
            app.remove_notification(index as usize);
        }
    });

    facade.on_transaction_filter_changed({
        let app = app.clone();

        move || {
            let mut app = app.borrow_mut();
            let ui = app.ui();
            let facade = ui.global::<Facade>();

            app.transaction_filters.clear();

            if facade.get_wallet_filter() >= 0 {
                app.transaction_filters.push(TransactionFilter::WalletIndex(facade.get_wallet_filter() as usize));
            }

            if !facade.get_currency_filter().is_empty() {
                app.transaction_filters.push(TransactionFilter::Currency(facade.get_currency_filter().into()));
            }

            if facade.get_text_filter().len() > 0 {
                let re = RegexBuilder::new(&regex::escape(facade.get_text_filter().as_str()))
                    .case_insensitive(true)
                    .build().unwrap();
                app.transaction_filters.push(TransactionFilter::Text(re));
            }

            if facade.get_warnings_filter() {
                app.transaction_filters.push(TransactionFilter::HasGainError);
            }

            ui_set_transactions(&app);
        }
    });

    ui.show()?;

    // Wrap the call to run_event_loop to ensure presence of a Tokio run-time.
    tokio::task::block_in_place(slint::run_event_loop).unwrap();

    ui.hide()?;

    app.borrow().save_state()?;

    Ok(())
}
