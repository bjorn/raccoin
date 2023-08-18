use chrono::{NaiveDateTime, TimeZone, FixedOffset, DateTime};
use chrono_tz::Europe::Berlin;
use std::error::Error;
use serde::{Deserializer, Deserialize, Serialize, Serializer};

// serialize function for reading NaiveDateTime
fn deserialize_date_time<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    Ok(NaiveDateTime::parse_from_str(&raw, "%Y-%m-%d %H:%M:%S").unwrap())
}
fn serialize_date_time<S: Serializer>(date: &NaiveDateTime, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&date.format("%Y-%m-%d %H:%M:%S").to_string())
}

#[derive(Debug, Deserialize)]
enum BitcoinDeActionType {
    Registration,
    Purchase,
    Disbursement,
    Deposit,
    Sale,
    #[serde(rename = "Network fee")]
    NetworkFee,
}

// struct for storing the following CSV columns:
// Date;Type;Currency;Reference;BTC-address;Price;"unit (rate)";"BTC incl. fee";"amount before fee";"unit (amount before fee)";"BTC excl. Bitcoin.de fee";"amount after Bitcoin.de-fee";"unit (amount after Bitcoin.de-fee)";"Incoming / Outgoing";"Account balance"
#[derive(Debug, Deserialize)]
struct BitcoinDeAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Type")]
    type_: BitcoinDeActionType,
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Reference")]
    reference: String,
    #[serde(rename = "BTC-address")]
    btc_address: String,
    // #[serde(rename = "Price")]
    // price: Option<f64>,
    // #[serde(rename = "unit (rate)")]
    // unit_rate: String,
    // #[serde(rename = "BTC incl. fee")]
    // btc_incl_fee: Option<f64>,
    // #[serde(rename = "amount before fee")]
    // amount_before_fee: Option<f64>,
    // #[serde(rename = "unit (amount before fee)")]
    // unit_amount_before_fee: String,
    // #[serde(rename = "BTC excl. Bitcoin.de fee")]
    // btc_excl_bitcoin_de_fee: Option<f64>,
    #[serde(rename = "amount after Bitcoin.de-fee")]
    amount_after_bitcoin_de_fee: Option<f64>,
    #[serde(rename = "unit (amount after Bitcoin.de-fee)")]
    unit_amount_after_bitcoin_de_fee: String,
    #[serde(rename = "Incoming / Outgoing")]
    incoming_outgoing: f64,
    // #[serde(rename = "Account balance")]
    // account_balance: f64,
}

#[derive(Debug, Clone, Deserialize)]
enum TransferType {
    #[serde(rename = "Sent to")]
    SentTo,
    #[serde(rename = "Received with")]
    ReceivedWith,
}

#[derive(Debug, Deserialize)]
struct BitcoinCoreAction {
    // #[serde(rename = "Confirmed")]
    // confirmed: bool,
    #[serde(rename = "Date")]
    date: NaiveDateTime,
    #[serde(rename = "Type")]
    type_: TransferType,
    #[serde(rename = "Label")]
    label: String,
    #[serde(rename = "Address")]
    address: String,
    #[serde(rename = "Amount (BTC)")]
    amount: f64,
    #[serde(rename = "ID")]
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

#[derive(Debug, Deserialize)]
struct BitonicAction {
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Action")]
    operation: Operation,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Price")]
    price: f64,
}

#[derive(Debug, Deserialize)]
struct ElectrumHistoryItem {
    transaction_hash: String,
    label: String,
    // confirmations: u64,
    value: f64,
    // fiat_value: f64,
    // fee: Option<f64>,
    // fiat_fee: Option<f64>,
    #[serde(deserialize_with = "deserialize_date_time")]
    timestamp: NaiveDateTime,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)] // we want to represent all possible values, not just used ones
enum CtcTxType {
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

#[derive(Debug, Serialize)]
struct CtcTx<'a> {
    #[serde(rename = "Timestamp (UTC)", serialize_with = "serialize_date_time")]
    timestamp: NaiveDateTime,

    /// This is the type of transaction, e.g., buy, sell.
    #[serde(rename = "Type")]
    operation: CtcTxType,

    /// The base currency of the trading pair. For example, if you purchase ETH using USD, the base currency is ETH.
    #[serde(rename = "Base Currency")]
    base_currency: &'a str,

    /// The amount excluding fee which corresponds to the base currency.
    #[serde(rename = "Base Amount")]
    base_amount: f64,

    /// The quote currency of the trading pair. For example, if you purchase ETH using USD, the quote currency is USD.
    #[serde(rename = "Quote Currency (Optional)")]
    quote_currency: Option<&'a str>,

    /// The amount of quote currency that was traded, excluding fees.
    #[serde(rename = "Quote Amount (Optional)")]
    quote_amount: Option<f64>,

    /// The currency in which the fee was paid.
    #[serde(rename = "Fee Currency (Optional)")]
    fee_currency: Option<&'a str>,

    /// The amount of fees that were paid.
    #[serde(rename = "Fee Amount (Optional)")]
    fee_amount: Option<f64>,

    /// The name of the Exchange/Wallet you are transferring from, if left blank, will default to CSV exchange name.
    ///
    /// Note: One CSV should only have the transactions for one wallet/exchange.
    #[serde(rename = "From (Optional)")]
    from: Option<&'a str>,

    /// The name of the Exchange/Wallet you are transferring to if left blank, will default to CSV exchange name.
    ///
    /// Note: One CSV should only have the transactions for one wallet/exchange.
    #[serde(rename = "To (Optional)")]
    to: Option<&'a str>,

    /// The blockchain where the transaction happened. This is particularly important for interacting with wallets that are imported on multiple chains.
    ///
    /// Note: Only the blockchains we support are valid. If an invalid blockchain is entered, this field will be ignored on the transaction.
    #[serde(rename = "Blockchain (Optional)")]
    blockchain: Option<&'a str>,

    /// Any transaction ID that you would like to associate to this transaction for easy searching on the review transactions page. It should be unique where possible.
    #[serde(rename = "ID (Optional)")]
    id: Option<&'a str>,

    #[serde(rename = "Description (Optional)")]
    description: Option<&'a str>,

    /// The price per unit of the "Base Currency". If left blank, the price defaults to market price.
    #[serde(rename = "Reference Price Per Unit (Optional)")]
    reference_price_per_unit: Option<f64>,

    /// This is the currency of the Reference Price Per Unit.
    /// - Only local currencies are available. Cryptocurrencies (including stablecoins) in this column will be ignored.
    /// - Only use this when Reference Price Per Unit is filled.
    /// - If left blank but with Reference Price Per Unit filled, this defaults to USD.
    #[serde(rename = "Reference Price Currency (Optional)")]
    reference_price_currency: Option<&'a str>,
}

impl<'a> CtcTx<'a> {
    /// Constructor that takes the timestamp, type, base currency, and base amount of the transaction
    /// All other fields are optional
    fn new(timestamp: NaiveDateTime, operation: CtcTxType, base_currency: &'a str, base_amount: f64) -> Self {
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

// converts the bitcoin.de csv file to one for CryptoTaxCalculator
fn convert_bitcoin_de_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinDeAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        // handle various record type
        match record.type_ {
            BitcoinDeActionType::Registration => {},
            BitcoinDeActionType::Purchase => {
                // When purchasing on Bitcoin.de, the EUR amount is actually sent directly to the seller.
                // To avoid building up a negative EUR balance, we add a fiat deposit.
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::FiatDeposit,
                    &record.unit_amount_after_bitcoin_de_fee,
                    record.amount_after_bitcoin_de_fee.expect("Purchase should have an amount")))?;

                wtr.serialize(CtcTx {
                    id: Some(&record.reference),
                    quote_currency: Some(&record.unit_amount_after_bitcoin_de_fee),
                    quote_amount: record.amount_after_bitcoin_de_fee,
                    // reference_price_per_unit: record.price,
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Buy,
                        &record.currency,
                        record.incoming_outgoing
                    )
                })?;
            },
            BitcoinDeActionType::Disbursement => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Send,
                        &record.currency,
                        -record.incoming_outgoing)
                })?;
            },
            BitcoinDeActionType::Deposit => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Receive,
                        &record.currency,
                        record.incoming_outgoing)
                })?;
            },
            BitcoinDeActionType::Sale => {
                // When selling on Bitcoin.de, the EUR amount is actually sent directly to the buyer.
                // To avoid building up a positive EUR balance, we add a fiat withdrawal.
                wtr.serialize(CtcTx {
                    id: Some(&record.reference),
                    quote_currency: Some(&record.unit_amount_after_bitcoin_de_fee),
                    quote_amount: record.amount_after_bitcoin_de_fee,
                    // reference_price_per_unit: record.price,
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Sell,
                        &record.currency,
                        -record.incoming_outgoing
                    )
                })?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::FiatWithdrawal,
                    &record.unit_amount_after_bitcoin_de_fee,
                    record.amount_after_bitcoin_de_fee.expect("Sale should have an amount")))?;
            },
            BitcoinDeActionType::NetworkFee => {
                wtr.serialize(CtcTx {
                    description: Some(&record.btc_address),
                    id: Some(&record.reference),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Fee,
                        &record.currency,
                        -record.incoming_outgoing)
                })?;
            },
        }
    }

    Ok(())
}

// converts the Bitcoin Core CSV file to one for CryptoTaxCalculator
fn convert_bitcoin_core_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitcoinCoreAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        match record.type_ {
            TransferType::SentTo => {
                wtr.serialize(CtcTx {
                    id: Some(&record.id),
                    description: Some(&format!("{} ({})", &record.label, &record.address)),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Send,
                        "BTC",
                        -record.amount
                    )
                })?;
            },
            TransferType::ReceivedWith => {
                wtr.serialize(CtcTx {
                    id: Some(&record.id),
                    // store label and address in the description
                    description: Some(&format!("{} ({})", &record.label, &record.address)),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Receive,
                        "BTC",
                        record.amount)
                })?;
            },
        }
    }

    Ok(())
}

// converts the Bitonic CSV file to one for CryptoTaxCalculator
fn convert_bitonic_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: BitonicAction = result?;
        let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();

        // Since Bitonic does not hold any fiat or crypto, we add dummy deposit and send transactions
        // for each buy/sell transaction.
        match record.operation {
            Operation::Buy => {
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::FiatDeposit,
                    "EUR",
                    -record.price
                ))?;
                wtr.serialize(CtcTx {
                    quote_currency: Some("EUR"),
                    quote_amount: Some(-record.price),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Buy,
                        "BTC",
                        record.amount
                )})?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::Send,
                    "BTC",
                    record.amount
                ))?;
            }
            Operation::Sell => {
                wtr.serialize(CtcTx::new(
                    utc_time - chrono::Duration::minutes(1),
                    CtcTxType::Receive,
                    "BTC",
                    -record.amount
                ))?;
                wtr.serialize(CtcTx {
                    quote_currency: Some("EUR"),
                    quote_amount: Some(record.price),
                    ..CtcTx::new(
                        utc_time,
                        CtcTxType::Sell,
                        "BTC",
                        -record.amount)
                })?;
                wtr.serialize(CtcTx::new(
                    utc_time + chrono::Duration::minutes(1),
                    CtcTxType::FiatWithdrawal,
                    "EUR",
                    record.price
                ))?;
            }
        }
    }

    Ok(())
}

fn convert_electrum_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting {} to {}", input_path, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(input_path)?;

    let mut wtr = csv::Writer::from_path(output_path)?;

    for result in rdr.deserialize() {
        let record: ElectrumHistoryItem = result?;
        let utc_time = Berlin.from_local_datetime(&record.timestamp).unwrap().naive_utc();

        wtr.serialize(CtcTx {
            id: Some(&record.transaction_hash),
            description: Some(&record.label),
            ..CtcTx::new(
                utc_time,
                if record.value < 0.0 { CtcTxType::Send } else { CtcTxType::Receive },
                "BTC",
                record.value.abs())
        })?;
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct PoloniexDeposit {
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Address")]
    address: String,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    // #[serde(rename = "Status")]
    // status: String,
}

#[derive(Debug, Deserialize)]
struct PoloniexWithdrawal {
    #[serde(rename = "Fee Deducted")]
    fee_deducted: f64,
    #[serde(rename = "Date", deserialize_with = "deserialize_date_time")]
    date: NaiveDateTime,
    #[serde(rename = "Currency")]
    currency: String,
    // #[serde(rename = "Amount")]
    // amount: f64,
    #[serde(rename = "Amount-Fee")]
    amount_fee: f64,
    #[serde(rename = "Address")]
    address: String,
    // #[serde(rename = "Status")]
    // status: String,  // always COMPLETED
}

// csv columns: Date,Market,Type,Side,Price,Amount,Total,Fee,Order Number,Fee Currency,Fee Total
#[derive(Debug, Deserialize)]
struct PoloniexTrade {
    #[serde(rename = "Date")]
    date: DateTime<FixedOffset>,
    #[serde(rename = "Market")]
    market: String,
    // #[serde(rename = "Type")]
    // type_: String,   // always LIMIT
    #[serde(rename = "Side")]
    side: Operation,
    // #[serde(rename = "Price")]
    // price: f64,
    #[serde(rename = "Amount")]
    amount: f64,
    #[serde(rename = "Total")]
    total: f64,
    #[serde(rename = "Fee")]
    fee: f64,
    #[serde(rename = "Order Number")]
    order_number: String,
    #[serde(rename = "Fee Currency")]
    fee_currency: String,
    // #[serde(rename = "Fee Total")]
    // fee_total: f64,  // always same as fee
}

fn convert_poloniex_to_ctc(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::Writer::from_path(output_path)?;

    // deposits
    let deposits_file = input_path.to_owned() + "/deposit.csv";
    println!("Converting {} to {}", deposits_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(deposits_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexDeposit = result?;
        // let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();
        wtr.serialize(CtcTx {
            description: Some(&record.address),
            ..CtcTx::new(
                record.date,
                CtcTxType::Receive,
                &record.currency,
                record.amount)
        })?;
    }

    // withdrawals
    let withdrawals_file = input_path.to_owned() + "/withdrawal.csv";
    println!("Converting {} to {}", withdrawals_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(withdrawals_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexWithdrawal = result?;
        // let utc_time = Berlin.from_local_datetime(&record.date).unwrap().naive_utc();
        wtr.serialize(CtcTx {
            description: Some(&record.address),
            fee_amount: Some(record.fee_deducted),
            fee_currency: Some(&record.currency),
            ..CtcTx::new(
                record.date,
                CtcTxType::Send,
                &record.currency,
                record.amount_fee)
        })?;
    }

    // trades
    let trades_file = input_path.to_owned() + "/all-trades.csv";
    println!("Converting {} to {}", trades_file, output_path);
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(trades_file)?;

    for result in rdr.deserialize() {
        let record: PoloniexTrade = result?;

        // split record.market at the underscore to obtain the base_currency and the quote_currency
        let collect = record.market.split("_").collect::<Vec<&str>>();
        let base_currency = collect[0];
        let quote_currency = collect[1];

        wtr.serialize(CtcTx {
            description: Some(&record.order_number),
            quote_amount: Some(record.total),
            quote_currency: Some(quote_currency),
            fee_amount: Some(record.fee),
            fee_currency: Some(&record.fee_currency),
            ..CtcTx::new(
                record.date.naive_utc(),
                match record.side {
                    Operation::Buy => CtcTxType::Buy,
                    Operation::Sell => CtcTxType::Sell,
                },
                base_currency,
                record.amount)
        })?;
    }

    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let bitcoin_de_csv_file = "bitcoin.de/btc_account_statement_20120831-20230831.csv";
    let bitcoin_de_ctc_csv_file = "bitcoin-de-for-ctc.csv";
    convert_bitcoin_de_to_ctc(bitcoin_de_csv_file, bitcoin_de_ctc_csv_file)?;

    let bitcoin_core_csv_file = "bitcoin-core-transactions.csv";
    let bitcoin_core_ctc_csv_file = "bitcoin-core-transactions-for-ctc.csv";
    convert_bitcoin_core_to_ctc(bitcoin_core_csv_file, bitcoin_core_ctc_csv_file)?;

    let bitonic_csv_file = "bitonic.csv";
    let bitonic_ctc_csv_file = "bitonic-for-ctc.csv";
    convert_bitonic_to_ctc(bitonic_csv_file, bitonic_ctc_csv_file)?;

    let electrum_csv_file = "electrum-history.csv";
    let electrum_ctc_csv_file = "electrum-for-ctc.csv";
    convert_electrum_to_ctc(electrum_csv_file, electrum_ctc_csv_file)?;

    let poloniex_path = "poloniex";
    let poloniex_ctc_csv_file = "poloniex-for-ctc.csv";
    convert_poloniex_to_ctc(poloniex_path, poloniex_ctc_csv_file)?;

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        println!("{}", err);
    }
}
