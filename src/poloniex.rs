use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Deserializer};

use crate::{
    base::{Amount, Transaction},
    time::deserialize_date_time,
    CsvSpec, TransactionSourceType,
};
use linkme::distributed_slice;

// deserialize function for trying a number of date-time formats, all of which
// have been seen in Poloniex trade CSV formats
pub(crate) fn deserialize_poloniex_timestamp<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<NaiveDateTime, D::Error> {
    let raw: &str = Deserialize::deserialize(d)?;
    let date_time: NaiveDateTime = DateTime::<FixedOffset>::parse_from_rfc3339(raw)
        .and_then(|dt| Ok(dt.naive_utc()))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y/%m/%d %H:%M"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .map_err(serde::de::Error::custom)?;
    Ok(date_time)
}

// Exported from https://www.poloniex.com/activity/wallet/deposit:
// Currency,Amount,Address,Date,Status
//
// Export requested through Support Ticket (2023):
// ,timestamp,currency,amount,address,status
//
// Export requested through Support Ticket (2025):
// f_created_at,currency,f_amount,f_address,f_status
#[derive(Debug, Deserialize)]
struct PoloniexDeposit {
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "f_amount", alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "f_address", alias = "Address")]
    address: String,
    #[serde(
        alias = "f_created_at",
        alias = "Date",
        deserialize_with = "deserialize_date_time"
    )]
    timestamp: NaiveDateTime,
    // #[serde(alias = "f_status", alias = "Status")]
    // status: String,
}

// Exported from https://www.poloniex.com/activity/wallet/withdraw:
// Fee Deducted,Date,Currency,Amount,Amount-Fee,Address,Status
//
// Export requested through Support Ticket (2023):
// ,timestamp,currency,amount,fee_deducted,status
//
// Export requested through Support Ticket (2025):
// f_date,currency,f_amount,f_feededucted,f_status
#[derive(Debug, Deserialize)]
struct PoloniexWithdrawal {
    #[serde(alias = "f_feededucted", alias = "Fee Deducted")]
    fee_deducted: Decimal,
    #[serde(
        alias = "f_date",
        alias = "Date",
        deserialize_with = "deserialize_date_time"
    )]
    timestamp: NaiveDateTime,
    #[serde(alias = "Currency")]
    currency: String,
    #[serde(alias = "f_amount", alias = "Amount")]
    amount: Decimal,
    // #[serde(rename = "Amount-Fee")]
    // amount_minus_fee: Decimal,
    #[serde(rename = "Address")]
    address: Option<String>,
    #[serde(alias = "f_status", alias = "Status")]
    status: String, // Can be "COMPLETED" or "COMPLETE: tx_hash"
}

#[derive(Debug, Clone, Deserialize)]
enum Operation {
    #[serde(alias = "BUY")]
    Buy,
    #[serde(alias = "SELL")]
    Sell,
}

// Exported from https://www.poloniex.com/activity/spot/trades:
// Date,Market,Type,Side,Price,Amount,Total,Fee,Order Number,Fee Currency,Fee Total
//
// Export requested through Support Ticket (2023):
// ,timestamp,trade_id,market,wallet,side,price,amount,fee,fee_currency,fee_total
//
// Export requested through Support Ticket (2025):
// order_id,activity,order_role,order_type,base_currency_name,quote_currency_name,fee_currency_name,price,amount,fee_amount,usd_amount,usd_fee_amount,utc_time
#[derive(Debug, Deserialize)]
struct PoloniexTrade {
    #[serde(
        alias = "utc_time",
        alias = "Date",
        deserialize_with = "deserialize_poloniex_timestamp"
    )]
    timestamp: NaiveDateTime,

    // Some formats have a "market" column while others have separate "base_currency_name" and
    // "quote_currency_name" columns
    #[serde(alias = "Market")]
    market: Option<String>,

    base_currency_name: Option<String>,
    quote_currency_name: Option<String>,

    // order_role: Option<String>,   // observed: maker, taker
    // #[serde(alias = "Type")]
    // order_type: String,   // observed: LIMIT, MARKET
    #[serde(alias = "activity", alias = "Side")]
    side: Operation,
    #[serde(alias = "Price")]
    price: Decimal,
    #[serde(alias = "Amount")]
    amount: Decimal,
    #[serde(alias = "Total")]
    total: Option<Decimal>,
    // #[serde(alias = "Fee")]
    // fee: Decimal,
    #[serde(rename = "Order Number", alias = "order_id")]
    order_number: Option<String>,
    #[serde(alias = "fee_currency_name", alias = "Fee Currency")]
    fee_currency: String,
    #[serde(alias = "fee_amount", alias = "Fee Total")]
    fee_total: Decimal,
}

// Export requested through Support Ticket (2025, but was labeled "Before August 1, 2022"):
// tradeid,markettradeid,base,quote,type,rate,amount,buyuser,selluser,buyerfee,sellerwallet,sellerfee,buyerwallet,buyerordernumber,sellerordernumber,date
//
// This format is rather special because it does not readily tell us if we are buying or selling. To
// import these transactions, we'll first need to figure out our user ID.
#[derive(Debug, Deserialize)]
struct PoloniexTradeBeforeAugust2022 {
    // #[serde(rename = "tradeid")]
    // trade_id: String,
    // #[serde(rename = "markettradeid")]
    // market_trade_id: String,
    base: String,
    quote: String,
    // #[serde(rename = "type")]
    // type_: u8,
    rate: Decimal,
    amount: Decimal,
    #[serde(rename = "buyuser")]
    buy_user: String,
    #[serde(rename = "selluser")]
    sell_user: String,
    #[serde(rename = "buyerfee")]
    buyer_fee: Decimal,
    // #[serde(rename = "sellerwallet")]
    // seller_wallet: String,
    #[serde(rename = "sellerfee")]
    seller_fee: Decimal,
    // #[serde(rename = "buyerwallet")]
    // buyer_wallet: String,
    #[serde(rename = "buyerordernumber")]
    buyer_order_number: String,
    #[serde(rename = "sellerordernumber")]
    seller_order_number: String,
    #[serde(deserialize_with = "deserialize_poloniex_timestamp")]
    date: NaiveDateTime,
}

// Poloniex reported XLM as STR
fn normalize_currency(currency: &str) -> &str {
    match currency {
        "STR" => "XLM",
        _ => currency,
    }
}

impl From<PoloniexDeposit> for Transaction {
    fn from(item: PoloniexDeposit) -> Self {
        let currency = normalize_currency(item.currency.as_str());
        let mut tx = Transaction::receive(
            item.timestamp,
            Amount::new(item.amount, currency.to_owned()),
        );
        tx.description = Some(item.address);
        tx
    }
}

impl From<PoloniexWithdrawal> for Transaction {
    fn from(item: PoloniexWithdrawal) -> Self {
        let currency = normalize_currency(item.currency.as_str());
        let mut tx = Transaction::send(
            item.timestamp,
            Amount::new(item.amount - item.fee_deducted, currency.to_owned()),
        );
        tx.fee = Some(Amount::new(item.fee_deducted, currency.to_owned()));
        tx.description = item.address;
        tx.tx_hash = if item.status.starts_with("COMPLETE: ") {
            Some(item.status.trim_start_matches("COMPLETE: ").to_owned())
        } else {
            None
        };
        tx.blockchain = Some(currency.to_owned());
        tx
    }
}

impl TryFrom<PoloniexTrade> for Transaction {
    type Error = &'static str;

    fn try_from(item: PoloniexTrade) -> Result<Self, Self::Error> {
        let (base_currency, quote_currency) = match (
            &item.market,
            &item.base_currency_name,
            &item.quote_currency_name,
        ) {
            (Some(market), _, _) => {
                // split record.market at the underscore or dash to obtain the base_currency and the quote_currency
                let mut split = market.split('_');
                match (split.next(), split.next()) {
                    (Some(base_currency), Some(quote_currency)) => {
                        Ok::<(&str, &str), &'static str>((base_currency, quote_currency))
                    }
                    _ => {
                        let mut split = market.split('-');
                        match (split.next(), split.next()) {
                            (Some(quote_currency), Some(base_currency)) => {
                                Ok((base_currency, quote_currency))
                            }
                            _ => return Err("Invalid Poloniex market"),
                        }
                    }
                }
            }
            (None, Some(base_currency), Some(quote_currency)) => {
                Ok((base_currency.as_str(), quote_currency.as_str()))
            }
            _ => return Err("Could not determine base_currency and quote_currency"),
        }?;

        let quote_currency = normalize_currency(quote_currency);
        let base_currency = normalize_currency(base_currency);
        let fee_currency = normalize_currency(&item.fee_currency);

        // Poloniex does not provide the total amount, so we need to calculate it based on the price
        // and amount. We truncate the result to 8 decimal places and hope it's accurate enough.
        let total = item.total.unwrap_or_else(|| {
            (item.price * item.amount).round_dp_with_strategy(8, RoundingStrategy::ToZero)
        });

        let mut tx = match item.side {
            Operation::Buy => Transaction::trade(
                item.timestamp,
                Amount::new(item.amount, base_currency.to_owned()),
                Amount::new(total, quote_currency.to_owned()),
            ),
            Operation::Sell => Transaction::trade(
                item.timestamp,
                Amount::new(total, quote_currency.to_owned()),
                Amount::new(item.amount, base_currency.to_owned()),
            ),
        };

        // Some Poloniex export formats report a more precise fee than the one
        // that is actually calculated, judging by balance errors.
        let actual_fee = item
            .fee_total
            .round_dp_with_strategy(8, RoundingStrategy::ToZero);
        tx.fee = Some(Amount::new(actual_fee, fee_currency.to_owned()));
        tx.description = item.order_number.map(|n| format!("Order #{}", n));

        Ok(tx)
    }
}

// loads a Poloniex Deposits CSV file into a list of unified transactions
fn load_poloniex_deposits_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexDeposit = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Withdrawals CSV file into a list of unified transactions
fn load_poloniex_withdrawals_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexWithdrawal = result?;
        transactions.push(record.into());
    }

    Ok(transactions)
}

// loads a Poloniex Trades CSV file into a list of unified transactions
fn load_poloniex_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for result in rdr.deserialize() {
        let record: PoloniexTrade = result?;
        match Transaction::try_from(record) {
            Ok(tx) => transactions.push(tx),
            Err(err) => {
                println!("Error: {:?}", err);
                continue;
            }
        };
    }

    Ok(transactions)
}

fn load_poloniex_trades_before_august_2022_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    // First, load all records into a vector
    let mut records: Vec<PoloniexTradeBeforeAugust2022> = Vec::new();
    for result in rdr.deserialize() {
        records.push(result?);
    }

    // Determine our user ID by finding an ID that appears on every trade (either as buyer or seller)
    let user_id = if records.is_empty() {
        None
    } else {
        let cand_a = &records[0].buy_user;
        let cand_b = &records[0].sell_user;

        if records
            .iter()
            .all(|r| &r.buy_user == cand_a || &r.sell_user == cand_a)
        {
            Some(cand_a.clone())
        } else if records
            .iter()
            .all(|r| &r.buy_user == cand_b || &r.sell_user == cand_b)
        {
            Some(cand_b.clone())
        } else {
            None
        }
    };

    let user_id =
        user_id.ok_or_else(|| anyhow::anyhow!("Could not determine user ID from trade records"))?;

    // Now convert records to transactions using the determined user ID
    for record in records {
        let base_currency = normalize_currency(&record.base);
        let quote_currency = normalize_currency(&record.quote);

        let total =
            (record.rate * record.amount).round_dp_with_strategy(8, RoundingStrategy::ToZero);

        let (incoming, outgoing, fee_rate, order_number) = if record.buy_user == user_id {
            // We are the buyer: we receive quote currency, we send base currency
            // e.g., base=BTC, quote=LTC: we pay BTC (total), receive LTC (amount)
            (
                Amount::new(record.amount, quote_currency.to_owned()),
                Amount::new(total, base_currency.to_owned()),
                record.buyer_fee,
                record.buyer_order_number,
            )
        } else {
            // We are the seller: we receive base currency, we send quote currency
            // e.g., base=BTC, quote=LTC: we pay LTC (amount), receive BTC (total)
            (
                Amount::new(total, base_currency.to_owned()),
                Amount::new(record.amount, quote_currency.to_owned()),
                record.seller_fee,
                record.seller_order_number,
            )
        };

        let mut tx = Transaction::trade(record.date, incoming.clone(), outgoing);

        let fee =
            (incoming.quantity * fee_rate).round_dp_with_strategy(8, RoundingStrategy::ToZero);
        tx.fee = Some(Amount::new(fee, incoming.currency));
        tx.description = Some(format!("Order #{}", order_number));

        transactions.push(tx);
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static POLONIEX_DEPOSITS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "PoloniexDepositsCsv",
    label: "Poloniex Deposits (CSV)",
    csv: &[
        CsvSpec::new(&["Currency", "Amount", "Address", "Date", "Status"]),
        CsvSpec::new(&["", "timestamp", "currency", "amount", "address", "status"]),
        CsvSpec::new(&[
            "f_created_at",
            "currency",
            "f_amount",
            "f_address",
            "f_status",
        ]),
    ],
    detect: None,
    load_sync: Some(load_poloniex_deposits_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static POLONIEX_TRADES_BEFORE_AUGUST_2022_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "PoloniexTradesBeforeAugust2022Csv",
    label: "Poloniex Trades (CSV, before August 2022)",
    csv: &[CsvSpec::new(&[
        "tradeid",
        "markettradeid",
        "base",
        "quote",
        "type",
        "rate",
        "amount",
        "buyuser",
        "selluser",
        "buyerfee",
        "sellerwallet",
        "sellerfee",
        "buyerwallet",
        "buyerordernumber",
        "sellerordernumber",
        "date",
    ])],
    detect: None,
    load_sync: Some(load_poloniex_trades_before_august_2022_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static POLONIEX_TRADES_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "PoloniexTradesCsv",
    label: "Poloniex Trades (CSV)",
    csv: &[
        CsvSpec::new(&[
            "Date",
            "Market",
            "Type",
            "Side",
            "Price",
            "Amount",
            "Total",
            "Fee",
            "Order Number",
            "Fee Currency",
            "Fee Total",
        ]),
        CsvSpec::new(&[
            "",
            "timestamp",
            "trade_id",
            "market",
            "wallet",
            "side",
            "price",
            "amount",
            "fee",
            "fee_currency",
            "fee_total",
        ]),
        CsvSpec::new(&[
            "order_id",
            "activity",
            "order_role",
            "order_type",
            "base_currency_name",
            "quote_currency_name",
            "fee_currency_name",
            "price",
            "amount",
            "fee_amount",
            "usd_amount",
            "usd_fee_amount",
            "utc_time",
        ]),
    ],
    detect: None,
    load_sync: Some(load_poloniex_trades_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static POLONIEX_WITHDRAWALS_CSV_SOURCE: TransactionSourceType = TransactionSourceType {
    id: "PoloniexWithdrawalsCsv",
    label: "Poloniex Withdrawals (CSV)",
    csv: &[
        CsvSpec::new(&[
            "Fee Deducted",
            "Date",
            "Currency",
            "Amount",
            "Amount-Fee",
            "Address",
            "Status",
        ]),
        CsvSpec::new(&["", "timestamp", "currency", "amount", "fee_deducted", "status"]),
        CsvSpec::new(&["f_date", "currency", "f_amount", "f_feededucted", "f_status"]),
    ],
    detect: None,
    load_sync: Some(load_poloniex_withdrawals_csv),
    load_async: None,
};
