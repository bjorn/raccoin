use chrono::NaiveDateTime;
use serde::{Serialize, Deserialize};

use crate::time::{serialize_date_time, deserialize_date_time};

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)] // we want to represent all possible values, not just used ones
pub(crate) enum CtcTxType {
    /// Purchase of cryptocurrency, which increases the balance remaining and effects cost basis.
    #[serde(rename = "buy")]
    Buy,

    /// A sale of cryptocurrency which decreases the balance remaining and triggers a capital gain event.
    #[serde(rename = "sell")]
    Sell,

    /// A deposit of your local currency into the exchange. Note, if you deposit a currency other then your local currency, you need to have a corresponding buy transaction of that currency.
    #[serde(rename = "fiat-deposit")]
    FiatDeposit,

    /// Use this if you cashed out from an exchange into your bank account.
    #[serde(rename = "fiat-withdrawal")]
    FiatWithdrawal,

    /// Use this if you have disposed of cryptocurrency to cover fee transactions generated as a result of other transactions, e.g., gas fees paid during on-chain Ethereum swaps. If using this category, don't include this fee amount in the fee column.
    #[serde(rename = "fee")]
    Fee,

    /// You approved the use of a smart contract. This is taxed the same way as a Fee, a disposal event. This category is listed in the Miscellaneous Expense Report.
    #[serde(rename = "approval")]
    Approval,

    /// A transfer of cryptocurrency to a wallet or exchange. Increases the balance remaining on the receiving address and decreases the balance remaining on the from address. Does not increase your overall balance remaining. Does not trigger a capital gain event.
    #[serde(rename = "receive", alias = "transfer-in")]
    Receive,

    /// A transfer of cryptocurrency from a wallet or exchange. Increases the balance remaining on the receiving address and decreases the balance remaining on the from address. Does not decrease your overall balance remaining. Does not trigger a capital gain event.
    #[serde(rename = "send", alias = "transfer-out")]
    Send,

    /// Use this if you acquired a new cryptocurrency as a result of a chain split (such as Bitcoin Cash being received by Bitcoin holders).
    #[serde(rename = "chain-split")]
    ChainSplit,

    /// This acts similar to a Sell. However you wish to label this as an expense. You can use this if you want to categorize an outgoing transaction as an expense (e.g. business paying out a salary). This category is listed in the Miscellaneous Expense Report.
    #[serde(rename = "expense")]
    Expense,

    /// Triggers a capital loss event with the sale price being zero.
    #[serde(rename = "stolen")]
    Stolen,

    /// Use this if you have lost the crypto, triggers a capital loss event similar to the stolen category.
    #[serde(rename = "lost")]
    Lost,

    /// Use this if you have sent your crypto / NFT to a burner address. It triggers a capital loss event similar to the stolen category.
    #[serde(rename = "burn")]
    Burn,

    /// Triggers an income tax event based on the market value at the time of receipt. Increase the balance remaining and is used for future cost basis calculations.
    #[serde(rename = "income")]
    Income,

    /// Similar to income but used for interest-bearing activities which don't suit other categories.
    #[serde(rename = "interest")]
    Interest,

    /// Use this if you received mining rewards (as a hobby).
    #[serde(rename = "mining")]
    Mining,

    /// Use this if you received a free token airdrop.
    #[serde(rename = "airdrop")]
    Airdrop,

    /// Use this if you earned interest from staking.
    #[serde(rename = "staking")]
    Staking,

    /// You deposited these coins into a staking pool. This acts similar to a withdrawal.
    #[serde(rename = "staking-deposit")]
    StakingDeposit,

    /// You have withdrawn these coins from the staking pool. This acts similar to a deposit.
    #[serde(rename = "staking-withdrawal")]
    StakingWithdrawal,

    /// Use this if you acquired cryptocurrency as a cash-back (e.g., credit card payment).
    #[serde(rename = "cashback")]
    Cashback,

    /// Use this if you have received payments from secondary sales (e.g., being an NFT creator).
    #[serde(rename = "royalties", alias = "royalty")]
    Royalties,

    /// Use this if you spent crypto on personal use and you want to ignore this transaction for tax purposes. Warning, this is only valid in very specific individual circumstances. Check with your tax professional before using this option.
    #[serde(rename = "personal-use")]
    PersonalUse,

    /// Use this if you have acquired cryptocurrency as a gift. If you have given a gift to someone else, use the sell category.
    #[serde(rename = "incoming-gift", alias = "gift")]
    IncomingGift,

    /// Use this If you have given a gift to someone else. This is similar to a sell.
    #[serde(rename = "outgoing-gift")]
    OutgoingGift,

    /// Use this if you have received (acquired) a cryptocurrency or cash as a loan.
    #[serde(rename = "borrow", alias = "loan")]
    Borrow,

    /// Use this if you have repaid a loan.
    #[serde(rename = "loan-repayment")]
    LoanRepayment,

    /// Use this if the lending platform you used has liquidated your collateral.
    #[serde(rename = "liquidate")]
    Liquidate,

    /// Advanced usage only - use this if you have performed margin, futures, derivates, etc. type trades and realized a profit from your trading activity.
    #[serde(rename = "realized-profit")]
    RealizedProfit,

    /// Advanced usage only - use this if you have performed margin, futures, derivates, etc., type trades, and realized a loss of your trading activity.
    #[serde(rename = "realized-loss")]
    RealizedLoss,

    /// Advanced usage only - use this if you have paid fees associated with a realized-profit or realized-loss trades.
    #[serde(rename = "margin-fee")]
    MarginFee,

    /// Used to transfer the cost basis from one blockchain to another. Note: A "bridge-in" and a "bridge-out" must match.
    #[serde(rename = "bridge-in")]
    BridgeIn,

    /// Used to transfer the cost basis from one blockchain to another. Note: A "bridge-in" and a "bridge-out" must match.
    #[serde(rename = "bridge-out")]
    BridgeOut,

    /// This acts similar to a 'buy'. A common use case is when a user is minting NFTs.
    #[serde(rename = "mint")]
    Mint,

    /// You have withdrawn these coins from a borrowing/lending platform. This acts similar to a deposit into your account.
    #[serde(rename = "collateral-withdrawal")]
    CollateralWithdrawal,

    /// You have set these coins aside as collateral for a loan. This acts as a withdrawal from your account.
    #[serde(rename = "collateral-deposit")]
    CollateralDeposit,

    /// You have added these coins into a liquidity pool
    #[serde(rename = "add-liquidity")]
    AddLiquidity,

    /// You have received tokens for adding coins into a liquidity pool.
    #[serde(rename = "receive-lp-token")]
    ReceiveLpToken,

    /// You have removed these coins from a liquidity pool.
    #[serde(rename = "remove-liquidity")]
    RemoveLiquidity,

    /// You have returned tokens for removing coins from a liquidity pool.
    #[serde(rename = "return-lp-token")]
    ReturnLpToken,

    /// A failed transaction. This will be ignored from tax and balance calculations. (Note: Any fees incurred from creating the transaction will be accounted for.)
    #[serde(rename = "failed-in")]
    FailedIn,

    /// A failed transaction. This will be ignored from tax and balance calculations. (Note: Any fees incurred from creating the transaction will be accounted for.)
    #[serde(rename = "failed-out")]
    FailedOut,

    /// Mark the transactions as spam and ignore them from tax and balance calculations.
    #[serde(rename = "spam")]
    Spam,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CtcTx<'a> {
    #[serde(rename = "Timestamp (UTC)", serialize_with = "serialize_date_time", deserialize_with = "deserialize_date_time")]
    pub timestamp: NaiveDateTime,

    /// This is the type of transaction, e.g., buy, sell.
    #[serde(rename = "Type")]
    pub operation: CtcTxType,

    /// The base currency of the trading pair. For example, if you purchase ETH using USD, the base currency is ETH.
    #[serde(rename = "Base Currency")]
    pub base_currency: &'a str,

    /// The amount excluding fee which corresponds to the base currency.
    #[serde(rename = "Base Amount")]
    pub base_amount: f64,

    /// The quote currency of the trading pair. For example, if you purchase ETH using USD, the quote currency is USD.
    #[serde(rename = "Quote Currency (Optional)")]
    pub quote_currency: Option<&'a str>,

    /// The amount of quote currency that was traded, excluding fees.
    #[serde(rename = "Quote Amount (Optional)")]
    pub quote_amount: Option<f64>,

    /// The currency in which the fee was paid.
    #[serde(rename = "Fee Currency (Optional)")]
    pub fee_currency: Option<&'a str>,

    /// The amount of fees that were paid.
    #[serde(rename = "Fee Amount (Optional)")]
    pub fee_amount: Option<f64>,

    /// The name of the Exchange/Wallet you are transferring from, if left blank, will default to CSV exchange name.
    ///
    /// Note: One CSV should only have the transactions for one wallet/exchange.
    #[serde(rename = "From (Optional)")]
    pub from: Option<&'a str>,

    /// The name of the Exchange/Wallet you are transferring to if left blank, will default to CSV exchange name.
    ///
    /// Note: One CSV should only have the transactions for one wallet/exchange.
    #[serde(rename = "To (Optional)")]
    pub to: Option<&'a str>,

    /// The blockchain where the transaction happened. This is particularly important for interacting with wallets that are imported on multiple chains.
    ///
    /// Note: Only the blockchains we support are valid. If an invalid blockchain is entered, this field will be ignored on the transaction.
    #[serde(rename = "Blockchain (Optional)")]
    pub blockchain: Option<&'a str>,

    /// Any transaction ID that you would like to associate to this transaction for easy searching on the review transactions page. It should be unique where possible.
    #[serde(rename = "ID (Optional)")]
    pub id: Option<&'a str>,

    #[serde(rename = "Description (Optional)")]
    pub description: Option<&'a str>,

    /// The price per unit of the "Base Currency". If left blank, the price defaults to market price.
    #[serde(rename = "Reference Price Per Unit (Optional)")]
    pub reference_price_per_unit: Option<f64>,

    /// This is the currency of the Reference Price Per Unit.
    /// - Only local currencies are available. Cryptocurrencies (including stablecoins) in this column will be ignored.
    /// - Only use this when Reference Price Per Unit is filled.
    /// - If left blank but with Reference Price Per Unit filled, this defaults to USD.
    #[serde(rename = "Reference Price Currency (Optional)")]
    pub reference_price_currency: Option<&'a str>,
}

impl<'a> CtcTx<'a> {
    /// Constructor that takes the timestamp, type, base currency, and base amount of the transaction
    /// All other fields are optional
    pub(crate) fn new(timestamp: NaiveDateTime, operation: CtcTxType, base_currency: &'a str, base_amount: f64) -> Self {
        Self {
            timestamp,
            operation,
            base_currency,
            base_amount,
            quote_currency: None,
            quote_amount: None,
            fee_currency: None,
            fee_amount: None,
            from: None,
            to: None,
            blockchain: None,
            id: None,
            description: None,
            reference_price_per_unit: None,
            reference_price_currency: None,
        }
    }
}
