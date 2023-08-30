use chrono::NaiveDateTime;

#[derive(Debug, Default)]
pub(crate) struct Amount {
    pub quantity: f64,
    pub currency: String,
}

/// Unified transaction type for all exchanges and wallets
#[derive(Debug, Default)]
pub(crate) enum Operation {
    #[default]
    Noop,
    Buy {
        incoming: Amount,
        outgoing: Amount,
    },
    Sell {
        incoming: Amount,
        outgoing: Amount,
    },
    FiatDeposit(Amount),
    FiatWithdrawal(Amount),
    Fee(Amount),
    // Approval,
    Receive(Amount),
    Send(Amount),
    ChainSplit,
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
#[derive(Debug, Default)]
pub(crate) struct Transaction {
    pub timestamp: NaiveDateTime,
    pub operation: Operation,
    pub description: String,
    pub tx_hash: Option<String>,
}

impl Transaction {
    pub(crate) fn noop(timestamp: NaiveDateTime) -> Self {
        Self {
            timestamp,
            operation: Operation::Noop,
            ..Default::default()
        }
    }

    pub(crate) fn fiat_deposit(timestamp: NaiveDateTime, amount: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::FiatDeposit(Amount {
                quantity: amount,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn fiat_withdrawal(timestamp: NaiveDateTime, amount: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::FiatDeposit(Amount {
                quantity: amount,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn send(timestamp: NaiveDateTime, amount: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Send(Amount {
                quantity: amount,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn receive(timestamp: NaiveDateTime, amount: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Receive(Amount {
                quantity: amount,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn fee(timestamp: NaiveDateTime, amount: f64, currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Fee(Amount {
                quantity: amount,
                currency: currency.to_string(),
            }),
            ..Default::default()
        }
    }

    pub(crate) fn buy(timestamp: NaiveDateTime, amount: f64, currency: &str, price: f64, price_currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Buy {
                incoming: Amount {
                    quantity: amount,
                    currency: currency.to_string(),
                },
                outgoing: Amount {
                    quantity: price,
                    currency: price_currency.to_string(),
                }
            },
            ..Default::default()
        }
    }

    pub(crate) fn sell(timestamp: NaiveDateTime, amount: f64, currency: &str, price: f64, price_currency: &str) -> Self {
        Self {
            timestamp,
            operation: Operation::Sell {
                incoming: Amount {
                    quantity: price,
                    currency: price_currency.to_string(),
                },
                outgoing: Amount {
                    quantity: amount,
                    currency: currency.to_string(),
                }
            },
            ..Default::default()
        }
    }
}
