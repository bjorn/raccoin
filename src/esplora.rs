use std::{error::Error, str::FromStr};
use bitcoin::{Address, Network};
use chrono::NaiveDateTime;
use esplora_client::{Builder, BlockingClient, Tx};
use rust_decimal::Decimal;

use crate::base::{Transaction, Amount};

pub(crate) fn blocking_esplora_client() -> Result<BlockingClient, esplora_client::Error> {
    let builder = Builder::new("https://blockstream.info/api");
    builder.build_blocking()
}

pub(crate) fn address_transactions(
    client: &BlockingClient,
    address: &str,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let address = Address::from_str(address)?.require_network(Network::Bitcoin)?;
    let script_pubkey = address.script_pubkey();
    let txs = address_txs(client, &address)?;

    let mut transactions = Vec::new();
    // iterate in reverse order since we want the transactions to be chronological
    for tx in txs.iter().rev() {
        // calculate the total of inputs for this address (spent amount)
        let total_in: u64 = tx.vin.iter().filter_map(|vin| {
            if let Some(prevout) = &vin.prevout {
                if prevout.scriptpubkey == script_pubkey {
                    return Some(prevout.value);
                }
            }
            None
        }).sum();

        // calculate the total of outputs for this address (change or received amount)
        let total_out: u64 = tx.vout.iter().filter_map(|vout| {
            if vout.scriptpubkey == script_pubkey {
                return Some(vout.value);
            }
            None
        }).sum();

        // determine timestamp
        let timestamp = tx.status.block_time.unwrap_or_default();
        let naive_utc = NaiveDateTime::from_timestamp_opt(timestamp as i64, 0).unwrap();

        // determine if send or receive, and convert Satoshi to BTC
        let mut transaction = if total_in > total_out {
            let spent_amount = (total_in - total_out - tx.fee);
            let mut transaction = Transaction::send(naive_utc, Amount::from_satoshis(spent_amount));
            transaction.fee = Some(Amount::from_satoshis(tx.fee));
            transaction
        } else {
            let received_amount = (total_out - total_in);
            Transaction::receive(naive_utc, Amount::from_satoshis(received_amount))
        };

        transaction.tx_hash = Some(tx.txid.to_string());

        transactions.push(transaction);
    }

    Ok(transactions)
}

pub(crate) fn address_txs(
    client: &BlockingClient,
    address: &Address,
) -> Result<Vec<Tx>, Box<dyn Error>> {
    let script_pubkey = address.script_pubkey();
    let script = script_pubkey.as_script();

    let mut txs = client.scripthash_txs(script, None)?;

    // we may get up to 50 unconfirmed transactions, so filter them
    txs.retain(|tx| tx.status.confirmed);

    // repeat the request until we have all transactions
    if txs.len() == 25 {
        loop {
            let mut more_txs = client.scripthash_txs(script, Some(txs.last().unwrap().txid))?;
            let n = more_txs.len();
            txs.append(&mut more_txs);
            if n < 25 {
                break;
            }
        }
    }

    Ok(txs)
}
