//! The contract between workloads and the runner.

use crate::measure::{OverBudget, Params, RepStats};
use crate::stats::Dist;

/// Link-distance statistics of a structure, used to decide whether a narrow relative link can
/// represent it. Distances are `|child index - parent index|`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    /// Number of links measured.
    pub links: u64,
    /// Fraction of links with distance at most 127 (fits an `i8`, conservatively).
    pub within_127: f64,
    /// Fraction of links with distance at most 32,767 (fits an `i16`, conservatively).
    pub within_32767: f64,
    /// Largest distance.
    pub max_abs_delta: u64,
}

impl Default for Reach {
    fn default() -> Self {
        Reach {
            links: 0,
            within_127: 1.0,
            within_32767: 1.0,
            max_abs_delta: 0,
        }
    }
}

impl Reach {
    /// Summarises the signed deltas of every link.
    pub fn from_deltas(deltas: impl IntoIterator<Item = i64>) -> Reach {
        let (mut links, mut within_127, mut within_32767, mut max) = (0u64, 0u64, 0u64, 0u64);
        for delta in deltas {
            let distance = delta.unsigned_abs();
            links += 1;
            within_127 += u64::from(distance <= 127);
            within_32767 += u64::from(distance <= 32_767);
            max = max.max(distance);
        }
        if links == 0 {
            return Reach::default();
        }
        Reach {
            links,
            within_127: within_127 as f64 / links as f64,
            within_32767: within_32767 as f64 / links as f64,
            max_abs_delta: max,
        }
    }

    /// True if every link distance is at most `max_delta`.
    pub fn fits(&self, max_delta: i64) -> bool {
        self.max_abs_delta <= max_delta as u64
    }
}

/// How many elements, or how far apart links, a variant can represent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// No practical limit (pointers, machine-word indices, hand-written `u32` indices).
    None,
    /// Absolute links: at most this many elements.
    Abs {
        /// Largest element count the width can address.
        max_elements: u64,
    },
    /// Relative links: every link distance at most this.
    Rel {
        /// Largest representable distance.
        max_delta: i64,
    },
}

impl Limit {
    /// True if a structure of `elements` elements with link distances `reach` is representable.
    pub fn admits(&self, elements: usize, reach: &Reach) -> bool {
        match *self {
            Limit::None => true,
            Limit::Abs { max_elements } => elements as u64 <= max_elements,
            Limit::Rel { max_delta } => reach.fits(max_delta),
        }
    }
}

/// Numbers reported for one finished cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    /// `size_of` one node in bytes (average record size for variable-size layouts).
    pub node_bytes: usize,
    /// Live heap bytes requested per node after the build; 0 when no counting allocator is installed.
    pub heap_bytes_per_node: f64,
    /// Build time in ns per node.
    pub build: Dist,
    /// Traversal time in ns per unit of work.
    pub traverse: Dist,
    /// Build CPU time in ns per node, when the platform reports it.
    pub build_cpu: Option<Dist>,
    /// Traversal CPU time in ns per unit of work, when the platform reports it.
    pub traverse_cpu: Option<Dist>,
    /// What one unit of traversal work is: `"lookup"`, `"visit"`, `"edge"` or `"node"`.
    pub unit: &'static str,
    /// Link-distance statistics, when the workload measured them.
    pub reach: Option<Reach>,
}

/// What running one variant of a workload produced.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The variant ran.
    Done(Metrics),
    /// The variant's links cannot represent this workload at this size.
    Unreachable(Reach),
    /// The cell was not run, with the reason.
    Skipped(String),
}

impl Outcome {
    /// Converts the result of [`run_reps`](crate::measure::run_reps) into an outcome.
    pub fn from_reps(
        result: Result<RepStats, OverBudget>,
        node_bytes: usize,
        unit: &'static str,
        reach: Option<Reach>,
    ) -> Outcome {
        match result {
            Ok(stats) => Outcome::Done(Metrics {
                node_bytes,
                heap_bytes_per_node: stats.heap_bytes_per_node,
                build: stats.build,
                traverse: stats.traverse,
                build_cpu: stats.build_cpu,
                traverse_cpu: stats.traverse_cpu,
                unit,
                reach,
            }),
            Err(over) => Outcome::Skipped(format!(
                "over the time budget: predicted {:.1?} for the remaining repetitions, budget {:.1?}",
                over.predicted, over.budget
            )),
        }
    }
}

/// A benchmark workload: one data structure task with several link-representation variants.
pub trait Workload: Sync {
    /// Stable name, e.g. `"bst-random"`.
    fn name(&self) -> &'static str;

    /// Variant names this workload accepts.
    fn variants(&self) -> &'static [&'static str];

    /// Largest `n` the default sweep runs for this workload.
    fn max_n(&self) -> usize {
        1 << 24
    }

    /// Rough upper bound of heap bytes per unit of `n`, used by the sweep to skip cells that
    /// would not fit in memory.
    fn footprint_per_n(&self) -> usize {
        64
    }

    /// Runs one variant. `Err` means the request itself was invalid (for example an unknown
    /// variant name).
    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_counts_distances_in_both_directions() {
        let reach = Reach::from_deltas([1, -1, 127, -128, 200, 40_000]);
        assert_eq!(reach.links, 6);
        assert_eq!(reach.max_abs_delta, 40_000);
        assert!((reach.within_127 - 3.0 / 6.0).abs() < 1e-12); // 1, 1, 127; 128 is outside
        assert!((reach.within_32767 - 5.0 / 6.0).abs() < 1e-12);
        assert!(!reach.fits(32_767));
        assert!(reach.fits(40_000));
    }

    #[test]
    fn no_links_fit_everything() {
        let reach = Reach::from_deltas([]);
        assert_eq!(reach, Reach::default());
        assert!(reach.fits(0));
    }

    #[test]
    fn limits_decide_representability() {
        let far = Reach::from_deltas([300]);
        assert!(Limit::None.admits(1 << 30, &far));
        assert!(
            Limit::Abs {
                max_elements: 65_535
            }
            .admits(65_535, &far)
        );
        assert!(
            !Limit::Abs {
                max_elements: 65_535
            }
            .admits(65_536, &far)
        );
        assert!(Limit::Rel { max_delta: 32_767 }.admits(10, &far));
        assert!(!Limit::Rel { max_delta: 127 }.admits(10, &far));
    }
}
