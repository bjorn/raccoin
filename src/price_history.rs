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

    /// Find the intersection of this range with another.
    /// Returns None if the ranges don't overlap.
    pub fn intersect(&self, other: &TimeRange) -> Option<TimeRange> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end {
            Some(TimeRange::new(start, end))
        } else {
            None
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
        let duration = Duration::seconds(self.interval_secs * (self.prices.len().saturating_sub(1)) as i64);
        TimeRange::new(self.start, self.start + duration)
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

    /// Extract a slice of this range covering the given time range.
    ///
    /// The slice will include all points that fall within the given time range.
    /// Returns None if there's no overlap or the overlap contains no points.
    pub fn slice(&self, time_range: &TimeRange) -> Option<PriceRange> {
        let self_range = self.time_range();

        // Find intersection
        let intersection = self_range.intersect(time_range)?;

        // Find the first index at or after intersection.start
        let start_elapsed = (intersection.start - self.start).num_seconds();
        let start_idx = if start_elapsed <= 0 {
            0
        } else {
            // Round up to get first point within intersection
            ((start_elapsed + self.interval_secs - 1) / self.interval_secs) as usize
        };

        // Find the last index at or before intersection.end
        let end_elapsed = (intersection.end - self.start).num_seconds();
        let end_idx = (end_elapsed / self.interval_secs) as usize;

        if start_idx >= self.prices.len() || end_idx < start_idx {
            return None;
        }

        let end_idx = end_idx.min(self.prices.len() - 1);

        let new_start = self.timestamp_at(start_idx);
        let new_prices = self.prices[start_idx..=end_idx].to_vec();

        if new_prices.is_empty() {
            return None;
        }

        Some(PriceRange {
            interval_secs: self.interval_secs,
            start: new_start,
            prices: new_prices,
        })
    }

    /// Subtract a time range from this price range.
    ///
    /// Returns 0, 1, or 2 price ranges depending on the overlap:
    /// - 0 ranges if this range is fully covered by the subtracted range
    /// - 1 range if the subtracted range overlaps at the start or end
    /// - 2 ranges if the subtracted range is in the middle (splits this range)
    pub fn subtract(&self, time_range: &TimeRange) -> Vec<PriceRange> {
        let self_range = self.time_range();

        // No overlap - return self unchanged
        if time_range.end < self_range.start || time_range.start > self_range.end {
            return vec![self.clone()];
        }

        let mut result = Vec::new();

        // Left remainder (part before time_range starts)
        if self_range.start < time_range.start {
            let left_range = TimeRange::new(self_range.start, time_range.start - Duration::seconds(1));
            if let Some(left) = self.slice(&left_range) {
                result.push(left);
            }
        }

        // Right remainder (part after time_range ends)
        if self_range.end > time_range.end {
            let right_range = TimeRange::new(time_range.end + Duration::seconds(1), self_range.end);
            if let Some(right) = self.slice(&right_range) {
                result.push(right);
            }
        }

        result
    }

    /// Check if this range can be merged with another range.
    ///
    /// Ranges can be merged if they have the same interval and are adjacent
    /// (the end of one is exactly one interval before the start of the other).
    pub fn can_merge_with(&self, other: &PriceRange) -> bool {
        if self.interval_secs != other.interval_secs {
            return false;
        }

        let self_range = self.time_range();
        let other_range = other.time_range();
        let interval = Duration::seconds(self.interval_secs);

        // Check if other starts exactly one interval after self ends
        // or self starts exactly one interval after other ends
        self_range.end + interval == other_range.start ||
        other_range.end + interval == self_range.start
    }

    /// Merge this range with another adjacent range.
    ///
    /// Assumes can_merge_with() returned true. The ranges must have the same
    /// interval and be adjacent.
    pub fn merge_with(&self, other: &PriceRange) -> PriceRange {
        debug_assert!(self.can_merge_with(other), "Ranges must be mergeable");

        // Determine which range comes first
        let (first, second) = if self.start < other.start {
            (self, other)
        } else {
            (other, self)
        };

        let mut prices = first.prices.clone();
        prices.extend(second.prices.iter().copied());

        PriceRange {
            interval_secs: self.interval_secs,
            start: first.start,
            prices,
        }
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

    /// Add a new price range, respecting interval-based priority.
    ///
    /// Ranges with smaller intervals (higher resolution) take priority over
    /// ranges with larger intervals. When intervals are equal, existing data
    /// is preserved.
    ///
    /// The new range may be split to fill gaps between existing higher-priority ranges.
    pub fn add_range(&mut self, new_range: PriceRange) {
        if new_range.is_empty() {
            return;
        }

        let new_time_range = new_range.time_range();
        let new_interval = new_range.interval_secs;

        // Step 1: Find gaps where the new range should be inserted
        // (subtract all existing ranges with equal or smaller intervals)
        let mut gaps = vec![new_time_range];

        for existing in &self.ranges {
            if existing.interval_secs <= new_interval {
                // This existing range has equal or better resolution - subtract it
                // Expand the existing range by one interval on each side to ensure
                // we don't create overlapping points at boundaries
                let existing_time_range = existing.time_range();
                let expanded = TimeRange::new(
                    existing_time_range.start - Duration::seconds(existing.interval_secs - 1),
                    existing_time_range.end + Duration::seconds(existing.interval_secs - 1),
                );
                gaps = gaps
                    .into_iter()
                    .flat_map(|gap| gap.subtract(&expanded))
                    .collect();
            }
        }

        // Step 2: Remove or trim existing ranges with larger intervals that overlap
        let mut i = 0;
        while i < self.ranges.len() {
            let existing = &self.ranges[i];
            if existing.interval_secs > new_interval {
                let existing_time_range = existing.time_range();
                if let Some(_intersection) = existing_time_range.intersect(&new_time_range) {
                    // This existing range has lower resolution and overlaps - trim or remove it
                    let remaining = existing.subtract(&new_time_range);
                    self.ranges.remove(i);
                    // Insert remaining pieces (in reverse to maintain order)
                    for piece in remaining.into_iter().rev() {
                        self.ranges.insert(i, piece);
                    }
                    // Don't increment i - we need to check the newly inserted pieces
                    // or the next original element
                    continue;
                }
            }
            i += 1;
        }

        // Step 3: Slice the new range to fill only the gaps and insert pieces
        for gap in gaps {
            if let Some(piece) = new_range.slice(&gap) {
                self.insert_range_sorted(piece);
            }
        }
    }

    /// Insert a range in sorted order, merging with adjacent ranges if possible.
    fn insert_range_sorted(&mut self, range: PriceRange) {
        if range.is_empty() {
            return;
        }

        // Find insertion position
        let insert_pos = self
            .ranges
            .iter()
            .position(|r| r.start > range.start)
            .unwrap_or(self.ranges.len());

        self.ranges.insert(insert_pos, range);

        // Try to merge with next range
        if insert_pos + 1 < self.ranges.len() {
            if self.ranges[insert_pos].can_merge_with(&self.ranges[insert_pos + 1]) {
                let next = self.ranges.remove(insert_pos + 1);
                let merged = self.ranges[insert_pos].merge_with(&next);
                self.ranges[insert_pos] = merged;
            }
        }

        // Try to merge with previous range
        if insert_pos > 0 {
            if self.ranges[insert_pos - 1].can_merge_with(&self.ranges[insert_pos]) {
                let current = self.ranges.remove(insert_pos);
                let merged = self.ranges[insert_pos - 1].merge_with(&current);
                self.ranges[insert_pos - 1] = merged;
            }
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
        self.add_range(PriceRange::from_points(points)?);
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
    fn test_add_range_higher_resolution_replaces_lower() {
        let mut data = CurrencyPriceData::new();

        // Add a range with 2-hour interval (lower resolution)
        let low_res_range = PriceRange::with_interval(
            Duration::hours(2),
            make_datetime(2024, 1, 1, 0),
            vec![dec!(100), dec!(120), dec!(140), dec!(160), dec!(180)], // hours 0, 2, 4, 6, 8
        );
        data.add_range(low_res_range);

        assert_eq!(data.ranges.len(), 1);
        assert_eq!(data.ranges[0].interval(), Duration::hours(2));

        // Add a range with 1-hour interval (higher resolution) that overlaps
        let high_res_points: Vec<PricePoint> = (2..7)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(200 + i as i64 * 10), // hours 2-6 with different prices
            })
            .collect();
        data.add_points(high_res_points).unwrap();

        // Should have 3 ranges now:
        // - hours 0 (low res, not replaced)
        // - hours 2-6 (high res, replaced the overlapping part)
        // - hour 8 (low res, not replaced)
        assert_eq!(data.ranges.len(), 3, "Should have 3 ranges after partial replacement");

        // First range should be the remaining low-res start
        assert_eq!(data.ranges[0].interval(), Duration::hours(2));
        assert_eq!(data.ranges[0].time_range().start, make_datetime(2024, 1, 1, 0));
        assert_eq!(data.ranges[0].time_range().end, make_datetime(2024, 1, 1, 0));

        // Second range should be the high-res replacement
        assert_eq!(data.ranges[1].interval(), Duration::hours(1));
        assert_eq!(data.ranges[1].time_range().start, make_datetime(2024, 1, 1, 2));
        assert_eq!(data.ranges[1].time_range().end, make_datetime(2024, 1, 1, 6));

        // Third range should be the remaining low-res end
        assert_eq!(data.ranges[2].interval(), Duration::hours(2));
        assert_eq!(data.ranges[2].time_range().start, make_datetime(2024, 1, 1, 8));
    }

    #[test]
    fn test_add_range_lower_resolution_fills_gaps() {
        let mut data = CurrencyPriceData::new();

        // Add high-resolution data for hours 5-9
        let high_res_points: Vec<PricePoint> = (5..10)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64),
            })
            .collect();
        data.add_points(high_res_points).unwrap();

        assert_eq!(data.ranges.len(), 1);

        // Add low-resolution data that spans hours 0-14 (overlapping the high-res data)
        let low_res_range = PriceRange::with_interval(
            Duration::hours(2),
            make_datetime(2024, 1, 1, 0),
            vec![dec!(10), dec!(12), dec!(14), dec!(16), dec!(18), dec!(20), dec!(22), dec!(24)], // hours 0, 2, 4, 6, 8, 10, 12, 14
        );
        data.add_range(low_res_range);

        // Should have 3 ranges:
        // - hours 0-4 (low res, fills gap before high res)
        // - hours 5-9 (high res, preserved)
        // - hours 10-14 (low res, fills gap after high res)
        assert_eq!(data.ranges.len(), 3, "Should have 3 ranges");

        // First range: low-res filling gap before
        assert_eq!(data.ranges[0].interval(), Duration::hours(2));
        assert_eq!(data.ranges[0].time_range().start, make_datetime(2024, 1, 1, 0));
        assert_eq!(data.ranges[0].time_range().end, make_datetime(2024, 1, 1, 4));

        // Second range: high-res preserved
        assert_eq!(data.ranges[1].interval(), Duration::hours(1));
        assert_eq!(data.ranges[1].time_range().start, make_datetime(2024, 1, 1, 5));
        assert_eq!(data.ranges[1].time_range().end, make_datetime(2024, 1, 1, 9));

        // Third range: low-res filling gap after
        assert_eq!(data.ranges[2].interval(), Duration::hours(2));
        assert_eq!(data.ranges[2].time_range().start, make_datetime(2024, 1, 1, 10));
        assert_eq!(data.ranges[2].time_range().end, make_datetime(2024, 1, 1, 14));
    }

    #[test]
    fn test_add_range_same_interval_preserves_existing() {
        let mut data = CurrencyPriceData::new();

        // Add first range: hours 0-9 with prices 100-109
        let points1: Vec<PricePoint> = (0..10)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(100 + i as i64),
            })
            .collect();
        data.add_points(points1).unwrap();

        // Add second range: hours 5-14 with different prices (200-209)
        let points2: Vec<PricePoint> = (5..15)
            .map(|i| PricePoint {
                timestamp: make_datetime(2024, 1, 1, i),
                price: Decimal::from(200 + i as i64),
            })
            .collect();
        data.add_points(points2).unwrap();

        // Should be merged into one range
        assert_eq!(data.ranges.len(), 1);

        // Check that the overlapping portion (hours 5-9) kept the ORIGINAL prices
        let (price_at_5, _) = data.estimate_price(make_datetime(2024, 1, 1, 5)).unwrap();
        assert_eq!(price_at_5, dec!(105), "Hour 5 should have original price 105, not 205");

        let (price_at_9, _) = data.estimate_price(make_datetime(2024, 1, 1, 9)).unwrap();
        assert_eq!(price_at_9, dec!(109), "Hour 9 should have original price 109, not 209");

        // Check that the new portion (hours 10-14) has the new prices
        let (price_at_10, _) = data.estimate_price(make_datetime(2024, 1, 1, 10)).unwrap();
        assert_eq!(price_at_10, dec!(210), "Hour 10 should have new price 210");
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
