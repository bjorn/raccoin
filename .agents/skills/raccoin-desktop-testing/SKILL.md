---
name: raccoin-desktop-testing
description: Run offline Raccoin Slint GUI smoke tests with working Linux file dialogs and deterministic CSV fixtures.
---

# Local desktop setup

- Follow the repo blueprint for build dependencies and `cargo run`.
- The rfd file dialogs on Linux require `xdg-desktop-portal` and a file-picker backend such as `xdg-desktop-portal-gtk`. If New Portfolio silently returns without a dialog, check these packages and the session bus.
- On a systemd user-session desktop, use the actual user's runtime directory and bus, e.g. `XDG_RUNTIME_DIR=/run/user/$(id -u)` and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`.
- Export `DISPLAY` to the user service environment (`systemctl --user import-environment DISPLAY`), then start `xdg-desktop-portal-gtk` and `xdg-desktop-portal` before launching the app with the same bus environment.
- File pickers may open behind the Slint modal. Click their taskbar entry and confirm the picker is visibly focused before typing. Ctrl+L accepts an absolute fixture path.
- Maximize Raccoin with `wmctrl -r Raccoin -b add,maximized_vert,maximized_horz`.

# Offline flow

1. New Portfolio saves a JSON file; keep it outside the repo, because changes autosave.
2. Wallets → New Wallet Name → Add Wallet → Add Source → CSV file.
3. `demo/bitstamp_TransactionsAll.csv` yields six transactions and a 2017 report: 0.5 BTC disposed, cost 429.50, gross proceeds 1229.00, gross gain 799.50, fee/loss 2.52, net gains 796.98.
4. `tests/data/bitcoin_de_english.csv` yields five transactions (registration excluded and withdrawal fee merged) and remaining BTC 0.001. Wallet Holdings excludes fiat by design.
5. Test these fixtures independently for deterministic gains. Combining incomplete historical data under universal FIFO can invalidate later disposal cost bases. Disable the bitcoin.de source to restore the Bitstamp-only report.
6. Reports → year → Capital Gains Report switches between summary and individual disposal rows.

Offline fixtures have missing fiat values/cost bases for unmatched transfers and income. Current holdings value may be zero without prices. These are not proof of complete tax valuation correctness. Do not click Update Price History or add blockchain addresses unless network testing is explicitly in scope.

# Devin Secrets Needed

None for local CSV import, navigation, and report-generation smoke tests.
