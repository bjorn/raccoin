mod base;
mod bitcoin_core;
mod bitcoin_de;
mod bitonic;
mod coinmarketcap;
mod coinpanda;
mod ctc;
mod electrum;
mod esplora;
mod fifo;
mod poloniex;
mod time;

use base::{Operation, Amount};
use bitcoin_core::load_bitcoin_core_csv;
use bitcoin_de::load_bitcoin_de_csv;
use bitonic::load_bitonic_csv;
use chrono::{NaiveDateTime, Duration};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use esplora::{blocking_esplora_client, address_transactions};
use fifo::fifo;
use std::error::Error;

fn run() -> Result<(), Box<dyn Error>> {
    let mut txs = Vec::new();

    let bitcoin_de_csv_file = "bitcoin.de/btc_account_statement_20120831-20230831.csv";
    txs.append(&mut load_bitcoin_de_csv(bitcoin_de_csv_file)?);

    let bitcoin_core_csv_file = "bitcoin-core-transactions.csv";
    txs.append(&mut load_bitcoin_core_csv(bitcoin_core_csv_file)?);

    let bitonic_csv_file = "bitonic.csv";
    txs.append(&mut load_bitonic_csv(bitonic_csv_file)?);

    let electrum_csv_file = "electrum-history.csv";
    // let electrum_ctc_csv_file = "electrum-for-ctc.csv";
    // convert_electrum_to_ctc(electrum_csv_file, electrum_ctc_csv_file)?;

    // let poloniex_path = "poloniex";
    // let poloniex_ctc_csv_file = "poloniex-for-ctc.csv";
    // convert_poloniex_to_ctc(poloniex_path, poloniex_ctc_csv_file)?;

    let esplora_client = blocking_esplora_client()?;

    // sort transactions by date
    txs.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    let prices = load_btc_price_history_data()?;

    fifo(&txs)?;

    // price estimate for testing purposes
    println!("BTC price estimate for 2014-01-01T12:00:00: {}", estimate_btc_price(NaiveDateTime::parse_from_str("2014-01-01T12:00:00", "%Y-%m-%dT%H:%M:%S").unwrap(), &prices).unwrap());

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        println!("{}", err);
    }
}
