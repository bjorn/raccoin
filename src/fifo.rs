use std::{collections::{VecDeque, HashMap}, path::Path};

use anyhow::Result;
use chrono::{NaiveDateTime, TimeZone, Local, Duration, Months};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum HoldingPeriod {
    Days(u32),
    Months(u32),
    Years(u32),
}

impl HoldingPeriod {
    fn add_to(self, dt: NaiveDateTime) -> NaiveDateTime {
        match self {
            HoldingPeriod::Days(d) => dt + Duration::days(d as i64),
            HoldingPeriod::Months(m) => dt
                .checked_add_months(Months::new(m))
                .expect("month addition should not overflow"),
            HoldingPeriod::Years(y) => dt
                .checked_add_months(Months::new(12 * y))
                .expect("year addition should not overflow"),
        }
    }
}

use crate::{base::{Operation, Transaction, Amount, GainError}, time::serialize_date_time};

/// A single entry in the FIFO (First-In-First-Out) queue representing a
/// cryptocurrency acquisition.
///
/// This structure tracks an individual purchase or acquisition of
/// cryptocurrency holdings, including when it was acquired, its cost basis, and
/// how much of the original amount remains to be disposed of. Each entry
/// represents a "lot" of cryptocurrency that will be consumed in FIFO order
/// when calculating capital gains for disposals.
#[derive(Debug, Clone)]
pub(crate) struct Lot {
    /// The timestamp when this cryptocurrency was acquired
    timestamp: NaiveDateTime,

    /// The index of the transaction that created this FIFO entry
    tx_index: usize,

    /// The unit price paid for each unit of cryptocurrency in this entry.
    /// Contains `Err(GainError)` if the cost basis could not be determined
    /// (e.g., missing fiat value), in which case a zero cost basis is used.
    unit_price: Result<Decimal, GainError>,

    /// The remaining quantity of cryptocurrency in this entry that has not yet
    /// been disposed of. This value decreases as holdings are sold or disposed of.
    quantity: Decimal,
}

impl Lot {
    fn cost_base(&self) -> Decimal {
        match self.unit_price {
            Ok(unit_price) => unit_price * self.quantity,
            Err(_) => Decimal::ZERO
        }
    }
}

/// A queue for managing cryptocurrency lots using FIFO (First-In-First-Out)
/// ordering.
///
/// This structure maintains a queue of lots ordered by acquisition time (oldest
/// first)
///
/// When disposing of holdings, the oldest entries are processed first to comply
/// with FIFO accounting rules for capital gains calculations.
#[derive(Default)]
pub(crate) struct LotQueue {
    /// Queue of lots ordered by acquisition time (oldest first)
    lots: VecDeque<Lot>,
}

impl LotQueue {
    pub(crate) fn is_empty(&self) -> bool {
        self.lots.is_empty()
    }

    /// Adds a lot to the queue while maintaining chronological order by timestamp.
    fn add(&mut self, lot: Lot) {
        // Most of the time we can just append at the end
        if self.lots.back().map_or(true, |last_lot| last_lot.timestamp <= lot.timestamp) {
            self.lots.push_back(lot);
            return;
        }

        // Find the correct position to insert while maintaining timestamp order using binary search
        let insert_index = self.lots.binary_search_by_key(&lot.timestamp, |existing_lot| existing_lot.timestamp)
            .unwrap_or_else(|pos| pos);
        self.lots.insert(insert_index, lot);
    }

    /// Removes the specified quantity from the queue in FIFO order.
    ///
    /// Returns a tuple containing:
    /// - A vector of lots that were consumed (fully or partially) to satisfy the removal
    /// - The remaining quantity that couldn't be satisfied due to insufficient holdings
    fn remove(&mut self, mut quantity: Decimal) -> (Vec<Lot>, Decimal) {
        let mut removed_lots = Vec::new();

        while let Some(lot) = self.lots.front_mut() {
            if lot.quantity <= quantity {
                // consume the lot and keep processing the remaining quantity
                quantity -= lot.quantity;
                removed_lots.push(self.lots.pop_front().unwrap());
                continue;
            }

            // we finished processing the disposal
            if !quantity.is_zero() {
                lot.quantity -= quantity;
                removed_lots.push(Lot {
                    quantity,
                    ..lot.clone()
                });
                quantity = Decimal::ZERO;
            }
            break;
        }

        (removed_lots, quantity)
    }

    fn total_quantity(&self) -> Decimal {
        self.lots.iter().map(|e| e.quantity).sum()
    }

    fn total_cost_base(&self) -> Decimal {
        self.lots.iter().map(Lot::cost_base).sum()
    }
}

/// A collection of cryptocurrency holdings organized by currency.
#[derive(Default)]
pub(crate) struct Holdings {
    lots_by_currency: HashMap<String, LotQueue>,
}

impl Holdings {
    pub(crate) fn inner(&self) -> &HashMap<String, LotQueue> {
        &self.lots_by_currency
    }

    fn add_lot(&mut self, currency: &str, lot: Lot) {
        match self.lots_by_currency.get_mut(currency) {
            Some(lots) => lots,
            None => self.lots_by_currency.entry(currency.to_owned()).or_default(),
        }.add(lot)
    }

    fn remove_lots(&mut self, currency: &str, quantity: Decimal) -> (Vec<Lot>, Decimal) {
        match self.lots_by_currency.get_mut(currency) {
            Some(lots) => lots.remove(quantity),
            None => (vec![], quantity),
        }
    }

    fn currency_balance(&self, currency: &str) -> Decimal {
        self.lots_by_currency.get(currency).map_or(Decimal::ZERO, LotQueue::total_quantity)
    }

    fn currency_cost_base(&self, currency: &str) -> Decimal {
        self.lots_by_currency.get(currency).map_or(Decimal::ZERO, LotQueue::total_cost_base)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CapitalGain {
    pub bought: NaiveDateTime,
    pub bought_tx_index: usize,
    pub sold: NaiveDateTime,
    pub sold_tx_index: usize,
    pub amount: Amount,
    pub cost: Decimal,
    pub proceeds: Decimal,
}

impl CapitalGain {
    pub(crate) fn is_held_for_at_least(&self, period: HoldingPeriod) -> bool {
        let threshold = period.add_to(self.bought);
        self.sold >= threshold
    }

    pub(crate) fn long_term(&self) -> bool {
        // Calendar-aware: 12 months from the buy timestamp
        self.is_held_for_at_least(HoldingPeriod::Years(1))
    }

    pub(crate) fn profit(&self) -> Decimal {
        self.proceeds - self.cost
    }
}

fn fiat_value(amount: Option<&Amount>) -> Result<Decimal, GainError> {
    match amount {
        Some(amount) => {
            if amount.is_fiat() {
                Ok(amount.quantity)
            } else {
                Err(GainError::InvalidFiatValue)
            }
        }
        None => Err(GainError::MissingFiatValue),
    }
}

pub(crate) struct FIFO {
    holdings: Holdings,
}

impl FIFO {
    pub(crate) fn new() -> Self {
        FIFO {
            holdings: Default::default(),
        }
    }

    pub(crate) fn process(&mut self, transactions: &mut [Transaction]) -> Vec<CapitalGain> {
        let mut capital_gains: Vec<CapitalGain> = Vec::new();

        for transaction in transactions {
            let mut fee = transaction.fee.as_ref();
            let mut fee_value = transaction.fee_value.as_ref();

            let mut try_include_fee = |amount: &Amount, value: &Option<Amount>| -> (Amount, Option<Amount>) {
                match (fee, fee_value, value) {
                    (Some(fee_amount), Some(fee_value_amount), Some(value)) => {
                        match (amount.try_add(fee_amount), value.try_add(fee_value_amount)) {
                            (Some(amount), Some(value)) => {
                                (fee, fee_value) = (None, None);
                                (amount, Some(value))
                            }
                            _ => (amount.clone(), Some(value.clone()))
                        }
                    }
                    _ => (amount.clone(), value.clone())
                }
            };

            let mut tx_gain: Option<Result<Decimal, GainError>> = None;

            match &transaction.operation {
                Operation::Staking(amount) |
                Operation::ChainSplit(amount) => {
                    if !amount.is_fiat() {
                        // Staking reward and Chain splits are treated as a zero-cost buy
                        tx_gain = Some(self.add_holdings(transaction, amount, Some(&Amount::new(Decimal::ZERO, "EUR".to_owned()))));
                    }
                }
                Operation::IncomingGift(amount) |
                Operation::Airdrop(amount) |
                Operation::Buy(amount) |
                Operation::Cashback(amount) |
                Operation::Income(amount) |     // todo: track income total
                Operation::Spam(amount) => {
                    if !amount.is_fiat() {
                        tx_gain = Some(self.add_holdings(transaction, amount, transaction.value.as_ref()));
                    }
                }
                Operation::Trade{incoming, outgoing} => {
                    // If we're paying a fee in the same currency as the
                    // outgoing currency, we can merge it with the outgoing
                    // amount to reduce capital gain events (in case the fee is
                    // crypto) as well as to take the fee into account for the
                    // cost base
                    let (outgoing, value) = try_include_fee(outgoing, &transaction.value);

                    // todo: when we're paying a fee in the same currency as the
                    // incoming currency, we could similarly reduce capital gain
                    // events by subtracting it from the incoming amount.
                    // (see also calculate_tax_reports)

                    // When we're trading crypto for crypto, it is technically
                    // handled as if we sold one crypto for fiat and then used
                    // fiat to buy another crypto.
                    if !outgoing.is_fiat() {
                        tx_gain = Some(self.dispose_holdings(&mut capital_gains, transaction, &outgoing, transaction.value.as_ref()));
                    }

                    if !incoming.is_fiat() {
                        let result = self.add_holdings(transaction, incoming, value.as_ref());
                        if result.is_err() && tx_gain.is_none() {
                            tx_gain = Some(result);
                        }
                    }
                }
                Operation::Swap { incoming, outgoing } => {
                    tx_gain = Some(if !outgoing.is_fiat() && !incoming.is_fiat() {
                        self.swap_holdings(transaction, outgoing, incoming)
                    } else {
                        // Swapping is not supported to/from fiat, handle as trade and return error
                        if !outgoing.is_fiat() {
                            let _ = self.dispose_holdings(&mut capital_gains, transaction, outgoing, transaction.value.as_ref());
                        }

                        if !incoming.is_fiat() {
                            let _ = self.add_holdings(transaction, incoming, transaction.value.as_ref());
                        }

                        Err(GainError::InvalidFiatValue)
                    })
                }
                Operation::Fee(amount) |
                Operation::Expense(amount) |
                Operation::Sell(amount) |
                Operation::OutgoingGift(amount) => {
                    if !amount.is_fiat() {
                        let (amount, value) = try_include_fee(amount, &transaction.value);
                        tx_gain = Some(self.dispose_holdings(&mut capital_gains, transaction, &amount, value.as_ref()));
                    }
                }
                // Lost/stolen/burned funds are handled as if they were sold for nothing
                Operation::Stolen(amount) |
                Operation::Lost(amount) |
                Operation::Burn(amount) => {
                    if !amount.is_fiat() {
                        let (amount, _) = try_include_fee(amount, &transaction.value);
                        tx_gain = Some(self.dispose_holdings(&mut capital_gains, transaction, &amount, Some(Amount::from_fiat(Decimal::ZERO)).as_ref()));
                    }
                }
                Operation::FiatDeposit(_) |
                Operation::FiatWithdrawal(_) => {
                    // We're not tracking fiat at the moment (it's not relevant for tax purposes)
                }
                Operation::Receive(_) |
                Operation::Send(_) => {
                    // Verify that these are matched as transfer, otherwise they should have been Buy/Sell
                    assert!(transaction.matching_tx.is_some(), "no matching tx");
                }
            }

            if let Some(fee) = fee {
                if !fee.is_fiat() {
                    match self.dispose_holdings(&mut capital_gains, transaction, fee, fee_value) {
                        Ok(gain) => {
                            match &mut tx_gain {
                                Some(Ok(g)) => {
                                    *g += gain;
                                }
                                Some(Err(_)) => {}
                                None => tx_gain = Some(Ok(gain)),
                            }
                        }
                        Err(err) => if tx_gain.is_none() {
                            tx_gain = Some(Err(err));
                        }
                    }
                }
            }

            transaction.gain = tx_gain;
        }

        capital_gains
    }

    /// Determines the capital gains made with this sale based on the oldest
    /// holdings and the current price. Consumes the holdings in the process.
    fn gains(&mut self, transaction: &Transaction, outgoing: &Amount, incoming_fiat: Decimal) -> Result<Vec<CapitalGain>, GainError> {
        let mut capital_gains: Vec<CapitalGain> = Vec::new();
        if outgoing.quantity.is_zero() {
            return Ok(capital_gains);
        }

        let sold_unit_price = incoming_fiat / outgoing.quantity;
        let mut cost_base_error = Ok(());

        let (lots, missing_quantity) = self.holdings.remove_lots(outgoing.token_currency().as_ref().unwrap_or(&outgoing.currency), outgoing.quantity);

        for lot in lots {
            if lot.timestamp > transaction.timestamp {
                return Err(GainError::InvalidTransactionOrder);
            }

            let cost = match lot.unit_price {
                Ok(price) => lot.quantity * price,
                Err(_) => {
                    cost_base_error = Err(GainError::MissingCostBase);
                    Decimal::ZERO
                }
            };
            capital_gains.push(CapitalGain {
                bought: lot.timestamp,
                bought_tx_index: lot.tx_index,
                sold: transaction.timestamp,
                sold_tx_index: transaction.index,
                amount: Amount {
                    quantity: lot.quantity,
                    currency: outgoing.currency.clone(),
                    token_id: outgoing.token_id.clone(),
                },
                cost,
                proceeds: lot.quantity * sold_unit_price,
            });
        }

        if missing_quantity > Decimal::ZERO {
            println!("warning: at {} a remaining sold amount of {} {} was not found in the holdings", transaction.timestamp, missing_quantity, outgoing.currency);
            return Err(GainError::InsufficientBalance(Amount::new(missing_quantity, outgoing.currency.clone())));
        }

        cost_base_error.map(|_| capital_gains)
    }

    pub(crate) fn currency_balance(&self, currency: &str) -> Decimal {
        self.holdings.currency_balance(currency)
    }

    pub(crate) fn currency_cost_base(&self, currency: &str) -> Decimal {
        self.holdings.currency_cost_base(currency)
    }

    /// Read-only access to the holdings.
    pub(crate) fn holdings(&self) -> &Holdings {
        &self.holdings
    }

    fn add_holdings(&mut self, tx: &Transaction, amount: &Amount, value: Option<&Amount>) -> Result<Decimal, GainError> {
        self.add_holdings_with_timestamp(tx, amount, value, tx.timestamp)
    }

    fn add_holdings_with_timestamp(&mut self, tx: &Transaction, amount: &Amount, value: Option<&Amount>, timestamp: NaiveDateTime) -> Result<Decimal, GainError> {
        // Refuse to add zero balances (and protect against division by zero)
        if amount.quantity.is_zero() {
            return Ok(Decimal::ZERO);
        }

        let unit_price = fiat_value(value).map(|value| value / amount.quantity);
        self.holdings.add_lot(amount.token_currency().as_ref().unwrap_or(&amount.currency), Lot {
            timestamp,
            tx_index: tx.index,
            unit_price,
            quantity: amount.quantity,
        });

        Ok(Decimal::ZERO)
    }

    fn dispose_holdings(&mut self, capital_gains: &mut Vec<CapitalGain>, transaction: &Transaction, outgoing: &Amount, value: Option<&Amount>) -> Result<Decimal, GainError> {
        let fiat = fiat_value(value);

        match self.gains(transaction, outgoing, *fiat.as_ref().unwrap_or(&Decimal::ZERO)) {
            Ok(gains) => {
                let gain = gains.iter().map(|f| f.proceeds - f.cost).sum();
                capital_gains.extend(gains);
                fiat.map(|_| gain).map_err(|_| GainError::MissingFiatValue)
            }
            Err(e) => Err(e),
        }
    }

    fn swap_holdings(&mut self, transaction: &Transaction, outgoing: &Amount, incoming: &Amount) -> Result<Decimal, GainError> {
        if outgoing.quantity.is_zero() && incoming.quantity.is_zero() {
            return Ok(Decimal::ZERO);
        }
        if outgoing.quantity.is_zero() || incoming.quantity.is_zero() {
            return Err(GainError::InvalidSwap);
        }

        let ratio = outgoing.quantity / incoming.quantity;

        match self.gains(transaction, outgoing, Decimal::ZERO) {
            Ok(gains) => {
                // Transfer the original acquisition cost and timestamp to the newly acquired currency
                for gain in gains {
                    let amount = Amount::new(ratio * gain.amount.quantity, incoming.currency.clone());
                    self.add_holdings_with_timestamp(transaction, &amount, Some(&Amount::from_fiat(gain.cost)), gain.bought)?;
                }
                Ok(Decimal::ZERO)
            }
            Err(e) => Err(e),
        }
    }
}

pub(crate) fn save_gains_to_csv(gains: &Vec<CapitalGain>, output_path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(output_path)?;

    #[derive(Serialize)]
    struct CsvGain<'a> {
        #[serde(rename = "Currency")]
        currency: &'a str,
        #[serde(rename = "Bought", serialize_with = "serialize_date_time")]
        bought: NaiveDateTime,
        #[serde(rename = "Sold", serialize_with = "serialize_date_time")]
        sold: NaiveDateTime,
        #[serde(rename = "Quantity")]
        quantity: Decimal,
        #[serde(rename = "Cost")]
        cost: Decimal,
        #[serde(rename = "Proceeds")]
        proceeds: Decimal,
        #[serde(rename = "Gain or Loss")]
        gain_or_loss: Decimal,
        #[serde(rename = "Long Term")]
        long_term: bool,
    }

    for gain in gains {
        wtr.serialize(CsvGain {
            currency: &gain.amount.currency,
            bought: Local.from_utc_datetime(&gain.bought).naive_local(),
            sold: Local.from_utc_datetime(&gain.sold).naive_local(),
            quantity: gain.amount.quantity,
            cost: gain.cost.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            proceeds: gain.proceeds.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            gain_or_loss: (gain.proceeds - gain.cost).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            long_term: gain.long_term(),
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use rust_decimal::Decimal;

    fn gain(bought: &str, sold: &str) -> CapitalGain {
        let bought = NaiveDateTime::parse_from_str(bought, "%Y-%m-%d %H:%M:%S").unwrap();
        let sold = NaiveDateTime::parse_from_str(sold, "%Y-%m-%d %H:%M:%S").unwrap();

        CapitalGain {
            bought,
            bought_tx_index: 0,
            sold,
            sold_tx_index: 0,
            amount: Amount::new(Decimal::ONE, "BTC".to_string()),
            cost: Decimal::ZERO,
            proceeds: Decimal::ZERO,
        }
    }

    #[test]
    fn long_term_exact_calendar_year_regular() {
        assert!(!gain("2021-01-01 00:00:00", "2021-12-31 23:59:59").long_term());
        assert!(gain("2021-01-01 00:00:00", "2022-01-01 00:00:00").long_term());
    }

    #[test]
    fn long_term_leap_feb29_to_feb28() {
        assert!(!gain("2020-02-29 12:00:00", "2021-02-28 11:59:59").long_term());
        assert!(gain("2020-02-29 12:00:00", "2021-02-28 12:00:00").long_term());
    }

    #[test]
    fn configurable_days_183_threshold() {
        // 183 days from 2021-01-01 00:00:00 is 2021-07-03 00:00:00
        assert!(!gain("2021-01-01 00:00:00", "2021-07-02 23:59:59").is_held_for_at_least(HoldingPeriod::Days(183)));
        assert!(gain("2021-01-01 00:00:00", "2021-07-03 00:00:00").is_held_for_at_least(HoldingPeriod::Days(183)));
    }
}
