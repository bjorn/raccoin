---
layout: page
title: Development Snapshot
permalink: /snapshot/
---

## How to Download

For each change committed to [the repository](https://github.com/bjorn/raccoin), Raccoin is built for all supported platforms. To download the latest development snapshot:

* Make sure you're logged-in to GitHub, otherwise the download links don't show.
* Click the most recent build from [the successful builds for the `master` branch](https://github.com/bjorn/raccoin/actions/workflows/rust.yml?query=branch%3Amaster+is%3Asuccess).
* Scroll down to find the build for your platform under "Artifacts".

## What's New

Below is a summary of the changes since [Raccoin 0.2]({{ site.baseurl }}{%
post_url 2024-01-04-raccoin-0-2 %}).

* Added support for per-wallet cost basis tracking ([#29](https://github.com/bjorn/raccoin/issues/29))
* Fixed handling of currencies that contain numbers ([#17](https://github.com/bjorn/raccoin/issues/17))
* Fixed handling of leap years in holding period calculation ([#32](https://github.com/bjorn/raccoin/issues/32))
* Fixed filtering on multiple conditions for transfers ([#54](https://github.com/bjorn/raccoin/pull/54))
* Handle loss of funds on arrival by setting a "receive fee" ([#52](https://github.com/bjorn/raccoin/pull/52))
* Handle airdrops as zero cost basis transactions ([#53](https://github.com/bjorn/raccoin/pull/53))
* Adjust to bitcoin.de CSV format changes ([#31](https://github.com/bjorn/raccoin/issues/31))
* Added support for new Bitstamp CSV format and detect SGB and FLR airdrops ([#51](https://github.com/bjorn/raccoin/pull/51))
* Added support for more Poloniex CSV formats ([#57](https://github.com/bjorn/raccoin/pull/57))
* Support Trezor CSV files using ',' as delimiter ([#56](https://github.com/bjorn/raccoin/pull/56))
* Show new wallets expanded by default
* Made the merging of consecutive trades optional
* Added BTC price history (EUR) for 2024 (by Òscar Casajuana)
* Bittrex CSV: Add 0.001 fee to BTC withdrawals (as with BCH)
* macOS: Added universal binary support
* macOS: Fixed app icon
* Updated dependencies (Slint 1.13)
