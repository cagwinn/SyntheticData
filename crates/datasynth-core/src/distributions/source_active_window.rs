//! Per-Source active-window sampler driven by SP2's ActiveLifetimePrior.

use std::collections::HashMap;

use rand::{Rng, RngExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveWindow {
    pub start_day: i64,
    pub end_day: i64,
}

impl ActiveWindow {
    pub fn contains(&self, day: i64) -> bool {
        day >= self.start_day && day <= self.end_day
    }
    pub fn length_days(&self) -> i64 {
        (self.end_day - self.start_day).max(0)
    }
}

#[derive(Clone)]
pub struct SourceActiveWindow {
    pub by_source: HashMap<String, ActiveWindow>,
    pub period_days: i64,
}

impl SourceActiveWindow {
    pub fn build<R: Rng>(
        sources: &[String],
        period_days: i64,
        mut lifetime_sampler: impl FnMut(&mut R) -> i64,
        rng: &mut R,
    ) -> Self {
        let mut by_source = HashMap::new();
        for src in sources {
            let life = lifetime_sampler(rng).min(period_days).max(0);
            let max_start = (period_days - life).max(0);
            let start = if max_start == 0 {
                0
            } else {
                rng.random_range(0..=max_start)
            };
            by_source.insert(
                src.clone(),
                ActiveWindow {
                    start_day: start,
                    end_day: start + life,
                },
            );
        }
        Self {
            by_source,
            period_days,
        }
    }

    pub fn is_active(&self, source: &str, day_in_period: i64) -> bool {
        match self.by_source.get(source) {
            Some(w) => w.contains(day_in_period),
            None => day_in_period >= 0 && day_in_period < self.period_days,
        }
    }
}

/// SP3.2 — per-Source multi-segment active windows. When present in the prior,
/// preferred over the single-window `SourceActiveWindow` because real Sources
/// have multimodal activity (month-end pile-ups, quarter close).
#[derive(Debug, Clone)]
pub struct MultiSegmentActiveWindow {
    pub by_source: std::collections::HashMap<String, Vec<ActiveWindow>>,
    pub period_days: i64,
}

impl MultiSegmentActiveWindow {
    /// Build by sampling segments from the prior.
    ///
    /// `fallback_lifetime` is used when a Source has no per-source prior entry.
    pub fn build_from_prior<R: rand::Rng>(
        sources: &[String],
        period_days: i64,
        prior: &crate::distributions::behavioral_priors::ActiveSegmentsPrior,
        mut fallback_lifetime: impl FnMut(&mut R) -> i64,
        rng: &mut R,
    ) -> Self {
        use rand::RngExt;
        let mut by_source: std::collections::HashMap<String, Vec<ActiveWindow>> =
            std::collections::HashMap::new();
        for src in sources {
            let segments = match prior.by_source.get(src) {
                Some(summary) => place_segments_from_prior(summary, period_days, rng),
                None => {
                    let life = fallback_lifetime(rng).min(period_days).max(0);
                    let max_start = (period_days - life).max(0);
                    let start = if max_start == 0 {
                        0
                    } else {
                        rng.random_range(0..=max_start)
                    };
                    vec![ActiveWindow {
                        start_day: start,
                        end_day: start + life,
                    }]
                }
            };
            by_source.insert(src.clone(), segments);
        }
        Self {
            by_source,
            period_days,
        }
    }

    pub fn is_active(&self, source: &str, day: i64) -> bool {
        match self.by_source.get(source) {
            Some(segments) => segments.iter().any(|w| w.contains(day)),
            None => day >= 0 && day < self.period_days,
        }
    }
}

fn place_segments_from_prior<R: rand::Rng>(
    summary: &crate::distributions::behavioral_priors::SourceSegmentSummary,
    period_days: i64,
    rng: &mut R,
) -> Vec<ActiveWindow> {
    use rand::RngExt;
    let n_segments = summary.segment_count_histogram.sample_bucket(rng).max(1) as usize;
    // Pre-sample lengths + gaps.
    let mut lengths: Vec<i64> = Vec::with_capacity(n_segments);
    let mut gaps: Vec<i64> = Vec::with_capacity(n_segments.saturating_sub(1));
    for _ in 0..n_segments {
        lengths.push(summary.segment_length_histogram.sample_bucket(rng).max(1) as i64);
    }
    for _ in 0..n_segments.saturating_sub(1) {
        gaps.push(summary.gap_length_histogram.sample_bucket(rng).max(1) as i64);
    }
    // Total span = sum(lengths) + sum(gaps).
    let total_span: i64 = lengths.iter().sum::<i64>() + gaps.iter().sum::<i64>();
    let max_start = (period_days - total_span).max(0);
    let mut cursor = if max_start == 0 {
        0
    } else {
        rng.random_range(0..=max_start)
    };
    let mut windows = Vec::with_capacity(n_segments);
    for (idx, len) in lengths.iter().enumerate() {
        if idx > 0 {
            cursor += gaps[idx - 1];
        }
        let end = (cursor + len).min(period_days);
        windows.push(ActiveWindow {
            start_day: cursor,
            end_day: end,
        });
        cursor = end;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn active_window_contains_known_range() {
        let w = ActiveWindow {
            start_day: 10,
            end_day: 20,
        };
        assert!(w.contains(10));
        assert!(w.contains(15));
        assert!(w.contains(20));
        assert!(!w.contains(9));
        assert!(!w.contains(21));
    }

    #[test]
    fn build_assigns_one_window_per_source() {
        let sources = vec!["KR".to_string(), "RE".to_string()];
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let saw = SourceActiveWindow::build(&sources, 365, |r| r.random_range(30..=180), &mut rng);
        assert_eq!(saw.by_source.len(), 2);
        for w in saw.by_source.values() {
            assert!(w.length_days() >= 30 && w.length_days() <= 180);
            assert!(w.start_day >= 0);
            assert!(w.end_day <= 365);
        }
    }

    #[test]
    fn is_active_unknown_source_full_period() {
        let saw = SourceActiveWindow {
            by_source: HashMap::new(),
            period_days: 100,
        };
        assert!(saw.is_active("UNKNOWN", 0));
        assert!(saw.is_active("UNKNOWN", 99));
        assert!(!saw.is_active("UNKNOWN", 100));
        assert!(!saw.is_active("UNKNOWN", -1));
    }

    #[test]
    fn multi_segment_active_window_respects_prior() {
        use crate::distributions::behavioral_priors::{
            ActiveSegmentsPrior, LineCountHistogram, SourceSegmentSummary,
            ACTIVE_LIFETIME_DAY_BUCKETS, SEGMENT_COUNT_BUCKETS, SEGMENT_GAP_BUCKETS,
        };
        let (count_h, _) = LineCountHistogram::build(&[3], SEGMENT_COUNT_BUCKETS);
        let (len_h, _) = LineCountHistogram::build(&[30, 30, 30], ACTIVE_LIFETIME_DAY_BUCKETS);
        let (gap_h, _) = LineCountHistogram::build(&[14, 14], SEGMENT_GAP_BUCKETS);
        let mut by_source = std::collections::BTreeMap::new();
        by_source.insert(
            "A".to_string(),
            SourceSegmentSummary {
                segment_count_histogram: count_h,
                segment_length_histogram: len_h,
                gap_length_histogram: gap_h,
            },
        );
        let prior = ActiveSegmentsPrior { by_source };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let saw = MultiSegmentActiveWindow::build_from_prior(
            &["A".to_string()],
            365,
            &prior,
            |r| {
                use rand::RngExt;
                r.random_range(30..=180)
            },
            &mut rng,
        );
        let segments = &saw.by_source["A"];
        assert!(!segments.is_empty(), "should place at least one segment");
        assert!(
            segments.len() <= 5,
            "got {} segments (expected ~3 ± noise, ≥1 covered by is_empty assert above)",
            segments.len()
        );
        for w in segments {
            assert!(w.start_day >= 0);
            assert!(w.end_day <= 365);
            assert!(w.end_day >= w.start_day);
        }
    }

    #[test]
    fn multi_segment_is_active_handles_unknown_source() {
        let saw = MultiSegmentActiveWindow {
            by_source: std::collections::HashMap::new(),
            period_days: 100,
        };
        assert!(saw.is_active("UNKNOWN", 0));
        assert!(saw.is_active("UNKNOWN", 99));
        assert!(!saw.is_active("UNKNOWN", 100));
    }
}
