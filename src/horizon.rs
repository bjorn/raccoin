use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use futures::try_join;
use rust_decimal::Decimal;
use stellar_base::amount::Stroops;
use stellar_base::PublicKey;
use stellar_horizon::api;
use stellar_horizon::client::{HorizonClient, HorizonHttpClient};
use stellar_horizon::request::PageRequest;
use stellar_horizon::resources::{Effect, Asset, operation};

use crate::{base::{Transaction, Amount, Operation}, LoadFuture, TransactionSource};
use linkme::distributed_slice;

const STELLAR_SCALE: u32 = 7;
const PAGE_LIMIT: u64 = 20;

impl From<&Stroops> for Amount {
    fn from(stroops: &Stroops) -> Self {
        let quantity = Decimal::new(stroops.to_i64(), STELLAR_SCALE);
        Amount::new(quantity, "XLM".to_owned())
    }
}

fn normalize_asset(code: &str, issuer: &str) -> String {
    match (code, issuer) {
        ("AQUA", "GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA") => "AQUA".to_owned(),
        ("USDC", "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN") => "USDC".to_owned(),
        _ => format!("{}:{}", code, issuer),
    }
}

fn asset_to_string(asset: &Asset) -> String {
    match (&asset.asset_code, &asset.asset_issuer) {
        (Some(code), Some(issuer)) => normalize_asset(code.as_str(), issuer.as_str()),
        _ => "XLM".to_owned(),
    }
}

async fn address_fees(client: &HorizonHttpClient, address: &str) -> Result<HashMap<String, (NaiveDateTime, Amount)>> {
    println!("Loading transactions for {}...", address);

    let account = PublicKey::from_account_id(address)?;
    let mut cursor = "".to_string();

    let mut transactions = HashMap::new();

    loop {
        let request = api::transactions::for_account(&account)
            .with_limit(PAGE_LIMIT)
            .with_cursor(&cursor);
        let (_, response) = client.request(request).await?;
        let records_len = response.records.len();

        println!("Processing {} transactions", records_len);

        for tx in response.records {
            cursor = tx.paging_token;

            if tx.fee_account != address {
                continue;
            }

            let timestamp = tx.created_at.naive_utc();
            let amount = Amount::new(Decimal::new(tx.fee_charged, STELLAR_SCALE), "XLM".to_owned());
            let previous = transactions.insert(tx.hash, (timestamp, amount));
            assert!(previous.is_none());
        }

        if records_len < PAGE_LIMIT as usize {
            break;
        }
    }

    Ok(transactions)
}

async fn address_payments(client: &HorizonHttpClient, address: &str) -> Result<Vec<Transaction>> {
    println!("Loading operations for {}...", address);

    let account = PublicKey::from_account_id(address)?;
    let mut cursor = "".to_string();

    let mut transactions = Vec::new();

    loop {
        let request = api::operations::for_account(&account)
            .with_limit(PAGE_LIMIT)
            .with_cursor(&cursor);
        let (_, response) = client.request(request).await?;
        let records_len = response.records.len();

        println!("Processing {} operations", records_len);

        for operation in response.records {
            let (base, sender, destination, amount, description) = match operation {
                operation::Operation::CreateAccount(op) => {
                    (op.base, op.funder, op.account, Amount::new(Decimal::from_str(&op.starting_balance)?, "XLM".to_owned()), None)
                }
                operation::Operation::Payment(op) => {
                    (op.base, op.from, op.to, Amount::new(Decimal::from_str(&op.amount)?, asset_to_string(&op.asset)), None)
                }
                operation::Operation::PathPaymentStrictReceive(op) => {
                    (op.base, op.from, op.to, Amount::new(Decimal::from_str(&op.amount)?, asset_to_string(&op.asset)), None)
                }
                operation::Operation::PathPaymentStrictSend(op) => {
                    (op.base, op.from, op.to, Amount::new(Decimal::from_str(&op.amount)?, asset_to_string(&op.asset)), None)
                }
                operation::Operation::AccountMerge(op) => {
                    let request = api::effects::for_operation(&op.base.id);
                    let (_, response) = client.request(request).await?;
                    println!("Looking up merged XLM amount for account merge...");
                    let amount = response.records.into_iter().find_map(|effect| {
                        match effect {
                            Effect::AccountDebited(effect) => {
                                let quantity = Decimal::from_str(&effect.amount);
                                let currency = asset_to_string(&effect.asset);
                                Some(quantity.map(|quantity| Amount::new(quantity, currency)))
                            }
                            _ => None,
                        }
                    }).context("Missing Effect::AccountDebited for Operation::AccountMerge")?;
                    (op.base, op.account, op.into, amount?, None)
                }
                operation::Operation::ClaimClaimableBalance(op) => {
                    let request = api::effects::for_operation(&op.base.id);
                    let (_, response) = client.request(request).await?;
                    println!("Looking up claimed amount for balance ID {}...", op.balance_id);
                    let amount = response.records.into_iter().find_map(|effect| {
                        match effect {
                            Effect::ClaimableBalanceClaimed(effect) => {
                                let quantity = Decimal::from_str(&effect.amount).map_err(anyhow::Error::from);
                                let currency = if effect.asset == "native" {
                                    Ok("XLM".to_owned())
                                } else {
                                    let mut split = effect.asset.split(':');
                                    match (split.next(), split.next()) {
                                        (Some(code), Some(issuer)) => {
                                            Ok(normalize_asset(code, issuer))
                                        }
                                        _ => Err(anyhow!("Invalid asset value, expected: 'NAME:ISSUER'")),
                                    }
                                };
                                Some(match (quantity, currency) {
                                    (Ok(quantity), Ok(currency)) => Ok(Amount::new(quantity, currency)),
                                    (Err(e), _) | (_, Err(e)) => Err(e)
                                })
                            },
                            _ => None
                        }
                    }).context("Missing Effect::ClaimableBalanceClaimed for Operation::ClaimClaimableBalance")?;
                    // todo: determine sender?
                    (op.base, String::new(), op.claimant, amount?, Some(format!("Claimable Balance ID {}", op.balance_id)))
                }
                operation => {
                    // assuming other operations are not relevant to account balance
                    cursor = operation.base().paging_token.clone();
                    continue;
                }
            };

            cursor = base.paging_token;

            let timestamp = base.created_at.naive_utc();
            let tx_hash = base.transaction_hash;

            if sender == destination {
                continue;
            }

            let operation = if sender == address {
                Operation::Send(amount)
            } else {
                // Crude spam recognition
                if amount.currency == "XLM" && amount.quantity > Decimal::ZERO && amount.quantity < (Decimal::ONE / Decimal::ONE_HUNDRED) {
                    Operation::Spam(amount)
                } else {
                    Operation::Receive(amount)
                }
            };

            let mut tx = Transaction::new(timestamp, operation);
            tx.description = description;
            tx.tx_hash = Some(tx_hash);
            tx.blockchain = Some("XLM".to_owned());
            transactions.push(tx);
        }

        if records_len < PAGE_LIMIT as usize {
            break;
        }
    }

    Ok(transactions)
}

async fn address_trades(client: &HorizonHttpClient, address: &str) -> Result<Vec<Transaction>> {
    println!("Loading trades for {}...", address);

    let account = PublicKey::from_account_id(address)?;
    let mut cursor = "".to_string();

    let mut transactions: Vec<Transaction> = Vec::new();

    loop {
        let request = api::trades::for_account(&account)
            .with_limit(PAGE_LIMIT)
            .with_cursor(&cursor);
        let (_, response) = client.request(request).await?;
        let records_len = response.records.len();

        println!("Processing {} trades", records_len);

        for trade in response.records {
            cursor = trade.paging_token;

            let timestamp = trade.ledger_close_time.naive_utc();

            let base_currency = asset_to_string(&trade.base_asset);
            let base_amount = Amount::new(Decimal::from_str(&trade.base_amount)?, base_currency);
            let counter_currency = asset_to_string(&trade.counter_asset);
            let counter_amount = Amount::new(Decimal::from_str(&trade.counter_amount)?, counter_currency);

            let operation = if trade.base_account.is_some_and(|base| base == address) {
                Operation::Trade { incoming: counter_amount, outgoing: base_amount }
            } else {
                Operation::Trade { incoming: base_amount, outgoing: counter_amount }
            };

            let mut transaction = Transaction::new(timestamp, operation);
            transaction.blockchain = Some("XLM".to_owned());
            transactions.push(transaction);
        }

        if records_len < PAGE_LIMIT as usize {
            break;
        }
    }

    Ok(transactions)
}

pub(crate) async fn address_transactions(
    address: &str,
) -> Result<Vec<Transaction>> {
    let client = HorizonHttpClient::new_from_str("https://horizon.stellar.org")?;
    let (mut payments, trades, mut fees) = try_join!(
        address_payments(&client, address),
        address_trades(&client, address),
        address_fees(&client, address)
    )?;

    // Associate fees with the payments
    for payment in &mut payments {
        if let Some((_, amount)) = fees.remove(payment.tx_hash.as_ref().unwrap()) {
            payment.fee = Some(amount);
        }
    }

    let mut transactions: Vec<Transaction> = Vec::new();

    // Turn the rest of the fees into separate Fee transactions
    for (hash, (timestamp, amount)) in fees {
        let mut tx = Transaction::new(timestamp, Operation::Fee(amount));
        tx.tx_hash = Some(hash);
        tx.blockchain = Some("XLM".to_owned());
        transactions.push(tx);
    }

    transactions.extend(payments);
    transactions.extend(trades);

    Ok(transactions)
}

pub(crate) fn load_stellar_account_async(source_path: String) -> LoadFuture {
    Box::pin(async move { address_transactions(&source_path).await })
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static STELLAR_ACCOUNT: TransactionSource = TransactionSource {
    id: "StellarAccount",
    label: "Stellar Account",
    csv: &[],
    detect: None,
    load_sync: None,
    load_async: Some(load_stellar_account_async),
};
