use std::error::Error;

use chrono::NaiveDateTime;
use ethers_core::types::{Chain, U256, H256, Address};
use ethers_etherscan::{Client, account::*};
use rust_decimal::{Decimal, prelude::FromPrimitive};

use crate::base::{Transaction, Amount, Operation};

fn u256_to_decimal(value: U256) -> Result<Decimal, Box<dyn Error>> {
    Decimal::from_u128(value.as_u128()).ok_or("value cannot be represented".into())
}

fn u256_to_eth(value: U256) -> Result<Decimal, Box<dyn Error>> {
    let mut value = u256_to_decimal(value)?;
    value.set_scale(18)?; // convert Wei to ETH
    Ok(value)
}

// Generic interface to the many different transaction types in the Etherscan API
trait EthereumTransaction {
    fn timestamp(&self) -> Result<NaiveDateTime, Box<dyn Error>> {
        let timestamp: i64 = self.timestamp_str().parse()?;
        NaiveDateTime::from_timestamp_opt(timestamp, 0).ok_or("invalid timestamp".into())
    }
    fn value(&self) -> Result<Amount, Box<dyn Error>>;
    fn hash(&self) -> Option<String> {
        self.hash_opt().map(|hash| serde_json::to_string(hash).unwrap().trim_matches('"').to_owned())
    }

    fn fee(&self) -> Result<Option<Amount>, Box<dyn Error>> {
        Ok(match self.gas_price() {
            Some(gas_price) => {
                let gas_used = u256_to_decimal(self.gas_used())?;
                let gas_price = u256_to_eth(gas_price)?;
                Some(Amount::new(gas_price * gas_used, "ETH".to_owned()))
            }
            None => None,
        })
    }

    fn to_transaction(&self, own_address: &Address) -> Result<Transaction, Box<dyn Error>> {
        let timestamp = self.timestamp()?;
        let mut fee: Option<Amount> = None;
        let operation = if self.to().is_some_and(|from_address| from_address == own_address) {
            Ok(Operation::Receive(self.value()?))
        } else if self.from().is_some_and(|from_address| from_address == own_address) {
            fee = self.fee()?;
            Ok(Operation::Send(self.value()?))
        } else {
            Err("unrecognized transaction".into())
        };
        let description = self.token_name().map(String::to_owned);

        operation.map(|operation| {
            let mut tx = Transaction::new(timestamp, operation);
            tx.tx_hash = self.hash();
            tx.blockchain = Some("ETH".to_owned());
            tx.fee = fee;
            tx.description = description;
            tx
        })
    }

    fn token_name(&self) -> Option<&String> { None }
    fn timestamp_str(&self) -> &String;
    fn hash_opt(&self) -> Option<&H256>;
    fn to(&self) -> Option<&Address>;
    fn from(&self) -> Option<&Address>;
    fn gas_price(&self) -> Option<U256>;
    fn gas_used(&self) -> U256;
}

impl EthereumTransaction for NormalTransaction {
    fn value(&self) -> Result<Amount, Box<dyn Error>> {
        u256_to_eth(self.value).map(|v| Amount::new(v, "ETH".to_owned()))
    }

    fn timestamp_str(&self) -> &String { &self.time_stamp }
    fn hash_opt(&self) -> Option<&H256> { self.hash.value() }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { self.from.value() }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for InternalTransaction {
    fn value(&self) -> Result<Amount, Box<dyn Error>> {
        u256_to_eth(self.value).map(|v| Amount::new(v, "ETH".to_owned()))
    }

    fn timestamp_str(&self) -> &String { &self.time_stamp }
    fn hash_opt(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.value() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { None }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC20TokenTransferEvent {
    fn value(&self) -> Result<Amount, Box<dyn Error>> {
        let scale: u32 = self.token_decimal.parse()?;
        let mut value = u256_to_decimal(self.value)?;
        value.set_scale(scale)?;
        Ok(Amount::new(value, self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&String> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &String { &self.time_stamp }
    fn hash_opt(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC721TokenTransferEvent {
    fn value(&self) -> Result<Amount, Box<dyn Error>> {
        Ok(Amount::new_token(self.token_id.clone(), self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&String> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &String { &self.time_stamp }
    fn hash_opt(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC1155TokenTransferEvent {
    fn value(&self) -> Result<Amount, Box<dyn Error>> {
        Ok(Amount::new_token(self.token_id.clone(), self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&String> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &String { &self.time_stamp }
    fn hash_opt(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

pub(crate) async fn address_transactions(
    address: &str,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let client = Client::new(Chain::Mainnet, "YU7CJTKTFHYUKSK9KUGCAJ448QW1U26NUN")?;
    let address = address.parse()?;

    println!("requesting normal transactions for address: {:?}...", address);
    let metadata = client.get_transactions(&address, None).await?;
    println!("received {} normal transactions", metadata.len());

    let mut transactions = Vec::new();

    for normal_transaction in metadata {
        match normal_transaction.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, normal_transaction),
        }
    }

    println!("requesting internal transactions for address: {:?}...", address);
    let metadata = client.get_internal_transactions(InternalTxQueryOption::ByAddress(address), None).await?;
    println!("received {} internal transactions", metadata.len());

    for internal_transaction in metadata {
        match internal_transaction.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, internal_transaction),
        }
    }

    println!("requesting erc-20 token transfers for address: {:?}...", address);
    let metadata = client.get_erc20_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-20 token transfers", metadata.len());

    for token_transfer in metadata {
        match token_transfer.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    println!("requesting erc-721 token transfers for address: {:?}...", address);
    let metadata = client.get_erc721_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-721 token transfers", metadata.len());

    for token_transfer in metadata {
        match token_transfer.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    println!("requesting erc-1155 token transfers for address: {:?}...", address);
    let metadata = client.get_erc1155_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-1155 token transfers", metadata.len());

    for token_transfer in metadata {
        match token_transfer.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    Ok(transactions)
}
