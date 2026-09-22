use anyhow::{bail, Context, Result};
use std::{str::FromStr, collections::{HashMap, HashSet, hash_map::Entry}};
use bitcoin::{Address, Network, bip32::{Xpub, DerivationPath, ChildNumber}, secp256k1::{Secp256k1, self}, base58, ScriptBuf};
use chrono::DateTime;
use esplora_client::{Builder, Tx};
use esplora_client::r#async::AsyncClient;

use crate::{base::{Transaction, Amount}, LoadFuture, TransactionSource};
use linkme::distributed_slice;

pub(crate) fn async_esplora_client() -> Result<AsyncClient, esplora_client::Error> {
    let builder = Builder::new("https://blockstream.info/api");
    builder.build_async()
}

pub(crate) async fn address_transactions(
    client: &AsyncClient,
    addresses: &Vec<String>,
) -> Result<Vec<Transaction>> {
    let mut pub_keys = HashSet::new();
    let mut address_transactions: HashMap<Address, Result<Vec<Tx>>> = HashMap::new();

    for address in addresses {
        let address = Address::from_str(address)?.require_network(Network::Bitcoin)?;
        pub_keys.insert(address.script_pubkey());

        match address_transactions.entry(address) {
            Entry::Occupied(_) => {}
            Entry::Vacant(e) => {
                let value = address_txs(client, e.key()).await;
                e.insert(value);
            }
        };
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
    let naive_utc = DateTime::from_timestamp(timestamp as i64, 0).unwrap().naive_utc();

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

async fn address_txs(
    client: &AsyncClient,
    address: &Address,
) -> Result<Vec<Tx>> {
    let script_pubkey = address.script_pubkey();
    let script = script_pubkey.as_script();

    let mut txs = client.scripthash_txs(script, None).await?;

    // we may get up to 50 unconfirmed transactions, so filter them
    txs.retain(|tx| tx.status.confirmed);

    // repeat the request until we have all transactions
    if txs.len() == 25 {
        loop {
            let mut more_txs = client.scripthash_txs(script, Some(txs.last().unwrap().txid)).await?;
            let n = more_txs.len();
            txs.append(&mut more_txs);
            if n < 25 {
                break;
            }
        }
    }

    Ok(txs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressType {
    P2PKH,          // Legacy
    P2SHWPKH,       // Legacy Segwit
    P2WPKH,         // Segwit
    P2TR,           // Taproot
}

struct BitcoinDescriptor {
    address_type: AddressType,
    xpub_key: Xpub,
    derivation_path: DerivationPath,
}

fn address_for_derivation_path<C: secp256k1::Verification>(
    secp: &Secp256k1<C>,
    xpub_key: &Xpub,
    derivation_path: &DerivationPath,
    address_type: AddressType,
) -> Result<Address> {
    let key = xpub_key.derive_pub(secp, derivation_path)?;
    let address = match address_type {
        AddressType::P2PKH => Address::p2pkh(&key.to_pub(), Network::Bitcoin),
        AddressType::P2SHWPKH => Address::p2shwpkh(&key.to_pub(), Network::Bitcoin),
        AddressType::P2WPKH => Address::p2wpkh(&key.to_pub(), Network::Bitcoin),
        AddressType::P2TR => Address::p2tr(secp, key.to_x_only_pub(), None, Network::Bitcoin),
    };
    Ok(address)
}

async fn scan_children<C: secp256k1::Verification>(
    client: &AsyncClient,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>>>,
    secp: &Secp256k1<C>,
    xpub_key: &Xpub,
    derivation_path: &DerivationPath,
    address_type: AddressType
) -> Result<()> {
    let mut iter = derivation_path.normal_children();
    let mut empty_addresses = 0;

    for child in iter.by_ref() {
        let address = address_for_derivation_path(secp, xpub_key, &child, address_type)?;

        println!("  checking address {}: {}", child, address);

        let txs = match address_transactions.entry(address) {
            Entry::Occupied(e) => {
                e.into_mut()
            }
            Entry::Vacant(e) => {
                let value = address_txs(client, e.key()).await;
                e.insert(value)
            }
        };

        println!("   transaction count: {}", txs.as_ref().map(Vec::len).unwrap_or_default());

        if !txs.as_ref().is_ok_and(|txs| { !txs.is_empty() }) {
            empty_addresses += 1;
            if empty_addresses > 10 {
                break;
            }
        }
    }

    Ok(())
}

fn decode_extended_pub_key(extended_pub_key: &str) -> Result<Xpub> {
    let mut xpub_data = base58::decode_check(extended_pub_key)?;
    if xpub_data.len() < 4 {
        bail!("Invalid extended public key");
    }

    // Replace the version bytes with 0488b21e, this way we can support ypub and zpub.
    xpub_data[0..4].copy_from_slice(&[0x04, 0x88, 0xb2, 0x1e]);
    Ok(Xpub::decode(&xpub_data)?)
}

fn address_type_for_extended_pub_key(extended_pub_key: &str) -> Result<AddressType> {
    let prefix = extended_pub_key
        .get(..4)
        .context("Extended public key is too short")?;
    match prefix {
        "xpub" => Ok(AddressType::P2PKH),
        "ypub" => Ok(AddressType::P2SHWPKH),
        "zpub" => Ok(AddressType::P2WPKH),
        _ => bail!("Unsupported extended public key prefix {}", prefix),
    }
}

async fn xpub_addresses_and_txs<C: secp256k1::Verification>(
    client: &AsyncClient,
    secp: &Secp256k1<C>,
    xpub: &str,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>>>,
) -> Result<()> {
    let xpub_key = decode_extended_pub_key(xpub)?;
    let address_type = address_type_for_extended_pub_key(xpub)?;

    println!("iterating addresses from xpub {}", xpub);

    println!(" receive addresses:");

    let receive_path = DerivationPath::master().child(ChildNumber::Normal { index: 0 });
    scan_children(client, address_transactions, secp, &xpub_key, &receive_path, address_type).await?;

    println!(" change addresses:");

    let change_path = DerivationPath::master().child(ChildNumber::Normal { index: 1 });
    scan_children(client, address_transactions, secp, &xpub_key, &change_path, address_type).await?;

    Ok(())
}

fn descriptor_body(descriptor: &str) -> Result<(AddressType, &str)> {
    let descriptor = descriptor.trim().split_once('#').map_or(descriptor.trim(), |(body, _)| body.trim());
    if let Some(body) = descriptor.strip_prefix("pkh(").and_then(|body| body.strip_suffix(')')) {
        return Ok((AddressType::P2PKH, body));
    }
    if let Some(body) = descriptor.strip_prefix("sh(wpkh(").and_then(|body| body.strip_suffix("))")) {
        return Ok((AddressType::P2SHWPKH, body));
    }
    if let Some(body) = descriptor.strip_prefix("wpkh(").and_then(|body| body.strip_suffix(')')) {
        return Ok((AddressType::P2WPKH, body));
    }
    if let Some(body) = descriptor.strip_prefix("tr(").and_then(|body| body.strip_suffix(')')) {
        return Ok((AddressType::P2TR, body));
    }
    bail!("Unsupported Bitcoin descriptor");
}

fn strip_key_origin(key_expression: &str) -> Result<&str> {
    let key_expression = key_expression.trim();
    if let Some(rest) = key_expression.strip_prefix('[') {
        let end = rest.find(']').context("Descriptor key origin is missing closing bracket")?;
        Ok(&rest[end + 1..])
    } else {
        Ok(key_expression)
    }
}

fn descriptor_derivation_path(path_suffix: Option<&str>) -> Result<DerivationPath> {
    let Some(path_suffix) = path_suffix else {
        return Ok(DerivationPath::master());
    };
    let path_prefix = if path_suffix == "*" {
        ""
    } else {
        path_suffix
            .strip_suffix("/*")
            .context("Descriptor extended public key path must end in /*")?
    };
    if path_prefix.is_empty() {
        Ok(DerivationPath::master())
    } else {
        Ok(DerivationPath::from_str(&format!("m/{}", path_prefix))?)
    }
}

fn parse_bitcoin_descriptor(descriptor: &str) -> Result<BitcoinDescriptor> {
    let (address_type, key_expression) = descriptor_body(descriptor)?;
    let key_expression = strip_key_origin(key_expression)?;
    let (extended_pub_key, path_suffix) = match key_expression.split_once('/') {
        Some((extended_pub_key, path_suffix)) => (extended_pub_key, Some(path_suffix)),
        None => (key_expression, None),
    };

    Ok(BitcoinDescriptor {
        address_type,
        xpub_key: decode_extended_pub_key(extended_pub_key)?,
        derivation_path: descriptor_derivation_path(path_suffix)?,
    })
}

async fn descriptor_addresses_and_txs<C: secp256k1::Verification>(
    client: &AsyncClient,
    secp: &Secp256k1<C>,
    descriptor: &str,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>>>,
) -> Result<()> {
    let descriptor = parse_bitcoin_descriptor(descriptor)?;
    println!("iterating addresses from descriptor");
    scan_children(
        client,
        address_transactions,
        secp,
        &descriptor.xpub_key,
        &descriptor.derivation_path,
        descriptor.address_type,
    ).await
}

pub(crate) async fn xpub_addresses_transactions(
    client: &AsyncClient,
    xpubs: &Vec<String>,
) -> Result<Vec<Transaction>> {
    let secp = Secp256k1::new();

    // Collect all relevant transactions in a map from Address -> Vec<Tx>
    let mut address_transactions: HashMap<Address, Result<Vec<Tx>>> = HashMap::new();

    // todo: do in parallel
    for xpub in xpubs {
        xpub_addresses_and_txs(client, &secp, xpub, &mut address_transactions).await?;
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

pub(crate) async fn descriptor_addresses_transactions(
    client: &AsyncClient,
    descriptors: &Vec<String>,
) -> Result<Vec<Transaction>> {
    let secp = Secp256k1::new();

    let mut address_transactions: HashMap<Address, Result<Vec<Tx>>> = HashMap::new();

    for descriptor in descriptors {
        descriptor_addresses_and_txs(client, &secp, descriptor, &mut address_transactions).await?;
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

fn split_whitespace_owned(value: &str) -> Vec<String> {
    value.split_ascii_whitespace().map(|item| item.to_owned()).collect()
}

pub(crate) fn load_bitcoin_addresses_async(source_path: String) -> LoadFuture {
    Box::pin(async move {
        let esplora_client = async_esplora_client().unwrap();
        address_transactions(&esplora_client, &split_whitespace_owned(&source_path)).await
    })
}

pub(crate) fn load_bitcoin_xpubs_async(source_path: String) -> LoadFuture {
    Box::pin(async move {
        let esplora_client = async_esplora_client().unwrap();
        xpub_addresses_transactions(&esplora_client, &split_whitespace_owned(&source_path)).await
    })
}

pub(crate) fn load_bitcoin_descriptors_async(source_path: String) -> LoadFuture {
    Box::pin(async move {
        let esplora_client = async_esplora_client().unwrap();
        descriptor_addresses_transactions(&esplora_client, &split_whitespace_owned(&source_path)).await
    })
}

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITCOIN_ADDRESSES: TransactionSource = TransactionSource {
    id: "BitcoinAddresses",
    label: "Bitcoin Address(es)",
    csv: &[],
    detect: None,
    load_sync: None,
    load_async: Some(load_bitcoin_addresses_async),
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITCOIN_XPUBS: TransactionSource = TransactionSource {
    id: "BitcoinXpubs",
    label: "Bitcoin HD Wallet(s)",
    csv: &[],
    detect: None,
    load_sync: None,
    load_async: Some(load_bitcoin_xpubs_async),
};

#[distributed_slice(crate::TRANSACTION_SOURCES)]
static BITCOIN_DESCRIPTORS: TransactionSource = TransactionSource {
    id: "BitcoinDescriptors",
    label: "Bitcoin Descriptor(s)",
    csv: &[],
    detect: None,
    load_sync: None,
    load_async: Some(load_bitcoin_descriptors_async),
};

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::bip32::Xpriv;

    fn account_xpub(path: &str) -> String {
        let secp = Secp256k1::new();
        let seed = [7; 32];
        let master = Xpriv::new_master(Network::Bitcoin, &seed).unwrap();
        let account = master
            .derive_priv(&secp, &DerivationPath::from_str(path).unwrap())
            .unwrap();
        Xpub::from_priv(&secp, &account).to_string()
    }

    #[test]
    fn parses_taproot_descriptor() {
        let xpub = account_xpub("m/86'/0'/0'");
        let descriptor = format!("tr([00000000/86'/0'/0']{}/0/*)", xpub);

        let parsed = parse_bitcoin_descriptor(&descriptor).unwrap();
        assert_eq!(parsed.address_type, AddressType::P2TR);
        assert_eq!(
            parsed.derivation_path,
            DerivationPath::from_str("m/0").unwrap()
        );

        let secp = Secp256k1::new();
        let first_path = parsed
            .derivation_path
            .child(ChildNumber::Normal { index: 0 });
        let address = address_for_derivation_path(
            &secp,
            &parsed.xpub_key,
            &first_path,
            parsed.address_type,
        ).unwrap();

        assert!(address.to_string().starts_with("bc1p"));
    }

    #[test]
    fn parses_wrapped_segwit_descriptor_with_checksum() {
        let xpub = account_xpub("m/49'/0'/0'");
        let descriptor = format!("sh(wpkh({}/1/*))#ignored", xpub);

        let parsed = parse_bitcoin_descriptor(&descriptor).unwrap();
        assert_eq!(parsed.address_type, AddressType::P2SHWPKH);
        assert_eq!(
            parsed.derivation_path,
            DerivationPath::from_str("m/1").unwrap()
        );
    }

    #[test]
    fn rejects_descriptor_without_wildcard() {
        let xpub = account_xpub("m/84'/0'/0'");
        let descriptor = format!("wpkh({}/0/1)", xpub);

        assert!(parse_bitcoin_descriptor(&descriptor).is_err());
    }

    #[test]
    fn parses_descriptor_with_direct_wildcard() {
        let xpub = account_xpub("m/84'/0'/0'");
        let descriptor = format!("wpkh({}/*)", xpub);

        let parsed = parse_bitcoin_descriptor(&descriptor).unwrap();
        assert_eq!(parsed.address_type, AddressType::P2WPKH);
        assert_eq!(parsed.derivation_path, DerivationPath::master());
    }
}

// Converts the transactions, using a set of tx_hash to skip duplicates
fn process_transactions(address_transactions: HashMap<Address, Result<Vec<Tx>>>, pub_keys: HashSet<ScriptBuf>) -> Vec<Transaction> {
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
