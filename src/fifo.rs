use std::{collections::{VecDeque, HashMap}, error::Error, path::Path};

use chrono::NaiveDateTime;
use chrono_tz::Europe;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

use crate::{base::{Operation, Transaction, Amount, GainError}, time::serialize_date_time};

// Temporary bookkeeping entry for FIFO
#[derive(Debug)]
pub(crate) struct Entry {
    timestamp: NaiveDateTime,
    unit_price: Result<Decimal, GainError>,
    remaining: Decimal,
}

impl Entry {
    fn cost_base(&self) -> Decimal {
        match self.unit_price {
            Ok(unit_price) => unit_price * self.remaining,
            Err(_) => Decimal::ZERO
        }
    }
}

#[derive(Debug)]
pub(crate) struct CapitalGain {
    pub bought: NaiveDateTime,
    pub sold: NaiveDateTime,
    pub amount: Amount,
    pub cost: Decimal,
    pub proceeds: Decimal,
}

impl CapitalGain {
    pub(crate) fn long_term(&self) -> bool {
        (self.sold - self.bought) > chrono::Duration::days(365)
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
    /// Holdings represented as a map of currency -> deque.
    holdings: HashMap<String, VecDeque<Entry>>,
}

impl FIFO {
    pub(crate) fn new() -> Self {
        FIFO {
            holdings: HashMap::new(),
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

            match &transaction.operation {
                Operation::Staking(amount) |
                Operation::ChainSplit(amount) => {
                    if !amount.is_fiat() {
                        // Staking reward and Chain splits are treated as a zero-cost buy
                        transaction.gain = Some(self.add_holdings(transaction, amount, Some(&Amount {
                            quantity: Decimal::ZERO,
                            currency: "EUR".to_owned()
                        })));
                    }
                }
                Operation::IncomingGift(amount) |
                Operation::Airdrop(amount) |
                Operation::Buy(amount) |
                Operation::Cashback(amount) |
                Operation::Income(amount) |     // todo: track income total
                Operation::Spam(amount) => {
                    if !amount.is_fiat() {
                        transaction.gain = Some(self.add_holdings(transaction, amount, transaction.value.as_ref()));
                    }
                }
                Operation::Trade{incoming, outgoing} => {
                    // todo: consider factoring in trading fees to reduce capital gain events

                    // When we're trading crypto for crypto, it is technically
                    // handled as if we sold one crypto for fiat and then used
                    // fiat to buy another crypto.
                    if !outgoing.is_fiat() {
                        transaction.gain = Some(self.dispose_holdings(&mut capital_gains, transaction.timestamp, outgoing, transaction.value.as_ref()));
                    }

                    if !incoming.is_fiat() {
                        let result = self.add_holdings(transaction, incoming, transaction.value.as_ref());
                        if result.is_err() && transaction.gain.is_none() {
                            transaction.gain = Some(result);
                        }
                    }
                }
                Operation::Fee(amount) |
                Operation::Expense(amount) |
                Operation::Sell(amount) |
                Operation::OutgoingGift(amount) => {
                    if !amount.is_fiat() {
                        let (amount, value) = try_include_fee(amount, &transaction.value);
                        transaction.gain = Some(self.dispose_holdings(&mut capital_gains, transaction.timestamp, &amount, value.as_ref()));
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
                    match self.dispose_holdings(&mut capital_gains, transaction.timestamp, &fee, fee_value) {
                        Ok(gain) => {
                            match &mut transaction.gain {
                                Some(Ok(g)) => {
                                    *g += gain;
                                }
                                Some(Err(_)) => {}
                                None => transaction.gain = Some(Ok(gain)),
                            }
                        }
                        Err(err) => if transaction.gain.is_none() {
                            transaction.gain = Some(Err(err));
                        }
                    }
                }
            }
        }

        capital_gains
    }

    /// Determines the capital gains made with this sale based on the oldest
    /// holdings and the current price. Consumes the holdings in the process.
    fn gains(&mut self, timestamp: NaiveDateTime, outgoing: &Amount, incoming_fiat: Decimal) -> Result<Vec<CapitalGain>, GainError> {
        let currency_holdings = self.holdings_for_currency(&outgoing.currency);

        let mut capital_gains: Vec<CapitalGain> = Vec::new();
        let mut sold_quantity = outgoing.quantity;
        if sold_quantity.is_zero() {
            return Ok(capital_gains);
        }

        let sold_unit_price = incoming_fiat / sold_quantity;
        let mut cost_base_error = Ok(());

        while let Some(holding) = currency_holdings.front_mut() {
            if holding.timestamp > timestamp {
                return Err(GainError::InvalidTransactionOrder);
            }

            // we can process up to the amount in the holding entry
            let processed_quantity = holding.remaining.min(sold_quantity);
            let cost = match holding.unit_price {
                Ok(price) => processed_quantity * price,
                Err(_) => {
                    cost_base_error = Err(GainError::MissingCostBase);
                    Decimal::ZERO
                }
            };
            let proceeds = processed_quantity * sold_unit_price;

            capital_gains.push(CapitalGain {
                bought: holding.timestamp,
                sold: timestamp,
                amount: Amount {
                    quantity: processed_quantity,
                    currency: outgoing.currency.clone(),
                },
                cost,
                proceeds,
            });

            sold_quantity -= processed_quantity;

            if holding.remaining == processed_quantity {
                // consume the holding and keep processing the remaining quantity
                currency_holdings.pop_front();
            } else {
                // we finished processing the sale
                holding.remaining -= processed_quantity;
                break;
            }
        }

        if sold_quantity > Decimal::ZERO {
            println!("warning: at {} a remaining sold amount of {} {} was not found in the holdings", timestamp, sold_quantity, outgoing.currency);
            return Err(GainError::InsufficientBalance(Amount::new(sold_quantity, outgoing.currency.clone())));
        }

        cost_base_error.map(|_| capital_gains)
    }

    pub(crate) fn currency_balance(&self, currency: &str) -> Decimal {
        self.holdings.get(currency).map_or(Decimal::ZERO, total_holdings)
    }

    pub(crate) fn currency_cost_base(&self, currency: &str) -> Decimal {
        self.holdings.get(currency).map_or(Decimal::ZERO, |h| h.iter().map(|e| e.cost_base()).sum())
    }

    /// Read-only access to the holdings.
    pub(crate) fn holdings(&self) -> &HashMap<String, VecDeque<Entry>> {
        &self.holdings
    }

    fn holdings_for_currency(&mut self, currency: &str) -> &mut VecDeque<Entry> {
        // match self.holdings.get_mut(currency) {
        //     Some(vec) => vec,
        //     None => self.holdings.entry(currency.to_owned()).or_default(),
        // }
        // Why does the above not work? It would avoid one needles lookup...
        // (see https://rust-lang.github.io/rfcs/2094-nll.html#problem-case-3-conditional-control-flow-across-functions)
        if self.holdings.contains_key(currency) {
            self.holdings.get_mut(currency).unwrap()
        } else {
            self.holdings.entry(currency.to_owned()).or_default()
        }
    }

    fn add_holdings(&mut self, tx: &Transaction, amount: &Amount, value: Option<&Amount>) -> Result<Decimal, GainError> {
        // Refuse to add zero balances (and protect against division by zero)
        if amount.quantity.is_zero() {
            return Ok(Decimal::ZERO);
        }

        let unit_price = fiat_value(value).map(|value| value / amount.quantity);
        self.holdings_for_currency(&amount.currency).push_back(Entry {
            timestamp: tx.timestamp,
            unit_price,
            remaining: amount.quantity,
        });

        Ok(Decimal::ZERO)
    }

    fn dispose_holdings(&mut self, capital_gains: &mut Vec<CapitalGain>, timestamp: NaiveDateTime, outgoing: &Amount, value: Option<&Amount>) -> Result<Decimal, GainError> {
        let fiat = fiat_value(value);

        match self.gains(timestamp, outgoing, *fiat.as_ref().unwrap_or(&Decimal::ZERO)) {
            Ok(gains) => {
                let gain = gains.iter().map(|f| f.proceeds - f.cost).sum();
                capital_gains.extend(gains);
                fiat.map(|_| gain).map_err(|_| GainError::MissingFiatValue)
            }
            Err(e) => Err(e),
        }
    }
}

fn total_holdings(holdings: &VecDeque<Entry>) -> Decimal {
    holdings.iter().map(|e| e.remaining).sum()
}

pub(crate) fn save_gains_to_csv(gains: &Vec<CapitalGain>, output_path: &Path) -> Result<(), Box<dyn Error>> {
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
            bought: gain.bought.and_utc().with_timezone(&Europe::Berlin).naive_local(),
            sold: gain.sold.and_utc().with_timezone(&Europe::Berlin).naive_local(),
            quantity: gain.amount.quantity,
            cost: gain.cost.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            proceeds: gain.proceeds.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            gain_or_loss: (gain.proceeds - gain.cost).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
            long_term: (gain.sold - gain.bought) > chrono::Duration::days(365),
        })?;
    }

    Ok(())
}
