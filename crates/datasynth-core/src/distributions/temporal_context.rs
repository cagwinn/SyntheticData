//! Unified temporal-context bundle (v3.4.1+).
//!
//! Historically each generator built its own [`HolidayCalendar`] +
//! [`BusinessDayCalculator`], typically only for a single year. That left
//! multi-year pipelines silently skipping holiday suppression for years
//! outside the first one, and required each generator to replicate the
//! same construction boilerplate.
//!
//! [`TemporalContext`] centralises that construction: given a `Region` and
//! an inclusive `(start_date, end_date)` span, it builds a single
//! [`BusinessDayCalculator`] whose holiday calendar is the **union** of
//! `HolidayCalendar::for_region(region, year)` for every year in the span.
//! Generators then hold an `Arc<TemporalContext>` and call the convenience
//! methods on it (`is_business_day`, `adjust_to_business_day`,
//! `sample_business_day_in_range`) instead of tracking their own state.
//!
//! Construction is `O(years × holidays-per-year)` — negligible. Lookups
//! route through the wrapped calculator, which is already tuned for the
//! existing JE generator at ~200k entries/sec.

use chrono::{Datelike, Duration, NaiveDate};
use rand::{Rng, RngExt};
use std::sync::Arc;

use super::business_day::BusinessDayCalculator;
use super::holidays::{HolidayCalendar, Region};

/// Unified temporal-context bundle for business-day / holiday awareness.
///
/// `Arc<TemporalContext>` is the intended ownership model — construct once
/// in the orchestrator, clone into each generator via
/// `generator.set_temporal_context(Arc::clone(&ctx))`.
#[derive(Debug, Clone)]
pub struct TemporalContext {
    region: Region,
    start_date: NaiveDate,
    end_date: NaiveDate,
    calculator: BusinessDayCalculator,
}

impl TemporalContext {
    /// Build a temporal context that covers the inclusive span
    /// `[start_date, end_date]` in the given `region`.
    ///
    /// Holiday calendars are loaded per-year and merged into a single
    /// calendar so multi-year pipelines work correctly.
    pub fn new(region: Region, start_date: NaiveDate, end_date: NaiveDate) -> Self {
        let start_year = start_date.year();
        // The end-year may extend by up to one year because settlement rules
        // (T+N, month-end-rollovers) can push posting dates past the raw
        // `end_date`. Add a one-year buffer so the calculator covers those.
        let end_year = end_date.year() + 1;

        let mut merged = HolidayCalendar::new(region, start_year);
        for year in start_year..=end_year {
            let yearly = HolidayCalendar::for_region(region, year);
            for holiday in yearly.holidays {
                merged.add_holiday(holiday);
            }
        }

        let calculator = BusinessDayCalculator::new(merged);

        Self {
            region,
            start_date,
            end_date,
            calculator,
        }
    }

    /// Convenience wrapper around [`TemporalContext::new`] that returns an
    /// `Arc`.
    pub fn shared(region: Region, start_date: NaiveDate, end_date: NaiveDate) -> Arc<Self> {
        Arc::new(Self::new(region, start_date, end_date))
    }

    /// Region this context was built for.
    pub fn region(&self) -> Region {
        self.region
    }

    /// Inclusive start of the covered span.
    pub fn start_date(&self) -> NaiveDate {
        self.start_date
    }

    /// Inclusive end of the covered span.
    pub fn end_date(&self) -> NaiveDate {
        self.end_date
    }

    /// Access the wrapped [`BusinessDayCalculator`] directly. Useful when
    /// callers need settlement-rule or half-day APIs the shortcuts below
    /// don't expose.
    pub fn calculator(&self) -> &BusinessDayCalculator {
        &self.calculator
    }

    /// Is `date` a business day in this region?
    pub fn is_business_day(&self, date: NaiveDate) -> bool {
        self.calculator.is_business_day(date)
    }

    /// Snap `date` forward to the next business day (inclusive — if `date`
    /// is already a business day, it's returned unchanged). Used by
    /// document-flow generators to ensure posting dates never land on a
    /// weekend or holiday.
    pub fn adjust_to_business_day(&self, date: NaiveDate) -> NaiveDate {
        self.calculator.next_business_day(date, true)
    }

    /// Snap `date` backward to the previous business day (inclusive).
    pub fn adjust_to_previous_business_day(&self, date: NaiveDate) -> NaiveDate {
        self.calculator.prev_business_day(date, true)
    }

    /// Sample a business day uniformly from `[start, end]` (inclusive).
    ///
    /// Implementation strategy: sample a raw offset from the RNG, then snap
    /// forward to the next business day. This preserves existing RNG call
    /// counts for unit tests that rely on a specific draw sequence — each
    /// sample consumes exactly one `rng.random_range(...)` call, just like
    /// the pre-v3.4.1 raw `rng.random_range(0..=days_range)` pattern.
    ///
    /// If snapping forward would exceed `end`, the result is clamped to the
    /// nearest preceding business day (to guarantee the result stays in
    /// range).
    pub fn sample_business_day_in_range<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
        start: NaiveDate,
        end: NaiveDate,
    ) -> NaiveDate {
        let span_days = (end - start).num_days().max(0) as u32;
        let raw_offset = rng.random_range(0..=span_days) as i64;
        let raw_date = start + Duration::days(raw_offset);
        let snapped = self.adjust_to_business_day(raw_date);
        if snapped > end {
            self.adjust_to_previous_business_day(end)
        } else {
            snapped
        }
    }
}

/// Parse a region code (e.g. "US", "DE") into [`Region`]. Falls back to
/// [`Region::US`] for unrecognised codes to match the legacy behaviour at
/// `je_generator.rs::parse_region`.
pub fn parse_region_code(code: &str) -> Region {
    match code.to_uppercase().as_str() {
        "US" => Region::US,
        "DE" => Region::DE,
        "GB" | "UK" => Region::GB,
        "FR" => Region::FR,
        "IT" => Region::IT,
        "ES" => Region::ES,
        "CA" => Region::CA,
        "CN" => Region::CN,
        "JP" => Region::JP,
        "IN" => Region::IN,
        "BR" => Region::BR,
        "MX" => Region::MX,
        "AU" => Region::AU,
        "SG" => Region::SG,
        "KR" => Region::KR,
        _ => Region::US,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Weekday;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn weekend_is_not_business_day() {
        let ctx = TemporalContext::new(Region::US, d(2024, 1, 1), d(2024, 12, 31));
        // 2024-01-06 was a Saturday
        assert_eq!(d(2024, 1, 6).weekday(), Weekday::Sat);
        assert!(!ctx.is_business_day(d(2024, 1, 6)));
        // 2024-01-08 was a Monday
        assert!(ctx.is_business_day(d(2024, 1, 8)));
    }

    #[test]
    fn adjust_snaps_weekend_forward() {
        let ctx = TemporalContext::new(Region::US, d(2024, 1, 1), d(2024, 12, 31));
        // Saturday Jan 6 → should snap to Monday Jan 8
        let adjusted = ctx.adjust_to_business_day(d(2024, 1, 6));
        assert_eq!(adjusted, d(2024, 1, 8));
    }

    #[test]
    fn sample_never_returns_weekend() {
        let ctx = TemporalContext::new(Region::US, d(2024, 1, 1), d(2024, 12, 31));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..1000 {
            let date = ctx.sample_business_day_in_range(&mut rng, d(2024, 1, 1), d(2024, 12, 31));
            assert!(
                ctx.is_business_day(date),
                "sampled non-business day: {date}"
            );
        }
    }

    #[test]
    fn multi_year_span_includes_both_years() {
        let ctx = TemporalContext::new(Region::US, d(2024, 1, 1), d(2025, 12, 31));
        // US Independence Day exists in both 2024 and 2025
        assert!(!ctx.is_business_day(d(2024, 7, 4)));
        assert!(!ctx.is_business_day(d(2025, 7, 4)));
    }

    #[test]
    fn parse_region_code_fallback() {
        assert_eq!(parse_region_code("US"), Region::US);
        assert_eq!(parse_region_code("de"), Region::DE);
        assert_eq!(parse_region_code("UK"), Region::GB);
        assert_eq!(parse_region_code("ZZ"), Region::US); // fallback
    }
}
