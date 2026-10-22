//! `bst-random`: unbalanced BST built by inserting random keys, then random lookups.

use super::{lookups, run_variant};
use crate::measure::Params;
use crate::rng::Rng;
use crate::variants::{PlainU32, TreeBuild, VARIANTS, with_variant};
use crate::workload::{Outcome, Workload};

/// Nodes are allocated in insertion order, so parent and child are far apart: poor locality.
pub struct BstRandom;

impl Workload for BstRandom {
    fn name(&self) -> &'static str {
        "bst-random"
    }

    fn variants(&self) -> &'static [&'static str] {
        VARIANTS
    }

    fn max_n(&self) -> usize {
        1 << 22
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let n = params.n;
        let mut rng = Rng::new(params.seed);
        let keys: Vec<u32> = (0..n).map(|_| rng.next_u32()).collect();
        let probes: Vec<u32> = (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    keys[rng.below(n as u64) as usize]
                } else {
                    rng.next_u32()
                }
            })
            .collect();

        // The link distances do not depend on the representation, so measure them once on the
        // hand-written u32 variant and let each variant check them against its width.
        let reference = {
            let mut tree = PlainU32::empty(n);
            for &key in &keys {
                tree.insert_bst(key);
            }
            tree
        };
        let (reach, nodes) = (reference.reach(), reference.len());
        drop(reference);

        with_variant!(variant, V => run_variant::<V>(
            params,
            nodes,
            &reach,
            "lookup",
            || {
                let mut tree = V::empty(n);
                for &key in &keys {
                    tree.insert_bst(key);
                }
                tree
            },
            |tree| lookups(tree, &probes),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::Metrics;

    fn done(outcome: Outcome) -> Metrics {
        match outcome {
            Outcome::Done(metrics) => metrics,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn random_insertion_gives_far_links_so_narrow_relative_widths_are_unreachable() {
        let params = Params {
            n: 4096,
            reps: 3,
            seed: 3,
            budget: std::time::Duration::from_secs(60),
        };
        let outcome = BstRandom.run("rel8", &params).unwrap();
        let Outcome::Unreachable(reach) = outcome else {
            panic!("rel8 must not fit")
        };
        assert!(reach.max_abs_delta > 127);
        assert!(reach.links > 4000);
    }

    #[test]
    fn wide_variants_report_reach_and_agree_on_node_counts() {
        let params = Params {
            n: 2048,
            reps: 3,
            seed: 3,
            budget: std::time::Duration::from_secs(60),
        };
        let a = done(BstRandom.run("u32", &params).unwrap());
        let b = done(BstRandom.run("abs32", &params).unwrap());
        assert_eq!(a.reach, b.reach);
        assert!(a.reach.unwrap().links > 2000);
        assert_eq!((a.node_bytes, b.node_bytes), (12, 12));
    }
}
