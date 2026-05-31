use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use linkme::distributed_slice;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    base::{Amount, Operation, Transaction},
    TransactionSource,
};

pub(crate) const CONFIG_KIND: &str = "raccoin-generic-import";
const DEFAULT_DELIMITER: &str = ",";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GenericImportFormat {
    Csv,
    Json,
}

impl Default for GenericImportFormat {
    fn default() -> Self {
        Self::Csv
    }
}

impl GenericImportFormat {
    pub(crate) fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("json") => Self::Json,
            _ => Self::Csv,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct GenericImportFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) transaction_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) received_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) received_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sent_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sent_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fee_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fee_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blockchain: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GenericOperationKind {
    Buy,
    Sell,
    Trade,
    Swap,
    FiatDeposit,
    FiatWithdrawal,
    Fee,
    Receive,
    Send,
    ChainSplit,
    Expense,
    Stolen,
    Lost,
    Burn,
    Income,
    Airdrop,
    Staking,
    Cashback,
    IncomingGift,
    OutgoingGift,
    Spam,
}

impl GenericOperationKind {
    fn from_builtin_label(label: &str) -> Option<Self> {
        let normalized = normalize_type_value(label);
        match normalized.as_str() {
            "buy" | "purchase" => Some(Self::Buy),
            "sell" | "sale" => Some(Self::Sell),
            "trade" => Some(Self::Trade),
            "swap" => Some(Self::Swap),
            "fiatdeposit" | "fiat-deposit" => Some(Self::FiatDeposit),
            "fiatwithdrawal" | "fiat-withdrawal" => Some(Self::FiatWithdrawal),
            "fee" => Some(Self::Fee),
            "receive" | "received" | "deposit" | "transferin" | "transfer-in" => {
                Some(Self::Receive)
            }
            "send" | "sent" | "withdraw" | "withdrawal" | "transferout" | "transfer-out" => {
                Some(Self::Send)
            }
            "chainsplit" | "chain-split" => Some(Self::ChainSplit),
            "expense" => Some(Self::Expense),
            "stolen" => Some(Self::Stolen),
            "lost" => Some(Self::Lost),
            "burn" => Some(Self::Burn),
            "income" | "reward" | "rewards" => Some(Self::Income),
            "airdrop" => Some(Self::Airdrop),
            "staking" | "stake" => Some(Self::Staking),
            "cashback" | "cash-back" => Some(Self::Cashback),
            "incominggift" | "incoming-gift" | "giftin" | "gift-in" => Some(Self::IncomingGift),
            "outgoinggift" | "outgoing-gift" | "giftout" | "gift-out" => Some(Self::OutgoingGift),
            "spam" => Some(Self::Spam),
            _ => None,
        }
    }

    fn into_operation(
        self,
        incoming: Option<Amount>,
        outgoing: Option<Amount>,
    ) -> Result<Operation> {
        let incoming_or_outgoing =
            |incoming: Option<Amount>, outgoing: Option<Amount>, label: &str| -> Result<Amount> {
                incoming
                    .or(outgoing)
                    .with_context(|| format!("Missing amount for {label} operation"))
            };
        let outgoing_or_incoming =
            |outgoing: Option<Amount>, incoming: Option<Amount>, label: &str| -> Result<Amount> {
                outgoing
                    .or(incoming)
                    .with_context(|| format!("Missing amount for {label} operation"))
            };
        let both = |incoming: Option<Amount>,
                    outgoing: Option<Amount>,
                    label: &str|
         -> Result<(Amount, Amount)> {
            let incoming = incoming
                .with_context(|| format!("Missing received amount for {label} operation"))?;
            let outgoing =
                outgoing.with_context(|| format!("Missing sent amount for {label} operation"))?;
            Ok((incoming, outgoing))
        };

        Ok(match self {
            Self::Buy => match (incoming, outgoing) {
                (Some(incoming), Some(outgoing)) => Operation::Trade { incoming, outgoing },
                (incoming, outgoing) => {
                    Operation::Buy(incoming_or_outgoing(incoming, outgoing, "buy")?)
                }
            },
            Self::Sell => match (incoming, outgoing) {
                (Some(incoming), Some(outgoing)) => Operation::Trade { incoming, outgoing },
                (incoming, outgoing) => {
                    Operation::Sell(outgoing_or_incoming(outgoing, incoming, "sell")?)
                }
            },
            Self::Trade => {
                let (incoming, outgoing) = both(incoming, outgoing, "trade")?;
                Operation::Trade { incoming, outgoing }
            }
            Self::Swap => {
                let (incoming, outgoing) = both(incoming, outgoing, "swap")?;
                Operation::Swap { incoming, outgoing }
            }
            Self::FiatDeposit => {
                Operation::FiatDeposit(incoming_or_outgoing(incoming, outgoing, "fiat deposit")?)
            }
            Self::FiatWithdrawal => Operation::FiatWithdrawal(outgoing_or_incoming(
                outgoing,
                incoming,
                "fiat withdrawal",
            )?),
            Self::Fee => Operation::Fee(outgoing_or_incoming(outgoing, incoming, "fee")?),
            Self::Receive => {
                Operation::Receive(incoming_or_outgoing(incoming, outgoing, "receive")?)
            }
            Self::Send => Operation::Send(outgoing_or_incoming(outgoing, incoming, "send")?),
            Self::ChainSplit => {
                Operation::ChainSplit(incoming_or_outgoing(incoming, outgoing, "chain split")?)
            }
            Self::Expense => {
                Operation::Expense(outgoing_or_incoming(outgoing, incoming, "expense")?)
            }
            Self::Stolen => Operation::Stolen(outgoing_or_incoming(outgoing, incoming, "stolen")?),
            Self::Lost => Operation::Lost(outgoing_or_incoming(outgoing, incoming, "lost")?),
            Self::Burn => Operation::Burn(outgoing_or_incoming(outgoing, incoming, "burn")?),
            Self::Income => Operation::Income(incoming_or_outgoing(incoming, outgoing, "income")?),
            Self::Airdrop => {
                Operation::Airdrop(incoming_or_outgoing(incoming, outgoing, "airdrop")?)
            }
            Self::Staking => {
                Operation::Staking(incoming_or_outgoing(incoming, outgoing, "staking")?)
            }
            Self::Cashback => {
                Operation::Cashback(incoming_or_outgoing(incoming, outgoing, "cashback")?)
            }
            Self::IncomingGift => {
                Operation::IncomingGift(incoming_or_outgoing(incoming, outgoing, "incoming gift")?)
            }
            Self::OutgoingGift => {
                Operation::OutgoingGift(outgoing_or_incoming(outgoing, incoming, "outgoing gift")?)
            }
            Self::Spam => Operation::Spam(incoming_or_outgoing(incoming, outgoing, "spam")?),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GenericImportConfig {
    pub(crate) kind: String,
    pub(crate) version: u32,
    pub(crate) source_file: String,
    pub(crate) format: GenericImportFormat,
    pub(crate) delimiter: String,
    pub(crate) has_headers: bool,
    pub(crate) trim: bool,
    pub(crate) datetime_formats: Vec<String>,
    pub(crate) fields: GenericImportFields,
    pub(crate) type_mappings: BTreeMap<String, GenericOperationKind>,
}

impl Default for GenericImportConfig {
    fn default() -> Self {
        Self {
            kind: CONFIG_KIND.to_owned(),
            version: 1,
            source_file: String::new(),
            format: GenericImportFormat::Csv,
            delimiter: DEFAULT_DELIMITER.to_owned(),
            has_headers: true,
            trim: true,
            datetime_formats: default_datetime_formats(),
            fields: GenericImportFields::default(),
            type_mappings: BTreeMap::new(),
        }
    }
}

pub(crate) struct GenericImportFilePreview {
    pub(crate) fields: Vec<String>,
    pub(crate) format: GenericImportFormat,
    pub(crate) delimiter: String,
}

pub(crate) fn default_datetime_formats() -> Vec<String> {
    [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
        "%d/%m/%Y %H:%M:%S",
        "%d/%m/%Y %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(crate) fn add_type_mappings(
    mappings: &mut BTreeMap<String, GenericOperationKind>,
    raw_values: &str,
    kind: GenericOperationKind,
) {
    for raw_value in raw_values
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        mappings.insert(raw_value.to_owned(), kind);
    }
}

pub(crate) fn preview_import_file(path: &Path) -> Result<GenericImportFilePreview> {
    match GenericImportFormat::from_path(path) {
        GenericImportFormat::Csv => preview_csv_file(path),
        GenericImportFormat::Json => preview_json_file(path),
    }
}

pub(crate) fn detect_generic_import_config(input_path: &Path) -> Result<bool> {
    if !input_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Ok(false);
    }

    let json = std::fs::read_to_string(input_path)?;
    let value: Value = serde_json::from_str(&json)?;
    Ok(value
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == CONFIG_KIND))
}

pub(crate) fn load_generic_import_config(input_path: &Path) -> Result<Vec<Transaction>> {
    let json = std::fs::read_to_string(input_path)
        .with_context(|| format!("Could not read import mapping {}", input_path.display()))?;
    let config: GenericImportConfig = serde_json::from_str(&json)
        .with_context(|| format!("Could not parse import mapping {}", input_path.display()))?;

    if config.kind != CONFIG_KIND {
        bail!("Unsupported import mapping kind '{}'", config.kind);
    }

    let source_path = resolve_source_path(input_path, &config.source_file);
    let records = read_records(&source_path, &config)
        .with_context(|| format!("Could not read source file {}", source_path.display()))?;

    records
        .iter()
        .map(|record| {
            record_to_transaction(record, &config)
                .with_context(|| format!("Could not import row {}", record.row_number))
        })
        .collect()
}

fn preview_csv_file(path: &Path) -> Result<GenericImportFilePreview> {
    let delimiter = detect_csv_delimiter(path)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(csv::Trim::All)
        .from_path(path)?;
    let fields = reader.headers()?.iter().map(str::to_owned).collect();

    Ok(GenericImportFilePreview {
        fields,
        format: GenericImportFormat::Csv,
        delimiter: delimiter_as_string(delimiter),
    })
}

fn preview_json_file(path: &Path) -> Result<GenericImportFilePreview> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let first =
        first_json_record(&value).context("JSON import source did not contain an object record")?;
    let mut fields = BTreeMap::new();
    flatten_json("", first, &mut fields);

    Ok(GenericImportFilePreview {
        fields: fields.keys().cloned().collect(),
        format: GenericImportFormat::Json,
        delimiter: DEFAULT_DELIMITER.to_owned(),
    })
}

fn read_records(source_path: &Path, config: &GenericImportConfig) -> Result<Vec<GenericRecord>> {
    match config.format {
        GenericImportFormat::Csv => read_csv_records(source_path, config),
        GenericImportFormat::Json => read_json_records(source_path),
    }
}

fn read_csv_records(
    source_path: &Path,
    config: &GenericImportConfig,
) -> Result<Vec<GenericRecord>> {
    let delimiter = config_delimiter(config)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(config.has_headers)
        .trim(if config.trim {
            csv::Trim::All
        } else {
            csv::Trim::None
        })
        .from_path(source_path)?;

    let headers = if config.has_headers {
        reader
            .headers()?
            .iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let mut records = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record?;
        let mut fields = BTreeMap::new();

        for (column_index, value) in record.iter().enumerate() {
            let key = headers
                .get(column_index)
                .cloned()
                .unwrap_or_else(|| format!("column_{}", column_index + 1));
            fields.insert(key, value.to_owned());
        }

        records.push(GenericRecord {
            row_number: index + if config.has_headers { 2 } else { 1 },
            fields,
        });
    }

    Ok(records)
}

fn read_json_records(source_path: &Path) -> Result<Vec<GenericRecord>> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(source_path)?)?;
    let records =
        json_records(&value).context("JSON import source did not contain object records")?;

    records
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let mut fields = BTreeMap::new();
            flatten_json("", value, &mut fields);
            Ok(GenericRecord {
                row_number: index + 1,
                fields,
            })
        })
        .collect()
}

#[derive(Debug)]
struct GenericRecord {
    row_number: usize,
    fields: BTreeMap<String, String>,
}

fn record_to_transaction(
    record: &GenericRecord,
    config: &GenericImportConfig,
) -> Result<Transaction> {
    let timestamp_raw = required_field(record, &config.fields.timestamp, "timestamp")?;
    let timestamp = parse_timestamp(&timestamp_raw, &config.datetime_formats)?;

    let incoming = read_amount(
        record,
        &config.fields.received_amount,
        &config.fields.received_currency,
        "received",
    )?;
    let outgoing = read_amount(
        record,
        &config.fields.sent_amount,
        &config.fields.sent_currency,
        "sent",
    )?;

    let operation_kind = operation_kind(record, config, incoming.is_some(), outgoing.is_some())?;
    let operation = operation_kind.into_operation(incoming, outgoing)?;

    let mut transaction = Transaction::new(timestamp, operation);
    transaction.fee = read_amount(
        record,
        &config.fields.fee_amount,
        &config.fields.fee_currency,
        "fee",
    )?;
    transaction.value = read_amount(
        record,
        &config.fields.value_amount,
        &config.fields.value_currency,
        "value",
    )?;
    transaction.tx_hash = optional_field(record, &config.fields.tx_hash);
    transaction.description = optional_field(record, &config.fields.description);
    transaction.blockchain = optional_field(record, &config.fields.blockchain);

    Ok(transaction)
}

fn operation_kind(
    record: &GenericRecord,
    config: &GenericImportConfig,
    has_incoming: bool,
    has_outgoing: bool,
) -> Result<GenericOperationKind> {
    if let Some(raw_type) = optional_field(record, &config.fields.transaction_type) {
        if let Some(kind) = config.type_mappings.get(raw_type.trim()).copied() {
            return Ok(kind);
        }

        let normalized_type = normalize_type_value(&raw_type);
        if let Some((_, kind)) = config
            .type_mappings
            .iter()
            .find(|(raw, _)| normalize_type_value(raw) == normalized_type)
        {
            return Ok(*kind);
        }

        return GenericOperationKind::from_builtin_label(&raw_type)
            .with_context(|| format!("Unmapped transaction type '{}'", raw_type));
    }

    match (has_incoming, has_outgoing) {
        (true, true) => Ok(GenericOperationKind::Trade),
        (true, false) => Ok(GenericOperationKind::Receive),
        (false, true) => Ok(GenericOperationKind::Send),
        (false, false) => bail!("Missing transaction type and amount fields"),
    }
}

fn required_field(record: &GenericRecord, field: &Option<String>, label: &str) -> Result<String> {
    optional_field(record, field).with_context(|| format!("Missing {label} field"))
}

fn optional_field(record: &GenericRecord, field: &Option<String>) -> Option<String> {
    let field = field.as_deref()?.trim();
    if field.is_empty() {
        return None;
    }

    record
        .fields
        .get(field)
        .or_else(|| {
            record
                .fields
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(field))
                .map(|(_, value)| value)
        })
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn read_amount(
    record: &GenericRecord,
    amount_field: &Option<String>,
    currency_field: &Option<String>,
    label: &str,
) -> Result<Option<Amount>> {
    let Some(quantity_raw) = optional_field(record, amount_field) else {
        return Ok(None);
    };

    let quantity = parse_decimal(&quantity_raw)
        .with_context(|| format!("Could not parse {label} amount '{}'", quantity_raw))?;
    if quantity.is_zero() {
        return Ok(None);
    }

    let currency = required_field(record, currency_field, &format!("{label} currency"))?;
    Ok(Some(Amount::new(quantity.abs(), currency)))
}

fn parse_decimal(raw: &str) -> Result<Decimal> {
    let mut value = raw.trim();
    let mut negative = false;

    if value.starts_with('(') && value.ends_with(')') {
        negative = true;
        value = &value[1..value.len() - 1];
    }

    let normalized = value.replace(',', "").replace(' ', "");
    let mut decimal = Decimal::from_str(&normalized)?;
    if negative {
        decimal = -decimal;
    }
    Ok(decimal)
}

fn parse_timestamp(raw: &str, formats: &[String]) -> Result<NaiveDateTime> {
    let raw = raw.trim();

    if let Ok(datetime) = DateTime::parse_from_rfc3339(raw) {
        return Ok(datetime.naive_utc());
    }

    for format in formats {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(raw, format) {
            return Ok(datetime);
        }
        if let Ok(date) = NaiveDate::parse_from_str(raw, format) {
            return date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow!("Could not convert date to midnight"));
        }
    }

    bail!("Could not parse timestamp '{}'", raw)
}

fn first_json_record(value: &Value) -> Option<&Value> {
    json_records(value).and_then(|records| records.first().copied())
}

fn json_records(value: &Value) -> Option<Vec<&Value>> {
    match value {
        Value::Array(items) => Some(items.iter().filter(|item| item.is_object()).collect()),
        Value::Object(map) => {
            for key in ["transactions", "records", "items", "data"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return Some(items.iter().filter(|item| item.is_object()).collect());
                }
            }
            Some(vec![value])
        }
        _ => None,
    }
}

fn flatten_json(prefix: &str, value: &Value, fields: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_json(&path, value, fields);
            }
        }
        Value::String(value) => {
            fields.insert(prefix.to_owned(), value.clone());
        }
        Value::Number(value) => {
            fields.insert(prefix.to_owned(), value.to_string());
        }
        Value::Bool(value) => {
            fields.insert(prefix.to_owned(), value.to_string());
        }
        Value::Null | Value::Array(_) => {}
    }
}

fn resolve_source_path(config_path: &Path, source_file: &str) -> PathBuf {
    let source_path = Path::new(source_file);
    if source_path.is_absolute() {
        source_path.to_owned()
    } else {
        config_path
            .parent()
            .unwrap_or(Path::new(""))
            .join(source_path)
    }
}

fn config_delimiter(config: &GenericImportConfig) -> Result<u8> {
    match config.delimiter.as_str() {
        "\\t" | "tab" => Ok(b'\t'),
        delimiter if delimiter.len() == 1 => Ok(delimiter.as_bytes()[0]),
        delimiter => bail!("Unsupported CSV delimiter '{}'", delimiter),
    }
}

fn detect_csv_delimiter(path: &Path) -> Result<u8> {
    let file = File::open(path)?;
    let mut first_line = String::new();
    BufReader::new(file).read_line(&mut first_line)?;

    let candidates = [b',', b';', b'\t'];
    Ok(candidates
        .into_iter()
        .max_by_key(|candidate| {
            first_line
                .as_bytes()
                .iter()
                .filter(|byte| *byte == candidate)
                .count()
        })
        .unwrap_or(b','))
}

fn delimiter_as_string(delimiter: u8) -> String {
    match delimiter {
        b'\t' => "\\t".to_owned(),
        other => (other as char).to_string(),
    }
}

fn normalize_type_value(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect()
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static GENERIC_IMPORT: TransactionSource = TransactionSource {
    id: "GenericImport",
    label: "Generic CSV/JSON mapping",
    csv: &[],
    detect: Some(detect_generic_import_config),
    load_sync: Some(load_generic_import_config),
    load_async: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("raccoin-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(path: &Path, config: &GenericImportConfig) {
        std::fs::write(path, serde_json::to_string_pretty(config).unwrap()).unwrap();
    }

    #[test]
    fn imports_csv_with_reusable_mapping() {
        let dir = temp_dir("generic-csv");
        let csv_path = dir.join("exchange.csv");
        let config_path = dir.join("exchange.raccoin-import.json");
        std::fs::write(
            &csv_path,
            "Date,Type,In Amount,In Currency,Out Amount,Out Currency,Fee Amount,Fee Currency,Tx ID,Note\n\
             2024-01-02 03:04:05,Buy,0.5,BTC,10000,EUR,10,EUR,abc123,first buy\n\
             2024-01-03 03:04:05,Withdrawal,,,0.1,BTC,,,def456,to wallet\n",
        )
        .unwrap();

        let mut mappings = BTreeMap::new();
        add_type_mappings(&mut mappings, "Buy", GenericOperationKind::Buy);
        add_type_mappings(&mut mappings, "Withdrawal", GenericOperationKind::Send);

        let config = GenericImportConfig {
            source_file: "exchange.csv".to_owned(),
            fields: GenericImportFields {
                timestamp: Some("Date".to_owned()),
                transaction_type: Some("Type".to_owned()),
                received_amount: Some("In Amount".to_owned()),
                received_currency: Some("In Currency".to_owned()),
                sent_amount: Some("Out Amount".to_owned()),
                sent_currency: Some("Out Currency".to_owned()),
                fee_amount: Some("Fee Amount".to_owned()),
                fee_currency: Some("Fee Currency".to_owned()),
                tx_hash: Some("Tx ID".to_owned()),
                description: Some("Note".to_owned()),
                ..Default::default()
            },
            type_mappings: mappings,
            ..Default::default()
        };
        write_config(&config_path, &config);

        let txs = load_generic_import_config(&config_path).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].tx_hash.as_deref(), Some("abc123"));
        assert_eq!(txs[0].description.as_deref(), Some("first buy"));
        match &txs[0].operation {
            Operation::Trade { incoming, outgoing } => {
                assert_eq!(incoming.quantity, dec!(0.5));
                assert_eq!(incoming.currency, "BTC");
                assert_eq!(outgoing.quantity, dec!(10000));
                assert_eq!(outgoing.currency, "EUR");
            }
            operation => panic!("Unexpected operation {operation:?}"),
        }
        assert_eq!(txs[0].fee.as_ref().unwrap().quantity, dec!(10));

        match &txs[1].operation {
            Operation::Send(amount) => {
                assert_eq!(amount.quantity, dec!(0.1));
                assert_eq!(amount.currency, "BTC");
            }
            operation => panic!("Unexpected operation {operation:?}"),
        }
    }

    #[test]
    fn imports_json_array_with_nested_fields() {
        let dir = temp_dir("generic-json");
        let json_path = dir.join("wallet.json");
        let config_path = dir.join("wallet.raccoin-import.json");
        std::fs::write(
            &json_path,
            r#"[
                {
                    "created_at": "2024-02-03T04:05:06Z",
                    "kind": "reward",
                    "asset": { "amount": "12.5", "currency": "XLM" },
                    "id": "json-1"
                }
            ]"#,
        )
        .unwrap();

        let config = GenericImportConfig {
            source_file: "wallet.json".to_owned(),
            format: GenericImportFormat::Json,
            fields: GenericImportFields {
                timestamp: Some("created_at".to_owned()),
                transaction_type: Some("kind".to_owned()),
                received_amount: Some("asset.amount".to_owned()),
                received_currency: Some("asset.currency".to_owned()),
                tx_hash: Some("id".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        write_config(&config_path, &config);

        let txs = load_generic_import_config(&config_path).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].tx_hash.as_deref(), Some("json-1"));
        match &txs[0].operation {
            Operation::Income(amount) => {
                assert_eq!(amount.quantity, dec!(12.5));
                assert_eq!(amount.currency, "XLM");
            }
            operation => panic!("Unexpected operation {operation:?}"),
        }
    }

    #[test]
    fn detects_mapping_config_kind() {
        let dir = temp_dir("generic-detect");
        let config_path = dir.join("source.raccoin-import.json");
        write_config(&config_path, &GenericImportConfig::default());

        assert!(detect_generic_import_config(&config_path).unwrap());
    }
}
