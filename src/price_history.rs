//! Price history module with range-based storage and efficient merging.
//!
//! This module provides a `PriceHistory` struct that stores price data in ranges,
//! allowing for efficient storage and retrieval of price points. Ranges are
//! automatically merged when they overlap or connect without gaps.

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
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
pub(crate) struct PricePoint {
    pub timestamp: NaiveDateTime,
    pub price: Decimal,
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
    /// Adjacent means the ranges connect without a gap (within the tolerance).
    pub fn overlaps_or_adjacent(&self, other: &TimeRange, tolerance: Duration) -> bool {
        // Ranges overlap or are adjacent if one starts before or at the point
        // where the other ends (plus tolerance)
        self.start <= other.end + tolerance && other.start <= self.end + tolerance
    }

    /// Check if this range fully contains a timestamp.
    pub fn contains(&self, timestamp: NaiveDateTime) -> bool {
        self.start <= timestamp && timestamp <= self.end
    }

    /// Merge this range with another, returning the combined range.
    pub fn merge(&self, other: &TimeRange) -> TimeRange {
        TimeRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Subtract another range from this one, returning the remaining parts.
    /// Returns 0, 1, or 2 ranges depending on the overlap.
    pub fn subtract(&self, other: &TimeRange) -> Vec<TimeRange> {
        // No overlap - return self unchanged
        if other.end < self.start || other.start > self.end {
            return vec![*self];
        }

        let mut result = Vec::new();

        // Left remainder (part of self before other starts)
        if self.start < other.start {
            result.push(TimeRange::new(self.start, other.start));
        }

        // Right remainder (part of self after other ends)
        if self.end > other.end {
            result.push(TimeRange::new(other.end, self.end));
        }

        result
    }
}

/// The default interval between price points in seconds (hourly data from CoinMarketCap).
const DEFAULT_INTERVAL_SECS: i64 = 3600;

/// A contiguous range of price data for a single currency.
///
/// Stores prices efficiently by only keeping the start time and interval,
/// with timestamps derived from index position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PriceRange {
    /// The interval between consecutive price points in seconds.
    interval_secs: i64,
    /// The start time of this price range.
    start: NaiveDateTime,
    /// The price values, where prices[i] corresponds to start + interval * i.
    prices: Vec<Decimal>,
}

impl PriceRange {
    /// Create a new price range with the default interval (1 hour).
    #[allow(dead_code)]
    pub fn new(start: NaiveDateTime, prices: Vec<Decimal>) -> Self {
        Self {
            interval_secs: DEFAULT_INTERVAL_SECS,
            start,
            prices,
        }
    }

    /// Create a new price range with a custom interval.
    #[allow(dead_code)]
    pub fn with_interval(interval: Duration, start: NaiveDateTime, prices: Vec<Decimal>) -> Self {
        Self {
            interval_secs: interval.num_seconds(),
            start,
            prices,
        }
    }

    /// Create a price range from a vector of price points.
    ///
    /// The interval is determined from the time between the first two points.
    /// All points must be evenly spaced at this interval, otherwise returns an error
    /// describing where the gap was detected.
    ///
    /// Points are sorted by timestamp before processing.
    pub fn from_points(mut points: Vec<PricePoint>) -> Result<Self, String> {
        if points.is_empty() {
            return Err("Cannot create PriceRange from empty points".to_string());
        }

        if points.len() == 1 {
            // Single point: use default interval
            return Ok(Self {
                interval_secs: DEFAULT_INTERVAL_SECS,
                start: points[0].timestamp,
                prices: vec![points[0].price],
            });
        }

        points.sort();

        // Determine interval from first two points
        let interval_secs = (points[1].timestamp - points[0].timestamp).num_seconds();
        if interval_secs <= 0 {
            return Err(format!(
                "Invalid interval: points at {} and {} have non-positive time difference",
                points[0].timestamp, points[1].timestamp
            ));
        }

        let start = points[0].timestamp;
        let mut prices = Vec::with_capacity(points.len());
        let mut expected_time = start;

        for point in points {
            if point.timestamp != expected_time {
                return Err(format!(
                    "Gap detected: expected point at {}, found point at {}",
                    expected_time, point.timestamp
                ));
            }
            prices.push(point.price);
            expected_time = expected_time + Duration::seconds(interval_secs);
        }

        Ok(Self {
            interval_secs,
            start,
            prices,
        })
    }

    /// Check if this range is empty.
    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// Get the interval as a Duration.
    pub fn interval(&self) -> Duration {
        Duration::seconds(self.interval_secs)
    }

    /// Get the interval in seconds.
    #[allow(dead_code)]
    pub fn interval_secs(&self) -> i64 {
        self.interval_secs
    }

    /// Get the start timestamp.
    #[allow(dead_code)]
    pub fn start(&self) -> NaiveDateTime {
        self.start
    }

    /// Get the timestamp for a given index.
    pub fn timestamp_at(&self, index: usize) -> NaiveDateTime {
        self.start + Duration::seconds(self.interval_secs * index as i64)
    }

    /// Get the time range covered by this price range.
    ///
    /// The range covers exactly from the first data point to the last data point.
    /// Interpolation is possible for any timestamp between these two points.
    pub fn time_range(&self) -> TimeRange {
        let end = self.start + Duration::seconds(self.interval_secs * (self.prices.len().saturating_sub(1)) as i64);
        TimeRange::new(self.start, end)
    }

    /// Get all price points as (timestamp, price) pairs.
    pub fn points(&self) -> impl Iterator<Item = PricePoint> + '_ {
        self.prices.iter().enumerate().map(|(i, &price)| PricePoint {
            timestamp: self.timestamp_at(i),
            price,
        })
    }

    /// Get the number of price points in this range.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    /// Get the first price point in this range.
    pub fn first_point(&self) -> Option<PricePoint> {
        self.prices.first().map(|&price| PricePoint {
            timestamp: self.start,
            price,
        })
    }

    /// Get the last price point at or before the given timestamp.
    /// Returns None if the timestamp is before the start of this range.
    pub fn last_point_at_or_before(&self, timestamp: NaiveDateTime) -> Option<PricePoint> {
        if timestamp < self.start {
            return None;
        }
        let elapsed_secs = (timestamp - self.start).num_seconds();
        let index = (elapsed_secs / self.interval_secs) as usize;
        let index = index.min(self.prices.len() - 1);
        Some(PricePoint {
            timestamp: self.timestamp_at(index),
            price: self.prices[index],
        })
    }

    /// Get the first price point at or after the given timestamp.
    /// Returns None if the timestamp is after the end of this range.
    pub fn first_point_at_or_after(&self, timestamp: NaiveDateTime) -> Option<PricePoint> {
        let time_range = self.time_range();
        if timestamp > time_range.end {
            return None;
        }
        if timestamp <= self.start {
            return self.first_point();
        }
        let elapsed_secs = (timestamp - self.start).num_seconds();
        // Round up to get the next point
        let index = ((elapsed_secs + self.interval_secs - 1) / self.interval_secs) as usize;
        if index >= self.prices.len() {
            return None;
        }
        Some(PricePoint {
            timestamp: self.timestamp_at(index),
            price: self.prices[index],
        })
    }
}

/// Price data for a single currency, stored as multiple ranges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CurrencyPriceData {
    /// Sorted list of non-overlapping price ranges.
    ranges: Vec<PriceRange>,
}

impl Default for CurrencyPriceData {
    fn default() -> Self {
        Self {
            ranges: Vec::new(),
        }
    }
}

impl CurrencyPriceData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge two price ranges into one.
    fn merge_ranges(&self, range: &mut PriceRange, other: PriceRange) {
        let self_range = range.time_range();
        let other_range = other.time_range();
        let interval_secs = range.interval_secs;

        // Determine the new combined range
        let new_start = range.start.min(other.start);
        let new_end = self_range.end.max(other_range.end);

        // Calculate new size
        let new_duration = new_end - new_start;
        let new_len = (new_duration.num_seconds() / interval_secs) as usize + 1;

        // Create new prices vector
        let mut new_prices = vec![Decimal::ZERO; new_len];

        // Copy prices from self
        let self_offset = ((range.start - new_start).num_seconds() / interval_secs) as usize;
        for (i, &price) in range.prices.iter().enumerate() {
            new_prices[self_offset + i] = price;
        }

        // Copy/overwrite prices from other (other takes precedence for overlaps)
        let other_offset = ((other.start - new_start).num_seconds() / interval_secs) as usize;
        for (i, price) in other.prices.into_iter().enumerate() {
            new_prices[other_offset + i] = price;
        }

        range.start = new_start;
        range.prices = new_prices;
    }

    /// Add a new price range, merging with existing ranges if they overlap or connect.
    pub fn add_range(&mut self, new_range: PriceRange) {
        if new_range.is_empty() {
            return;
        }

        // Tolerance for considering ranges as adjacent (same as the range's interval)
        let tolerance = new_range.interval();
        let new_time_range = new_range.time_range();

        // Find all ranges that overlap or are adjacent to the new range
        let mut to_merge: Vec<usize> = Vec::new();
        for (i, existing) in self.ranges.iter().enumerate() {
            if existing.time_range().overlaps_or_adjacent(&new_time_range, tolerance) {
                to_merge.push(i);
            }
        }

        if to_merge.is_empty() {
            // No overlapping ranges, just insert in sorted order
            let insert_pos = self
                .ranges
                .iter()
                .position(|r| r.start > new_range.start)
                .unwrap_or(self.ranges.len());
            self.ranges.insert(insert_pos, new_range);
        } else {
            // Merge all overlapping ranges into one
            let mut merged = new_range;

            // Remove ranges in reverse order to maintain indices
            for &i in to_merge.iter().rev() {
                let removed = self.ranges.remove(i);
                self.merge_ranges(&mut merged, removed);
            }

            // Insert the merged range in sorted order
            let insert_pos = self
                .ranges
                .iter()
                .position(|r| r.start > merged.start)
                .unwrap_or(self.ranges.len());
            self.ranges.insert(insert_pos, merged);
        }
    }

    /// Add price points, creating a range and merging as needed.
    ///
    /// The points must be evenly spaced (interval is determined from the data).
    /// Returns an error if the points have gaps or inconsistent spacing.
    pub fn add_points(&mut self, points: Vec<PricePoint>) -> Result<(), String> {
        if points.is_empty() {
            return Ok(());
        }
        let new_range = PriceRange::from_points(points)?;
        self.add_range(new_range);
        Ok(())
    }

    /// Find the range needed to cover a specific timestamp.
    /// Returns None if the timestamp is already covered.
    pub fn missing_range_for_timestamp(
        &self,
        timestamp: NaiveDateTime,
        padding: Duration,
    ) -> Option<TimeRange> {
        // Check if any existing range covers this timestamp
        for range in &self.ranges {
            if range.time_range().contains(timestamp) {
                return None;
            }
        }

        // Return a range centered on the timestamp with padding
        Some(TimeRange::new(timestamp - padding, timestamp + padding))
    }

    /// Get the ranges needed to cover multiple timestamps.
    /// Returns a list of missing ranges, merged where possible.
    /// The returned ranges are trimmed to exclude any parts already covered by existing data.
    pub fn missing_ranges_for_timestamps(
        &self,
        timestamps: &[NaiveDateTime],
        padding: Duration,
    ) -> Vec<TimeRange> {
        let mut missing: Vec<TimeRange> = Vec::new();
        // Use the interval from the first range, or default to 1 hour if no ranges exist
        let tolerance = self.ranges.first()
            .map(|r| r.interval())
            .unwrap_or_else(|| Duration::seconds(DEFAULT_INTERVAL_SECS));

        for &ts in timestamps {
            if let Some(needed) = self.missing_range_for_timestamp(ts, padding) {
                // Try to merge with existing missing ranges
                let mut merged = false;
                for existing in &mut missing {
                    if existing.overlaps_or_adjacent(&needed, tolerance) {
                        *existing = existing.merge(&needed);
                        merged = true;
                        break;
                    }
                }
                if !merged {
                    missing.push(needed);
                }
            }
        }

        // Sort and merge any overlapping ranges
        missing.sort_by_key(|r| r.start);
        let mut merged: Vec<TimeRange> = Vec::new();
        for range in missing {
            if let Some(last) = merged.last_mut() {
                if last.overlaps_or_adjacent(&range, tolerance) {
                    *last = last.merge(&range);
                    continue;
                }
            }
            merged.push(range);
        }

        // Subtract already-covered ranges from the missing ranges
        let covered: Vec<TimeRange> = self.ranges.iter().map(|r| r.time_range()).collect();
        let mut result: Vec<TimeRange> = Vec::new();

        for missing_range in merged {
            // Subtract each covered range from this missing range
            let mut remaining = vec![missing_range];
            for covered_range in &covered {
                remaining = remaining
                    .into_iter()
                    .flat_map(|r| r.subtract(covered_range))
                    .collect();
            }
            result.extend(remaining);
        }

        // Final sort
        result.sort_by_key(|r| r.start);
        result
    }

    /// Estimate the price at a given timestamp using linear interpolation.
    ///
    /// Finds the nearest price points before and after the timestamp and
    /// performs linear interpolation. If only one side is available,
    /// uses that price with appropriate accuracy.
    pub fn estimate_price(&self, timestamp: NaiveDateTime) -> Option<(Decimal, Duration)> {
        if self.ranges.is_empty() {
            return None;
        }

        // Use partition_point to find the first range that starts after the timestamp
        let index = self.ranges.partition_point(|r| r.start() <= timestamp);

        // The previous range (if any) might contain or precede the timestamp
        // The next range follows the timestamp
        let prev_range = if index > 0 { self.ranges.get(index - 1) } else { None };
        let next_range = self.ranges.get(index);

        let prev_price_point = prev_range
            .and_then(|r| r.last_point_at_or_before(timestamp));

        let next_price_point = prev_range
            .and_then(|r| r.first_point_at_or_after(timestamp))
            .or_else(|| next_range.and_then(|r| r.first_point()));

        match (prev_price_point, next_price_point) {
            (Some(prev_price), Some(next_price)) => {
                let price_difference = next_price.price - prev_price.price;
                let total_duration: Decimal = (next_price.timestamp - prev_price.timestamp).num_seconds().into();

                // The accuracy is the minimum time difference between the requested time and a price point
                let accuracy = (timestamp - prev_price.timestamp).abs().min((next_price.timestamp - timestamp).abs());

                if total_duration > Decimal::ZERO {
                    let time_since_prev: Decimal = (timestamp - prev_price.timestamp).num_seconds().into();
                    let time_ratio = time_since_prev / total_duration;

                    Some((prev_price.price + time_ratio * price_difference, accuracy))
                } else {
                    Some((next_price.price, accuracy))
                }
            },
            (Some(price), None) |
            (None, Some(price)) => {
                Some((price.price, (price.timestamp - timestamp).abs()))
            },
            (None, None) => None
        }
    }

    /// Get all price points across all ranges.
    #[allow(dead_code)]
    pub fn all_points(&self) -> Vec<PricePoint> {
        self.ranges
            .iter()
            .flat_map(|r| r.points())
            .collect()
    }

    /// Get the covered time ranges.
    #[allow(dead_code)]
    pub fn covered_ranges(&self) -> Vec<TimeRange> {
        self.ranges.iter().map(|r| r.time_range()).collect()
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

    /// Add price points for a currency.
    pub fn price_data(&mut self, currency: String) -> &mut CurrencyPriceData {
        self.currencies
            .entry(currency)
            .or_default()
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

    /// Get the missing price ranges needed for a set of timestamps per currency.
    /// The padding is the amount of time to request before/after each timestamp.
    pub fn missing_ranges(
        &self,
        requirements: &HashMap<String, Vec<NaiveDateTime>>,
        padding: Duration,
    ) -> HashMap<String, Vec<TimeRange>> {
        let mut result = HashMap::new();

        for (currency, timestamps) in requirements {
            if currency == "EUR" {
                continue; // No price data needed for base currency
            }

            let missing = self
                .currencies
                .get(currency)
                .map(|data| data.missing_ranges_for_timestamps(timestamps, padding))
                .unwrap_or_else(|| {
                    // No data for this currency, need ranges for all timestamps
                    let temp = CurrencyPriceData::new();
                    temp.missing_ranges_for_timestamps(timestamps, padding)
                });

            if !missing.is_empty() {
                result.insert(currency.clone(), missing);
            }
        }

        result
    }

    /// Get covered ranges for a currency.
    #[allow(dead_code)]
    pub fn covered_ranges(&self, currency: &str) -> Vec<TimeRange> {
        self.currencies
            .get(currency)
            .map(|d| d.covered_ranges())
            .unwrap_or_default()
    }

    /// Print debug info about the price history coverage.
    #[allow(dead_code)]
    pub fn debug_dump(&self) {
        eprintln!("=== PriceHistory Debug Dump ===");
        eprintln!("Currencies: {}", self.currencies.len());
        for (currency, data) in &self.currencies {
            eprintln!("  {}: {} ranges", currency, data.ranges.len());
            for (i, range) in data.ranges.iter().enumerate() {
                let time_range = range.time_range();
                eprintln!("    Range {}: {} to {} ({} points, interval={}s)",
                    i, time_range.start, time_range.end, range.len(), range.interval_secs());
            }
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
    pub fn missing_ranges(
        &self,
        price_history: &PriceHistory,
        padding: Duration,
    ) -> HashMap<String, Vec<TimeRange>> {
        price_history.missing_ranges(&self.requirements, padding)
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
    let mut rdr = csv::ReaderBuilder::new()
        .from_path(path)?;

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
    fn test_currency_data_merge_overlapping() {
        let mut data = CurrencyPriceData::new();

        let start1 = make_datetime(2024, 1, 1, 0);
        let start2 = make_datetime(2024, 1, 1, 5);

        data.add_points(make_price_points(start1, 10)).unwrap(); // hours 0-9
        data.add_points(make_price_points(start2, 10)).unwrap(); // hours 5-14

        assert_eq!(data.ranges.len(), 1);
        let time_range = data.ranges[0].time_range();
        assert_eq!(time_range.start, make_datetime(2024, 1, 1, 0));
        assert_eq!(time_range.end, make_datetime(2024, 1, 1, 14));
    }

    #[test]
    fn test_currency_data_merge_adjacent() {
        let mut data = CurrencyPriceData::new();

        let start1 = make_datetime(2024, 1, 1, 0);
        let start2 = make_datetime(2024, 1, 1, 10);

        data.add_points(make_price_points(start1, 10)).unwrap(); // hours 0-9
        data.add_points(make_price_points(start2, 10)).unwrap(); // hours 10-19

        // Should merge because they're adjacent (within 1 hour tolerance)
        assert_eq!(data.ranges.len(), 1);
    }

    #[test]
    fn test_currency_data_separate_ranges() {
        let mut data = CurrencyPriceData::new();

        let start1 = make_datetime(2024, 1, 1, 0);
        let start2 = make_datetime(2024, 1, 2, 0); // Next day

        data.add_points(make_price_points(start1, 10)).unwrap(); // hours 0-9 on day 1
        data.add_points(make_price_points(start2, 10)).unwrap(); // hours 0-9 on day 2

        // Should not merge because there's a gap
        assert_eq!(data.ranges.len(), 2);
    }

    #[test]
    fn test_missing_ranges() {
        let mut data = CurrencyPriceData::new();

        let start = make_datetime(2024, 1, 1, 10);
        data.add_points(make_price_points(start, 10)).unwrap(); // hours 10-19

        // Request at hour 5 (before range) - should be missing
        let missing = data.missing_range_for_timestamp(make_datetime(2024, 1, 1, 5), Duration::hours(2));
        assert!(missing.is_some());

        // Request at hour 15 (within range) - should not be missing
        let missing = data.missing_range_for_timestamp(make_datetime(2024, 1, 1, 15), Duration::hours(2));
        assert!(missing.is_none());
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
        data.add_points(points).unwrap();

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
        data.add_points(points).unwrap();

        // Test: estimate before all ranges (hour 5)
        // Should return the first known price (100) with accuracy = 5 hours
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 5)).unwrap();
        assert_eq!(price, dec!(100), "Before range: should use first price");
        assert_eq!(accuracy, Duration::hours(5), "Before range: accuracy should be distance to first point");

        // Test: estimate after all ranges (hour 22)
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
    fn test_price_estimation_gap_between_ranges() {
        let mut data = CurrencyPriceData::new();

        // Create two ranges with a gap:
        // Range 1: hours 0-4, prices 100-140
        // Gap: hours 5-9
        // Range 2: hours 10-14, prices 200-240
        let points1: Vec<PricePoint> = (0..5)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64 * 10),
            })
            .collect();
        data.add_points(points1).unwrap();

        let points2: Vec<PricePoint> = (0..5)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, 10 + i),
                price: Decimal::from(200 + i as i64 * 10),
            })
            .collect();
        data.add_points(points2).unwrap();

        assert_eq!(data.ranges.len(), 2, "Should have two separate ranges");

        // Test: estimate in the gap (hour 7)
        // Should interpolate between hour 4 (140) and hour 10 (200)
        // Gap is 6 hours, hour 7 is 3 hours in -> halfway -> (140 + 200) / 2 = 170
        let (price, accuracy) = data.estimate_price(make_datetime(2024, 1, 1, 7)).unwrap();
        assert_eq!(price, dec!(170), "In gap: should interpolate between ranges");
        assert_eq!(accuracy, Duration::hours(3), "In gap: accuracy should be distance to nearest point");
    }

    #[test]
    fn test_price_estimation_empty() {
        let data = CurrencyPriceData::new();

        // No ranges at all - should return None
        let result = data.estimate_price(make_datetime(2024, 1, 1, 12));
        assert!(result.is_none(), "Empty data: should return None");
    }

    #[test]
    fn test_price_history_missing_ranges() {
        let mut history = PriceHistory::new();

        let start = make_datetime(2024, 1, 1, 10);
        history.price_data("ETH".to_owned()).add_points(make_price_points(start, 10)).unwrap();

        let mut requirements = HashMap::new();
        requirements.insert(
            "ETH".to_owned(),
            vec![
                make_datetime(2024, 1, 1, 5),  // Before range
                make_datetime(2024, 1, 1, 15), // Within range
                make_datetime(2024, 1, 2, 5),  // After range (next day)
            ],
        );
        requirements.insert(
            "BNB".to_owned(), // No data for this currency
            vec![make_datetime(2024, 1, 1, 12)],
        );

        let missing = history.missing_ranges(&requirements, Duration::hours(2));

        // ETH should have missing ranges for timestamps outside coverage
        assert!(missing.contains_key("ETH"));
        assert!(!missing["ETH"].is_empty());

        // BNB should have missing range (no data at all)
        assert!(missing.contains_key("BNB"));
    }

    #[test]
    fn test_time_range_subtract() {
        let range = TimeRange::new(
            make_datetime(2024, 1, 1, 10),
            make_datetime(2024, 1, 1, 20),
        );

        // No overlap - subtract range before
        let before = TimeRange::new(make_datetime(2024, 1, 1, 0), make_datetime(2024, 1, 1, 5));
        let result = range.subtract(&before);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], range);

        // No overlap - subtract range after
        let after = TimeRange::new(make_datetime(2024, 1, 1, 22), make_datetime(2024, 1, 1, 23));
        let result = range.subtract(&after);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], range);

        // Complete overlap - subtract range that covers entire range
        let covers = TimeRange::new(make_datetime(2024, 1, 1, 5), make_datetime(2024, 1, 1, 22));
        let result = range.subtract(&covers);
        assert_eq!(result.len(), 0);

        // Partial overlap at start - subtract range that overlaps beginning
        let start_overlap = TimeRange::new(make_datetime(2024, 1, 1, 5), make_datetime(2024, 1, 1, 15));
        let result = range.subtract(&start_overlap);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, make_datetime(2024, 1, 1, 15));
        assert_eq!(result[0].end, make_datetime(2024, 1, 1, 20));

        // Partial overlap at end - subtract range that overlaps end
        let end_overlap = TimeRange::new(make_datetime(2024, 1, 1, 15), make_datetime(2024, 1, 1, 22));
        let result = range.subtract(&end_overlap);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, make_datetime(2024, 1, 1, 10));
        assert_eq!(result[0].end, make_datetime(2024, 1, 1, 15));

        // Middle overlap - subtract range in the middle, creating two pieces
        let middle = TimeRange::new(make_datetime(2024, 1, 1, 13), make_datetime(2024, 1, 1, 17));
        let result = range.subtract(&middle);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start, make_datetime(2024, 1, 1, 10));
        assert_eq!(result[0].end, make_datetime(2024, 1, 1, 13));
        assert_eq!(result[1].start, make_datetime(2024, 1, 1, 17));
        assert_eq!(result[1].end, make_datetime(2024, 1, 1, 20));
    }

    #[test]
    fn test_missing_ranges_trimmed_by_existing_coverage() {
        let mut data = CurrencyPriceData::new();

        // Add two ranges with a gap between them:
        // Range 1: hours 0-9
        // Range 2: hours 20-29
        // Gap: hours 10-19
        let start1 = make_datetime(2024, 1, 1, 0);
        let start2 = make_datetime(2024, 1, 1, 20);
        data.add_points(make_price_points(start1, 10)).unwrap(); // hours 0-9
        data.add_points(make_price_points(start2, 10)).unwrap(); // hours 20-29

        assert_eq!(data.ranges.len(), 2);

        // Request a timestamp in the gap with large padding that would overlap both ranges
        let timestamps = vec![make_datetime(2024, 1, 1, 15)]; // In the gap
        let padding = Duration::hours(10); // Would create range 05:00 to 25:00

        let missing = data.missing_ranges_for_timestamps(&timestamps, padding);

        // The missing range should be trimmed to just the actual gap
        assert_eq!(missing.len(), 1);
        // Should start after range 1 ends (hour 9) and end before range 2 starts (hour 20)
        assert_eq!(missing[0].start, make_datetime(2024, 1, 1, 9));
        assert_eq!(missing[0].end, make_datetime(2024, 1, 1, 20));
    }

    #[test]
    fn test_from_points_detects_gaps() {
        // Create points with a gap in the middle
        let mut points = Vec::new();

        // First range: hours 0-4
        for i in 0..5 {
            points.push(PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64),
            });
        }

        // Gap: hours 5-9 missing

        // Second range: hours 10-14
        for i in 10..15 {
            points.push(PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(200 + i as i64),
            });
        }

        // from_points should detect the gap and return an error
        let result = PriceRange::from_points(points);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Gap detected"));
    }

    #[test]
    fn test_from_points_with_custom_interval() {
        // Create points with 2-hour interval
        let points: Vec<PricePoint> = (0..5)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, 0) + Duration::hours(i * 2),
                price: Decimal::from(100 + i),
            })
            .collect();

        let range = PriceRange::from_points(points).unwrap();

        assert_eq!(range.len(), 5);
        assert_eq!(range.interval(), Duration::hours(2));
        assert_eq!(range.start(), make_datetime(2024, 1, 1, 0));
    }

    #[test]
    fn test_from_points_single_point() {
        let points = vec![PricePoint {
            timestamp: make_datetime(2024, 1, 1, 5),
            price: dec!(123),
        }];

        let range = PriceRange::from_points(points).unwrap();

        assert_eq!(range.len(), 1);
        assert_eq!(range.start(), make_datetime(2024, 1, 1, 5));
    }

    #[test]
    fn test_from_points_empty() {
        let points: Vec<PricePoint> = Vec::new();
        let result = PriceRange::from_points(points);
        assert!(result.is_err());
    }
}
