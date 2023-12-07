use anyhow::{anyhow, Result, Context};
use chrono::NaiveDateTime;
use ethers_core::types::{Chain, U256, H256, Address};
use ethers_etherscan::{Client, account::*};
use rust_decimal::{Decimal, prelude::FromPrimitive};

use crate::base::{Transaction, Amount, Operation};

fn u256_to_decimal(value: U256) -> Result<Decimal> {
    Decimal::from_u128(value.as_u128()).context("value cannot be represented")
}

fn u256_to_eth(value: U256) -> Result<Decimal> {
    let mut value = u256_to_decimal(value)?;
    value.set_scale(18)?; // convert Wei to ETH
    Ok(value)
}

// Generic interface to the many different transaction types in the Etherscan API
trait EthereumTransaction {
    fn timestamp(&self) -> Result<NaiveDateTime> {
        let timestamp: i64 = self.timestamp_str().parse()?;
        NaiveDateTime::from_timestamp_opt(timestamp, 0).context("invalid timestamp")
    }
    fn value(&self) -> Result<Amount>;
    fn hash(&self) -> Option<String> {
        self.hash_h256().map(|hash| serde_json::to_string(hash).unwrap().trim_matches('"').to_owned())
    }

    fn fee(&self) -> Result<Option<Amount>> {
        Ok(match self.gas_price() {
            Some(gas_price) => {
                let fee = u256_to_eth(gas_price * self.gas_used())?;
                if fee.is_zero() {
                    None
                } else {
                    Some(Amount::new(fee, "ETH".to_owned()))
                }
            }
            None => None,
        })
    }

    fn to_transaction(&self, own_address: &Address) -> Result<Transaction> {
        let timestamp = self.timestamp()?;
        let mut fee: Option<Amount> = None;
        let operation = if self.to().is_some_and(|from_address| from_address == own_address) {
            Ok(Operation::Receive(self.value()?))
        } else if self.from().is_some_and(|from_address| from_address == own_address) {
            fee = self.fee()?;
            let value = self.value()?;
            if value.is_zero() && fee.is_some() {
                Ok(Operation::Fee(fee.take().unwrap()))
            } else {
                Ok(Operation::Send(value))
            }
        } else {
            Err(anyhow!("unrecognized transaction"))
        }?;
        let description = self.token_name().map(str::to_owned);

        let mut tx = Transaction::new(timestamp, operation);
        tx.tx_hash = self.hash();
        tx.blockchain = Some("ETH".to_owned());
        tx.fee = fee;
        tx.description = description;
        Ok(tx)
    }

    fn token_name(&self) -> Option<&str> { None }
    fn timestamp_str(&self) -> &str;
    fn hash_h256(&self) -> Option<&H256>;
    fn to(&self) -> Option<&Address>;
    fn from(&self) -> Option<&Address>;
    fn gas_price(&self) -> Option<U256>;
    fn gas_used(&self) -> U256;
}

impl EthereumTransaction for NormalTransaction {
    fn value(&self) -> Result<Amount> {
        u256_to_eth(self.value).map(|v| Amount::new(v, "ETH".to_owned()))
    }

    fn timestamp_str(&self) -> &str { &self.time_stamp }
    fn hash_h256(&self) -> Option<&H256> { self.hash.value() }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { self.from.value() }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for InternalTransaction {
    fn value(&self) -> Result<Amount> {
        u256_to_eth(self.value).map(|v| Amount::new(v, "ETH".to_owned()))
    }

    fn timestamp_str(&self) -> &str { &self.time_stamp }
    fn hash_h256(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.value() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { None }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC20TokenTransferEvent {
    fn value(&self) -> Result<Amount> {
        let scale: u32 = self.token_decimal.parse()?;
        let mut value = u256_to_decimal(self.value)?;
        value.set_scale(scale)?;
        Ok(Amount::new(value, self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&str> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &str { &self.time_stamp }
    fn hash_h256(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC721TokenTransferEvent {
    fn value(&self) -> Result<Amount> {
        Ok(Amount::new_token(self.token_id.clone(), self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&str> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &str { &self.time_stamp }
    fn hash_h256(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

impl EthereumTransaction for ERC1155TokenTransferEvent {
    fn value(&self) -> Result<Amount> {
        Ok(Amount::new_token(self.token_id.clone(), self.token_symbol.clone()))
    }

    fn token_name(&self) -> Option<&str> { Some(&self.token_name) }
    fn timestamp_str(&self) -> &str { &self.time_stamp }
    fn hash_h256(&self) -> Option<&H256> { Some(&self.hash) }
    fn to(&self) -> Option<&Address> { self.to.as_ref() }
    fn from(&self) -> Option<&Address> { Some(&self.from) }
    fn gas_price(&self) -> Option<U256> { self.gas_price }
    fn gas_used(&self) -> U256 { self.gas_used }
}

pub(crate) async fn address_transactions(
    address: &str,
) -> Result<Vec<Transaction>> {
    let client = Client::new(Chain::Mainnet, "YU7CJTKTFHYUKSK9KUGCAJ448QW1U26NUN")?;
    let address = address.parse()?;

    println!("requesting normal transactions for address: {:?}...", address);
    let normal_transactions = client.get_transactions(&address, None).await?;
    println!("received {} normal transactions", normal_transactions.len());

    let mut transactions = Vec::new();

    for normal_transaction in normal_transactions {
        match normal_transaction.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, normal_transaction),
        }
    }

    println!("requesting internal transactions for address: {:?}...", address);
    let internal_transactions = client.get_internal_transactions(InternalTxQueryOption::ByAddress(address), None).await?;
    println!("received {} internal transactions", internal_transactions.len());

    for internal_transaction in internal_transactions {
        match internal_transaction.to_transaction(&address) {
            Ok(tx) => transactions.push(tx),
            Err(err) => println!("{:?}: {:?}", err, internal_transaction),
        }
    }

    println!("requesting erc-20 token transfers for address: {:?}...", address);
    let erc20_transfers = client.get_erc20_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-20 token transfers", erc20_transfers.len());

    let mut merge_or_add_to_transactions = |transaction: Transaction| {
        let mut merged = false;

        // Try to find a transaction to merge with
        if let Some(matching_tx) = transactions.iter_mut().find(|tx| tx.tx_hash == transaction.tx_hash) {
            match (&matching_tx.operation, &transaction.operation) {
                // A Receive turns an existing Send into a Trade
                (Operation::Send(send_amount), Operation::Receive(receive_amount)) => {
                    matching_tx.operation = Operation::Trade { incoming: receive_amount.clone(), outgoing: send_amount.clone() };
                    merged = true;
                }
                // A Send turns an existing Receive into a Trade, and transfers the fee
                (Operation::Receive(send_amount), Operation::Send(receive_amount)) => {
                    assert!(matching_tx.fee.is_none());
                    matching_tx.fee = transaction.fee.clone();
                    matching_tx.operation = Operation::Trade { incoming: receive_amount.clone(), outgoing: send_amount.clone() };
                    merged = true;
                }
                // Transfer an existing Fee to the fee field for the operation
                (Operation::Fee(fee_amount), op) => {
                    assert!(matching_tx.fee.is_none());
                    matching_tx.fee = Some(fee_amount.clone());
                    matching_tx.operation = op.clone();
                    if matching_tx.description.is_none() {
                        matching_tx.description = transaction.description.clone();
                    }
                    merged = true;
                }
                _ => {},
            }
        }

        if !merged {
            transactions.push(transaction);
        }
    };

    for token_transfer in erc20_transfers {
        match token_transfer.to_transaction(&address) {
            Ok(transaction) => merge_or_add_to_transactions(transaction),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    println!("requesting erc-721 token transfers for address: {:?}...", address);
    let erc721_transfers = client.get_erc721_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-721 token transfers", erc721_transfers.len());

    for token_transfer in erc721_transfers {
        match token_transfer.to_transaction(&address) {
            Ok(transaction) => merge_or_add_to_transactions(transaction),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    println!("requesting erc-1155 token transfers for address: {:?}...", address);
    let erc1155_transfers = client.get_erc1155_token_transfer_events(TokenQueryOption::ByAddress(address), None).await?;
    println!("received {} erc-1155 token transfers", erc1155_transfers.len());

    for token_transfer in erc1155_transfers {
        match token_transfer.to_transaction(&address) {
            Ok(transaction) => merge_or_add_to_transactions(transaction),
            Err(err) => println!("{:?}: {:?}", err, token_transfer),
        }
    }

    Ok(transactions)
}
