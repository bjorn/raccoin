//! Price history module with simple sorted vector storage.
//!
//! This module provides a `PriceHistory` struct that stores price data as sorted
//! vectors of price points per currency, allowing for efficient linear interpolation.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::base::Amount;

/// A single price point with timestamp and price.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "(NaiveDateTime, Decimal)", into = "(NaiveDateTime, Decimal)")]
pub(crate) struct PricePoint {
    pub timestamp: NaiveDateTime,
    pub price: Decimal,
}

impl From<(NaiveDateTime, Decimal)> for PricePoint {
    fn from((timestamp, price): (NaiveDateTime, Decimal)) -> Self {
        Self { timestamp, price }
    }
}

impl From<PricePoint> for (NaiveDateTime, Decimal) {
    fn from(point: PricePoint) -> Self {
        (point.timestamp, point.price)
    }
}

impl PartialOrd for PricePoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PricePoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

/// A time range with start and end timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TimeRange {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

impl TimeRange {
    pub fn new(start: NaiveDateTime, end: NaiveDateTime) -> Self {
        debug_assert!(start <= end, "TimeRange start must be <= end");
        Self { start, end }
    }

    /// Check if this range overlaps with or is adjacent to another range.
    /// Adjacent means the ranges connect without a gap (within the padding).
    pub fn overlaps_or_adjacent(&self, other: &TimeRange, padding: Duration) -> bool {
        self.start <= other.end + padding && other.start <= self.end + padding
    }

    /// Merge this range with another, returning the combined range.
    pub fn merge(&self, other: &TimeRange) -> TimeRange {
        TimeRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Price data for a single currency, stored as a sorted vector of price points.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CurrencyPriceData {
    /// Sorted list of price points.
    prices: Vec<PricePoint>,
}

impl CurrencyPriceData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add price points, inserting each into the sorted position.
    pub fn add_points(&mut self, price_points: Vec<PricePoint>) {
        for price_point in price_points {
            let pos = self.prices.partition_point(|p| *p < price_point);
            // Skip duplicates (same timestamp)
            if pos < self.prices.len() && self.prices[pos].timestamp == price_point.timestamp {
                continue;
            }
            self.prices.insert(pos, price_point);
        }
    }

    /// Get the ranges needed to cover multiple timestamps.
    /// Returns a list of missing ranges, merged where possible.
    ///
    /// A timestamp is considered covered if estimate_price returns an accuracy
    /// within the tolerance. Missing ranges are tolerance-wide around each
    /// timestamp, and padding is used to determine whether to merge nearby ranges.
    pub fn missing_ranges_for_timestamps(
        &self,
        timestamps: &[NaiveDateTime],
        tolerance: Duration,
        padding: Duration,
    ) -> Vec<TimeRange> {
        let mut missing: Vec<TimeRange> = Vec::new();

        assert!(timestamps.is_sorted(), "Timestamps must be sorted");

        for &ts in timestamps {
            // Check if we have sufficient accuracy for this timestamp
            let needs_data = match self.estimate_price(ts) {
                Some((_, accuracy)) => accuracy > tolerance,
                None => true,
            };

            if needs_data {
                let needed = TimeRange::new(ts - tolerance, ts + tolerance);

                // Try to merge with the last missing range
                if let Some(last) = missing.last_mut() {
                    if last.overlaps_or_adjacent(&needed, padding) {
                        *last = last.merge(&needed);
                        continue;
                    }
                }
                missing.push(needed);
            }
        }

        missing
    }

    /// Estimate the price at a given timestamp using linear interpolation.
    ///
    /// Finds the nearest price points before and after the timestamp and
    /// performs linear interpolation. If only one side is available,
    /// uses that price with appropriate accuracy.
    pub fn estimate_price(&self, time: NaiveDateTime) -> Option<(Decimal, Duration)> {
        let index = self.prices.partition_point(|p| p.timestamp < time);
        let next_price_point = self.prices.get(index).or_else(|| self.prices.last());
        let prev_price_point = if index > 0 { self.prices.get(index - 1) } else { None };

        match (prev_price_point, next_price_point) {
            (Some(prev_price), Some(next_price)) => {
                // Calculate the most probable price by linear interpolation
                let price_difference = next_price.price - prev_price.price;
                let total_duration: Decimal = (next_price.timestamp - prev_price.timestamp).num_seconds().into();

                // The accuracy is the minimum time difference between the requested time and a price point
                let accuracy = (time - prev_price.timestamp).abs().min((next_price.timestamp - time).abs());

                if total_duration > Decimal::ZERO {
                    let time_since_prev: Decimal = (time - prev_price.timestamp).num_seconds().into();
                    let time_ratio = time_since_prev / total_duration;

                    Some((prev_price.price + time_ratio * price_difference, accuracy))
                } else {
                    Some((next_price.price, accuracy))
                }
            },
            (Some(price), None) | (None, Some(price)) => {
                Some((price.price, (price.timestamp - time).abs()))
            },
            (None, None) => None,
        }
    }

    /// Get all price points.
    #[allow(dead_code)]
    pub fn all_points(&self) -> &[PricePoint] {
        &self.prices
    }
}

/// The main price history storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PriceHistory {
    currencies: HashMap<String, CurrencyPriceData>,
}

impl PriceHistory {
    pub fn new() -> Self {
        Self {
            currencies: HashMap::new(),
        }
    }

    /// Load price history from a directory.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let path = dir.join("price_history.bin");
        if !path.exists() {
            return Ok(Self::new());
        }

        let file = File::open(&path).context("Failed to open price history file")?;
        let reader = BufReader::new(file);
        let history: PriceHistory =
            ciborium::from_reader(reader).context("Failed to deserialize price history")?;

        Ok(history)
    }

    /// Save price history to a directory.
    pub fn save_to_dir(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).context("Failed to create price history directory")?;

        let path = dir.join("price_history.bin");
        let file = File::create(&path).context("Failed to create price history file")?;
        let writer = BufWriter::new(file);
        ciborium::into_writer(self, writer).context("Failed to serialize price history")?;

        Ok(())
    }

    /// Save price history to a directory as JSON file.
    #[allow(dead_code)]
    pub fn save_to_dir_as_json(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).context("Failed to create price history directory")?;

        let path = dir.join("price_history.json");
        let file = File::create(&path).context("Failed to create price history file")?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self).context("Failed to serialize price history")?;

        Ok(())
    }

    /// Get or create price data for a currency.
    pub fn price_data(&mut self, currency: String) -> &mut CurrencyPriceData {
        self.currencies.entry(currency).or_default()
    }

    /// Estimate the price of a currency at a given timestamp.
    pub fn estimate_price(&self, timestamp: NaiveDateTime, currency: &str) -> Option<Decimal> {
        match currency {
            "EUR" => Some(Decimal::ONE),
            _ => self
                .currencies
                .get(currency)
                .and_then(|data| data.estimate_price(timestamp))
                .map(|(price, _)| price),
        }
    }

    /// Estimate the price with accuracy information.
    #[allow(dead_code)]
    pub fn estimate_price_with_accuracy(
        &self,
        timestamp: NaiveDateTime,
        currency: &str,
    ) -> Option<(Decimal, Duration)> {
        match currency {
            "EUR" => Some((Decimal::ONE, Duration::zero())),
            _ => self
                .currencies
                .get(currency)
                .and_then(|data| data.estimate_price(timestamp)),
        }
    }

    /// Estimate the value of an amount at a given timestamp.
    pub fn estimate_value(&self, timestamp: NaiveDateTime, amount: &Amount) -> Option<Amount> {
        self.estimate_price(timestamp, &amount.currency)
            .map(|price| Amount::new(price * amount.quantity, "EUR".to_owned()))
    }

    /// Print debug info about the price history coverage.
    #[allow(dead_code)]
    pub fn debug_dump(&self) {
        eprintln!("=== PriceHistory Debug Dump ===");
        eprintln!("Currencies: {}", self.currencies.len());
        for (currency, data) in &self.currencies {
            eprintln!("  {}: {} price points", currency, data.prices.len());
        }
        eprintln!("=== End Debug Dump ===");
    }
}

/// Helper to collect price requirements from transactions.
#[derive(Debug, Default)]
pub(crate) struct PriceRequirements {
    requirements: HashMap<String, Vec<NaiveDateTime>>,
}

impl PriceRequirements {
    pub fn new() -> Self {
        Self {
            requirements: HashMap::new(),
        }
    }

    /// Add a requirement for a price at a given timestamp.
    pub fn add(&mut self, currency: &str, timestamp: NaiveDateTime) {
        if currency == "EUR" {
            return; // No price needed for base currency
        }
        self.requirements
            .entry(currency.to_owned())
            .or_default()
            .push(timestamp);
    }

    /// Get missing ranges given a price history.
    ///
    /// - `tolerance`: maximum acceptable accuracy for price estimation
    /// - `padding`: how close ranges need to be to merge them together
    pub fn missing_ranges(
        &self,
        price_history: &PriceHistory,
        tolerance: Duration,
        padding: Duration,
    ) -> HashMap<String, Vec<TimeRange>> {
        let mut result = HashMap::new();

        for (currency, timestamps) in &self.requirements {
            if currency == "EUR" {
                continue; // No price data needed for base currency
            }

            let missing = price_history
                .currencies
                .get(currency)
                .map(|data| data.missing_ranges_for_timestamps(timestamps, tolerance, padding))
                .unwrap_or_else(|| {
                    // No data for this currency, need ranges for all timestamps
                    let temp = CurrencyPriceData::new();
                    temp.missing_ranges_for_timestamps(timestamps, tolerance, padding)
                });

            if !missing.is_empty() {
                result.insert(currency.clone(), missing);
            }
        }

        result
    }
}

#[allow(dead_code)]
pub(crate) fn save_price_history_data(prices: &Vec<PricePoint>, path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    for price in prices {
        wtr.serialize(price)?;
    }

    Ok(())
}

#[allow(dead_code)]
pub(crate) fn load_price_history_data(path: &Path) -> Result<Vec<PricePoint>> {
    let mut rdr = csv::ReaderBuilder::new().from_path(path)?;

    let mut prices: Vec<PricePoint> = Vec::new();
    for result in rdr.deserialize() {
        let record: PricePoint = result?;
        prices.push(record);
    }

    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_datetime(year: i32, month: u32, day: u32, hour: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
    }

    fn make_price_points(start: NaiveDateTime, count: usize) -> Vec<PricePoint> {
        (0..count)
            .map(|i| PricePoint {
                timestamp: start + Duration::hours(i as i64),
                price: Decimal::from(100 + i as i64),
            })
            .collect()
    }

    #[test]
    fn test_time_range_overlaps() {
        let r1 = TimeRange::new(make_datetime(2024, 1, 1, 0), make_datetime(2024, 1, 1, 10));
        let r2 = TimeRange::new(make_datetime(2024, 1, 1, 5), make_datetime(2024, 1, 1, 15));
        let r3 = TimeRange::new(make_datetime(2024, 1, 1, 11), make_datetime(2024, 1, 1, 20));
        let r4 = TimeRange::new(make_datetime(2024, 1, 1, 12), make_datetime(2024, 1, 1, 20));

        assert!(r1.overlaps_or_adjacent(&r2, Duration::zero()));
        assert!(r1.overlaps_or_adjacent(&r3, Duration::hours(1)));
        assert!(!r1.overlaps_or_adjacent(&r4, Duration::zero()));
    }

    #[test]
    fn test_time_range_merge() {
        let r1 = TimeRange::new(make_datetime(2024, 1, 1, 0), make_datetime(2024, 1, 1, 10));
        let r2 = TimeRange::new(make_datetime(2024, 1, 1, 5), make_datetime(2024, 1, 1, 15));

        let merged = r1.merge(&r2);
        assert_eq!(merged.start, make_datetime(2024, 1, 1, 0));
        assert_eq!(merged.end, make_datetime(2024, 1, 1, 15));
    }

    #[test]
    fn test_add_points_maintains_sorted_order() {
        let mut data = CurrencyPriceData::new();

        // Add points out of order
        data.add_points(vec![
            PricePoint { timestamp: make_datetime(2024, 1, 1, 5), price: dec!(150) },
            PricePoint { timestamp: make_datetime(2024, 1, 1, 0), price: dec!(100) },
            PricePoint { timestamp: make_datetime(2024, 1, 1, 10), price: dec!(200) },
        ]);

        assert_eq!(data.prices.len(), 3);
        assert_eq!(data.prices[0].timestamp, make_datetime(2024, 1, 1, 0));
        assert_eq!(data.prices[1].timestamp, make_datetime(2024, 1, 1, 5));
        assert_eq!(data.prices[2].timestamp, make_datetime(2024, 1, 1, 10));
    }

    #[test]
    fn test_add_points_skips_duplicates() {
        let mut data = CurrencyPriceData::new();

        data.add_points(vec![
            PricePoint { timestamp: make_datetime(2024, 1, 1, 5), price: dec!(150) },
        ]);
        data.add_points(vec![
            PricePoint { timestamp: make_datetime(2024, 1, 1, 5), price: dec!(999) }, // duplicate
        ]);

        assert_eq!(data.prices.len(), 1);
        assert_eq!(data.prices[0].price, dec!(150)); // Original price kept
    }

    #[test]
    fn test_add_points_merges_overlapping() {
        let mut data = CurrencyPriceData::new();

        let start1 = make_datetime(2024, 1, 1, 0);
        let start2 = make_datetime(2024, 1, 1, 5);

        data.add_points(make_price_points(start1, 10)); // hours 0-9
        data.add_points(make_price_points(start2, 10)); // hours 5-14

        // Should have 15 unique points (0-14)
        assert_eq!(data.prices.len(), 15);
        assert_eq!(data.prices.first().unwrap().timestamp, make_datetime(2024, 1, 1, 0));
        assert_eq!(data.prices.last().unwrap().timestamp, make_datetime(2024, 1, 1, 14));
    }

    #[test]
    fn test_missing_ranges() {
        let mut data = CurrencyPriceData::new();

        let start = make_datetime(2024, 1, 1, 10);
        data.add_points(make_price_points(start, 10)); // hours 10-19

        let tolerance = Duration::hours(2);
        let padding = Duration::hours(1);

        // Request at hour 5 (before range with low accuracy) - should be missing
        let missing = data.missing_ranges_for_timestamps(
            &[make_datetime(2024, 1, 1, 5)],
            tolerance,
            padding,
        );
        assert!(!missing.is_empty());

        // Request at hour 15 (within range with good accuracy) - should not be missing
        let missing = data.missing_ranges_for_timestamps(
            &[make_datetime(2024, 1, 1, 15)],
            tolerance,
            padding,
        );
        assert!(missing.is_empty());
    }

    #[test]
    fn test_price_estimation() {
        let mut data = CurrencyPriceData::new();

        // Create contiguous hourly points where price increases by 10 each hour
        // hour 0 = 100, hour 1 = 110, hour 2 = 120, etc.
        let points: Vec<PricePoint> = (0..11)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64 * 10),
            })
            .collect();
        data.add_points(points);

        // Price at hour 5 should be exactly 150 (at a data point)
        let (price, _) = data.estimate_price(make_datetime(2024, 1, 1, 5)).unwrap();
        assert_eq!(price, dec!(150));

        // Price at hour 5:30 should be interpolated to 155 (halfway between 150 and 160)
        let (price, _) = data.estimate_price(
            make_datetime(2024, 1, 1, 5) + Duration::minutes(30)
        ).unwrap();
        assert_eq!(price, dec!(155));
    }

    #[test]
    fn test_price_estimation_edge_cases() {
        let mut data = CurrencyPriceData::new();

        // Create a range from hour 10 to hour 19 (10 points)
        // hour 10 = 100, hour 11 = 110, ..., hour 19 = 190
        let points: Vec<PricePoint> = (0..10)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, 10 + i),
                price: Decimal::from(100 + i as i64 * 10),
            })
            .collect();
        data.add_points(points);

        // Test: estimate before all points (hour 5)
        // Should return the first known price (100) with accuracy = 5 hours
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 5)).unwrap();
        assert_eq!(price, dec!(100), "Before range: should use first price");
        assert_eq!(accuracy, Duration::hours(5), "Before range: accuracy should be distance to first point");

        // Test: estimate after all points (hour 22)
        // Should return the last known price (190) with accuracy = 3 hours
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 22)).unwrap();
        assert_eq!(price, dec!(190), "After range: should use last price");
        assert_eq!(accuracy, Duration::hours(3), "After range: accuracy should be distance to last point");

        // Test: estimate exactly at beginning of range (hour 10)
        // Should return exactly 100 with accuracy = 0
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 10)).unwrap();
        assert_eq!(price, dec!(100), "At start: should return exact price");
        assert_eq!(accuracy, Duration::zero(), "At start: accuracy should be zero");

        // Test: estimate exactly at end of range (hour 19)
        // Should return exactly 190 with accuracy = 0
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 19)).unwrap();
        assert_eq!(price, dec!(190), "At end: should return exact price");
        assert_eq!(accuracy, Duration::zero(), "At end: accuracy should be zero");

        // Test: estimate within range (hour 14:30)
        // Should interpolate between hour 14 (140) and hour 15 (150) -> 145
        let (price, accuracy) = data.estimate_price(
            make_datetime(2024, 1, 1, 14) + Duration::minutes(30)
        ).unwrap();
        assert_eq!(price, dec!(145), "Within range: should interpolate");
        assert_eq!(accuracy, Duration::minutes(30), "Within range: accuracy should be distance to nearest point");
    }

    #[test]
    fn test_price_estimation_gap_between_points() {
        let mut data = CurrencyPriceData::new();

        // Create two sets of points with a gap:
        // Points at hours 0-4, prices 100-140
        // Gap: hours 5-9
        // Points at hours 10-14, prices 200-240
        let points1: Vec<PricePoint> = (0..5)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64 * 10),
            })
            .collect();
        data.add_points(points1);

        let points2: Vec<PricePoint> = (0..5)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, 10 + i),
                price: Decimal::from(200 + i as i64 * 10),
            })
            .collect();
        data.add_points(points2);

        assert_eq!(data.prices.len(), 10);

        // Test: estimate in the gap (hour 7)
        // Should interpolate between hour 4 (140) and hour 10 (200)
        // Gap is 6 hours, hour 7 is 3 hours in -> halfway -> (140 + 200) / 2 = 170
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 7)).unwrap();
        assert_eq!(price, dec!(170), "In gap: should interpolate between points");
        assert_eq!(accuracy, Duration::hours(3), "In gap: accuracy should be distance to nearest point");
    }

    #[test]
    fn test_price_estimation_empty() {
        let data = CurrencyPriceData::new();

        // No points at all - should return None
        let result = data.estimate_price(make_datetime(2024, 1, 1, 12));
        assert!(result.is_none(), "Empty data: should return None");
    }

    #[test]
    fn test_price_history_missing_ranges() {
        let mut history = PriceHistory::new();

        let start = make_datetime(2024, 1, 1, 10);
        history.price_data("ETH".to_owned()).add_points(make_price_points(start, 10));

        let mut requirements = PriceRequirements::new();
        requirements.add("ETH", make_datetime(2024, 1, 1, 5));  // Before range
        requirements.add("ETH", make_datetime(2024, 1, 1, 15)); // Within range
        requirements.add("ETH", make_datetime(2024, 1, 2, 5));  // After range (next day)
        requirements.add("BNB", make_datetime(2024, 1, 1, 12)); // No data for this currency

        let tolerance = Duration::hours(2);
        let padding = Duration::hours(1);
        let missing = requirements.missing_ranges(&history, tolerance, padding);

        // ETH should have missing ranges for timestamps outside coverage
        assert!(missing.contains_key("ETH"));
        assert!(!missing["ETH"].is_empty());

        // BNB should have missing range (no data at all)
        assert!(missing.contains_key("BNB"));
    }
}
