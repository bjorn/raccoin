use std::{collections::HashMap, convert::TryFrom, path::Path};

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};
use serde::de::IntoDeserializer;

use crate::{
    base::{Amount, Transaction},
    time::parse_date_time,
    CsvSpec, TransactionSource,
};
use linkme::distributed_slice;

const KRAKEN_LEDGER_HEADERS: &[&str] = &[
    "txid",
    "refid",
    "time",
    "type",
    "subtype",
    "aclass",
    "asset",
    "wallet",
    "amount",
    "fee",
    "balance",
];

const KRAKEN_LEDGER_HEADERS_EXTENDED: &[&str] = &[
    "txid",
    "refid",
    "time",
    "type",
    "subtype",
    "aclass",
    "subclass",
    "asset",
    "wallet",
    "amount",
    "fee",
    "balance",
];

const KRAKEN_TRADES_HEADERS: &[&str] = &[
    "txid",
    "ordertxid",
    "pair",
    "time",
    "type",
    "ordertype",
    "price",
    "cost",
    "fee",
    "vol",
    "margin",
    "misc",
    "ledgers",
];

const KRAKEN_TRADES_HEADERS_EXTENDED: &[&str] = &[
    "txid",
    "ordertxid",
    "pair",
    "aclass",
    "subclass",
    "time",
    "type",
    "ordertype",
    "price",
    "cost",
    "fee",
    "vol",
    "margin",
    "misc",
    "ledgers",
    "posttxid",
    "posstatuscode",
    "cprice",
    "ccost",
    "cfee",
    "cvol",
    "cmargin",
    "net",
    "trades",
];

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "lowercase")]
enum LedgerType {
    Trade,
    #[serde(rename = "margin trade")]
    MarginTrade,
    Earn,
    Rollover,
    Deposit,
    Withdrawal,
    Transfer,
    Adjustment,
    Spend,
    Receive,
    Settled,
    Staking,
    #[serde(rename = "invite bonus")]
    InviteBonus,
}

impl LedgerType {
    fn label(&self) -> &'static str {
        match self {
            LedgerType::Trade => "trade",
            LedgerType::MarginTrade => "margin trade",
            LedgerType::Earn => "earn",
            LedgerType::Rollover => "rollover",
            LedgerType::Deposit => "deposit",
            LedgerType::Withdrawal => "withdrawal",
            LedgerType::Transfer => "transfer",
            LedgerType::Adjustment => "adjustment",
            LedgerType::Spend => "spend",
            LedgerType::Receive => "receive",
            LedgerType::Settled => "settled",
            LedgerType::Staking => "staking",
            LedgerType::InviteBonus => "invite bonus",
        }
    }
}

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "lowercase")]
enum LedgerSubtype {
    Allocation,
    Deallocation,
    Autoallocate,
    Reward,
    Migration,
    SpotToStaking,
    StakingFromSpot,
    StakingToSpot,
    SpotFromStaking,
    SpotToFutures,
    SpotFromFutures,
}

impl LedgerSubtype {
    fn is_internal_transfer(&self) -> bool {
        matches!(
            self,
            LedgerSubtype::Allocation
                | LedgerSubtype::Deallocation
                | LedgerSubtype::Autoallocate
                | LedgerSubtype::Migration
                | LedgerSubtype::SpotToStaking
                | LedgerSubtype::StakingFromSpot
                | LedgerSubtype::StakingToSpot
                | LedgerSubtype::SpotFromStaking
                | LedgerSubtype::SpotToFutures
                | LedgerSubtype::SpotFromFutures
        )
    }
}

fn deserialize_optional_subtype<'de, D>(d: D) -> std::result::Result<Option<LedgerSubtype>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: String = Deserialize::deserialize(d)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        LedgerSubtype::deserialize(trimmed.into_deserializer()).map(Some)
    }
}

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "lowercase")]
enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
struct KrakenLedgerRecord {
    /// Transaction ID
    // txid: String,
    refid: String,
    #[serde(deserialize_with = "deserialize_kraken_timestamp")]
    time: NaiveDateTime,
    #[serde(rename = "type")]
    type_: LedgerType,
    #[serde(deserialize_with = "deserialize_optional_subtype")]
    subtype: Option<LedgerSubtype>,
    // /// Asset Class. Value is always "currency". Not a useful field.
    // aclass: String,
    /// The asset of focus in the ledger entry.
    asset: String,
    // /// Generally "spot / main"
    // wallet: String,
    /// Amount debited (-) or credited (+) to that asset's balance.
    amount: Decimal,
    //// Fee paid to Kraken (if any) in the asset.
    fee: Decimal,
    // /// New asset balance after debiting/crediting transaction amount and debiting fee.
    // /// balance = old_balance +/- amount - fee
    // balance: Decimal,
}

impl KrakenLedgerRecord {
    fn normalized_currency(&self) -> &str {
        normalize_currency(&self.asset)
    }

    fn amount_abs(&self) -> Amount {
        Amount::new(self.amount.abs(), self.normalized_currency().to_owned())
    }

    fn fee_amount(&self) -> Option<Amount> {
        if self.fee.is_zero() {
            None
        } else {
            Some(Amount::new(self.fee, self.normalized_currency().to_owned()))
        }
    }

    fn is_incoming(&self) -> bool {
        self.amount >= Decimal::ZERO
    }

    fn compose_description(&self) -> Option<String> {
        let mut parts = Vec::new();

        parts.push(format!("Ledger: {}", self.type_.label()));

        if let Some(refid) = non_empty(&self.refid) {
            parts.push(format!("Ref: {}", refid));
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" | "))
        }
    }

    fn into_transaction(self) -> Result<Option<Transaction>> {
        if matches!(self.type_, LedgerType::Trade) {
            return Ok(None);
        }

        if matches!(self.subtype, Some(subtype) if subtype.is_internal_transfer()) {
            return Ok(None);
        }

        let timestamp = self.time;
        let amount = self.amount_abs();
        let is_fiat = amount.is_fiat();

        let mut tx = match self.type_ {
            LedgerType::Deposit | LedgerType::Receive | LedgerType::InviteBonus => {
                if is_fiat {
                    Transaction::fiat_deposit(timestamp, amount)
                } else {
                    Transaction::receive(timestamp, amount)
                }
            }
            LedgerType::Earn | LedgerType::Staking => {
                if self.is_incoming() {
                    if is_fiat {
                        Transaction::fiat_deposit(timestamp, amount)
                    } else {
                        Transaction::new(timestamp, crate::base::Operation::Staking(amount))
                    }
                } else if is_fiat {
                    Transaction::fiat_withdrawal(timestamp, amount)
                } else {
                    Transaction::send(timestamp, amount)
                }
            }
            LedgerType::Withdrawal | LedgerType::Spend => {
                if is_fiat {
                    Transaction::fiat_withdrawal(timestamp, amount)
                } else {
                    Transaction::send(timestamp, amount)
                }
            }
            LedgerType::Transfer
            | LedgerType::Rollover
            | LedgerType::Adjustment
            | LedgerType::Settled
            | LedgerType::MarginTrade => {
                if self.is_incoming() {
                    if is_fiat {
                        Transaction::fiat_deposit(timestamp, amount)
                    } else {
                        Transaction::receive(timestamp, amount)
                    }
                } else if is_fiat {
                    Transaction::fiat_withdrawal(timestamp, amount)
                } else {
                    Transaction::send(timestamp, amount)
                }
            }
            LedgerType::Trade => return Ok(None),
        };

        tx.fee = self.fee_amount();
        tx.description = self.compose_description();

        Ok(Some(tx))
    }
}

struct PendingLedgerTrade {
    spend: Option<KrakenLedgerRecord>,
    receive: Option<KrakenLedgerRecord>,
    trade_out: Option<KrakenLedgerRecord>,
    trade_in: Option<KrakenLedgerRecord>,
}

impl PendingLedgerTrade {
    fn new() -> Self {
        Self {
            spend: None,
            receive: None,
            trade_out: None,
            trade_in: None,
        }
    }

    fn insert_row(&mut self, row: KrakenLedgerRecord) {
        match row.type_ {
            LedgerType::Spend => {
                self.spend = Some(row);
            }
            LedgerType::Receive => {
                self.receive = Some(row);
            }
            LedgerType::Trade => {
                if row.is_incoming() {
                    self.trade_in = Some(row);
                } else {
                    self.trade_out = Some(row);
                }
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        (self.spend.is_some() && self.receive.is_some())
            || (self.trade_out.is_some() && self.trade_in.is_some())
    }

    fn take_trade(self, refid: &str) -> Result<Option<Transaction>> {
        let Self {
            trade_out,
            trade_in,
            spend,
            receive,
        } = self;

        if let (Some(outgoing), Some(incoming)) = (trade_out, trade_in) {
            let incoming_amount = incoming.amount_abs();
            let outgoing_amount = outgoing.amount_abs();
            let mut tx = Transaction::trade(outgoing.time, incoming_amount, outgoing_amount);
            tx.fee = outgoing.fee_amount().or_else(|| incoming.fee_amount());
            tx.description = Some(format!("Ref: {}", refid));
            return Ok(Some(tx));
        }

        match (spend, receive) {
            (Some(spend), Some(receive)) => {
                let incoming = receive.amount_abs();
                let outgoing = spend.amount_abs();
                let mut tx = Transaction::trade(spend.time, incoming, outgoing);
                tx.fee = spend.fee_amount().or_else(|| receive.fee_amount());
                tx.description = Some(format!("Ledger: spend/receive | Ref: {}", refid));
                Ok(Some(tx))
            }
            (Some(spend), None) => spend.into_transaction(),
            (None, Some(receive)) => receive.into_transaction(),
            (None, None) => Ok(None),
        }
    }
}

// Kraken trades CSV:
// "txid","ordertxid","pair","time","type","ordertype","price","cost","fee","vol","margin","misc","ledgers"
#[derive(Debug, Deserialize)]
struct KrakenTradeRecord {
    // txid: String,
    ordertxid: String,
    /// Base currency + Quote currency.
    pair: String,
    #[serde(deserialize_with = "deserialize_kraken_timestamp")]
    time: NaiveDateTime,
    #[serde(rename = "type")]
    type_: TradeSide,
    // ordertype: String,
    // price: Decimal,
    /// Amount of quote currency deducted on buy or received on sell (does not include fees).
    cost: Decimal,
    /// Fee amount in quote currency (not necessarily the fee that was deducted from the account!).
    fee: Decimal,
    /// Amount of the base currency bought/sold.
    vol: Decimal,
    // /// Amount of used margin (in quote currency)
    // margin: Decimal,
    // misc: String,
    // /// Corresponding ledger entry IDs
    // ledgers: String,
}

impl KrakenTradeRecord {
    fn parse_pair(&self) -> Result<(String, String)> {
        let mut split = self.pair.split('/');
        match (split.next(), split.next()) {
            (Some(base), Some(quote)) => {
                Ok((
                    normalize_currency(base).to_owned(),
                    normalize_currency(quote).to_owned(),
                ))
            }
            _ => Err(anyhow!(
                "Invalid pair value '{}', expected '<base>/<quote>'",
                self.pair
            )),
        }
    }

    fn description(&self) -> Option<String> {
        non_empty(&self.ordertxid).map(|order_id| format!("Order ID: {}", order_id))
    }
}

impl TryFrom<KrakenTradeRecord> for Transaction {
    type Error = anyhow::Error;

    fn try_from(record: KrakenTradeRecord) -> Result<Self> {
        let (base_currency, quote_currency) = record.parse_pair()?;
        let base = Amount::new(record.vol, base_currency);
        let quote = Amount::new(record.cost, quote_currency.clone());

        let mut tx = match record.type_ {
            TradeSide::Buy => Transaction::trade(record.time, base, quote),
            TradeSide::Sell => Transaction::trade(record.time, quote, base),
        };

        if !record.fee.is_zero() {
            tx.fee = Some(Amount::new(record.fee, quote_currency));
        }

        tx.description = record.description();

        Ok(tx)
    }
}

fn process_kraken_ledger_record(
    row: KrakenLedgerRecord,
    pending_by_refid: &mut HashMap<String, PendingLedgerTrade>,
    transactions: &mut Vec<Transaction>,
) -> Result<()> {
    let timestamp = row.time;

    if matches!(row.subtype, Some(subtype) if subtype.is_internal_transfer()) {
        return Ok(());
    }

    match (row.type_, non_empty(&row.refid).map(|id| id.to_owned())) {
        (LedgerType::Spend | LedgerType::Receive | LedgerType::Trade, Some(refid)) => {
            let mut entry = pending_by_refid.remove(&refid).unwrap_or_else(PendingLedgerTrade::new);
            entry.insert_row(row);

            if entry.is_complete() {
                if let Some(tx) = entry.take_trade(&refid)? {
                    transactions.push(tx);
                }
            } else {
                pending_by_refid.insert(refid, entry);
            }
        }
        _ => {
            if let Some(tx) = row
                .into_transaction()
                .with_context(|| format!("Failed to convert Kraken ledger row dated {}", timestamp))?
            {
                transactions.push(tx);
            }
        }
    }

    Ok(())
}

fn drain_pending_kraken_trades(
    pending_by_refid: HashMap<String, PendingLedgerTrade>,
    transactions: &mut Vec<Transaction>,
) -> Result<()> {
    for (refid, entry) in pending_by_refid {
        if let Some(tx) = entry.take_trade(&refid)? {
            transactions.push(tx);
        }
    }

    Ok(())
}

fn load_kraken_ledger_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();
    let mut pending_by_refid: HashMap<String, PendingLedgerTrade> = HashMap::new();

    for record in reader.deserialize() {
        let row: KrakenLedgerRecord = record?;
        process_kraken_ledger_record(row, &mut pending_by_refid, &mut transactions)?;
    }

    drain_pending_kraken_trades(pending_by_refid, &mut transactions)?;

    Ok(transactions)
}

fn load_kraken_trades_csv(input_path: &Path) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new().from_path(input_path)?;
    let mut transactions = Vec::new();

    for record in reader.deserialize() {
        let row: KrakenTradeRecord = record?;
        let timestamp = row.time;
        let tx = Transaction::try_from(row)
            .with_context(|| format!("Failed to convert Kraken trade row dated {}", timestamp))?;
        transactions.push(tx);
    }

    Ok(transactions)
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static KRAKEN_LEDGER_CSV: TransactionSource = TransactionSource {
    id: "KrakenLedgerCsv",
    label: "Kraken Ledger (CSV)",
    csv: &[
        CsvSpec::new(KRAKEN_LEDGER_HEADERS),
        CsvSpec::new(KRAKEN_LEDGER_HEADERS_EXTENDED),
    ],
    detect: None,
    load_sync: Some(load_kraken_ledger_csv),
    load_async: None,
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static KRAKEN_TRADES_CSV: TransactionSource = TransactionSource {
    id: "KrakenTradesCsv",
    label: "Kraken Trades (CSV)",
    csv: &[
        CsvSpec::new(KRAKEN_TRADES_HEADERS),
        CsvSpec::new(KRAKEN_TRADES_HEADERS_EXTENDED),
    ],
    detect: None,
    load_sync: Some(load_kraken_trades_csv),
    load_async: None,
};

fn deserialize_kraken_timestamp<'de, D>(d: D) -> std::result::Result<NaiveDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: &str = Deserialize::deserialize(d)?;
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| parse_date_time(raw))
        .map_err(|e| {
            serde::de::Error::custom(format!(
                "Failed to parse datetime '{}': {} (expected format: %Y-%m-%d %H:%M:%S[.f])",
                raw, e
            ))
        })
}

fn normalize_currency(raw: &str) -> &str {
    let trimmed = raw.trim();
    let base = match trimmed.split_once('.') {
        Some((base, _)) => base,
        None => trimmed,
    };

    // See https://www.reddit.com/r/KrakenSupport/comments/1i5vwd0/comment/m873xad/
    match base {
        "KFEE" => "FEE",
        "XETC" => "ETC",
        "XETH" => "ETH",
        "XLTC" => "LTC",
        "XMLN" => "MLN",
        "XREP" => "REP",
        "XXBT" | "XBT" => "BTC",
        "XXDG" | "XDG" => "DOGE",
        "XXLM" => "XLM",
        "XXMR" => "XMR",
        "XXRP" => "XRP",
        "XZEC" => "ZEC",
        "ZAUD" => "AUD",
        "ZCAD" => "CAD",
        "ZEUR" => "EUR",
        "ZGBP" => "GBP",
        "ZJPY" => "JPY",
        "ZUSD" => "USD",
        _ => base,
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Operation;
    use csv::StringRecord;
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    #[test]
    fn ledger_deposit_becomes_fiat_deposit() {
        let csv = "\"TX-DEP-001\",\"REF-DEP-001\",\"2024-01-01 00:00:00\",\"deposit\",\"\",\"currency\",\"EUR\",\"spot / main\",1000.0000,0,1000.0000";
        let tx = parse_ledger_row(csv).unwrap().unwrap();

        match tx.operation {
            Operation::FiatDeposit(amount) => {
                assert_eq!(amount.quantity, dec!(1000.0000));
                assert_eq!(amount.currency, "EUR");
            }
            other => panic!("expected fiat deposit, got {:?}", other),
        }
        assert!(tx.fee.is_none());
    }

    #[test]
    fn ledger_trade_is_skipped() {
        let csv = "\"TX-TRADE-001\",\"REF-TRADE-001\",\"2024-01-02 03:04:05\",\"trade\",\"\",\"currency\",\"EUR\",\"spot / main\",-8.1800,0.0131,293.4704";
        assert!(parse_ledger_row(csv).unwrap().is_none());
    }

    #[test]
    fn ledger_withdrawal_becomes_send() {
        let csv = "\"txid2\",\"ref2\",\"2023-02-15 09:00:00\",\"withdrawal\",\"\",\"currency\",\"BTC\",\"spot / main\",-0.001,0.00001,0.0";
        let tx = parse_ledger_row(csv).unwrap().unwrap();

        match tx.operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.001));
                assert_eq!(amount.currency, "BTC");
            }
            other => panic!("expected send, got {:?}", other),
        }
        let fee = tx.fee.expect("fee set");
        assert_eq!(fee.quantity, dec!(0.00001));
        assert_eq!(fee.currency, "BTC");
    }

    #[test]
    fn trade_row_becomes_trade_transaction() {
        let csv = "\"TX-ORDER-001\",\"ORDER-001\",\"BTC/EUR\",\"2024-01-02 03:04:05.1234\",\"buy\",\"limit\",20649.70000,8.17997,0.01309,0.00039613,0.00000,\"\",\"LEDGER-001,LEDGER-002\"";
        let tx = parse_trade_row(csv).unwrap();

        match tx.operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.00039613));
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(8.17997));
                assert_eq!(outgoing.currency, "EUR");
            }
            other => panic!("expected trade, got {:?}", other),
        }
        let fee = tx.fee.expect("fee set");
        assert_eq!(fee.quantity, dec!(0.01309));
        assert_eq!(fee.currency, "EUR");
    }

    #[test]
    fn ledger_spend_receive_pair_becomes_trade() {
        let csv = concat!(
            "\"txid\",\"refid\",\"time\",\"type\",\"subtype\",\"aclass\",\"asset\",\"wallet\",\"amount\",\"fee\",\"balance\"\n",
            "\"AAA111-BBB222-CCC333\",\"REF-PAIR-001\",\"2024-05-10 12:00:00\",\"spend\",\"\",\"currency\",\"USD\",\"spot / main\",-250.00,2.50,0.00\n",
            "\"DDD444-EEE555-FFF666\",\"REF-PAIR-001\",\"2024-05-10 12:00:00\",\"receive\",\"\",\"currency\",\"SOL\",\"spot / main\",1.250000,0,1.250000\n"
        );

        let rows = parse_ledger_rows(csv);
        let txs = process_ledger_rows(rows).unwrap();

        assert_eq!(txs.len(), 1);
        match &txs[0].operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "SOL");
                assert_eq!(incoming.quantity, dec!(1.250000));
                assert_eq!(outgoing.currency, "USD");
                assert_eq!(outgoing.quantity, dec!(250.00));
            }
            other => panic!("expected trade, got {:?}", other),
        }
        let fee = txs[0].fee.clone().expect("fee set");
        assert_eq!(fee.currency, "USD");
        assert_eq!(fee.quantity, dec!(2.50));
    }

    #[test]
    fn ledger_trade_rows_pair_into_trade_with_randomized_amounts() {
        let csv = concat!(
            "\"txid\",\"refid\",\"time\",\"type\",\"subtype\",\"aclass\",\"asset\",\"wallet\",\"amount\",\"fee\",\"balance\"\n",
            "\"TX-OUT\",\"REF-FIXED-001\",\"2024-01-02 03:04:05\",\"trade\",\"\",\"currency\",\"EUR\",\"spot / main\",-123.4567,0.12,0\n",
            "\"TX-IN\",\"REF-FIXED-001\",\"2024-01-02 03:04:05\",\"trade\",\"\",\"currency\",\"BTC\",\"spot / main\",0.00432123,0,0\n"
        );

        let rows = parse_ledger_rows(csv);
        let txs = process_ledger_rows(rows).unwrap();

        assert_eq!(txs.len(), 1);
        match &txs[0].operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(incoming.quantity, dec!(0.00432123));
                assert_eq!(outgoing.currency, "EUR");
                assert_eq!(outgoing.quantity, dec!(123.4567));
            }
            other => panic!("expected trade, got {:?}", other),
        }
        let fee_amount = txs[0].fee.clone().expect("fee set");
        assert_eq!(fee_amount.currency, "EUR");
        assert_eq!(fee_amount.quantity, dec!(0.12));
    }

    fn process_ledger_rows(rows: Vec<KrakenLedgerRecord>) -> Result<Vec<Transaction>> {
        let mut txs = Vec::new();
        let mut pending_by_refid: HashMap<String, PendingLedgerTrade> = HashMap::new();

        for row in rows {
            process_kraken_ledger_record(row, &mut pending_by_refid, &mut txs)?;
        }

        drain_pending_kraken_trades(pending_by_refid, &mut txs)?;

        Ok(txs)
    }

    fn parse_ledger_rows(csv: &str) -> Vec<KrakenLedgerRecord> {
        let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        reader
            .deserialize()
            .map(|row| row.unwrap())
            .collect::<Vec<KrakenLedgerRecord>>()
    }



    fn parse_ledger_row(csv: &str) -> Result<Option<Transaction>> {
        let header = StringRecord::from(KRAKEN_LEDGER_HEADERS);
        let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        reader.set_headers(header);
        let record: KrakenLedgerRecord = reader.deserialize().next().unwrap().unwrap();
        record.into_transaction()
    }

    fn parse_trade_row(csv: &str) -> Result<Transaction> {
        let header = StringRecord::from(KRAKEN_TRADES_HEADERS);
        let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
        reader.set_headers(header);
        let record: KrakenTradeRecord = reader.deserialize().next().unwrap().unwrap();
        Transaction::try_from(record)
    }
}
