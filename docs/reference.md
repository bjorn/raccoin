---
layout: page
title: Reference
permalink: /reference/
---

## Supported Import Formats

### CSV Formats

Raccoin can import CSV files exported from the following sources:

* [Alby](https://getalby.com/) (web wallet)
* [Alby Hub](https://albyhub.com/) (web wallet)
* [Binance](https://www.binance.com/) (exchange)
* [Bison](https://bisonapp.com/) (exchange)
* [Bitcoin Core](https://bitcoin.org/en/bitcoin-core/) (desktop wallet)
* [Bitcoin.de](https://www.bitcoin.de/de) (exchange)
* [Bitstamp](https://www.bitstamp.net/) (exchange)
* [Bittrex](https://bittrex.com/) (exchange) (order and transaction history for disabled accounts)
* [Blink](https://www.blink.sv/) (mobile wallet)
* [CryptoTaxCalculator](https://cryptotaxcalculator.io/) custom [CSV import
  format](https://help.cryptotaxcalculator.io/en/articles/5777675-advanced-manual-custom-csv-import)
* [Electrum](https://www.electrum.org/) (desktop wallet)
* [FTX](https://ftx.com/) (exchange)
* [Kraken](https://www.kraken.com/) (exchange)
* [Liquid](https://www.liquid.com/) (exchange)
* [Mycelium](https://wallet.mycelium.com/) (mobile wallet)
* [Peercoin](https://www.peercoin.net/wallet) (desktop wallet)
* [Phoenix](https://phoenix.acinq.co/) (mobile wallet)
* [Poloniex](https://poloniex.com/) (exchange)
* [Reddcoin](https://www.reddcoin.com/reddwallet/) (desktop wallet)
* [Trezor Suite](https://trezor.io/trezor-suite) (desktop and web wallet)
* [Wallet of Satoshi](https://walletofsatoshi.com/) (mobile wallet)
* [wave.space](https://www.wave.space/) (Bitcoin crypto card)

> Is your wallet or exchange missing? Feel free to [open an
> issue](https://github.com/bjorn/raccoin/issues) describing the contents of
> that file and its origins. It is usually very easy to add support for
> additional formats!

### Blockchains

Raccoin can also synchronize wallets from certain blockchains directly.
Supported are:

* [Bitcoin](https://bitcoin.org/) wallets (either plain addresses or x/y/zpub addresses, using <https://blockstream.info/>)
* [Ethereum](https://ethereum.org/) wallets (using <https://etherscan.io/>)
* [Stellar](https://stellar.org/) accounts (using <https://horizon.stellar.org/>)

> As of Raccoin 0.2, adding these wallets required manually editing the portfolio JSON
> file since the UI for adding them still needed to be written. Also, Stellar and Ethereum wallets
> can no longer be synchronized since the APIs and protocols needed an update. To use these
> features, please install a [development snapshot](/snapshot/) of Raccoin.

### JSON Format

> There is currently no way to import transactions exported to JSON from the UI,
> but you can add them by manually adding a snippet to the portfolio JSON file,
> similar to above:
> ```json
> "sources": [
>   {
>     "source_type": "Json",
>     "path": "relative/path/to/transactions.json",
>     "enabled": true
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
