//! Robust summary statistics.

/// Median and median absolute deviation of a sample set.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Dist {
    /// The median.
    pub median: f64,
    /// The median absolute deviation from the median.
    pub mad: f64,
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Median and MAD of `samples`. Panics if `samples` is empty.
pub fn dist(samples: &[f64]) -> Dist {
    assert!(!samples.is_empty(), "dist of an empty sample set");
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = median_of_sorted(&sorted);
    let mut deviations: Vec<f64> = sorted.iter().map(|s| (s - median).abs()).collect();
    deviations.sort_by(|a, b| a.total_cmp(b));
    Dist {
        median,
        mad: median_of_sorted(&deviations),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_count_uses_the_middle_value() {
        let d = dist(&[1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(
            d,
            Dist {
                median: 3.0,
                mad: 1.0
            }
        );
    }

    #[test]
    fn even_count_averages_the_middle_pair() {
        let d = dist(&[4.0, 1.0, 3.0, 2.0]);
        assert_eq!(
            d,
            Dist {
                median: 2.5,
                mad: 1.0
            }
        );
    }

    #[test]
    fn single_sample_has_zero_deviation() {
        assert_eq!(
            dist(&[7.0]),
            Dist {
                median: 7.0,
                mad: 0.0
            }
        );
    }

    #[test]
    #[should_panic(expected = "empty")]
    fn empty_input_panics() {
        dist(&[]);
    }
}
