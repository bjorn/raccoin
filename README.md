# Raccoin

### Crypto Portfolio and Tax Reporting Tool

Raccoin makes it easy to see the current state and the history of your crypto
portfolio and generates relevant reports for declaring capital gain income tax.
It currently works in EUR using the FIFO method, but since it is open source
anyone can extend it to suit their needs.

<img src="raccoin_ui/ui/icons/app-icon.svg" width="64" height="64">

## Supported Formats

### CSV Formats

Raccoin can import CSV files exported from the following sources:

* [Alby](https://getalby.com/) (web wallet)
* [Alby Hub](https://albyhub.com/) (web wallet)
* [Binance](https://www.binance.com/) (exchange)
* [Bitcoin Core](https://bitcoin.org/en/bitcoin-core/) (desktop wallet)
* [Bitcoin.de](https://www.bitcoin.de/de) (exchange)
* [Bitstamp](https://www.bitstamp.net/) (exchange)
* [Bittrex](https://bittrex.com/) (exchange) (order and transaction history for disabled accounts)
* [Blink](https://www.blink.sv/) (mobile wallet)
* [CryptoTaxCalculator](https://cryptotaxcalculator.io/) custom [CSV import format](https://help.cryptotaxcalculator.io/en/articles/5777675-advanced-manual-custom-csv-import)
* [Electrum](https://www.electrum.org/) (desktop wallet)
* [FTX](https://ftx.com/) (exchange)
* [Liquid](https://www.liquid.com/) (exchange)
* [Mycelium](https://wallet.mycelium.com/) (mobile wallet)
* [Peercoin](https://www.peercoin.net/wallet) (desktop wallet)
* [Phoenix](https://phoenix.acinq.co/) (mobile wallet)
* [Poloniex](https://poloniex.com/) (exchange)
* [Reddcoin](https://www.reddcoin.com/reddwallet/) (desktop wallet)
* [Trezor Suite](https://trezor.io/trezor-suite) (desktop and web wallet)
* [Wallet of Satoshi](https://walletofsatoshi.com/) (mobile wallet)

### Blockchains

Raccoin can also synchronize wallets from certain blockchains directly. Currently, adding these wallets requires manually editing the portfolio JSON file since the UI for adding them still needs to be written. Supported are:

* [Bitcoin](https://bitcoin.org/) wallets (either plain addresses or x/y/zpub addresses)
* [Ethereum](https://ethereum.org/) wallets
* [Stellar](https://stellar.org/) accounts

### JSON

Transactions can be exported to JSON and can also be imported from that format again.

## Attribution

This application was written in [Rust](https://www.rust-lang.org/) using the excellent [Slint UI framework](https://slint.dev/).

[Codeium](https://codeium.com) has been a major help, especially since I was still relatively new to Rust.

Thanks to my wife for suggesting "Raccoin", which is a combination of "raccoon" and "coin" and for helping me with the icon. Raccoons are known to be smart animals, and you'd be smart to use Raccoin!

[![built with Codeium](https://codeium.com/badges/main)](https://codeium.com/profile)
