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
* Added some icons to the UI.
* Added AQUA currency.
* Added support for more variations of Poloniex CSV export.
* Updated to Slint 1.3.2.
* Separated the long-term and short-term capital losses on the "Reports" page.
* Disabled UI accessibility feature, due to severe performance issues.
* Fixed switching to "Portfolio" page each time a change is made.
* Fixed UI getting blocked while synchronizing wallets.
* Fixed 100% CPU usage or freezing while synchronizing wallets.
* Fixed timestamps to use local timezone in UI and CSV export, rather than UTC and Europe/Berlin respectively.
* Fixed handling of fee on transactions ignored by currency.