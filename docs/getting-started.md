---
layout: page
title: Getting Started
permalink: /getting-started/
---

## Create a Portfolio

After [downloading Raccoin](index.md) and launching it for the first time, we
are greeted with the welcome screen.

![Raccoin Welcome](/screenshots/raccoin-welcome.png)

Click on "New Portfolio" and choose where to save the portfolio JSON file. The
portfolio will be automatically saved each time it is modified.

## Add a Wallet

Switch to the "Wallets" page and add your first wallet, by typing its name and
pressing <kbd>Enter</kbd>.

![Wallets Page](/screenshots/raccoin-wallets-0.2.png)

Generally, you'll want to set up a wallet for each individual location where you
are holding currencies, but a single wallet can hold multiple currencies (as is
typical for exchanges).

A single wallet can also import transactions from multiple sources, which is
often required since the files exported from an exchange might only cover a
subset of the relavant transactions.

## Add a Source

A wallet is just a name for a group of transaction sources. To add transactions
to your wallet, now add a source. Currently, the UI only allows adding CSV files
in [various formats](/reference). The format of the CSV file will be auto-detected.

> If your specific CSV file format is not supported, please [open an
> issue](https://github.com/bjorn/raccoin/issues) describing the contents of
> that file and its origins! It is usually very easy to add support for
> additional formats.

## See Balances and Transactions

Now that you have added a wallet and at least one source of transactions, you
can see the resulting current balance on the Portfolio page. You can also see
the individual transactions on the Transactions page.

![Transactions Page](/screenshots/raccoin-transactions-0.2.png)

### Filtering Transactions

The transactions can be filtered in various ways.

The filter input above the transactions can be used to filter transaction by
their description or hash. The warning button can be used to show only those
transactions that triggered a warning, if there are any.

To filter transactions by wallet, go to the Wallets page and click on the badge
showing the number of transactions in that wallet.

To filter transaction by currency, go to the Portfolio page or the Reports page
and click the currency you want to filter on.

## Tax Reports

A report is generated for every year in which transactions occurred. There is
also an "All Time" report, which covers the entire history.

![Reports Page](/screenshots/raccoin-reports-0.2.png)

There are currently two types of reports. You can switch between them using the
combo box.

Individual reports can be exported using the "Export" button, but you can also
export all available reports to a chosen directory using the "Export All"
button.

### Report Summary

The report summary displays the starting balance and ending balance for each
currency you were holding during the selected time period. It also displays how
much of that currency you have disposed, what the base cost for that amount was
and the proceeds obtained by the disposal. We also see the capital profit or
loss, calculated by subtracting the cost and fees from the proceeds. Finally,
the income displays the total income, valued in fiat, generated in the given
currency.

### Capital Gain Report

This report displays all the individual capital gain events that happened during
the selected time period.
