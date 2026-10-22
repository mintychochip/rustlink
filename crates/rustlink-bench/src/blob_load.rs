//! Blob workloads: `blob-load` and `blob-lookup`.
//!
//! Both use the `bst-bulk` tree stored in a rustlink `Arena` and frozen into a blob.
//!
//! * `blob-load`: build time is rebuilding the arena from the plan (what a program without a
//!   blob has to do at start-up); the traverse metric is `BlobRef::verify` over the frozen image,
//!   in ns per node.
//! * `blob-lookup`: the traverse metric is random lookups through the *verified blob view*, to
//!   compare with `bst-bulk` lookups through the arena.

use rustlink::{Arena, BlobRef, Link, Links, Region, Width, impl_links};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::measure::{Params, Pass, run_reps};
use crate::plan::{Plan, Spec};
use crate::rng::Rng;
use crate::workload::{Limit, Outcome, Reach, Workload};

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![Box::new(BlobLoad), Box::new(BlobLookup)]
}

const VARIANTS: &[&str] = &["abs32", "abs16", "rel32", "rel16", "rel8"];

/// A node type that can be frozen into a blob and searched as a BST.
trait BlobNode:
    Copy + FromBytes + IntoBytes + Immutable + KnownLayout + Unaligned + Links + 'static
{
    type Child: Link<Self>;
    const LIMIT: Limit;
    fn from_spec(owner: usize, spec: Spec) -> Self;
    fn payload(&self) -> u32;
    fn a(&self) -> Self::Child;
    fn b(&self) -> Self::Child;
}

macro_rules! blob_node {
    (
        $node:ident, $link:ident, $w:ty, $schema:expr, $limit:expr,
        |$owner:ident, $child:ident| $make:expr
    ) => {
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
        #[repr(C)]
        struct $node {
            payload: U32,
            a: $link<$node, $w>,
            b: $link<$node, $w>,
        }

        impl_links!($node, schema = $schema, links = [a, b]);

        impl BlobNode for $node {
            type Child = $link<$node, $w>;
            const LIMIT: Limit = $limit;

            fn from_spec(owner: usize, spec: Spec) -> Self {
                let link = |$owner: usize, $child: Option<u32>| {
                    let $child = $child.map(|c| c as usize);
                    $make.expect("reach checked by the workload")
                };
                $node {
                    payload: U32::new(spec.payload),
                    a: link(owner, spec.a),
                    b: link(owner, spec.b),
                }
            }

            fn payload(&self) -> u32 {
                self.payload.get()
            }

            fn a(&self) -> Self::Child {
                self.a
            }

            fn b(&self) -> Self::Child {
                self.b
            }
        }
    };
}

use rustlink::{OptAbs, OptRel};

blob_node!(
    Abs32,
    OptAbs,
    u32,
    0x424C_4F42_0001,
    Limit::Abs {
        max_elements: <u32 as Width>::MAX
    },
    |_owner, child| OptAbs::new(child)
);
blob_node!(
    Abs16,
    OptAbs,
    u16,
    0x424C_4F42_0002,
    Limit::Abs {
        max_elements: <u16 as Width>::MAX
    },
    |_owner, child| OptAbs::new(child)
);
blob_node!(
    Rel32,
    OptRel,
    u32,
    0x424C_4F42_0003,
    Limit::Rel {
        max_delta: <u32 as Width>::SIGNED_MAX
    },
    |owner, child| OptRel::new(owner, child)
);
blob_node!(
    Rel16,
    OptRel,
    u16,
    0x424C_4F42_0004,
    Limit::Rel {
        max_delta: <u16 as Width>::SIGNED_MAX
    },
    |owner, child| OptRel::new(owner, child)
);
blob_node!(
    Rel8,
    OptRel,
    u8,
    0x424C_4F42_0005,
    Limit::Rel {
        max_delta: <u8 as Width>::SIGNED_MAX
    },
    |owner, child| OptRel::new(owner, child)
);

fn rebuild<N: BlobNode>(plan: &Plan) -> Arena<N> {
    let mut arena = Arena::with_capacity(plan.nodes.len());
    for (owner, spec) in plan.nodes.iter().enumerate() {
        arena.push(N::from_spec(owner, *spec));
    }
    arena
}

fn contains<N: BlobNode, R: Region<N>>(region: &R, root: usize, key: u32) -> bool {
    let Some(mut cursor) = region.cursor(root) else {
        return false;
    };
    loop {
        let node = cursor.get();
        let payload = node.payload();
        if key == payload {
            return true;
        }
        let child = if key < payload { node.a() } else { node.b() };
        match cursor.follow(child) {
            Some(next) => cursor = next,
            None => return false,
        }
    }
}

fn lookups<N: BlobNode, R: Region<N>>(region: &R, root: usize, probes: &[u32]) -> Pass {
    let hits = probes
        .iter()
        .filter(|&&key| contains(region, root, key))
        .count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

fn inputs(params: &Params) -> (Plan, Reach, Vec<u32>) {
    let n = params.n;
    let plan = Plan::balanced_bst(n);
    let reach = plan.reach();
    let mut rng = Rng::new(params.seed);
    let probes = (0..n).map(|_| rng.below(2 * n as u64) as u32).collect();
    (plan, reach, probes)
}

fn run_load<N: BlobNode>(params: &Params, plan: &Plan, reach: &Reach) -> Result<Outcome, String> {
    if !N::LIMIT.admits(plan.nodes.len(), reach) {
        return Ok(Outcome::Unreachable(*reach));
    }
    let image = rebuild::<N>(plan).freeze();
    let nodes = plan.nodes.len() as u64;
    Ok(Outcome::from_reps(
        run_reps(
            params,
            nodes,
            || rebuild::<N>(plan),
            |_| {
                let view = BlobRef::<N>::verify(&image).expect("a frozen image verifies");
                Pass {
                    checksum: view.len() as u64,
                    units: nodes,
                }
            },
        ),
        size_of::<N>(),
        "node",
        Some(*reach),
    ))
}

fn run_lookup<N: BlobNode>(
    params: &Params,
    plan: &Plan,
    reach: &Reach,
    probes: &[u32],
) -> Result<Outcome, String> {
    if !N::LIMIT.admits(plan.nodes.len(), reach) {
        return Ok(Outcome::Unreachable(*reach));
    }
    // Verification happens once, outside the timed region; the view borrows the local image.
    let image = rebuild::<N>(plan).freeze();
    let view = BlobRef::<N>::verify(&image).expect("a frozen image verifies");
    let root = plan.root().unwrap_or(0);
    Ok(Outcome::from_reps(
        run_reps(
            params,
            plan.nodes.len() as u64,
            || rebuild::<N>(plan),
            |_| lookups(&view, root, probes),
        ),
        size_of::<N>(),
        "lookup",
        Some(*reach),
    ))
}

/// Rebuild-from-plan versus `BlobRef::verify`.
pub struct BlobLoad;

impl Workload for BlobLoad {
    fn name(&self) -> &'static str {
        "blob-load"
    }

    fn variants(&self) -> &'static [&'static str] {
        VARIANTS
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (plan, reach, _) = inputs(params);
        match variant {
            "abs32" => run_load::<Abs32>(params, &plan, &reach),
            "abs16" => run_load::<Abs16>(params, &plan, &reach),
            "rel32" => run_load::<Rel32>(params, &plan, &reach),
            "rel16" => run_load::<Rel16>(params, &plan, &reach),
            "rel8" => run_load::<Rel8>(params, &plan, &reach),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

/// Random lookups through a verified blob view.
pub struct BlobLookup;

impl Workload for BlobLookup {
    fn name(&self) -> &'static str {
        "blob-lookup"
    }

    fn variants(&self) -> &'static [&'static str] {
        VARIANTS
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (plan, reach, probes) = inputs(params);
        match variant {
            "abs32" => run_lookup::<Abs32>(params, &plan, &reach, &probes),
            "abs16" => run_lookup::<Abs16>(params, &plan, &reach, &probes),
            "rel32" => run_lookup::<Rel32>(params, &plan, &reach, &probes),
            "rel16" => run_lookup::<Rel16>(params, &plan, &reach, &probes),
            "rel8" => run_lookup::<Rel8>(params, &plan, &reach, &probes),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::Metrics;

    fn params(n: usize) -> Params {
        Params {
            n,
            reps: 3,
            seed: 2,
            budget: std::time::Duration::from_secs(60),
        }
    }

    fn done(outcome: Outcome) -> Metrics {
        match outcome {
            Outcome::Done(metrics) => metrics,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn view_matches_arena<N: BlobNode>() {
        let plan = Plan::balanced_bst(1000);
        let arena = rebuild::<N>(&plan);
        let image = arena.freeze();
        let view = BlobRef::<N>::verify(&image).unwrap();
        let root = plan.root().unwrap();
        for key in 0..2000 {
            assert_eq!(
                contains(&view, root, key),
                contains(&arena, root, key),
                "key {key}"
            );
            assert_eq!(contains(&view, root, key), key < 1000, "key {key}");
        }
    }

    #[test]
    fn verified_views_answer_lookups_like_the_arena_they_came_from() {
        view_matches_arena::<Abs32>();
        view_matches_arena::<Abs16>();
        view_matches_arena::<Rel32>();
        view_matches_arena::<Rel16>();
    }

    #[test]
    fn blob_load_reports_node_verification_and_rebuild_costs() {
        for variant in ["abs32", "abs16", "rel32", "rel16"] {
            let m = done(BlobLoad.run(variant, &params(1024)).unwrap());
            assert_eq!(m.unit, "node", "{variant}");
            assert!(m.traverse.median > 0.0 && m.build.median > 0.0, "{variant}");
        }
        assert_eq!(
            done(BlobLoad.run("abs32", &params(1024)).unwrap()).node_bytes,
            12
        );
        assert_eq!(
            done(BlobLoad.run("rel16", &params(1024)).unwrap()).node_bytes,
            8
        );
    }

    #[test]
    fn blob_lookup_reports_lookup_cost_through_the_view() {
        let m = done(BlobLookup.run("rel32", &params(1024)).unwrap());
        assert_eq!(m.unit, "lookup");
        assert!(m.traverse.median > 0.0);
    }

    #[test]
    fn rel8_cannot_hold_the_balanced_tree() {
        for workload in workloads() {
            assert!(
                matches!(
                    workload.run("rel8", &params(1024)).unwrap(),
                    Outcome::Unreachable(_)
                ),
                "{}",
                workload.name()
            );
        }
    }

    #[test]
    fn unknown_variants_are_errors_and_both_workloads_register() {
        for workload in workloads() {
            assert!(workload.run("box", &params(16)).is_err());
            assert_eq!(workload.variants(), VARIANTS);
        }
        let names: Vec<&str> = workloads().iter().map(|w| w.name()).collect();
        assert_eq!(names, ["blob-load", "blob-lookup"]);
    }

    #[test]
    fn schemas_are_distinct_per_node_type() {
        let schemas = [
            Abs32::SCHEMA,
            Abs16::SCHEMA,
            Rel32::SCHEMA,
            Rel16::SCHEMA,
            Rel8::SCHEMA,
        ];
        let unique: std::collections::BTreeSet<u64> = schemas.into_iter().collect();
        assert_eq!(unique.len(), 5);
    }
}
