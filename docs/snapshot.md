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

Below is a summary of the changes since the current release.

* Automatically restore the previously open portfolio.
* Added a "Close" button to close the portfolio, and a "Load" button to switch to a different portfolio.
* Added an "Export All" button to export all available reports to a directory.
* Added the summary displayed on the "Reports" page to the "Report Summary" CSV export.
* Added a filter to show only transactions with warnings.
* Added a text filer for transaction descriptions.
* Added support for lost, stolen and burn transactions.
* Added importing of Liquid exchange CSV export.
* Added importing of FTX exchange CSV export.
* Added importing of Trezor Suite JSON export.
* Added some icons to the UI.
* Added AQUA currency.
* Added block explorer link for Monero transactions.
* Added support for more variations of Poloniex CSV export.
* Added button to ignore a currency to the Portfolio page.
* Adding a new wallet can now be triggered with Enter key in the name field.
* Improved Ethereum address import to recognize token trades
* Separated the long-term and short-term capital losses on the "Reports" page.
* Timestamps of capital gain events are now clickable to jump to the relevant transaction.
* Currency in "Report Summary" is now clickable to filter transactions.
* Disabled UI accessibility feature, due to severe performance issues.
* Fixed switching to "Portfolio" page each time a change is made.
* Fixed UI getting blocked while synchronizing wallets.
* Fixed 100% CPU usage or freezing while synchronizing wallets.
* Fixed timestamps to use local timezone in UI and CSV export, rather than UTC and Europe/Berlin respectively.
* Fixed handling of fee on transactions ignored by currency.
* Updated to Slint 1.3.2.
