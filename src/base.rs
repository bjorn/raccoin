use std::{error::Error, path::Path, fmt};

use chrono::NaiveDateTime;
use serde::{Serialize, Deserialize};
use rust_decimal::prelude::*;

#[derive(Debug)]
pub enum GainError {
    InvalidTransactionOrder,    // should only happen in case of a bug
    MissingFiatValue,
    InvalidFiatValue,
    InsufficientBalance,
}

impl fmt::Display for GainError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GainError::InvalidTransactionOrder => f.write_str("Invalid transaction order"),
            GainError::MissingFiatValue => f.write_str("Missing fiat value"),
            GainError::InvalidFiatValue => f.write_str("Invalid fiat value"),
            GainError::InsufficientBalance => f.write_str("Insufficient balance"),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct Amount {
    pub quantity: Decimal,
    pub currency: String,
}

impl Amount {
    pub(crate) fn new(quantity: Decimal, currency: String) -> Self {
        Self {
            quantity,
            currency,
        }
    }

    pub(crate) fn from_f64(quantity: f64, currency: String) -> Self {
        Self {
            quantity: Decimal::from_f64(quantity).unwrap(),
            currency: currency,
        }
    }

    pub(crate) fn from_satoshis(quantity: u64) -> Self {
        Self {
            quantity: Decimal::new(quantity as i64, 8),
            currency: "BTC".to_owned(),
        }
    }

    pub(crate) fn is_fiat(&self) -> bool {
        self.currency == "EUR"
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

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Operation {
    Buy(Amount),
    Sell(Amount),
    Trade {
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
    // Stolen(Amount),
    // Lost(Amount),
    // Burn(Amount),
    Income(Amount),
    // Interest(Amount),
    // Mining(Amount),
    Airdrop(Amount),
    Staking(Amount),
    // StakingDeposit,
    // StakingWithdrawal,
    // Cashback,
    // Royalties,
    // PersonalUse,
    IncomingGift(Amount),
    OutgoingGift(Amount),
    // Borrow,
    // LoanRepayment,
    // Liquidate,
    // RealizedProfit,
    // RealizedLoss,
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
    /// Returns `true` if the operation is [`Send`].
    ///
    /// [`Send`]: Operation::Send
    #[must_use]
    pub(crate) fn is_send(&self) -> bool {
        matches!(self, Self::Send(..))
    }
}

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(skip)]
    pub source_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Amount>,
    #[serde(skip)]
    pub matching_tx: Option<usize>,
}

impl Transaction {
    pub(crate) fn new(timestamp: NaiveDateTime, operation: Operation) -> Self {
        Self {
            timestamp,
            operation,
            description: None,
            tx_hash: None,
            blockchain: None,
            fee: None,
            fee_value: None,
            gain: None,
            source_index: 0,
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
}

pub(crate) fn save_transactions_to_json(transactions: &Vec<Transaction>, output_path: &Path) -> Result<(), Box<dyn Error>> {
    println!("Saving {}", output_path.display());

    let json = serde_json::to_string_pretty(&transactions)?;
    std::fs::write(output_path, json)?;

    Ok(())
}

pub(crate) fn load_transactions_from_json(input_path: &Path) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let json = std::fs::read_to_string(input_path)?;
    let transactions: Vec<Transaction> = serde_json::from_str(&json)?;
    Ok(transactions)
}
