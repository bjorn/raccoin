use std::path::Path;

use anyhow::Result;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Serialize, Deserialize};

use crate::{time::{serialize_date_time, deserialize_date_time}, base::{Transaction, Operation, Amount}};

#[derive(Debug, Serialize, Deserialize)]
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

    /// To account for a non-taxable transaction where one asset is traded for another. It works by assigning the cost basis and purchase date of the original asset to the new one.
    #[serde(rename = "swap-in")]
    SwapIn,

    /// To account for a non-taxable transaction where one asset is traded for another. It works by assigning the cost basis and purchase date of the original asset to the new one.
    #[serde(rename = "swap-out")]
    SwapOut,
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
    pub base_amount: Decimal,

    /// The quote currency of the trading pair. For example, if you purchase ETH using USD, the quote currency is USD.
    #[serde(rename = "Quote Currency (Optional)")]
    pub quote_currency: Option<&'a str>,

    /// The amount of quote currency that was traded, excluding fees.
    #[serde(rename = "Quote Amount (Optional)")]
    pub quote_amount: Option<Decimal>,

    /// The currency in which the fee was paid.
    #[serde(rename = "Fee Currency (Optional)")]
    pub fee_currency: Option<&'a str>,

    /// The amount of fees that were paid.
    #[serde(rename = "Fee Amount (Optional)")]
    pub fee_amount: Option<Decimal>,

    /// The name of the Exchange/Wallet you are transferring from. If left blank, will default to CSV exchange name.
    ///
    /// Note: One CSV should only have the transactions for one wallet/exchange.
    #[serde(rename = "From (Optional)")]
    pub from: Option<&'a str>,

    /// The name of the Exchange/Wallet you are transferring to. If left blank, will default to CSV exchange name.
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
    pub reference_price_per_unit: Option<Decimal>,

    /// This is the currency of the Reference Price Per Unit.
    /// - Only local currencies are available. Cryptocurrencies (including stablecoins) in this column will be ignored.
    /// - Only use this when Reference Price Per Unit is filled.
    /// - If left blank but with Reference Price Per Unit filled, this defaults to USD.
    #[serde(rename = "Reference Price Currency (Optional)")]
    pub reference_price_currency: Option<&'a str>,
}

impl<'a> From<&'a Transaction> for CtcTx<'a> {
    fn from(item: &'a Transaction) -> Self {
        let (operation, base, quote) = match &item.operation {
            Operation::Buy(amount) => (CtcTxType::Buy, amount, item.value.as_ref()),
            Operation::Sell(amount) => (CtcTxType::Sell, amount, item.value.as_ref()),
            Operation::Trade { incoming, outgoing } => {
                if outgoing.is_fiat() {
                    (CtcTxType::Buy, incoming, Some(outgoing))
                } else {
                    (CtcTxType::Sell, outgoing, Some(incoming))
                }
            }
            Operation::Swap { incoming: _, outgoing: _ } => {
                todo!("A Swap needs to be exported as SwapIn and SwapOut pair");
            }
            Operation::FiatDeposit(amount) => (CtcTxType::FiatDeposit, amount, None),
            Operation::FiatWithdrawal(amount) => (CtcTxType::FiatWithdrawal, amount, None),
            Operation::Fee(amount) => (CtcTxType::Fee, amount, None),
            Operation::Receive(amount) => (CtcTxType::Receive, amount, None),
            Operation::Send(amount) => (CtcTxType::Send, amount, None),
            Operation::ChainSplit(amount) => (CtcTxType::ChainSplit, amount, None),
            Operation::Expense(amount) => (CtcTxType::Expense, amount, None),
            Operation::Stolen(amount) => (CtcTxType::Stolen, amount, None),
            Operation::Lost(amount) => (CtcTxType::Lost, amount, None),
            Operation::Burn(amount) => (CtcTxType::Burn, amount, None),
            Operation::Income(amount) => (CtcTxType::Income, amount, None),
            Operation::Airdrop(amount) => (CtcTxType::Airdrop, amount, None),
            Operation::Staking(amount) => (CtcTxType::Staking, amount, None),
            Operation::Cashback(amount) => (CtcTxType::Cashback, amount, None),
            Operation::IncomingGift(amount) => (CtcTxType::IncomingGift, amount, None),
            Operation::OutgoingGift(amount) => (CtcTxType::OutgoingGift, amount, None),
            Operation::RealizedProfit(amount) => (CtcTxType::RealizedProfit, amount, None),
            Operation::RealizedLoss(amount) => (CtcTxType::RealizedLoss, amount, None),
            Operation::Spam(amount) => (CtcTxType::Spam, amount, None),
        };
        Self {
            timestamp: item.timestamp,
            operation,
            base_currency: &base.currency,
            base_amount: base.quantity,
            quote_currency: quote.map(|item| item.currency.as_str()),
            quote_amount: quote.map(|item| item.quantity),
            fee_currency: item.fee.as_ref().map(|fee| fee.currency.as_str()),
            fee_amount: item.fee.as_ref().map(|fee| fee.quantity),
            from: None,
            to: None,
            blockchain: item.blockchain.as_deref(),
            id: item.tx_hash.as_deref(),
            description: item.description.as_deref(),
            reference_price_per_unit: None,
            reference_price_currency: None,
        }
    }
}

impl<'a> From<CtcTx<'a>> for Transaction {
    fn from(item: CtcTx<'a>) -> Self {
        let base_amount = Amount::new(item.base_amount, item.base_currency.to_owned());
        let quote_amount = if let (Some(quote_amount), Some(quote_currency)) = (item.quote_amount, item.quote_currency) {
            Some(Amount::new(quote_amount, quote_currency.to_owned()))
        } else {
            None
        };

        let operation = match item.operation {
            CtcTxType::Buy => Operation::Trade { incoming: base_amount, outgoing: quote_amount.expect("Buy or Sell should have quote") },
            CtcTxType::Sell => Operation::Trade { incoming: quote_amount.expect("Buy or Sell should have quote"), outgoing: base_amount },
            CtcTxType::FiatDeposit => Operation::FiatDeposit(base_amount),
            CtcTxType::FiatWithdrawal => Operation::FiatWithdrawal(base_amount),
            CtcTxType::Fee => Operation::Fee(base_amount),
            CtcTxType::Approval => todo!(),
            CtcTxType::Receive => Operation::Receive(base_amount),
            CtcTxType::Send => Operation::Send(base_amount),
            CtcTxType::ChainSplit => Operation::ChainSplit(base_amount),
            CtcTxType::Expense => Operation::Expense(base_amount),
            CtcTxType::Stolen => Operation::Stolen(base_amount),
            CtcTxType::Lost => Operation::Lost(base_amount),
            CtcTxType::Burn => Operation::Burn(base_amount),
            CtcTxType::Income => Operation::Income(base_amount),
            CtcTxType::Interest => todo!(),
            CtcTxType::Mining => todo!(),
            CtcTxType::Airdrop => Operation::Airdrop(base_amount),
            CtcTxType::Staking => todo!(),
            CtcTxType::StakingDeposit => todo!(),
            CtcTxType::StakingWithdrawal => todo!(),
            CtcTxType::Cashback => todo!(),
            CtcTxType::Royalties => todo!(),
            CtcTxType::PersonalUse => todo!(),
            CtcTxType::IncomingGift => Operation::IncomingGift(base_amount),
            CtcTxType::OutgoingGift => Operation::OutgoingGift(base_amount),
            CtcTxType::Borrow => todo!(),
            CtcTxType::LoanRepayment => todo!(),
            CtcTxType::Liquidate => todo!(),
            CtcTxType::RealizedProfit => todo!(),
            CtcTxType::RealizedLoss => todo!(),
            CtcTxType::MarginFee => todo!(),
            CtcTxType::BridgeIn => todo!(),
            CtcTxType::BridgeOut => todo!(),
            CtcTxType::Mint => todo!(),
            CtcTxType::CollateralWithdrawal => todo!(),
            CtcTxType::CollateralDeposit => todo!(),
            CtcTxType::AddLiquidity => todo!(),
            CtcTxType::ReceiveLpToken => todo!(),
            CtcTxType::RemoveLiquidity => todo!(),
            CtcTxType::ReturnLpToken => todo!(),
            CtcTxType::FailedIn => todo!(),
            CtcTxType::FailedOut => todo!(),
            CtcTxType::Spam => Operation::Spam(base_amount),
            CtcTxType::SwapIn => todo!(),
            CtcTxType::SwapOut => todo!(),
        };

        let mut tx = Transaction::new(item.timestamp, operation);
        tx.description = item.description.map(|s| s.to_owned());
        tx.tx_hash = item.id.map(|s| s.to_owned());
        tx.blockchain = item.blockchain.map(|s| s.to_owned());
        tx.fee = if let (Some(fee_amount), Some(fee_currency)) = (item.fee_amount, item.fee_currency) {
            Some(Amount::new(fee_amount, fee_currency.to_owned()))
        } else {
            None
        };
        tx.value = item.reference_price_per_unit.map(|price| Amount::new(price * item.base_amount, item.reference_price_currency.unwrap_or("EUR").to_owned()));
        tx
    }
}

pub(crate) fn save_transactions_to_ctc_csv(transactions: &Vec<Transaction>, output_path: &Path) -> Result<()> {
    println!("Saving {}", output_path.display());

    let mut wtr = csv::Writer::from_path(output_path)?;

    for tx in transactions {
        let ctc_tx: CtcTx = tx.into();
        wtr.serialize(ctc_tx)?;
    }

    Ok(())
}

// loads a CSV file that was prepared in CryptoTaxCalculator import format
pub(crate) fn load_ctc_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();

    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;
    let mut raw_record = csv::StringRecord::new();
    let headers = rdr.headers()?.clone();

    while rdr.read_record(&mut raw_record)? {
        let record: CtcTx = raw_record.deserialize(Some(&headers))?;
        transactions.push(record.into());
    }

    Ok(transactions)
}
