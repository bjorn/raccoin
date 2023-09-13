use std::{error::Error, path::Path, fmt};

use chrono::NaiveDateTime;
use serde::{Serialize, Deserialize};

#[derive(Debug)]
pub enum GainError {
    InvalidTransactionOrder,    // should only happen in case of a bug
    MissingTransactionValue,
    InvalidTransactionValue,
    InsufficientBalance,
}

impl fmt::Display for GainError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GainError::InvalidTransactionOrder => f.write_str("Invalid transaction order"),
            GainError::MissingTransactionValue => f.write_str("Missing transaction value"),
            GainError::InvalidTransactionValue => f.write_str("Invalid transaction value (not fiat?)"),
            GainError::InsufficientBalance => f.write_str("Insufficient balance"),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct Amount {
    pub quantity: f64,
    pub currency: String,
}

impl Amount {
    pub(crate) fn is_fiat(&self) -> bool {
        self.currency == "EUR"
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.currency.as_str() {
            "EUR" => write!(f, "{:.2} €", self.quantity),
            _ => write!(f, "{} {}", self.quantity, self.currency),
        }
    }
}

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Operation {
    #[default]
    Noop,
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
    // Staking,
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

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Transaction {
    pub timestamp: NaiveDateTime,
    pub operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee: Option<Amount>,
    #[serde(skip)]
    pub gain: Option<Result<f64, GainError>>,
    #[serde(skip)]
    pub source_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Amount>,
}

impl Transaction {
    pub(crate) fn noop(timestamp: NaiveDateTime) -> Self {
        Self {
            timestamp,
            operation: Operation::Noop,
            ..Default::default()
        }
    }

    pub(crate) fn fiat_deposit(timestamp: NaiveDateTime, quantity: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::FiatDeposit(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn fiat_withdrawal(timestamp: NaiveDateTime, quantity: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::FiatDeposit(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn send(timestamp: NaiveDateTime, quantity: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Send(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn receive(timestamp: NaiveDateTime, quantity: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Receive(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn fee(timestamp: NaiveDateTime, quantity: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Fee(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn trade(timestamp: NaiveDateTime, incoming: Amount, outgoing: Amount) -> Self {
        Self {
            timestamp,
            operation: Operation::Trade {
                incoming,
                outgoing,
            },
            ..Default::default()
        }
    }

    pub(crate) fn buy(timestamp: NaiveDateTime, quantity: f64, currency: &str, price: f64, price_currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Buy(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            value: Some(Amount {
                quantity: price,
                currency: price_currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn sell(timestamp: NaiveDateTime, quantity: f64, currency: &str, price: f64, price_currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Sell(Amount {
                quantity,
                currency: currency.to_string(),
            }),
            value: Some(Amount {
                quantity: price,
                currency: price_currency.to_string(),
            }),
            ..Default::default()
        }
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

    println!("Loaded {} transactions from {}", transactions.len(), input_path.display());

    Ok(transactions)
}
