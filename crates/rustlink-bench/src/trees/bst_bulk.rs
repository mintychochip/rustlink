//! `bst-bulk`: perfectly balanced BST built children-first, then random lookups.

use super::{lookups, run_variant};
use crate::measure::Params;
use crate::plan::Plan;
use crate::rng::Rng;
use crate::variants::{TreeBuild, VARIANTS, with_variant};
use crate::workload::{Outcome, Workload};

/// Nodes are allocated in post-order, so most links join close neighbours near the leaves and a
/// few join distant ones near the root: good locality with a long tail.
pub struct BstBulk;

impl Workload for BstBulk {
    fn name(&self) -> &'static str {
        "bst-bulk"
    }

    fn variants(&self) -> &'static [&'static str] {
        VARIANTS
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let n = params.n;
        let plan = Plan::balanced_bst(n);
        let reach = plan.reach();
        let mut rng = Rng::new(params.seed);
        let probes: Vec<u32> = (0..n).map(|_| rng.below(2 * n as u64) as u32).collect();

        with_variant!(variant, V => run_variant::<V>(
            params,
            n,
            &reach,
            "lookup",
            || V::from_plan(&plan),
            |tree| lookups(tree, &probes),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variants::{PlainU32, contains};

    #[test]
    fn lookups_hit_about_half_of_the_probes() {
        let n = 4096;
        let tree = PlainU32::from_plan(&Plan::balanced_bst(n));
        let mut rng = Rng::new(1);
        let hits = (0..n)
            .filter(|_| contains(&tree, rng.below(2 * n as u64) as u32))
            .count();
        assert!((hits as f64 / n as f64 - 0.5).abs() < 0.1, "{hits} of {n}");
    }
}
