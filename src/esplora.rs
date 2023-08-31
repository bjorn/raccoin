use std::{error::Error, str::FromStr};
use bitcoin::{Address, Network};
use chrono::NaiveDateTime;
use esplora_client::{Builder, BlockingClient};

use crate::base::Transaction;

pub(crate) fn blocking_esplora_client() -> Result<BlockingClient, esplora_client::Error> {
    let builder = Builder::new("https://blockstream.info/api");
    builder.build_blocking()
}

pub(crate) fn address_transactions(
    client: &BlockingClient,
    address: &str,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let script_pubkey = Address::from_str(address)?.require_network(Network::Bitcoin)?.script_pubkey();
    let script = script_pubkey.as_script();

    let mut txs = client.scripthash_txs(script, None)?;
    txs.retain(|tx| tx.status.confirmed);

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

    let mut transactions = Vec::new();
    for tx in txs {
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
            let spent_amount = (total_in - total_out) as f64 / 100_000_000.0;
            Transaction::send(naive_utc, spent_amount, "BTC")
        } else {
            let received_amount = (total_out - total_in) as f64 / 100_000_000.0;
            Transaction::receive(naive_utc, received_amount, "BTC")
        };

        transaction.tx_hash = Some(tx.txid.to_string());

        transactions.push(transaction);
    }

    println!("Imported {} transactions for address {}", transactions.len(), address);

    Ok(transactions)
}
