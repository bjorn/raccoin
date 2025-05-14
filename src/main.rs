#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod base;
mod binance;
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
mod liquid;
mod mycelium;
mod poloniex;
mod time;
mod trezor;

use anyhow::{Context, Result, anyhow};
use base::{Operation, Amount, Transaction, cmc_id, PriceHistory};
use chrono::{Duration, Datelike, Utc, TimeZone, Local};
use directories::ProjectDirs;
use raccoin_ui::*;
use fifo::{FIFO, CapitalGain};
use regex::{Regex, RegexBuilder};
use rust_decimal_macros::dec;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use slice_group_by::GroupByMut;
use slint::{VecModel, StandardListViewItem, SharedString, ModelRc};
use strum::{EnumIter, IntoEnumIterator};
use std::{rc::Rc, path::{Path, PathBuf}, env, collections::HashMap, cmp::{Eq, Ordering}, hash::Hash, default::Default, ffi::OsString, sync::{Arc, Mutex}, fs::File, io::BufReader};

fn rounded_to_cent(amount: Decimal) -> Decimal {
    amount.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

#[derive(EnumIter, Serialize, Deserialize, Clone, Copy)]
enum TransactionsSourceType {
    BitcoinAddresses,
    BitcoinXpubs,
    BitcoinCoreCsv,
    BitcoinDeCsv,
    BitonicCsv,     // todo: remove custom format
    BitstampCsv,
    BittrexOrderHistoryCsv,
    BittrexTransactionHistoryCsv,
    CtcImportCsv,
    ElectrumCsv,
    EthereumAddress,
    Json,
    MyceliumCsv,
    PeercoinCsv,
    FtxDepositsCsv,
    FtxWithdrawalsCsv,
    FtxTradesCsv,
    LiquidDepositsCsv,
    LiquidTradesCsv,
    LiquidWithdrawalsCsv,
    PoloniexDepositsCsv,
    PoloniexDepositsSupportCsv,
    PoloniexTradesCsv,
    PoloniexTradesSupportCsv,
    PoloniexWithdrawalsCsv,
    PoloniexWithdrawalsSupportCsv,
    StellarAccount,
    BinanceBnbConvertCsv,  // todo: document custom format
    BinanceSpotTradeHistoryCsv,
    BinanceTransactionHistoryCsv,
    ReddcoinCoreCsv,
    TrezorCsv,
    TrezorJson,
}

fn csv_file_has_headers(path: &Path, delimiter: u8, skip_lines: usize, headers: &[&str]) -> Result<bool> {
    let file = File::open(path)?;
    let mut buf_reader = BufReader::new(file);

    use std::io::BufRead;

    let mut line = String::new();
    for _ in 0..skip_lines {
        buf_reader.read_line(&mut line)?;
    }

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(buf_reader);

    Ok(rdr.headers().map_or(false, |s| s == headers))
}

impl TransactionsSourceType {
    fn detect_from_file(path: &Path) -> Option<Self> {
        Self::iter().find(|source_type| {
            source_type.delimiter().is_some_and(|delimiter| {
                csv_file_has_headers(path, delimiter, source_type.skip_lines(), source_type.headers()).is_ok_and(|x| x)
            })
        })
    }

    fn delimiter(&self) -> Option<u8> {
        match self {
            TransactionsSourceType::BitcoinAddresses |
            TransactionsSourceType::BitcoinXpubs |
            TransactionsSourceType::EthereumAddress |
            TransactionsSourceType::StellarAccount |
            TransactionsSourceType::TrezorJson |
            TransactionsSourceType::Json => None,

            TransactionsSourceType::BitcoinDeCsv |
            TransactionsSourceType::TrezorCsv => Some(b';'),

            _ => Some(b','),
        }
    }

    fn skip_lines(&self) -> usize {
        match self {
            TransactionsSourceType::LiquidTradesCsv => 2,
            _ => 0,
        }
    }

    fn headers(&self) -> &[&str] {
        match self {
            TransactionsSourceType::BitcoinAddresses |
            TransactionsSourceType::BitcoinXpubs |
            TransactionsSourceType::EthereumAddress |
            TransactionsSourceType::StellarAccount |
            TransactionsSourceType::TrezorJson |
            TransactionsSourceType::Json => &[],

            TransactionsSourceType::BitcoinDeCsv => &["Date", "Type", "Currency", "Reference", "BTC-address", "Price", "unit (rate)", "BTC incl. fee", "amount before fee", "unit (amount before fee)", "BTC excl. Bitcoin.de fee", "amount after Bitcoin.de-fee", "unit (amount after Bitcoin.de-fee)", "Incoming / Outgoing", "Account balance"],
            TransactionsSourceType::TrezorCsv => &["Timestamp", "Date", "Time", "Type", "Transaction ID", "Fee", "Fee unit", "Address", "Label", "Amount", "Amount unit", "Fiat (EUR)", "Other"],

            TransactionsSourceType::BitcoinCoreCsv => &["Confirmed", "Date", "Type", "Label", "Address", "Amount (BTC)", "ID"],
            TransactionsSourceType::PeercoinCsv => &["Confirmed", "Date", "Type", "Label", "Address", "Amount (PPC)", "ID"],
            TransactionsSourceType::ReddcoinCoreCsv => &["Confirmed", "Date", "Type", "Label", "Address", "Amount (RDD)", "ID"],
            TransactionsSourceType::BitonicCsv => &["Date", "Action", "Amount", "Price"],
            TransactionsSourceType::BitstampCsv => &["Type", "Datetime", "Account", "Amount", "Value", "Rate", "Fee", "Sub Type"],
            TransactionsSourceType::BittrexOrderHistoryCsv => &["Date", "Market", "Side", "Type", "Price", "Quantity", "Total"],
            TransactionsSourceType::BittrexTransactionHistoryCsv => &["Date", "Currency", "Type", "Address", "Memo/Tag", "TxId", "Amount"],
            TransactionsSourceType::CtcImportCsv => &["Timestamp (UTC)", "Type", "Base Currency", "Base Amount", "Quote Currency (Optional)", "Quote Amount (Optional)", "Fee Currency (Optional)", "Fee Amount (Optional)", "From (Optional)", "To (Optional)", "Blockchain (Optional)", "ID (Optional)", "Description (Optional)", "Reference Price Per Unit (Optional)", "Reference Price Currency (Optional)"],
            TransactionsSourceType::ElectrumCsv => &["transaction_hash", "label", "confirmations", "value", "fiat_value", "fee", "fiat_fee", "timestamp"],
            TransactionsSourceType::MyceliumCsv => &["Account", "Transaction ID", "Destination Address", "Timestamp", "Value", "Currency", "Transaction Label"],
            TransactionsSourceType::FtxDepositsCsv => &[" ", "Time", "Coin", "Amount", "Status", "Additional info", "Transaction ID"],
            TransactionsSourceType::FtxWithdrawalsCsv => &[" ", "Time", "Coin", "Amount", "Destination", "Status", "Transaction ID", "fee"],
            TransactionsSourceType::FtxTradesCsv => &["ID", "Time", "Market", "Side", "Order Type", "Size", "Price", "Total", "Fee", "Fee Currency", "TWAP"],
            TransactionsSourceType::LiquidDepositsCsv => &["ID", "Type", "Amount", "Status", "Created (YY/MM/DD)", "Hash"],
            TransactionsSourceType::LiquidTradesCsv => &["Quoted currency", "Base currency", "Qex/liquid", "Execution", "Type", "Date", "Open qty", "Price", "Fee", "Fee currency", "Amount", "Trade side"],
            TransactionsSourceType::LiquidWithdrawalsCsv => &["ID", "Wallet label", "Amount", "Created On", "Transfer network", "Status", "Address", "Liquid Fee", "Network Fee", "Broadcasted At", "Hash"],
            TransactionsSourceType::PoloniexDepositsCsv => &["Currency", "Amount", "Address", "Date", "Status"],
            TransactionsSourceType::PoloniexDepositsSupportCsv => &["", "timestamp", "currency", "amount", "address", "status"],
            TransactionsSourceType::PoloniexTradesCsv => &["Date", "Market", "Type", "Side", "Price", "Amount", "Total", "Fee", "Order Number", "Fee Currency", "Fee Total"],
            TransactionsSourceType::PoloniexTradesSupportCsv => &["", "timestamp", "trade_id", "market", "wallet", "side", "price", "amount", "fee", "fee_currency", "fee_total"],
            TransactionsSourceType::PoloniexWithdrawalsCsv => &["Fee Deducted", "Date", "Currency", "Amount", "Amount-Fee", "Address", "Status"],
            TransactionsSourceType::PoloniexWithdrawalsSupportCsv => &["", "timestamp", "currency", "amount", "fee_deducted", "status"],
            TransactionsSourceType::BinanceBnbConvertCsv => &["Date", "Coin", "Amount", "Fee (BNB)", "Converted BNB"],
            TransactionsSourceType::BinanceSpotTradeHistoryCsv => &["Date(UTC)", "Pair", "Side", "Price", "Executed", "Amount", "Fee"],
            TransactionsSourceType::BinanceTransactionHistoryCsv => &["User_ID", "UTC_Time", "Account", "Operation", "Coin", "Change", "Remark"],
        }
    }
}

impl ToString for TransactionsSourceType {
    fn to_string(&self) -> String {
        match self {
            TransactionsSourceType::BitcoinAddresses => "Bitcoin Address(es)".to_owned(),
            TransactionsSourceType::BitcoinXpubs => "Bitcoin HD Wallet(s)".to_owned(),
            TransactionsSourceType::BitcoinCoreCsv => "Bitcoin Core (CSV)".to_owned(),
            TransactionsSourceType::BitcoinDeCsv => "bitcoin.de (CSV)".to_owned(),
            TransactionsSourceType::BitonicCsv => "Bitonic (CSV)".to_owned(),
            TransactionsSourceType::BitstampCsv => "Bitstamp (CSV)".to_owned(),
            TransactionsSourceType::BittrexOrderHistoryCsv => "Bittrex Order History (CSV)".to_owned(),
            TransactionsSourceType::BittrexTransactionHistoryCsv => "Bittrex Transaction History (CSV)".to_owned(),
            TransactionsSourceType::ElectrumCsv => "Electrum (CSV)".to_owned(),
            TransactionsSourceType::EthereumAddress => "Ethereum Address".to_owned(),
            TransactionsSourceType::Json => "JSON".to_owned(),
            TransactionsSourceType::CtcImportCsv => "CryptoTaxCalculator import (CSV)".to_owned(),
            TransactionsSourceType::MyceliumCsv => "Mycelium (CSV)".to_owned(),
            TransactionsSourceType::PeercoinCsv => "Peercoin Qt (CSV)".to_owned(),
            TransactionsSourceType::FtxDepositsCsv => "FTX Deposits (CSV)".to_owned(),
            TransactionsSourceType::FtxWithdrawalsCsv => "FTX Withdrawal (CSV)".to_owned(),
            TransactionsSourceType::FtxTradesCsv => "FTX Trades (CSV)".to_owned(),
            TransactionsSourceType::LiquidDepositsCsv => "Liquid Deposits (CSV)".to_owned(),
            TransactionsSourceType::LiquidTradesCsv => "Liquid Trades (CSV)".to_owned(),
            TransactionsSourceType::LiquidWithdrawalsCsv => "Liquid Withdrawals (CSV)".to_owned(),
            TransactionsSourceType::PoloniexDepositsCsv => "Poloniex Deposits (CSV)".to_owned(),
            TransactionsSourceType::PoloniexDepositsSupportCsv => "Poloniex Deposits from Support (CSV)".to_owned(),
            TransactionsSourceType::PoloniexTradesCsv => "Poloniex Trades (CSV)".to_owned(),
            TransactionsSourceType::PoloniexTradesSupportCsv => "Poloniex Trades from Support (CSV)".to_owned(),
            TransactionsSourceType::PoloniexWithdrawalsCsv => "Poloniex Withdrawals (CSV)".to_owned(),
            TransactionsSourceType::PoloniexWithdrawalsSupportCsv => "Poloniex Withdrawals from Support (CSV)".to_owned(),
            TransactionsSourceType::StellarAccount => "Stellar Account".to_owned(),
            TransactionsSourceType::BinanceBnbConvertCsv => "Binance BNB Convert (CSV)".to_owned(),
            TransactionsSourceType::BinanceSpotTradeHistoryCsv => "Binance Spot Trade History (CSV)".to_owned(),
            TransactionsSourceType::BinanceTransactionHistoryCsv => "Binance Transaction History (CSV)".to_owned(),
            TransactionsSourceType::ReddcoinCoreCsv => "Reddcoin Core (CSV)".to_owned(),
            TransactionsSourceType::TrezorCsv => "Trezor (CSV)".to_owned(),
            TransactionsSourceType::TrezorJson => "Trezor (JSON)".to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TransactionSource {
    source_type: TransactionsSourceType,
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
    sources: Vec<TransactionSource>,
    /// Currency balances, as calculated based on the transactions imported from this source.
    #[serde(skip)]
    balances: HashMap<String, Decimal>,
}

impl Wallet {
    fn new(name: String) -> Self {
        Self {
            name,
            enabled: true,
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
                (Some(amount), None) => &amount.currency == currency,
                (Some(incoming), Some(outgoing)) => {
                    &incoming.currency == currency || &outgoing.currency == currency
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

    transaction_filters: Vec<TransactionFilter>,

    ui_weak: slint::Weak<AppWindow>,
}

impl App {
    fn new() -> Self {
        let mut price_history = PriceHistory::new();

        let project_dirs = ProjectDirs::from("org", "raccoin",  "Raccoin");
        let state = project_dirs.as_ref().and_then(|dirs| {
            let config_dir = dirs.config_local_dir();
            std::fs::create_dir_all(config_dir).map(|_| config_dir.join("state.json")).ok()
        }).and_then(|state_file| std::fs::read_to_string(state_file).ok()).and_then(|json| {
            serde_json::from_str::<AppState>(&json).ok()
        }).unwrap_or_default();

        Self {
            project_dirs,
            state,
            portfolio: Portfolio::default(),
            transactions: Vec::new(),
            reports: Vec::new(),
            price_history,

            transaction_filters: Vec::default(),

            ui_weak: slint::Weak::default(),
        }
    }

    fn load_portfolio(&mut self, file_path: &Path) -> Result<()> {
        // todo: report portfolio loading error in UI
        let mut portfolio: Portfolio = serde_json::from_str(&std::fs::read_to_string(file_path)?)?;
        let portfolio_path = file_path.parent().unwrap_or(Path::new(""));
        portfolio.wallets.iter_mut().for_each(|w| w.sources.iter_mut().for_each(|source| {
            match source.source_type {
                TransactionsSourceType::BitcoinAddresses |
                TransactionsSourceType::BitcoinXpubs |
                TransactionsSourceType::EthereumAddress |
                TransactionsSourceType::StellarAccount => {}
                _ => {
                    source.full_path = portfolio_path.join(&source.path);
                }
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
                match source.source_type {
                    TransactionsSourceType::BitcoinAddresses |
                    TransactionsSourceType::BitcoinXpubs |
                    TransactionsSourceType::EthereumAddress |
                    TransactionsSourceType::StellarAccount => {}
                    _ => {
                        if let Some(relative_path) = pathdiff::diff_paths(&source.full_path, portfolio_path) {
                            source.path = relative_path.to_str().unwrap_or_default().to_owned();
                        }
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
        self.transactions = load_transactions(&mut self.portfolio, &self.price_history).unwrap_or_default();
        self.reports = calculate_tax_reports(&mut self.transactions);
    }

    fn ui(&self) -> AppWindow {
        self.ui_weak.unwrap()
    }

    fn wallets_model(&self) -> ModelRc<UiWallet> {
        self.ui().global::<Facade>().get_wallets()
    }

    fn transactions_model(&self) -> ModelRc<UiTransaction> {
        self.ui().global::<Facade>().get_transactions()
    }

    fn report_years_model(&self) -> ModelRc<StandardListViewItem> {
        self.ui().global::<Facade>().get_report_years()
    }

    fn reports_model(&self) -> ModelRc<UiTaxReport> {
        self.ui().global::<Facade>().get_reports()
    }

    fn report_error(&self, message: &str) {
        let notifications_rc = self.ui().global::<Facade>().get_notifications();
        let notifications = slint::Model::as_any(&notifications_rc).downcast_ref::<VecModel<UiNotification>>().unwrap();
        notifications.push(UiNotification {
            notification_type: UiNotificationType::Error,
            message: message.into(),
        });
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

fn load_transactions(portfolio: &mut Portfolio, price_history: &PriceHistory) -> Result<Vec<Transaction>> {
    let (wallets, ignored_currencies) = (&mut portfolio.wallets, &portfolio.ignored_currencies);
    let mut transactions = Vec::new();

    for (wallet_index, wallet) in wallets.iter_mut().enumerate() {
        let mut wallet_transactions = Vec::new();

        for source in wallet.sources.iter_mut() {
            if !source.enabled || !wallet.enabled {
                source.transaction_count = 0;
                continue
            }

            let source_txs = match source.source_type {
                TransactionsSourceType::BitcoinAddresses |
                TransactionsSourceType::BitcoinXpubs |
                TransactionsSourceType::EthereumAddress |
                TransactionsSourceType::StellarAccount => {
                    anyhow::Ok(source.transactions.clone())
                }
                TransactionsSourceType::BitcoinCoreCsv => {
                    bitcoin_core::load_bitcoin_core_csv(&source.full_path)
                }
                TransactionsSourceType::BitcoinDeCsv => {
                    bitcoin_de::load_bitcoin_de_csv(&source.full_path)
                }
                TransactionsSourceType::BitonicCsv => {
                    bitonic::load_bitonic_csv(&source.full_path)
                }
                TransactionsSourceType::BitstampCsv => {
                    bitstamp::load_bitstamp_csv(&source.full_path)
                }
                TransactionsSourceType::BittrexOrderHistoryCsv => {
                    bittrex::load_bittrex_order_history_csv(&source.full_path)
                }
                TransactionsSourceType::BittrexTransactionHistoryCsv => {
                    bittrex::load_bittrex_transaction_history_csv(&source.full_path)
                }
                TransactionsSourceType::ElectrumCsv => {
                    electrum::load_electrum_csv(&source.full_path)
                }
                TransactionsSourceType::Json => {
                    base::load_transactions_from_json(&source.full_path)
                }
                TransactionsSourceType::CtcImportCsv => {
                    ctc::load_ctc_csv(&source.full_path)
                }
                TransactionsSourceType::MyceliumCsv => {
                    mycelium::load_mycelium_csv(&source.full_path)
                }
                TransactionsSourceType::PeercoinCsv => {
                    bitcoin_core::load_peercoin_csv(&source.full_path)
                }
                TransactionsSourceType::FtxDepositsCsv => {
                    ftx::load_ftx_deposits_csv(&source.full_path)
                }
                TransactionsSourceType::FtxWithdrawalsCsv => {
                    ftx::load_ftx_withdrawals_csv(&source.full_path)
                }
                TransactionsSourceType::FtxTradesCsv => {
                    ftx::load_ftx_trades_csv(&source.full_path)
                }
                TransactionsSourceType::LiquidDepositsCsv => {
                    liquid::load_liquid_deposits_csv(&source.full_path)
                }
                TransactionsSourceType::LiquidTradesCsv => {
                    liquid::load_liquid_trades_csv(&source.full_path)
                }
                TransactionsSourceType::LiquidWithdrawalsCsv => {
                    liquid::load_liquid_withdrawals_csv(&source.full_path)
                }
                TransactionsSourceType::PoloniexDepositsCsv |
                TransactionsSourceType::PoloniexDepositsSupportCsv => {
                    poloniex::load_poloniex_deposits_csv(&source.full_path)
                }
                TransactionsSourceType::PoloniexTradesCsv |
                TransactionsSourceType::PoloniexTradesSupportCsv => {
                    poloniex::load_poloniex_trades_csv(&source.full_path)
                }
                TransactionsSourceType::PoloniexWithdrawalsCsv |
                TransactionsSourceType::PoloniexWithdrawalsSupportCsv => {
                    poloniex::load_poloniex_withdrawals_csv(&source.full_path)
                }
                TransactionsSourceType::BinanceBnbConvertCsv => {
                    binance::load_binance_bnb_convert_csv(&source.full_path)
                }
                TransactionsSourceType::BinanceSpotTradeHistoryCsv => {
                    binance::load_binance_spot_trades_csv(&source.full_path)
                }
                TransactionsSourceType::BinanceTransactionHistoryCsv => {
                    binance::load_binance_transaction_records_csv(&source.full_path)
                }
                TransactionsSourceType::ReddcoinCoreCsv => {
                    bitcoin_core::load_reddcoin_core_csv(&source.full_path)
                }
                TransactionsSourceType::TrezorCsv => {
                    trezor::load_trezor_csv(&source.full_path)
                }
                TransactionsSourceType::TrezorJson => {
                    trezor::load_trezor_json(&source.full_path)
                }
            };

            match source_txs {
                Ok(mut source_transactions) => {
                    // sort transactions
                    source_transactions.sort_by(|a, b| a.cmp(b) );

                    // merge consecutive trades that are really the same order
                    if portfolio.merge_consecutive_trades {
                        merge_consecutive_trades(&mut source_transactions);
                    }

                    // remove transactions with ignored currencies
                    source_transactions.retain_mut(|tx| {
                        let retain_tx = match tx.incoming_outgoing() {
                            (None, None) => true,
                            (None, Some(amount)) |
                            (Some(amount), None) => !ignored_currencies.contains(&amount.currency),
                            (Some(incoming), Some(outgoing)) => {
                                // Trades can only be ignored, if both the incoming and outgoing currencies are ignored
                                !(ignored_currencies.contains(&incoming.currency) && ignored_currencies.contains(&outgoing.currency))
                            }
                        };

                        retain_tx || tx.fee.take().is_some_and(|fee| {
                            if !ignored_currencies.contains(&fee.currency) {
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
    transactions.sort_by(|a, b| a.cmp(b) );

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
    estimate_transaction_values(&mut transactions, price_history);

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
                        }
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
            }
            _ => {}
        }
    }

    unmatched_sends_receives.iter().for_each(|unmatched_send| {
        let tx = &mut transactions[*unmatched_send];
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

    for (send_index, receive_index) in matching_pairs {
        transactions[send_index].matching_tx = Some(receive_index);
        transactions[receive_index].matching_tx = Some(send_index);

        // Derive the fee based on received amount and sent amount
        let adjusted = match (&transactions[send_index].operation, &transactions[receive_index].operation) {
            (Operation::Send(sent), Operation::Receive(received)) if received.quantity < sent.quantity => {
                assert!(sent.currency == received.currency);

                let implied_fee = Amount::new(sent.quantity - received.quantity, sent.currency.clone());
                match &transactions[send_index].fee {
                    Some(existing_fee) => {
                        if existing_fee.currency != implied_fee.currency {
                            println!("warning: send/receive amounts imply fee, but existing fee is set in a different currency for transaction {:?}", transactions[send_index]);
                            None
                        } else if existing_fee.quantity != implied_fee.quantity {
                            println!("warning: replacing existing fee {:} with implied fee of {:} and adjusting sent amount to {:}", existing_fee, implied_fee, received);
                            Some((received.clone(), implied_fee))
                        } else {
                            println!("warning: fee {:} appears to have been included in the sent amount {:}, adjusting sent amount to {:}", existing_fee, sent, received);
                            Some((received.clone(), implied_fee))
                        }
                    }
                    None => {
                        println!("warning: a fee of {:} appears to have been included in the sent amount {:}, adjusting sent amount to {:} and setting fee", implied_fee, sent, received);
                        Some((received.clone(), implied_fee))
                    }
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
        let gains = fifo.process(txs);

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

        // Make sure there is an entry for each held currency, even if it didn't generate gains or losses
        fifo.holdings().iter().for_each(|(currency, holdings)| {
            if !holdings.is_empty() {
                let _ = summary_for(&mut currencies, currency);
            }
        });

        currencies.iter_mut().for_each(|summary| {
            summary.balance_end = fifo.currency_balance(&summary.currency);
            summary.cost_end = fifo.currency_cost_base(&summary.currency);
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

    let source_types: Vec<SharedString> = TransactionsSourceType::iter().map(|s| SharedString::from(s.to_string())).collect();
    facade.set_source_types(Rc::new(VecModel::from(source_types)).into());

    facade.set_wallets(ModelRc::new(VecModel::<UiWallet>::default()));
    facade.set_transactions(ModelRc::new(VecModel::<UiTransaction>::default()));
    facade.set_report_years(ModelRc::new(VecModel::<StandardListViewItem>::default()));
    facade.set_reports(ModelRc::new(VecModel::<UiTaxReport>::default()));
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
                println!("No explorer URL defind for blockchain: {}", blockchain);
                Ok(())
            }
        };
    });

    Ok(ui)
}

fn ui_set_wallets(app: &App) {
    let ui_wallets: Vec<UiWallet> = app.portfolio.wallets.iter().map(|wallet| {
        let ui_sources: Vec<UiTransactionSource> = wallet.sources.iter().map(|source| {
            UiTransactionSource {
                source_type: source.source_type.to_string().into(),
                name: source.name.clone().into(),
                path: source.path.clone().into(),
                enabled: source.enabled,
                can_sync: match &source.source_type {
                    TransactionsSourceType::BitcoinAddresses |
                    TransactionsSourceType::BitcoinXpubs |
                    TransactionsSourceType::EthereumAddress |
                    TransactionsSourceType::StellarAccount => true,
                    _ => false,
                },
                transaction_count: source.transaction_count as i32,
            }
        }).collect();

        UiWallet {
            // source_type: source.source_type.to_string().into(),
            name: wallet.name.clone().into(),
            enabled: wallet.enabled,
            transaction_count: wallet.transaction_count() as i32,
            sources: Rc::new(VecModel::from(ui_sources)).into(),
        }
    }).collect();

    let wallets_model_rc = app.wallets_model();
    let wallets_model = slint::Model::as_any(&wallets_model_rc).downcast_ref::<VecModel<UiWallet>>().unwrap();
    wallets_model.set_vec(ui_wallets);
}

fn ui_set_transactions(app: &App) {
    let wallets = &app.portfolio.wallets;
    let transactions = &app.transactions;
    let filters = &app.transaction_filters;
    let mut transaction_warning_count = 0;

    let mut ui_transactions = Vec::new();

    for transaction in transactions {
        let filters_match = |transaction: &Transaction| {
            filters.iter().all(|filter| filter.matches(transaction))
        };

        if !filters_match(transaction) &&
            (!transaction.operation.is_send() ||
             !transaction.matching_tx.map(|index| filters_match(&transactions[index])).unwrap_or(false))
        {
            continue;
        }

        let wallet = wallets.get(transaction.wallet_index);
        let wallet_name: Option<SharedString> = wallet.map(|source| source.name.clone().into());

        let mut value = transaction.value.as_ref();
        let mut description = transaction.description.clone();
        let mut tx_hash = transaction.tx_hash.as_ref();
        let mut blockchain = transaction.blockchain.as_ref();

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
            Operation::Send(send_amount) => {
                // matching_tx has to be set at this point, otherwise it should have been a Sell
                let matching_receive = &transactions[transaction.matching_tx.expect("Send should have matched a Receive transaction")];
                if let Operation::Receive(receive_amount) = &matching_receive.operation {
                    let receive_wallet = wallets.get(matching_receive.wallet_index);
                    let receive_wallet_name = receive_wallet.map(|source| source.name.clone().into());

                    value = value.or(matching_receive.value.as_ref());
                    tx_hash = tx_hash.or(matching_receive.tx_hash.as_ref());
                    blockchain = blockchain.or(matching_receive.blockchain.as_ref());
                    description = match (description, &matching_receive.description) {
                        (Some(s), Some(r)) => Some(s + ", " + r),
                        (Some(s), None) => Some(s),
                        (None, Some(r)) => Some(r.to_owned()),
                        (None, None) => None,
                    };

                    (UiTransactionType::Transfer, Some(send_amount), Some(receive_amount), wallet_name, receive_wallet_name)
                } else {
                    unreachable!("Send was matched with a non-Receive transaction");
                }
            }
            Operation::Receive(_) => {
                assert!(transaction.matching_tx.is_some(), "Unmatched Receive should have been changed to Buy");
                continue;   // added as a Transfer when handling the Send
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
            Operation::IncomingGift(amount) => {
                (UiTransactionType::Gift, None, Some(amount), None, wallet_name)
            }
            Operation::OutgoingGift(amount) => {
                (UiTransactionType::Gift, Some(amount), None, wallet_name, None)
            }
            Operation::RealizedProfit(amount) => {
                (UiTransactionType::RealizedPnl, None, Some(amount), None, wallet_name)
            }
            Operation::RealizedLoss(amount) => {
                (UiTransactionType::RealizedPnl, Some(amount), None, wallet_name, None)
            }
            Operation::Spam(amount) => {
                (UiTransactionType::Spam, None, Some(amount), None, wallet_name)
            }
        };

        let (gain, gain_error) = match &transaction.gain {
            Some(Ok(gain)) => (*gain, None),
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
            fee: transaction.fee.as_ref().map_or_else(String::default, Amount::to_string).into(),
            value: value.map_or_else(String::default, Amount::to_string).into(),
            gain: rounded_to_cent(gain).try_into().unwrap(),
            gain_error: gain_error.unwrap_or_default().into(),
            description: description.unwrap_or_default().into(),
            tx_hash: tx_hash.map(|s| s.to_owned()).unwrap_or_default().into(),
            blockchain: blockchain.map(|s| s.to_owned()).unwrap_or_default().into(),
        });
    }

    let transactions_model_rc = app.transactions_model();
    let transactions_model = slint::Model::as_any(&transactions_model_rc).downcast_ref::<VecModel<UiTransaction>>().unwrap();
    transactions_model.set_vec(ui_transactions);
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
    let report_years_model_rc = app.report_years_model();
    let report_years_model = slint::Model::as_any(&report_years_model_rc).downcast_ref::<VecModel<StandardListViewItem>>().unwrap();
    report_years_model.set_vec(report_years);

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

    let reports_model_rc = app.reports_model();
    let reports_model = slint::Model::as_any(&reports_model_rc).downcast_ref::<VecModel<UiTaxReport>>().unwrap();
    reports_model.set_vec(ui_reports);
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
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();

    // Load portfolio from command-line or from previous application state
    if let Some(portfolio_file) = env::args_os().nth(1).map(OsString::into).or_else(|| app.state.portfolio_file.to_owned()) {
        if let Err(e) = app.load_portfolio(&portfolio_file) {
            println!("Error loading portfolio from {}: {}", portfolio_file.display(), e);
            return Ok(());
        }
    }

    let ui = initialize_ui(&mut app)?;
    app.refresh_ui();

    let app = Arc::new(Mutex::new(app));
    let facade = ui.global::<Facade>();

    facade.on_ui_index_for_transaction({
        let app = app.clone();

        move |tx_index| {
            // todo: This method copies each UiTransaction instance in order to
            // find one by its id. This copying could be avoided if the VecModel
            // provided an as_slice method.
            use slint::Model;
            let ui_index = app.lock().unwrap().transactions_model().iter().position(|tx| {
                tx.id == tx_index
            }).map(|i| i as i32).unwrap_or(-1);
            ui_index
        }
    });

    facade.on_balances_for_currency({
        let app = app.clone();

        move |currency| {
            let app = app.lock().unwrap();
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
            let app = app.lock().unwrap();
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
                    let mut app = app.lock().unwrap();
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
                    let mut app = app.lock().unwrap();
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
            let mut app = app.lock().unwrap();
            app.close_portfolio();
            app.refresh_ui();
        }
    });

    facade.on_add_wallet({
        let app = app.clone();

        move |name| {
            let mut app = app.lock().unwrap();
            let wallet = Wallet::new(name.into());
            app.portfolio.wallets.push(wallet);
            ui_set_wallets(&app);
            app.save_portfolio(None);
        }
    });

    facade.on_remove_wallet({
        let app = app.clone();

        move |index| {
            let mut app = app.lock().unwrap();
            app.portfolio.wallets.remove(index as usize);
            app.refresh_transactions();
            app.refresh_ui();
            app.save_portfolio(None);
        }
    });

    facade.on_add_source({
        let app = app.clone();

        move |wallet_index| {
            let mut app = app.lock().unwrap();
            let mut dialog = rfd::FileDialog::new()
                .set_title("Add Transaction Source")
                .add_filter("CSV", &["csv"]);

            if let Some(last_source_directory) = &app.state.last_source_directory {
                println!("Using last source directory: {}", last_source_directory.display());
                dialog = dialog.set_directory(last_source_directory);
            }

            if let Some(wallet) = app.portfolio.wallets.get_mut(wallet_index as usize) {
                if let Some(file_name) = dialog.pick_file() {
                    if let Some(source_type) = TransactionsSourceType::detect_from_file(&file_name) {
                        let source_directory = file_name.parent().unwrap().to_owned();
                        wallet.sources.push(TransactionSource {
                            source_type,
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

    facade.on_remove_source({
        let app = app.clone();

        move |wallet_index, source_index| {
            let mut app = app.lock().unwrap();
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
            let mut app = app.lock().unwrap();
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
            let mut app = app.lock().unwrap();
            if let Some(wallet) = app.portfolio.wallets.get_mut(index as usize) {
                wallet.enabled = enabled;

                app.refresh_transactions();
                app.refresh_ui();
                app.save_portfolio(None);
            }
        }
    });

    facade.on_set_source_enabled({
        let app = app.clone();

        move |wallet_index, source_index, enabled| {
            let mut app = app.lock().unwrap();
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
            let mut app = app.lock().unwrap();
            let source = app.portfolio.wallets.get_mut(wallet_index as usize)
                .and_then(|wallet| wallet.sources.get_mut(source_index as usize));

            if source.is_none() {
                return
            }
            let source = source.unwrap();

            let source_type = source.source_type;
            let source_path = source.path.clone();

            tokio::task::spawn(async move {
                let esplora_client = esplora::async_esplora_client().unwrap();
                let mut transactions = match source_type {
                    TransactionsSourceType::BitcoinAddresses => {
                        esplora::address_transactions(&esplora_client, &source_path.split_ascii_whitespace().map(|s| s.to_owned()).collect()).await
                    }
                    TransactionsSourceType::BitcoinXpubs => {
                        esplora::xpub_addresses_transactions(&esplora_client, &source_path.split_ascii_whitespace().map(|s| s.to_owned()).collect()).await
                    }
                    TransactionsSourceType::EthereumAddress => {
                        etherscan::address_transactions(&source_path).await
                    }
                    TransactionsSourceType::StellarAccount => {
                        horizon::address_transactions(&source_path).await
                    }
                    _ => {
                        Err(anyhow!("Sync not supported for this source type"))
                    }
                };

                let _ = transactions.as_mut().map(|transactions| {
                    transactions.sort_by(|a, b| a.cmp(b) );
                });

                slint::invoke_from_event_loop(move || {
                    let mut app = app_for_future.lock().unwrap();

                    match transactions {
                        Ok(transactions) => {
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
                        },
                    }
                }).unwrap();
            });
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
            let app = app.lock().unwrap();
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
            let app = app.lock().unwrap();
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
            let mut app = app.lock().unwrap();
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
                    let app = app.lock().unwrap();
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
                    let app = app.lock().unwrap();
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
            let app = app.lock().unwrap();
            app.remove_notification(index as usize);
        }
    });

    facade.on_transaction_filter_changed({
        let app = app.clone();

        move || {
            let mut app = app.lock().unwrap();
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

    ui.run()?;

    app.lock().unwrap().save_state()?;

    Ok(())
}
