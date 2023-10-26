---
layout: page
title: Reference
---

## Supported Formats

### CSV Formats

Raccoin can import CSV files exported from the following sources:

* [Binance](https://www.binance.com/) (exchange)
* [Bitcoin Core](https://bitcoin.org/en/bitcoin-core/) (desktop wallet)
* [Bitcoin.de](https://www.bitcoin.de/de) (exchange)
* [Bitstamp](https://www.bitstamp.net/) (exchange)
* [Bittrex](https://bittrex.com/) (exchange) (order and transaction history for disabled accounts)
* [CryptoTaxCalculator](https://cryptotaxcalculator.io/) custom [CSV import format](https://help.cryptotaxcalculator.io/en/articles/5777675-advanced-manual-custom-csv-import)
* [Electrum](https://www.electrum.org/) (desktop wallet)
* [Mycelium](https://wallet.mycelium.com/) (mobile wallet)
* [Peercoin](https://www.peercoin.net/wallet) (desktop wallet)
* [Poloniex](https://poloniex.com/) (exchange)
* [Reddcoin](https://www.reddcoin.com/reddwallet.html) (desktop wallet)
* [Trezor Suite](https://trezor.io/trezor-suite) (desktop and web wallet)

### Blockchains

Raccoin can also synchronize wallets from certain blockchains directly. Currently, adding these wallets requires manually editing the portfolio JSON file since the UI for adding them still needs to be written. Supported are:

* [Bitcoin](https://bitcoin.org/) wallets (either plain addresses or x/y/zpub addresses)
* [Ethereum](https://ethereum.org/) wallets
* [Stellar](https://stellar.org/) accounts

### JSON

Transactions can be exported to JSON and can also be imported from that format again.
