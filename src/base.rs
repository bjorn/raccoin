use std::{path::Path, fmt, cmp::Ordering, collections::HashMap};

use anyhow::Result;
use chrono::{NaiveDateTime, Duration};
use serde::{Serialize, Deserialize, Deserializer};
use rust_decimal::prelude::*;

/// Maps currencies to their CMC ID
/// todo: support more currencies and load from file
pub(crate) fn cmc_id(currency: &str) -> i32 {
    const CMC_ID_MAP: &[(&str, i32)] = &[
        ("1INCH", 8104),
        ("1INCH (1INCH Token)", 8104),
        ("AQUA", 14112),
        ("BCH", 1831),
        ("BCN", 372),
        ("BCX", 2281),
        ("BNB", 1839),
        ("BTC", 1),
        ("BUSD", 4687),
        ("DASH", 131),
        ("ETH", 1027),
        ("EUR", 2790),
        ("FLR", 7950),
        ("FTC", 8),
        ("FTT", 4195),
        ("IOTA", 1720),
        ("LTC", 2),
        ("LUNA", 20314),
        ("LUNC", 4172),
        ("MANA", 1966),
        ("MANA (Decentraland)", 1966),
        ("NXT", 66),
        ("PPC", 5),
        ("RDD", 118),
        ("SBTC", 2282),
        ("SGB", 12186),
        ("SNT", 1759),
        ("USDC", 3408),
        ("USDT", 825),
        ("XEM", 873),
        ("XLM", 512),
        ("XMR", 328),
        ("XNO", 1567),
        ("XRP", 52),
        ("ZCL", 1447),
        ("ZEC", 1437),
        ("st1INCH (1INCH Token (Staked))", 8104),
    ];
    match CMC_ID_MAP.binary_search_by(|(cur, _)| (*cur).cmp(currency)) {
        Ok(index) => CMC_ID_MAP[index].1,
        Err(_) => -1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GainError {
    InvalidTransactionOrder,    // should only happen in case of a bug
    MissingFiatValue,
    MissingCostBase,
    InvalidFiatValue,
    InvalidSwap,
    InsufficientBalance(Amount),
}

impl fmt::Display for GainError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GainError::InvalidTransactionOrder => f.write_str("Invalid transaction order"),
            GainError::MissingFiatValue => f.write_str("Missing fiat value"),
            GainError::MissingCostBase => f.write_str("Missing cost base"),
            GainError::InvalidFiatValue => f.write_str("Invalid fiat value"),
            GainError::InvalidSwap => f.write_str("Invalid swap"),
            GainError::InsufficientBalance(amount) => f.write_str(format!("Missing {}", amount).as_str()),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Amount {
    pub quantity: Decimal,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
}

impl Amount {
    pub(crate) fn new(quantity: Decimal, currency: String) -> Self {
        Self {
            quantity,
            currency,
            token_id: None,
        }
    }

    pub(crate) fn new_token(token_id: String, contract: String) -> Self {
        Self {
            quantity: Decimal::ONE,
            currency: contract,
            token_id: Some(token_id),
        }
    }

    pub(crate) fn from_fiat(quantity: Decimal) -> Self {
        Self::new(quantity, "EUR".to_owned())
    }

    pub(crate) fn from_satoshis(quantity: u64) -> Self {
        Self {
            quantity: Decimal::new(quantity as i64, 8),
            currency: "BTC".to_owned(),
            token_id: None,
        }
    }

    pub(crate) fn is_fiat(&self) -> bool {
        self.currency == "EUR"
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.quantity.is_zero()
    }

    pub(crate) fn token_currency(&self) -> Option<String> {
        self.token_id.as_ref().map(|token_id| format!("{}:{}", token_id, self.currency))
    }

    pub(crate) fn try_add(&self, amount: &Amount) -> Option<Amount> {
        if self.token_id.is_some() || amount.token_id.is_some() {
            None
        } else if self.currency == amount.currency {
            Some(Amount {
                quantity: self.quantity + amount.quantity,
                currency: self.currency.clone(),
                token_id: None,
            })
        } else {
            None
        }
    }

    pub(crate) fn cmc_id(&self) -> i32 {
        cmc_id(&self.currency)
    }
}

impl TryFrom<&str> for Amount {
    type Error = &'static str;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        // This parses the formats '<amount> <currency>' and '<amount><currency>'
        let mut quantity_str = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let currency = &s[quantity_str.len()..];
        quantity_str = quantity_str.trim_end();

        // Strip commas when necessary, since Decimal::try_from doesn't like those
        let quantity_owned: String;
        if quantity_str.contains(',') {
            quantity_owned = quantity_str.replace(',', "");
            quantity_str = quantity_owned.as_str();
        }

        match Decimal::try_from(quantity_str) {
            Ok(quantity) if !currency.is_empty() => {
                Ok(Amount::new(quantity, currency.to_owned()))
            }
            _ => Err("Invalid format, expected: '<amount> <currency>' or '<amount><currency>'"),
        }
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.currency.as_str() {
            "EUR" => write!(f, "{:.2} €", self.quantity),
            _ => write!(f, "{} {}", self.quantity.normalize(), self.currency),
        }
    }
}

pub (crate) fn deserialize_amount<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Amount, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(Amount::try_from(raw).unwrap())
}

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "type")]
pub(crate) enum Operation {
    Buy(Amount),
    Sell(Amount),
    Trade {
        incoming: Amount,
        outgoing: Amount,
    },
    Swap {
        incoming: Amount,
        outgoing: Amount,
    },
    FiatDeposit(Amount),
    FiatWithdrawal(Amount),
    Fee(Amount),
    // Approval,
    Receive(Amount),
    Send(Amount),
    ChainSplit(Amount),
    Expense(Amount),
    Stolen(Amount),
    Lost(Amount),
    Burn(Amount),
    Income(Amount),
    // Interest(Amount),
    // Mining(Amount),
    Airdrop(Amount),
    Staking(Amount),
    // StakingDeposit,
    // StakingWithdrawal,
    Cashback(Amount),
    // Royalties,
    // PersonalUse,
    IncomingGift(Amount),
    OutgoingGift(Amount),
    // Borrow,
    // LoanRepayment,
    // Liquidate,
    RealizedProfit(Amount),
    RealizedLoss(Amount),
    // MarginFee,
    // BridgeIn,
    // BridgeOut,
    // Mint(Amount),
    // CollateralWithdrawal,
    // CollateralDeposit,
    // AddLiquidity,
    // ReceiveLpToken,
    // RemoveLiquidity,
    // ReturnLpToken,
    // FailedIn,
    // FailedOut,
    Spam(Amount),
}

impl Operation {
    /// Returns `true` if the operation is [`Receive`].
    ///
    /// [`Receive`]: Operation::Receive
    #[must_use]
    pub(crate) fn is_receive(&self) -> bool {
        matches!(self, Self::Receive(..))
    }

    /// Returns `true` if the operation is [`Send`].
    ///
    /// [`Send`]: Operation::Send
    #[must_use]
    pub(crate) fn is_send(&self) -> bool {
        matches!(self, Self::Send(..))
    }

    /// Returns `true` if the operation is [`Trade`].
    ///
    /// [`Trade`]: Operation::Trade
    #[must_use]
    pub(crate) fn is_trade(&self) -> bool {
        matches!(self, Self::Trade { .. })
    }
}

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct Transaction {
    pub timestamp: NaiveDateTime,
    pub operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blockchain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee: Option<Amount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_value: Option<Amount>,
    #[serde(skip)]
    pub gain: Option<Result<Decimal, GainError>>,
    /// The index of this transaction in the list of loaded transactions.
    #[serde(skip)]
    pub index: usize,
    /// The index of the wallet from which this transaction originated.
    #[serde(skip)]
    pub wallet_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Amount>,
    #[serde(skip)]
    pub matching_tx: Option<usize>,
}

pub(crate) struct MergeError;

impl Transaction {
    pub(crate) fn new(timestamp: NaiveDateTime, operation: Operation) -> Self {
        Self {
            index: 0,
            timestamp,
            operation,
            description: None,
            tx_hash: None,
            blockchain: None,
            fee: None,
            fee_value: None,
            gain: None,
            wallet_index: 0,
            value: None,
            matching_tx: None,
        }
    }

    pub(crate) fn fiat_deposit(timestamp: NaiveDateTime, amount: Amount) -> Self {
        Self::new(timestamp, Operation::FiatDeposit(amount))
    }

    pub(crate) fn fiat_withdrawal(timestamp: NaiveDateTime, amount: Amount) -> Self {
        Self::new(timestamp, Operation::FiatWithdrawal(amount))
    }

    pub(crate) fn send(timestamp: NaiveDateTime, amount: Amount) -> Self {
        Self::new(timestamp, Operation::Send(amount))
    }

    pub(crate) fn receive(timestamp: NaiveDateTime, amount: Amount) -> Self {
        Self::new(timestamp, Operation::Receive(amount))
    }

    pub(crate) fn fee(timestamp: NaiveDateTime, amount: Amount) -> Self {
        Self::new(timestamp, Operation::Fee(amount))
    }

    pub(crate) fn trade(timestamp: NaiveDateTime, incoming: Amount, outgoing: Amount) -> Self {
        Self::new(timestamp, Operation::Trade { incoming, outgoing })
    }

    pub(crate) fn incoming_outgoing(&self) -> (Option<&Amount>, Option<&Amount>) {
        match &self.operation {
            Operation::Buy(amount) |
            Operation::FiatDeposit(amount) |
            Operation::Receive(amount) |
            Operation::ChainSplit(amount) |
            Operation::Income(amount) |
            Operation::Airdrop(amount) |
            Operation::Staking(amount) |
            Operation::Cashback(amount) |
            Operation::IncomingGift(amount) |
            Operation::RealizedProfit(amount) |
            Operation::Spam(amount) => {
                (Some(amount), None)
            }
            Operation::Sell(amount) |
            Operation::FiatWithdrawal(amount) |
            Operation::Fee(amount) |
            Operation::Send(amount) |
            Operation::Expense(amount) |
            Operation::Stolen(amount) |
            Operation::Lost(amount) |
            Operation::Burn(amount) |
            Operation::OutgoingGift(amount) |
            Operation::RealizedLoss(amount) => {
                (None, Some(amount))
            }
            Operation::Trade { incoming, outgoing } |
            Operation::Swap { incoming, outgoing } => {
                (Some(incoming), Some(outgoing))
            }
        }
    }

    pub(crate) fn has_incoming(&self) -> bool {
        self.incoming_outgoing().0.is_some()
    }

    /// Used to sort transactions by date, and placing incoming transactions
    /// before outgoing ones.
    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        match self.timestamp.cmp(&other.timestamp) {
            Ordering::Less => Ordering::Less,
            Ordering::Equal => {
                match (self.has_incoming(), other.has_incoming()) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => {
                        // If both sides are a trade, order by fee currency,
                        // since this helps with grouping certain trades
                        if self.operation.is_trade() && other.operation.is_trade() {
                            match (self.fee.as_ref(), other.fee.as_ref()) {
                                (Some(a), Some(b)) => a.currency.cmp(&b.currency),
                                (Some(_), None) => Ordering::Less,
                                (None, Some(_)) => Ordering::Greater,
                                (None, None) => Ordering::Equal,
                            }
                        } else {
                            Ordering::Equal
                        }
                    },
                }
            }
            Ordering::Greater => Ordering::Greater,
        }
    }

    /// Used to merge trade operations to avoid clutter.
    pub(crate) fn merge_trades(&mut self, other: &Self) -> Result<(), MergeError> {
        // Some things should be equal before we will merge transactions
        if self.blockchain != other.blockchain ||
            self.tx_hash != other.tx_hash
        {
            return Err(MergeError);
        }

        // The transactions should be close in time
        if other.timestamp - self.timestamp > Duration::minutes(5) {
            return Err(MergeError);
        }

        fn merge_optional_amounts(amount: &Option<Amount>, other_amount: &Option<Amount>) -> Result<Option<Amount>, MergeError> {
            match (amount, other_amount) {
                (None, None) => Ok(None),
                (Some(a), None) => Ok(Some(a.clone())),
                (None, Some(b)) => Ok(Some(b.clone())),
                (Some(a), Some(b)) => a.try_add(b).ok_or(MergeError).map(Some),
            }
        }

        // Check if we can add up the fees and values
        let merged_fee = merge_optional_amounts(&self.fee, &other.fee)?;
        let merged_fee_value = merge_optional_amounts(&self.fee_value, &other.fee_value)?;
        let merged_value = merge_optional_amounts(&self.value, &other.value)?;

        // And then we only merge trades
        match (&mut self.operation, &other.operation) {
            (Operation::Trade { incoming, outgoing }, Operation::Trade { incoming: other_incoming, outgoing: other_outgoing }) => {
                if incoming == other_outgoing {
                    println!("Removing intermediate trade amount: {}, replacing with target amount: {}", incoming, other_incoming);
                    // If the incoming and outgoing amounts are equal, we can
                    // ignore the intermediate currency and shortcut the trade.
                    *incoming = other_incoming.clone();
                } else {
                    // When the incoming and outgoing amounts can be added, we
                    // can merge the trades.
                    let total_incoming = incoming.try_add(other_incoming).ok_or(MergeError)?;
                    let total_outgoing = outgoing.try_add(other_outgoing).ok_or(MergeError)?;
                    *incoming = total_incoming;
                    *outgoing = total_outgoing;
                }
            }
            _ => Err(MergeError)?,
        }

        if self.description != other.description {
            self.description = match (&self.description, &other.description) {
                (None, None) => None,
                (Some(description), None) => Some(description.clone()),
                (None, Some(description)) => Some(description.clone()),
                (Some(self_description), Some(other_description)) => {
                    Some(format!("{}, {}", self_description, other_description))
                }
            };
        }
        self.fee = merged_fee;
        self.fee_value = merged_fee_value;
        self.value = merged_value;

        Ok(())
    }
}

pub(crate) fn save_transactions_to_json(transactions: &Vec<Transaction>, output_path: impl AsRef<Path>) -> Result<()> {
    println!("Saving {}", output_path.as_ref().display());

    let json = serde_json::to_string_pretty(&transactions)?;
    std::fs::write(output_path, json)?;

    Ok(())
}

pub(crate) fn load_transactions_from_json(input_path: &Path) -> Result<Vec<Transaction>> {
    let json = std::fs::read_to_string(input_path)?;
    let transactions: Vec<Transaction> = serde_json::from_str(&json)?;
    Ok(transactions)
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PricePoint {
    pub timestamp: NaiveDateTime,
    pub price: Decimal,
}

pub(crate) struct PriceHistory {
    prices: HashMap<String, Vec<PricePoint>>,
}

impl PriceHistory {
    pub(crate) fn new() -> Self {
        let mut prices = HashMap::new();

        if let Ok(price_points) = load_btc_price_history_data() {
            prices.insert("BTC".to_owned(), price_points);
        }

        Self { prices }
    }

    pub(crate) fn insert_price_points(&mut self, currency: String, price_points: Vec<PricePoint>) {
        self.prices.insert(currency, price_points);
    }

    // todo: would be nice to expose the accuracy in the UI
    pub(crate) fn estimate_price(&self, timestamp: NaiveDateTime, currency: &str) -> Option<Decimal> {
        match currency {
            "EUR" => Some(Decimal::ONE),
            _ => {
                self.prices.get(currency).and_then(|price_points| {
                    estimate_price(timestamp, price_points).map(|(price, _)| price)
                })
            }
        }
    }

    pub(crate) fn estimate_value(&self, timestamp: NaiveDateTime, amount: &Amount) -> Option<Amount> {
        self.estimate_price(timestamp, &amount.currency).map(|price| Amount::new(price * amount.quantity, "EUR".to_owned()))
    }
}

#[allow(dead_code)]
pub(crate) fn save_price_history_data(prices: &Vec<PricePoint>, path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    for price in prices {
        wtr.serialize(price)?;
    }

    Ok(())
}

pub(crate) fn load_price_history_data(path: &Path) -> Result<Vec<PricePoint>> {
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(path)?;

    let mut prices: Vec<PricePoint> = Vec::new();
    for result in rdr.deserialize() {
        let record: PricePoint = result?;
        prices.push(record);
    }

    Ok(prices)
}

pub(crate) fn load_btc_price_history_data() -> Result<Vec<PricePoint>> {
    // The following file was saved using the above function with data loaded
    // from the CoinMarketCap API.
    let btc_price_history_eur = include_bytes!("data/btc-price-history-eur.csv");

    let mut rdr = csv::Reader::from_reader(btc_price_history_eur.as_slice());
    let mut prices: Vec<PricePoint> = Vec::new();
    for result in rdr.deserialize() {
        let record: PricePoint = result?;
        prices.push(record);
    }
    Ok(prices)
}

fn estimate_price(time: NaiveDateTime, prices: &Vec<PricePoint>) -> Option<(Decimal, Duration)> {
    let index = prices.partition_point(|p| p.timestamp < time);
    let next_price_point = prices.get(index).or_else(|| prices.last());
    let prev_price_point = if index > 0 { prices.get(index - 1) } else { None };

    if let (Some(next_price), Some(prev_price)) = (next_price_point, prev_price_point) {
        // calculate the most probable price, by linear iterpolation based on the previous and next price
        let price_difference = next_price.price - prev_price.price;
        let total_duration: Decimal = (next_price.timestamp - prev_price.timestamp).num_seconds().into();

        // The accuracy is the minimum time difference between the requested time and a price point
        let accuracy = (time - prev_price.timestamp).abs().min((next_price.timestamp - time).abs());

        if total_duration > Decimal::ZERO {
            let time_since_prev: Decimal = (time - prev_price.timestamp).num_seconds().into();
            let time_ratio = time_since_prev / total_duration;

            Some((prev_price.price + time_ratio * price_difference, accuracy))
        } else {
            Some((next_price.price, accuracy))
        }
    } else {
        None
    }
}
