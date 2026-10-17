use anyhow::Result;
use std::{str::FromStr, collections::{HashMap, HashSet, hash_map::Entry}};
use bitcoin::{Address, Network, bip32::{Xpub, DerivationPath, ChildNumber}, secp256k1::{Secp256k1, self}, base58, ScriptBuf};
use chrono::DateTime;
use esplora_client::{Builder, Tx};
use esplora_client::r#async::AsyncClient;

use crate::{base::{Transaction, Amount}, LoadFuture, TransactionSource};
use crate::bitcoin_descriptor::BitcoinDescriptor;
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

#[derive(Debug, Clone, Copy)]
enum AddressType {
    P2PKH,          // Legacy
    P2SHWPKH,       // Legacy Segwit
    P2WPKH,         // Segwit
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
        let key = xpub_key.derive_pub(secp, &child)?.to_pub();
        let address = match address_type {
            AddressType::P2PKH => Address::p2pkh(&key, Network::Bitcoin),
            AddressType::P2SHWPKH => Address::p2shwpkh(&key, Network::Bitcoin),
            AddressType::P2WPKH => Address::p2wpkh(&key, Network::Bitcoin),
        };

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

async fn xpub_addresses_and_txs<C: secp256k1::Verification>(
    client: &AsyncClient,
    secp: &Secp256k1<C>,
    xpub: &str,
    address_transactions: &mut HashMap<Address, Result<Vec<Tx>>>,
) -> Result<()> {
    let mut xpub_data = base58::decode_check(xpub)?;

    // replace the version bytes with 0488b21e, this way we can support ypub and zpub
    xpub_data[0..4].copy_from_slice(&[0x04, 0x88, 0xb2, 0x1e]);

    let xpub_key = Xpub::decode(&xpub_data)?;
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
    scan_children(client, address_transactions, secp, &xpub_key, &receive_path, address_type).await?;

    println!(" change addresses:");

    let change_path = DerivationPath::master().child(ChildNumber::Normal { index: 1 });
    scan_children(client, address_transactions, secp, &xpub_key, &change_path, address_type).await?;

    Ok(())
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

fn split_whitespace_owned(value: &str) -> Vec<String> {
    value.split_ascii_whitespace().map(|item| item.to_owned()).collect()
}

pub(crate) async fn descriptor_addresses_transactions(
    client: &AsyncClient,
    input: &str,
) -> Result<Vec<Transaction>> {
    let descriptors = BitcoinDescriptor::parse_list(input)?;
    let mut address_transactions: HashMap<Address, Result<Vec<Tx>>> = HashMap::new();
    let mut pub_keys = HashSet::new();
    let mut used_addresses = HashSet::new();

    for descriptor in descriptors {
        let mut empty_addresses = 0;
        for index in 0..(1 << 31) {
            let address = descriptor.address(index)?;
            pub_keys.insert(address.script_pubkey());
            let has_history = match address_transactions.entry(address) {
                Entry::Occupied(entry) => used_addresses.contains(entry.key()),
                Entry::Vacant(entry) => {
                    // An API failure is not evidence that an address is unused.
                    let (txs, has_history) = descriptor_address_txs(client, entry.key()).await?;
                    if has_history {
                        used_addresses.insert(entry.key().clone());
                    }
                    entry.insert(Ok(txs));
                    has_history
                }
            };
            if has_history {
                empty_addresses = 0;
            } else {
                empty_addresses += 1;
            }
            if !descriptor.has_wildcard() || empty_addresses >= 20 {
                break;
            }
        }
    }

    Ok(process_transactions(address_transactions, pub_keys))
}

async fn descriptor_address_txs(
    client: &AsyncClient,
    address: &Address,
) -> Result<(Vec<Tx>, bool)> {
    let script = address.script_pubkey();
    let mut txs = client.scripthash_txs(&script, None).await?;
    // Mempool activity also marks an address as used, but only confirmed
    // transactions are imported, consistent with the other Bitcoin sources.
    let has_history = !txs.is_empty();
    txs.retain(|tx| tx.status.confirmed);
    if txs.len() == 25 {
        loop {
            let mut more = client
                .scripthash_txs(&script, Some(txs.last().unwrap().txid))
                .await?;
            let count = more.len();
            txs.append(&mut more);
            if count < 25 {
                break;
            }
        }
    }
    Ok((txs, has_history))
}

pub(crate) fn load_bitcoin_descriptors_async(source_path: String) -> LoadFuture {
    Box::pin(async move {
        let esplora_client = async_esplora_client()?;
        descriptor_addresses_transactions(&esplora_client, &source_path).await
    })
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
    label: "Bitcoin Output Descriptor(s)",
    csv: &[],
    detect: None,
    load_sync: None,
    load_async: Some(load_bitcoin_descriptors_async),
};

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

#[cfg(test)]
mod descriptor_tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    // Public BIP 382 test key. These tests only contact a local HTTP fixture.
    const XPUB: &str = "xpub69H7F5d8KSRgmmdJg2KhpAK8SR3DjMwAdkxj3ZuxV27CprR9LgpeyGmXUbC6wb7ERfvrnKZjXoUmmDznezpbZb7ap6r1D3tgFxHmwMkQTPH";

    struct MockEsplora {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl MockEsplora {
        fn new(response: impl Fn(usize) -> (u16, String) + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let worker = {
                let requests = requests.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                stream.set_nonblocking(false).unwrap();
                                stream
                                    .set_read_timeout(Some(Duration::from_secs(2)))
                                    .unwrap();
                                let mut reader = BufReader::new(stream);
                                let mut request = String::new();
                                reader.read_line(&mut request).unwrap();
                                loop {
                                    let mut header = String::new();
                                    if reader.read_line(&mut header).unwrap() == 0
                                        || header == "\r\n"
                                    {
                                        break;
                                    }
                                }
                                let index = {
                                    let mut requests = requests.lock().unwrap();
                                    requests.push(request);
                                    requests.len() - 1
                                };
                                let (status, body) = response(index);
                                let mut stream = reader.into_inner();
                                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(1));
                            }
                            Err(error) => panic!("Local fixture failed: {error}"),
                        }
                    }
                })
            };
            Self {
                url,
                requests,
                stop,
                worker: Some(worker),
            }
        }

        fn client(&self) -> AsyncClient {
            Builder::new(&self.url)
                .max_retries(0)
                .build_async()
                .unwrap()
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    impl Drop for MockEsplora {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let result = self.worker.take().unwrap().join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }

    fn confirmed_transaction(scripts: &[ScriptBuf]) -> String {
        serde_json::json!([{
            "txid": "0000000000000000000000000000000000000000000000000000000000000001",
            "version": 2, "locktime": 0, "vin": [],
            "vout": scripts.iter().map(|script| serde_json::json!({
                "scriptpubkey": script.to_hex_string(), "value": 1000
            })).collect::<Vec<_>>(),
            "size": 100, "weight": 400, "fee": 0,
            "status": {"confirmed": true, "block_height": 1, "block_time": 1231006505}
        }])
        .to_string()
    }

    #[tokio::test]
    async fn fixed_descriptor_and_duplicate_source_are_looked_up_once() {
        let server = MockEsplora::new(|_| (200, "[]".to_owned()));
        let descriptor = format!("wpkh({XPUB}/0/0)");
        let input = format!("{descriptor} {descriptor}");
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(server.request_count(), 1);
        assert!(server.requests.lock().unwrap()[0].starts_with("GET /scripthash/"));
    }

    #[tokio::test]
    async fn ranged_descriptor_stops_after_twenty_empty_addresses() {
        let server = MockEsplora::new(|_| (200, "[]".to_owned()));
        let input = format!("wpkh({XPUB}/0/*)");
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(server.request_count(), 20);
    }

    #[tokio::test]
    async fn activity_resets_the_gap_and_imports_the_payment() {
        let input = format!("wpkh({XPUB}/0/*)");
        let script = BitcoinDescriptor::parse_list(&input).unwrap()[0]
            .address(19)
            .unwrap()
            .script_pubkey();
        let body = confirmed_transaction(&[script]);
        let server = MockEsplora::new(move |index| {
            (
                200,
                if index == 19 {
                    body.clone()
                } else {
                    "[]".to_owned()
                },
            )
        });
        let transactions = descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap();
        assert_eq!(server.request_count(), 40);
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].operation,
            crate::base::Operation::Receive(Amount::from_satoshis(1000))
        );
    }

    #[tokio::test]
    async fn multipath_scans_receive_and_change_branches_independently() {
        let server = MockEsplora::new(|_| (200, "[]".to_owned()));
        let input = format!("wsh(sortedmulti(1,{XPUB}/<0;1>/*))");
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(server.request_count(), 40);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.iter().collect::<HashSet<_>>().len(), 40);
    }

    #[tokio::test]
    async fn overlapping_descriptor_transactions_are_counted_once() {
        let input = format!("wpkh({XPUB}/0/0) wpkh({XPUB}/0/1)");
        let descriptors = BitcoinDescriptor::parse_list(&input).unwrap();
        let scripts: Vec<_> = descriptors
            .iter()
            .map(|descriptor| descriptor.address(0).unwrap().script_pubkey())
            .collect();
        let body = confirmed_transaction(&scripts);
        let server = MockEsplora::new(move |_| (200, body.clone()));
        let transactions = descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap();
        assert_eq!(server.request_count(), 2);
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].operation,
            crate::base::Operation::Receive(Amount::from_satoshis(2000))
        );
    }

    #[tokio::test]
    async fn mempool_activity_at_gap_boundary_resets_scan_without_being_imported() {
        let input = format!("wpkh({XPUB}/0/*)");
        let script = BitcoinDescriptor::parse_list(&input).unwrap()[0]
            .address(19)
            .unwrap()
            .script_pubkey();
        let mut transaction: serde_json::Value =
            serde_json::from_str(&confirmed_transaction(&[script])).unwrap();
        transaction[0]["status"] = serde_json::json!({"confirmed": false});
        let body = transaction.to_string();
        let server = MockEsplora::new(move |index| {
            (
                200,
                if index == 19 {
                    body.clone()
                } else {
                    "[]".to_owned()
                },
            )
        });
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(server.request_count(), 40);
    }

    #[tokio::test]
    async fn invalid_list_is_rejected_before_any_network_request() {
        let server = MockEsplora::new(|_| (200, "[]".to_owned()));
        let input = format!("wpkh({XPUB}/0/*) invalid");
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .is_err());
        assert_eq!(server.request_count(), 0);
    }

    #[tokio::test]
    async fn api_failure_is_propagated_instead_of_becoming_an_empty_address() {
        let server = MockEsplora::new(|_| (400, "unavailable".to_owned()));
        let input = format!("wpkh({XPUB}/0/*)");
        assert!(descriptor_addresses_transactions(&server.client(), &input)
            .await
            .is_err());
        assert_eq!(server.request_count(), 1);
    }
}
