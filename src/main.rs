mod base;
mod bitcoin_core;
mod bitcoin_de;
mod bitonic;
mod coinmarketcap;
mod coinpanda;
mod ctc;
mod electrum;
mod fifo;
mod poloniex;
mod time;

use bitcoin_core::convert_bitcoin_core_to_ctc;
use bitcoin_de::{convert_bitcoin_de_to_ctc, load_bitcoin_de_csv};
use bitonic::convert_bitonic_to_ctc;
use chrono::{NaiveDateTime, Duration};
use coinmarketcap::{load_btc_price_history_data, estimate_btc_price};
use electrum::convert_electrum_to_ctc;
use fifo::fifo;
use poloniex::convert_poloniex_to_ctc;
use std::error::Error;

use crate::{bitonic::load_bitonic_csv, bitcoin_core::load_bitcoin_core_csv, base::{Operation, Amount}};

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

    let mut txs = Vec::new();

    txs.append(&mut load_bitcoin_de_csv(bitcoin_de_csv_file)?);
    txs.append(&mut load_bitcoin_core_csv(bitcoin_core_csv_file)?);
    txs.append(&mut load_bitonic_csv(bitonic_csv_file)?);

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
