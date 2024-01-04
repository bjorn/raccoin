---
layout: page
title: Reference
permalink: /reference/
---

## Supported Import Formats

### CSV Formats

Raccoin can import CSV files exported from the following sources:

* [Binance](https://www.binance.com/) (exchange)
* [Bitcoin Core](https://bitcoin.org/en/bitcoin-core/) (desktop wallet)
* [Bitcoin.de](https://www.bitcoin.de/de) (exchange)
* [Bitstamp](https://www.bitstamp.net/) (exchange)
* [Bittrex](https://bittrex.com/) (exchange) (order and transaction history for disabled accounts)
* [CryptoTaxCalculator](https://cryptotaxcalculator.io/) custom [CSV import
  format](https://help.cryptotaxcalculator.io/en/articles/5777675-advanced-manual-custom-csv-import)
* [Electrum](https://www.electrum.org/) (desktop wallet)
* [FTX](https://ftx.com/) (exchange)
* [Liquid](https://www.liquid.com/) (exchange)
* [Mycelium](https://wallet.mycelium.com/) (mobile wallet)
* [Peercoin](https://www.peercoin.net/wallet) (desktop wallet)
* [Poloniex](https://poloniex.com/) (exchange)
* [Reddcoin](https://www.reddcoin.com/reddwallet.html) (desktop wallet)
* [Trezor Suite](https://trezor.io/trezor-suite) (desktop and web wallet)

> Is your wallet or exchange missing? Feel free to [open an
> issue](https://github.com/bjorn/raccoin/issues) describing the contents of
> that file and its origins. It is usually very easy to add support for
> additional formats!

### Blockchains

Raccoin can also synchronize wallets from certain blockchains directly.
Supported are:

* [Bitcoin](https://bitcoin.org/) wallets (either plain addresses or x/y/zpub addresses)
* [Ethereum](https://ethereum.org/) wallets
* [Stellar](https://stellar.org/) accounts

> Currently, adding these wallets requires manually editing the portfolio JSON
> file since the UI for adding them still needs to be written. An example
> snippet for adding an Ethereum wallet would be:
> ```json
> "sources": [
>   {
>     "source_type": "EthereumAddress",
>     "path": "0xf87eC316C04bf44D87200AdCa0c9b4d6ecBd91D4",
>     "enabled": true,
>   }
> ]
> ```
> After making the edit, you'll have to restart Raccoin since it won't
> auto-reload the portfolio. Then, on the Wallets page, click the "Sync" button
> to fetch the transations. They will be stored inside the portfolio JSON file.
>
> Supported source types:
>
> * `BitcoinAddresses`: Plain Bitcoin addresses (separated by space)
> * `BitcoinXpubs`: HD wallet x/y/zpub addresses (separated by space)
> * `EthereumAddress`: A single Ethereum address
> * `StellarAccount`: A single Stellar account

### JSON Format

> There is currently no way to import transactions exported to JSON from the UI,
> but you can add them by manually adding a snippet to the portfolio JSON file,
> similar to above:
> ```json
> "sources": [
>   {
>     "source_type": "Json",
>     "path": "relative/path/to/transactions.json",
>     "enabled": true,
>   }
> ]
> ```
>
> Raccoin also supports the JSON format which can be exported from [Trezor
> Suite](https://docs.trezor.io/trezor-suite/features/transactions/export.html).
> In this case, set `source_type` to `TrezorJson`.

## Supported Export Formats

To export transactions, click either the "Export (JSON)" or "Export (CSV)"
button on the Transactions page. This will currently export _all transactions_
from enabled wallets / sources, regardless of any active filter.

### Export as JSON

The JSON format is a custom format used by Raccoin, which can also serve as an
input format (see above).

### Export as CSV

Currently when exporting transactions as CSV, they are exported in the [custom
CSV import
format](https://help.cryptotaxcalculator.io/en/articles/5777675-advanced-manual-custom-csv-import)
used by [CryptoTaxCalculator](https://cryptotaxcalculator.io/).

> Feel free to [open an issue](https://github.com/bjorn/raccoin/issues) when you
> have the need to export to any other format!
