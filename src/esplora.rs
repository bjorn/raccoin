use std::{error::Error, str::FromStr, collections::{HashMap, HashSet}};
use bitcoin::{Address, Network, bip32::{ExtendedPubKey, DerivationPath, ChildNumber}, secp256k1::{Secp256k1, self}, base58, ScriptBuf};
use chrono::NaiveDateTime;
use esplora_client::{Builder, BlockingClient, Tx};

use crate::base::{Transaction, Amount};

pub(crate) fn blocking_esplora_client() -> Result<BlockingClient, esplora_client::Error> {
    let builder = Builder::new("https://blockstream.info/api");
    builder.build_blocking()
}

pub(crate) fn address_transactions(
    client: &BlockingClient,
    addresses: &Vec<String>,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut pub_keys = HashSet::new();
    let mut address_transactions: HashMap<Address, Result<Vec<Tx>, Box<dyn Error>>> = HashMap::new();

    for address in addresses {
        let address = Address::from_str(address)?.require_network(Network::Bitcoin)?;
        pub_keys.insert(address.script_pubkey());

        let _ = address_transactions.entry(address).or_insert_with_key(|k| {
            address_txs(client, &k)
        });
    }

    Ok(process_transactions(address_transactions, pub_keys))
}

fn tx_to_transaction(
    addresses: &HashSet<ScriptBuf>,
    tx: &Tx
) -> Transaction {
    // let total_in: u64 = tx.vin.iter().filter_map(|vin| { vin.prevout.as_ref().map(|o| o.value) }).sum();
    // let total_out: u64 = tx.vout.iter().map(|vout| { vout.value }).sum();

    // if total_in - total_out != tx.fee {
    //     println!("total_in - total_out != tx.fee, tx id: {}", tx.txid);
    // }

    // calculate the total of inputs from known addresses (spent amount)
    let own_in: u64 = tx.vin.iter().filter_map(|vin| {
        if let Some(prevout) = &vin.prevout {
            if addresses.contains(&prevout.scriptpubkey) {
                return Some(prevout.value);
            }
        }
        None
    }).sum();

    // if any input is from this wallet, all of them should be from this wallet
    // if own_in > 0 && own_in != total_in {
    //     println!("all inputs should be from this wallet, or none, otherwise we're probably missing an address, tx id: {}", tx.txid);
    // }

    // calculate the total of outputs to known addresses (change or received amount)
    let own_out: u64 = tx.vout.iter().filter_map(|vout| {
        if addresses.contains(&vout.scriptpubkey) {
            return Some(vout.value);
        }
        None
    }).sum();


    // determine timestamp
    let timestamp = tx.status.block_time.unwrap_or_default();
    let naive_utc = NaiveDateTime::from_timestamp_opt(timestamp as i64, 0).unwrap();

    // determine if send or receive, and convert Satoshi to BTC
    let mut transaction = if own_in > own_out {
        let spent_amount = own_in - own_out - tx.fee;
        if spent_amount > 0 {
            let mut transaction = Transaction::send(naive_utc, Amount::from_satoshis(spent_amount));
            transaction.fee = Some(Amount::from_satoshis(tx.fee));
            transaction
        } else {
            Transaction::fee(naive_utc, Amount::from_satoshis(tx.fee))
        }
    } else {
        let received_amount = own_out - own_in;
        Transaction::receive(naive_utc, Amount::from_satoshis(received_amount))
    };

    transaction.tx_hash = Some(tx.txid.to_string());
    transaction.blockchain = Some("BTC".to_owned());

    transaction
}

fn address_txs(
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

#[derive(Debug, Clone, Copy)]
enum AddressType {
    P2PKH,          // Legacy
    P2SHWPKH,       // Legacy Segwit
    P2WPKH,         // Segwit
}

fn scan_children<C: secp256k1::Verification>(
    client: &BlockingClient,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>, Box<dyn Error>>>,
    secp: &Secp256k1<C>,
    xpub_key: &ExtendedPubKey,
    derivation_path: &DerivationPath,
    address_type: AddressType
) -> Result<(), Box<dyn Error>> {
    let mut iter = derivation_path.normal_children();
    let mut empty_addresses = 0;

    for child in iter.by_ref() {
        let key = xpub_key.derive_pub(secp, &child)?.to_pub();
        let address = match address_type {
            AddressType::P2PKH => Address::p2pkh(&key, Network::Bitcoin),
            AddressType::P2SHWPKH => Address::p2shwpkh(&key, Network::Bitcoin)?,
            AddressType::P2WPKH => Address::p2wpkh(&key, Network::Bitcoin)?,
        };

        println!("  checking address {}: {}", child, address);

        let txs = address_transactions.entry(address).or_insert_with_key(|k| {
            address_txs(client, &k)
        });

        println!("   transaction count: {}", txs.as_ref().map(Vec::len).unwrap_or_default());

        if !txs.as_ref().is_ok_and(|txs| { !txs.is_empty() }) {
            empty_addresses += 1;
            if empty_addresses > 3 {
                break;
            }
        }
    }

    Ok(())
}

fn xpub_addresses_and_txs<C: secp256k1::Verification>(
    client: &BlockingClient,
    secp: &Secp256k1<C>,
    xpub: &str,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>, Box<dyn Error>>>,
) -> Result<(), Box<dyn Error>> {
    let mut xpub_data = base58::decode_check(xpub)?;

    // replace the version bytes with 0488b21e, this way we can support ypub and zpub
    xpub_data[0..4].copy_from_slice(&[0x04, 0x88, 0xb2, 0x1e]);

    let xpub_key = ExtendedPubKey::decode(&xpub_data)?;
    let xpub_prefix = xpub.split_at(4).0;
    let address_type = match xpub_prefix {
        "xpub" => AddressType::P2PKH,
        "ypub" => AddressType::P2SHWPKH,
        "zpub" => AddressType::P2WPKH,
        _ => panic!("unsupported xpub prefix {}", xpub_prefix), // todo: return error instead
    };

    println!("iterating addresses from xpub {}", xpub);

    println!(" receive addresses:");

    let receive_path = DerivationPath::master().child(ChildNumber::Normal { index: 0 });
    scan_children(client, address_transactions, &secp, &xpub_key, &receive_path, address_type)?;

    println!(" change addresses:");

    let change_path = DerivationPath::master().child(ChildNumber::Normal { index: 1 });
    scan_children(client, address_transactions, &secp, &xpub_key, &change_path, address_type)?;

    Ok(())
}

pub(crate) fn xpub_addresses_transactions(
    client: &BlockingClient,
    xpubs: &Vec<String>,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let secp = Secp256k1::new();

    // Collect all relevant transactions in a map from Address -> Vec<Tx>
    let mut address_transactions: HashMap<Address, Result<Vec<Tx>, Box<dyn Error>>> = HashMap::new();

    for xpub in xpubs {
        xpub_addresses_and_txs(client, &secp, xpub, &mut address_transactions)?;
    }

    let mut pub_keys = HashSet::new();
    pub_keys.extend(address_transactions.iter().filter_map(|(address, txs)| {
        match txs {
            Ok(txs) if !txs.is_empty() => Some(address.script_pubkey()),
            _ => None,
        }
    }));

    println!("collected {} active addresses (scanned {})", pub_keys.len(), address_transactions.len());

    Ok(process_transactions(address_transactions, pub_keys))
}

// Converts the transactions, using a set of tx_hash to skip duplicates
fn process_transactions(address_transactions: HashMap<Address, Result<Vec<Tx>, Box<dyn Error>>>, pub_keys: HashSet<ScriptBuf>) -> Vec<Transaction> {
    let mut processed_txs = HashSet::new();
    let mut transactions = Vec::new();

    address_transactions.values().for_each(|txs| {
        if let Ok(txs) = txs {
            // iterate in reverse order to make the transactions somewhat chronological (at least per address...)
            txs.iter().rev().for_each(|tx| {
                if !processed_txs.contains(&tx.txid) {
                    processed_txs.insert(tx.txid);
                    transactions.push(tx_to_transaction(&pub_keys, tx));
                }
            })
        }
    });

    println!("processed {} unique transactions", processed_txs.len());
    transactions
}
