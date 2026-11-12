// SPDX-License-Identifier: GPL-3.0-or-later

//! Summary statistics for timing samples.

/// Total, mean, median, 95th percentile and maximum of a set of samples.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub total: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl Summary {
    /// Summarise `samples` (any order, any unit). Percentiles are
    /// nearest-rank: the smallest sample with at least that share of the
    /// samples at or below it, so they are always values that were measured.
    /// No samples give all zeros.
    pub fn of(samples: &[f64]) -> Summary {
        if samples.is_empty() {
            return Summary::default();
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let total: f64 = sorted.iter().sum();
        Summary {
            total,
            mean: total / sorted.len() as f64,
            p50: nearest_rank(&sorted, 50),
            p95: nearest_rank(&sorted, 95),
            max: sorted[sorted.len() - 1],
        }
    }
}

/// The `percent`th percentile of non-empty, ascending `sorted`.
fn nearest_rank(sorted: &[f64], percent: usize) -> f64 {
    let rank = (sorted.len() * percent).div_ceil(100);
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_all_zeros() {
        assert_eq!(Summary::of(&[]), Summary::default());
    }

    #[test]
    fn one_sample() {
        let s = Summary::of(&[2.5]);
        assert_eq!(
            s,
            Summary {
                total: 2.5,
                mean: 2.5,
                p50: 2.5,
                p95: 2.5,
                max: 2.5
            }
        );
    }

    #[test]
    fn nearest_rank_percentiles() {
        // 1..=100 shuffled: the pth percentile is the value p.
        let mut v: Vec<f64> = (1..=100).map(f64::from).collect();
        v.reverse();
        let s = Summary::of(&v);
        assert_eq!(s.p50, 50.0);
        assert_eq!(s.p95, 95.0);
        assert_eq!(s.max, 100.0);
        assert_eq!(s.total, 5050.0);
        assert_eq!(s.mean, 50.5);
    }

    #[test]
    fn even_and_odd_counts() {
        assert_eq!(Summary::of(&[4.0, 1.0, 3.0, 2.0]).p50, 2.0);
        assert_eq!(Summary::of(&[5.0, 1.0, 3.0, 2.0, 4.0]).p50, 3.0);
        assert_eq!(Summary::of(&[1.0, 2.0]).p95, 2.0);
    }

    #[test]
    fn ordering_invariant() {
        let s = Summary::of(&[0.3, 9.0, 0.1, 0.2, 0.25, 0.4, 0.15]);
        assert!(s.p50 <= s.p95 && s.p95 <= s.max);
        assert!(s.mean <= s.max);
    }
}
