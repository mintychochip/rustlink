//! Tree workloads: `bst-random`, `bst-bulk`, `ast` and `ast-bytes`.

mod ast;
mod ast_bytes;
mod bst_bulk;
mod bst_random;

use crate::measure::{Params, Pass, run_reps};
use crate::variants::{TreeBuild, TreeView, contains};
use crate::workload::{Outcome, Reach, Workload};

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(bst_random::BstRandom),
        Box::new(bst_bulk::BstBulk),
        Box::new(ast::Ast),
        Box::new(ast_bytes::AstBytes),
    ]
}

/// Runs one variant: reports `Unreachable` when its links cannot represent the structure, and
/// otherwise times `build` and `traverse`.
pub(crate) fn run_variant<V: TreeBuild>(
    params: &Params,
    nodes: usize,
    reach: &Reach,
    unit: &'static str,
    build: impl FnMut() -> V,
    traverse: impl FnMut(&V) -> Pass,
) -> Result<Outcome, String> {
    if !V::LIMIT.admits(nodes, reach) {
        return Ok(Outcome::Unreachable(*reach));
    }
    Ok(Outcome::from_reps(
        run_reps(params, nodes as u64, build, traverse),
        V::NODE_BYTES,
        unit,
        Some(*reach),
    ))
}

/// A lookup pass: counts the probes that are found.
pub(crate) fn lookups<V: TreeView>(view: &V, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|&&key| contains(view, key)).count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variants::VARIANTS;
    use crate::workload::Metrics;

    fn done(outcome: Outcome) -> Metrics {
        match outcome {
            Outcome::Done(metrics) => metrics,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn params(n: usize) -> Params {
        Params {
            n,
            reps: 3,
            seed: 1,
            budget: std::time::Duration::from_secs(60),
        }
    }

    #[test]
    fn the_module_registers_its_four_workloads() {
        let names: Vec<&str> = workloads().iter().map(|w| w.name()).collect();
        assert_eq!(names, ["bst-random", "bst-bulk", "ast", "ast-bytes"]);
    }

    #[test]
    fn every_tree_workload_reports_done_or_unreachable_for_every_variant_at_1024() {
        for workload in workloads() {
            for &variant in workload.variants() {
                let outcome = workload.run(variant, &params(1024)).unwrap();
                match outcome {
                    Outcome::Done(m) => {
                        assert!(m.traverse.median > 0.0, "{} {variant}", workload.name());
                        assert!(m.build.median > 0.0, "{} {variant}", workload.name());
                        assert!(m.node_bytes > 0, "{} {variant}", workload.name());
                    }
                    Outcome::Unreachable(reach) => {
                        assert!(
                            matches!(variant, "abs16" | "rel16" | "rel8"),
                            "{} {variant} unexpectedly unreachable: {reach:?}",
                            workload.name()
                        );
                    }
                    Outcome::Skipped(why) => panic!("{} {variant} skipped: {why}", workload.name()),
                }
            }
        }
    }

    #[test]
    fn node_sizes_and_units_are_reported_per_variant() {
        let bulk = bst_bulk::BstBulk;
        let sizes: Vec<(&str, usize)> = ["box", "usize", "u32", "abs32", "rel32"]
            .into_iter()
            .map(|v| (v, done(bulk.run(v, &params(1024)).unwrap()).node_bytes))
            .collect();
        assert_eq!(
            sizes,
            [
                ("box", 24),
                ("usize", 24),
                ("u32", 12),
                ("abs32", 12),
                ("rel32", 12)
            ]
        );
        assert_eq!(done(bulk.run("u32", &params(1024)).unwrap()).unit, "lookup");
        assert_eq!(
            done(ast::Ast.run("u32", &params(1024)).unwrap()).unit,
            "visit"
        );
    }

    #[test]
    fn balanced_trees_do_not_fit_rel8_but_do_fit_the_wide_variants() {
        let bulk = bst_bulk::BstBulk;
        assert!(matches!(
            bulk.run("rel8", &params(1024)).unwrap(),
            Outcome::Unreachable(_)
        ));
        for variant in ["box", "usize", "u32", "abs32", "rel32"] {
            assert!(matches!(
                bulk.run(variant, &params(1024)).unwrap(),
                Outcome::Done(_)
            ));
        }
        assert!(matches!(
            bulk.run("abs16", &params(1024)).unwrap(),
            Outcome::Done(_)
        ));
        assert!(matches!(
            bulk.run("abs16", &params(1 << 17)).unwrap(),
            Outcome::Unreachable(_)
        ));
    }

    #[test]
    fn unknown_variants_are_errors() {
        for workload in workloads() {
            assert!(
                workload.run("nope", &params(16)).is_err(),
                "{}",
                workload.name()
            );
        }
    }

    #[test]
    fn variant_lists_are_the_shared_list_except_for_ast_bytes() {
        for workload in workloads() {
            if workload.name() == "ast-bytes" {
                assert_eq!(workload.variants(), ["bytes"]);
            } else {
                assert_eq!(workload.variants(), VARIANTS);
            }
        }
    }

    #[test]
    fn bst_random_has_a_smaller_max_n() {
        assert_eq!(bst_random::BstRandom.max_n(), 1 << 22);
        assert_eq!(bst_bulk::BstBulk.max_n(), 1 << 24);
    }
}
