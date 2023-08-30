mod base;
mod bitcoin_core;
mod bitcoin_de;
mod bitonic;
mod coinmarketcap;
mod coinpanda;
mod ctc;
mod electrum;
mod poloniex;
mod time;

use bitcoin_core::convert_bitcoin_core_to_ctc;
use bitcoin_de::convert_bitcoin_de_to_ctc;
use bitonic::convert_bitonic_to_ctc;
use electrum::convert_electrum_to_ctc;
use poloniex::convert_poloniex_to_ctc;
use std::error::Error;



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
