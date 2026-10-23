//! Node-graph workloads: `ngraph-mesh`, `ngraph-banded`, `ngraph-random` and `ngraph-chase`.
//!
//! Every node is `{ payload: u32, DEGREE links }` and a link may be null, so links are most of a
//! node: 16 of 20 bytes with 32-bit links, 8 of 12 with 16-bit links, 32 of 40 with pointers. The
//! graphs differ in where links go:
//!
//! * `ngraph-mesh`: the four neighbours on a grid, at distance 1 and the grid width;
//! * `ngraph-banded`: four random targets within [`BAND`] nodes, as after reordering a graph to
//!   narrow its bandwidth;
//! * `ngraph-random` and `ngraph-chase`: four uniformly random targets.
//!
//! The first three run a *sweep*: every node in index order sums its own and its neighbours'
//! payloads, so the loads are independent and the CPU can overlap them. `ngraph-chase` runs a
//! *chase* on the random graph: one dependent walk of `n` steps whose next link is picked from the
//! payload just read, so every step waits for the previous load and the time is the memory
//! latency of the structure's footprint.
//!
//! The variants are `rc` (an `Rc` per node with `Weak` links, the safe idiom for cyclic graphs),
//! `usize` and `u32` (hand-written indices in a `Vec`, what petgraph does), and rustlink `abs*`
//! and `rel*` links in an [`Arena`], followed through [`Cursor::follow`]. `abs32-idx` and `rel16-idx`
//! read the same arenas without a `Cursor`: the public [`Link::target`] decodes the link and the
//! index is checked and used on the arena's slice. They separate what the `Cursor` access path
//! costs from what the link representation costs.

#![allow(clippy::unnecessary_cast)]

use std::cell::OnceCell;
use std::rc::{Rc, Weak};

use rustlink::{Arena, Cursor, Link, OptAbs, OptRel, Region, Target, Width};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::measure::{Params, Pass, run_reps};
use crate::rng::Rng;
use crate::workload::{Limit, Outcome, Reach, Workload};

/// Links per node.
pub const DEGREE: usize = 4;
/// A missing link in a plan.
pub const NIL: u32 = u32::MAX;
/// How far from its node a banded graph's link may reach, in either direction.
pub const BAND: usize = 1000;
/// Multiplier of the chase's running hash.
const CHASE_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(NodeGraph(Which::Mesh)),
        Box::new(NodeGraph(Which::Banded)),
        Box::new(NodeGraph(Which::Random)),
        Box::new(NodeGraph(Which::Chase)),
    ]
}

/// Where the links of a generated graph go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The four neighbours on a grid.
    Mesh,
    /// Random targets within [`BAND`] nodes.
    Banded,
    /// Uniformly random targets.
    Random,
}

/// A graph described independently of any link representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodePlan {
    /// The payload of every node.
    pub payload: Vec<u32>,
    /// The targets of every node's links, [`NIL`] where a link is missing.
    pub edges: Vec<[u32; DEGREE]>,
}

/// The width of the grid of an `n`-node mesh: the integer square root, at least 1.
pub fn mesh_width(n: usize) -> usize {
    let mut width = (n as f64).sqrt() as usize;
    while width * width > n {
        width -= 1;
    }
    while (width + 1) * (width + 1) <= n {
        width += 1;
    }
    width.max(1)
}

fn mesh_edges(n: usize) -> Vec<[u32; DEGREE]> {
    let width = mesh_width(n);
    (0..n)
        .map(|i| {
            let x = i % width;
            let up = if i >= width { (i - width) as u32 } else { NIL };
            let left = if x > 0 { (i - 1) as u32 } else { NIL };
            let right = if x + 1 < width && i + 1 < n {
                (i + 1) as u32
            } else {
                NIL
            };
            let down = if i + width < n {
                (i + width) as u32
            } else {
                NIL
            };
            [up, left, right, down]
        })
        .collect()
}

fn banded_edges(node: usize, n: usize, rng: &mut Rng) -> [u32; DEGREE] {
    let low = node.saturating_sub(BAND);
    let high = (node + BAND).min(n - 1);
    let candidates = high - low; // the band without the node itself
    if candidates == 0 {
        return [NIL; DEGREE];
    }
    std::array::from_fn(|_| {
        let mut target = low + rng.below(candidates as u64) as usize;
        if target >= node {
            target += 1;
        }
        target as u32
    })
}

fn random_edges(node: usize, n: usize, rng: &mut Rng) -> [u32; DEGREE] {
    if n < 2 {
        return [NIL; DEGREE];
    }
    std::array::from_fn(|_| {
        let mut target = rng.below((n - 1) as u64) as usize;
        if target >= node {
            target += 1;
        }
        target as u32
    })
}

impl NodePlan {
    /// A graph of `n` nodes. Links never point at their own node.
    pub fn generate(kind: Kind, n: usize, seed: u64) -> NodePlan {
        let mut rng = Rng::new(seed);
        let payload: Vec<u32> = (0..n).map(|_| rng.next_u32()).collect();
        let edges = match kind {
            Kind::Mesh => mesh_edges(n),
            Kind::Banded => (0..n).map(|i| banded_edges(i, n, &mut rng)).collect(),
            Kind::Random => (0..n).map(|i| random_edges(i, n, &mut rng)).collect(),
        };
        NodePlan { payload, edges }
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// True if the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// Distances `target - node` of every link, which decide whether a relative link fits.
    pub fn reach(&self) -> Reach {
        Reach::from_deltas(self.edges.iter().enumerate().flat_map(|(from, targets)| {
            targets
                .iter()
                .filter(|&&target| target != NIL)
                .map(move |&target| i64::from(target) - from as i64)
        }))
    }
}

/// A node graph in one link representation.
trait NodeStore: Sized {
    /// A reference to one node: an index, a cursor or a reference-counted pointer.
    type Ref<'a>
    where
        Self: 'a;

    /// `size_of` one node in bytes (one allocation for `Rc`).
    const NODE_BYTES: usize;
    /// What the representation can address.
    const LIMIT: Limit;

    fn build(plan: &NodePlan) -> Self;
    fn len(&self) -> usize;
    fn node(&self, index: usize) -> Option<Self::Ref<'_>>;
    fn payload(&self, node: &Self::Ref<'_>) -> u32;
    /// The node the `k`-th link of `node` leads to, `None` if the link is null.
    fn link<'a>(&'a self, node: &Self::Ref<'a>, k: usize) -> Option<Self::Ref<'a>>;
}

/// Sums every node's payload and its neighbours', in node order. One unit is a link followed.
fn sweep<S: NodeStore>(graph: &S) -> Pass {
    let (mut sum, mut edges) = (0u64, 0u64);
    for index in 0..graph.len() {
        let Some(node) = graph.node(index) else {
            continue;
        };
        sum = sum.wrapping_add(u64::from(graph.payload(&node)));
        for k in 0..DEGREE {
            if let Some(target) = graph.link(&node, k) {
                sum = sum.wrapping_add(u64::from(graph.payload(&target)));
                edges += 1;
            }
        }
    }
    Pass {
        checksum: sum,
        units: edges,
    }
}

/// A dependent walk of `n` steps from node 0. The first link tried at each node comes from a hash
/// of the payloads seen so far, so a step cannot start before the previous one has loaded its
/// node. A node without links sends the walk to a hash-chosen node. One unit is a link followed.
fn chase<S: NodeStore>(graph: &S) -> Pass {
    let n = graph.len();
    let Some(mut at) = graph.node(0) else {
        return Pass {
            checksum: 0,
            units: 0,
        };
    };
    let (mut acc, mut steps) = (0u64, 0u64);
    for _ in 0..n {
        acc = acc
            .wrapping_mul(CHASE_MIX)
            .wrapping_add(u64::from(graph.payload(&at)));
        let first = (acc >> 33) as usize % DEGREE;
        let mut next = None;
        for j in 0..DEGREE {
            next = graph.link(&at, (first + j) % DEGREE);
            if next.is_some() {
                break;
            }
        }
        at = match next {
            Some(target) => {
                steps += 1;
                target
            }
            None => graph
                .node((acc >> 17) as usize % n)
                .expect("an index below the length"),
        };
    }
    Pass {
        checksum: acc,
        units: steps,
    }
}

macro_rules! plain_graph {
    ($(#[$doc:meta])* $store:ident, $node:ident, $link:ty) => {
        #[derive(Clone, Copy)]
        #[repr(C)]
        struct $node {
            payload: u32,
            edges: [$link; DEGREE],
        }

        $(#[$doc])*
        struct $store {
            nodes: Vec<$node>,
        }

        impl NodeStore for $store {
            type Ref<'a> = usize;

            const NODE_BYTES: usize = size_of::<$node>();
            const LIMIT: Limit = Limit::None;

            fn build(plan: &NodePlan) -> Self {
                let mut nodes = Vec::with_capacity(plan.len());
                for (&payload, targets) in plan.payload.iter().zip(&plan.edges) {
                    nodes.push($node {
                        payload,
                        edges: targets.map(|t| if t == NIL { <$link>::MAX } else { t as $link }),
                    });
                }
                $store { nodes }
            }

            fn len(&self) -> usize {
                self.nodes.len()
            }

            fn node(&self, index: usize) -> Option<usize> {
                (index < self.nodes.len()).then_some(index)
            }

            fn payload(&self, node: &usize) -> u32 {
                self.nodes[*node].payload
            }

            fn link(&self, node: &usize, k: usize) -> Option<usize> {
                let target = self.nodes[*node].edges[k];
                (target != <$link>::MAX).then_some(target as usize)
            }
        }
    };
}

plain_graph! {
    /// Nodes in a `Vec`, links are `usize` indices (40-byte nodes).
    UsizeGraph, UsizeNode, usize
}

plain_graph! {
    /// Nodes in a `Vec`, links are `u32` indices (20-byte nodes): what a Rust developer writes by
    /// hand, and the baseline `abs32` must match.
    U32Graph, U32Node, u32
}

macro_rules! narrow_graph {
    (
        $(#[$doc:meta])* $store:ident, $node:ident, $link:ident, $w:ty,
        $limit:expr, |$owner:ident, $target:ident| $make:expr
    ) => {
        /// One node: a little-endian payload and narrow links, 1-byte aligned.
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
        #[repr(C)]
        struct $node {
            payload: U32,
            edges: [$link<$node, $w>; DEGREE],
        }

        $(#[$doc])*
        struct $store {
            arena: Arena<$node>,
        }

        impl NodeStore for $store {
            type Ref<'a> = Cursor<'a, $node>;

            const NODE_BYTES: usize = size_of::<$node>();
            const LIMIT: Limit = $limit;

            fn build(plan: &NodePlan) -> Self {
                let mut arena = Arena::with_capacity(plan.len());
                for (index, (&payload, targets)) in
                    plan.payload.iter().zip(&plan.edges).enumerate()
                {
                    arena.push($node {
                        payload: U32::new(payload),
                        edges: std::array::from_fn(|k| {
                            let ($owner, $target) = (
                                index,
                                (targets[k] != NIL).then_some(targets[k] as usize),
                            );
                            $make.expect("reach checked by the workload")
                        }),
                    });
                }
                $store { arena }
            }

            fn len(&self) -> usize {
                self.arena.len()
            }

            fn node(&self, index: usize) -> Option<Cursor<'_, $node>> {
                self.arena.cursor(index)
            }

            fn payload(&self, node: &Cursor<'_, $node>) -> u32 {
                node.get().payload.get()
            }

            fn link<'a>(&'a self, node: &Cursor<'a, $node>, k: usize) -> Option<Cursor<'a, $node>> {
                node.follow(node.get().edges[k])
            }
        }
    };
}

narrow_graph! {
    /// `OptAbs<_, u32>` links (20-byte nodes).
    Abs32Graph, Abs32Node, OptAbs, u32,
    Limit::Abs { max_elements: <u32 as Width>::MAX },
    |_owner, target| OptAbs::new(target)
}

narrow_graph! {
    /// `OptAbs<_, u16>` links (12-byte nodes).
    Abs16Graph, Abs16Node, OptAbs, u16,
    Limit::Abs { max_elements: <u16 as Width>::MAX },
    |_owner, target| OptAbs::new(target)
}

narrow_graph! {
    /// `OptRel<_, u32>` links (20-byte nodes).
    Rel32Graph, Rel32Node, OptRel, u32,
    Limit::Rel { max_delta: <u32 as Width>::SIGNED_MAX },
    |owner, target| OptRel::new(owner, target)
}

narrow_graph! {
    /// `OptRel<_, u16>` links (12-byte nodes).
    Rel16Graph, Rel16Node, OptRel, u16,
    Limit::Rel { max_delta: <u16 as Width>::SIGNED_MAX },
    |owner, target| OptRel::new(owner, target)
}

narrow_graph! {
    /// `OptRel<_, u8>` links (8-byte nodes).
    Rel8Graph, Rel8Node, OptRel, u8,
    Limit::Rel { max_delta: <u8 as Width>::SIGNED_MAX },
    |owner, target| OptRel::new(owner, target)
}

/// Rustlink links read without a `Cursor`: the arena of `$inner`, a link decoded by
/// [`Link::target`] and the index it gives checked against the slice it indexes.
macro_rules! indexed_graph {
    ($(#[$doc:meta])* $store:ident, $inner:ident) => {
        $(#[$doc])*
        struct $store($inner);

        impl NodeStore for $store {
            type Ref<'a> = usize;

            const NODE_BYTES: usize = <$inner as NodeStore>::NODE_BYTES;
            const LIMIT: Limit = <$inner as NodeStore>::LIMIT;

            fn build(plan: &NodePlan) -> Self {
                $store(<$inner as NodeStore>::build(plan))
            }

            fn len(&self) -> usize {
                self.0.arena.len()
            }

            fn node(&self, index: usize) -> Option<usize> {
                (index < self.0.arena.len()).then_some(index)
            }

            fn payload(&self, node: &usize) -> u32 {
                self.0.arena.as_slice()[*node].payload.get()
            }

            fn link(&self, node: &usize, k: usize) -> Option<usize> {
                let nodes = self.0.arena.as_slice();
                match nodes[*node].edges[k].target(*node) {
                    Target::Index(target) if target < nodes.len() => Some(target),
                    _ => None,
                }
            }
        }
    };
}

indexed_graph! {
    /// The `abs32` arena read by decoding links and indexing the slice (20-byte nodes).
    Abs32Idx, Abs32Graph
}

indexed_graph! {
    /// The `rel16` arena read by decoding links and indexing the slice (12-byte nodes).
    Rel16Idx, Rel16Graph
}

/// One node of the `Rc` graph. The links are weak because a graph has cycles, and are set once
/// after every node exists.
struct RcNode {
    payload: u32,
    edges: OnceCell<[Weak<RcNode>; DEGREE]>,
}

/// Nodes owned by a `Vec`, linked by `Weak` pointers: the safe way to write a cyclic graph with
/// node objects.
struct RcGraph {
    owners: Vec<Rc<RcNode>>,
}

/// A node reached by index, borrowed from the owners, or by following a link, upgraded to an `Rc`.
enum RcRef<'a> {
    Owned(&'a RcNode),
    Upgraded(Rc<RcNode>),
}

impl RcRef<'_> {
    fn node(&self) -> &RcNode {
        match self {
            RcRef::Owned(node) => node,
            RcRef::Upgraded(node) => node,
        }
    }
}

impl NodeStore for RcGraph {
    type Ref<'a> = RcRef<'a>;

    const NODE_BYTES: usize = 2 * size_of::<usize>() + size_of::<RcNode>();
    const LIMIT: Limit = Limit::None;

    fn build(plan: &NodePlan) -> Self {
        let mut owners = Vec::with_capacity(plan.len());
        for &payload in &plan.payload {
            owners.push(Rc::new(RcNode {
                payload,
                edges: OnceCell::new(),
            }));
        }
        for (owner, targets) in owners.iter().zip(&plan.edges) {
            let links = std::array::from_fn(|k| {
                if targets[k] == NIL {
                    Weak::new()
                } else {
                    Rc::downgrade(&owners[targets[k] as usize])
                }
            });
            owner
                .edges
                .set(links)
                .expect("the links of a node are set once");
        }
        RcGraph { owners }
    }

    fn len(&self) -> usize {
        self.owners.len()
    }

    fn node(&self, index: usize) -> Option<RcRef<'_>> {
        self.owners.get(index).map(|node| RcRef::Owned(node))
    }

    fn payload(&self, node: &RcRef<'_>) -> u32 {
        node.node().payload
    }

    fn link<'a>(&'a self, node: &RcRef<'a>, k: usize) -> Option<RcRef<'a>> {
        node.node().edges.get()?[k].upgrade().map(RcRef::Upgraded)
    }
}

/// The operation a workload runs.
#[derive(Clone, Copy)]
enum Op {
    Sweep,
    Chase,
}

/// The four workloads of this module.
#[derive(Clone, Copy)]
pub enum Which {
    /// `ngraph-mesh`.
    Mesh,
    /// `ngraph-banded`.
    Banded,
    /// `ngraph-random`.
    Random,
    /// `ngraph-chase`.
    Chase,
}

impl Which {
    fn kind(self) -> Kind {
        match self {
            Which::Mesh => Kind::Mesh,
            Which::Banded => Kind::Banded,
            Which::Random | Which::Chase => Kind::Random,
        }
    }

    fn op(self) -> Op {
        match self {
            Which::Chase => Op::Chase,
            _ => Op::Sweep,
        }
    }
}

/// A node graph in every link representation, swept or chased.
pub struct NodeGraph(pub Which);

fn run_store<S: NodeStore>(
    which: Which,
    params: &Params,
    plan: &NodePlan,
    reach: &Reach,
) -> Result<Outcome, String> {
    if !S::LIMIT.admits(plan.len(), reach) {
        return Ok(Outcome::Unreachable(*reach));
    }
    let nodes = plan.len() as u64;
    let (result, unit) = match which.op() {
        Op::Sweep => (
            run_reps(params, nodes, || S::build(plan), |graph| sweep(graph)),
            "edge",
        ),
        Op::Chase => (
            run_reps(params, nodes, || S::build(plan), |graph| chase(graph)),
            "step",
        ),
    };
    Ok(Outcome::from_reps(
        result,
        S::NODE_BYTES,
        unit,
        Some(*reach),
    ))
}

impl Workload for NodeGraph {
    fn name(&self) -> &'static str {
        match self.0 {
            Which::Mesh => "ngraph-mesh",
            Which::Banded => "ngraph-banded",
            Which::Random => "ngraph-random",
            Which::Chase => "ngraph-chase",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        &[
            "rc",
            "usize",
            "u32",
            "abs32",
            "abs32-idx",
            "abs16",
            "rel32",
            "rel16",
            "rel16-idx",
            "rel8",
        ]
    }

    fn max_n(&self) -> usize {
        1 << 22
    }

    fn footprint_per_n(&self) -> usize {
        192
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let plan = NodePlan::generate(self.0.kind(), params.n, params.seed);
        let reach = plan.reach();
        match variant {
            "rc" => run_store::<RcGraph>(self.0, params, &plan, &reach),
            "usize" => run_store::<UsizeGraph>(self.0, params, &plan, &reach),
            "u32" => run_store::<U32Graph>(self.0, params, &plan, &reach),
            "abs32" => run_store::<Abs32Graph>(self.0, params, &plan, &reach),
            "abs32-idx" => run_store::<Abs32Idx>(self.0, params, &plan, &reach),
            "abs16" => run_store::<Abs16Graph>(self.0, params, &plan, &reach),
            "rel32" => run_store::<Rel32Graph>(self.0, params, &plan, &reach),
            "rel16" => run_store::<Rel16Graph>(self.0, params, &plan, &reach),
            "rel16-idx" => run_store::<Rel16Idx>(self.0, params, &plan, &reach),
            "rel8" => run_store::<Rel8Graph>(self.0, params, &plan, &reach),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const KINDS: [Kind; 3] = [Kind::Mesh, Kind::Banded, Kind::Random];
    const SIZES: [usize; 5] = [1, 2, 7, 1000, 1024];

    /// The sweep computed on the plan itself, independent of every representation.
    fn reference_sweep(plan: &NodePlan) -> Pass {
        let (mut sum, mut edges) = (0u64, 0u64);
        for (payload, targets) in plan.payload.iter().zip(&plan.edges) {
            sum = sum.wrapping_add(u64::from(*payload));
            for &target in targets.iter().filter(|&&t| t != NIL) {
                sum = sum.wrapping_add(u64::from(plan.payload[target as usize]));
                edges += 1;
            }
        }
        Pass {
            checksum: sum,
            units: edges,
        }
    }

    /// The dependent walk computed on the plan itself.
    fn reference_chase(plan: &NodePlan) -> Pass {
        let n = plan.len();
        let (mut at, mut acc, mut steps) = (0usize, 0u64, 0u64);
        for _ in 0..n {
            acc = acc
                .wrapping_mul(CHASE_MIX)
                .wrapping_add(u64::from(plan.payload[at]));
            let first = (acc >> 33) as usize % DEGREE;
            let next = (0..DEGREE)
                .map(|j| plan.edges[at][(first + j) % DEGREE])
                .find(|&t| t != NIL);
            match next {
                Some(target) => {
                    at = target as usize;
                    steps += 1;
                }
                None => at = (acc >> 17) as usize % n,
            }
        }
        Pass {
            checksum: acc,
            units: steps,
        }
    }

    fn agrees<S: NodeStore>(plan: &NodePlan, what: &str) {
        if !S::LIMIT.admits(plan.len(), &plan.reach()) {
            return;
        }
        let store = S::build(plan);
        assert_eq!(sweep(&store), reference_sweep(plan), "{what}: sweep");
        assert_eq!(chase(&store), reference_chase(plan), "{what}: chase");
    }

    #[test]
    fn mesh_links_are_the_four_grid_neighbours_up_left_right_down() {
        let plan = NodePlan::generate(Kind::Mesh, 16, 1);
        assert_eq!(mesh_width(16), 4);
        assert_eq!(plan.edges[5], [1, 4, 6, 9]);
        assert_eq!(plan.edges[0], [NIL, NIL, 1, 4]);
        assert_eq!(plan.edges[15], [11, 14, NIL, NIL]);
    }

    #[test]
    fn a_mesh_that_is_not_a_square_ends_in_a_partial_row_and_stays_symmetric() {
        let n = 1000; // 31 wide, 33 rows, the last of 8 nodes
        let plan = NodePlan::generate(Kind::Mesh, n, 1);
        assert_eq!(mesh_width(n), 31);
        assert_eq!(plan.edges[999], [968, 998, NIL, NIL]);
        for (from, targets) in plan.edges.iter().enumerate() {
            for &to in targets.iter().filter(|&&t| t != NIL) {
                assert!((to as usize) < n);
                assert!(
                    plan.edges[to as usize].contains(&(from as u32)),
                    "{from} -> {to} has no way back"
                );
            }
        }
    }

    #[test]
    fn banded_links_stay_within_the_band_and_never_loop_back_to_their_node() {
        let plan = NodePlan::generate(Kind::Banded, 5000, 1);
        let mut widest = 0usize;
        for (from, targets) in plan.edges.iter().enumerate() {
            for &to in targets {
                let to = to as usize;
                assert!(to < 5000 && to != from);
                widest = widest.max(to.abs_diff(from));
            }
        }
        assert!(widest <= BAND);
        assert!(
            widest > BAND / 2,
            "the band is used, not just the neighbours"
        );
        assert_eq!(NodePlan::generate(Kind::Banded, 2, 1).edges[0], [1; DEGREE]);
        assert_eq!(
            NodePlan::generate(Kind::Banded, 1, 1).edges[0],
            [NIL; DEGREE]
        );
    }

    #[test]
    fn random_links_cover_the_whole_range_and_never_loop_back_to_their_node() {
        let plan = NodePlan::generate(Kind::Random, 3000, 1);
        let mut highest = 0;
        for (from, targets) in plan.edges.iter().enumerate() {
            for &to in targets {
                assert!((to as usize) < 3000 && to as usize != from);
                highest = highest.max(to);
            }
        }
        assert!(highest > 2900);
        assert_eq!(
            NodePlan::generate(Kind::Random, 1, 1).edges[0],
            [NIL; DEGREE]
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_plan_and_another_seed_a_different_one() {
        for kind in KINDS {
            let plan = NodePlan::generate(kind, 500, 7);
            assert_eq!(plan, NodePlan::generate(kind, 500, 7));
            assert_ne!(plan.payload, NodePlan::generate(kind, 500, 8).payload);
        }
    }

    #[test]
    fn the_reach_of_a_mesh_is_its_width_and_counts_every_directed_link() {
        let reach = NodePlan::generate(Kind::Mesh, 4096, 1).reach(); // 64 x 64
        assert_eq!(reach.max_abs_delta, 64);
        assert_eq!(reach.links, 2 * (64 * 63 + 64 * 63));
    }

    #[test]
    fn every_representation_computes_what_the_plan_says() {
        for kind in KINDS {
            for n in SIZES {
                let plan = NodePlan::generate(kind, n, 3);
                let what = format!("{kind:?} n={n}");
                agrees::<RcGraph>(&plan, &what);
                agrees::<UsizeGraph>(&plan, &what);
                agrees::<U32Graph>(&plan, &what);
                agrees::<Abs32Graph>(&plan, &what);
                agrees::<Abs16Graph>(&plan, &what);
                agrees::<Rel32Graph>(&plan, &what);
                agrees::<Rel16Graph>(&plan, &what);
                agrees::<Rel8Graph>(&plan, &what);
                agrees::<Abs32Idx>(&plan, &what);
                agrees::<Rel16Idx>(&plan, &what);
            }
        }
    }

    #[test]
    fn a_chase_follows_a_link_at_every_step_when_no_node_is_a_dead_end() {
        for kind in KINDS {
            let plan = NodePlan::generate(kind, 1000, 1);
            assert_eq!(chase(&U32Graph::build(&plan)).units, 1000, "{kind:?}");
        }
    }

    #[test]
    fn node_sizes_are_the_documented_layouts() {
        assert_eq!(size_of::<UsizeNode>(), 40);
        assert_eq!(size_of::<U32Node>(), 20);
        assert_eq!(size_of::<Abs32Node>(), 20);
        assert_eq!(size_of::<Rel32Node>(), 20);
        assert_eq!(size_of::<Abs16Node>(), 12);
        assert_eq!(size_of::<Rel16Node>(), 12);
        assert_eq!(size_of::<Rel8Node>(), 8);
        // two reference counts, the payload, its padding and four weak links
        assert_eq!(RcGraph::NODE_BYTES, 16 + 4 + 4 + 32);
        assert_eq!(Abs32Idx::NODE_BYTES, 20);
        assert_eq!(Rel16Idx::NODE_BYTES, 12);
    }

    #[test]
    fn variants_whose_links_cannot_represent_the_graph_are_reported_unreachable() {
        let outcome = |which, variant: &str, n| {
            let params = Params {
                n,
                reps: 2,
                seed: 1,
                budget: Duration::from_secs(60),
            };
            NodeGraph(which).run(variant, &params).unwrap()
        };
        // mesh neighbours are w apart: 64 fits rel8, 128 does not
        assert!(matches!(
            outcome(Which::Mesh, "rel8", 4096),
            Outcome::Done(_)
        ));
        assert!(matches!(
            outcome(Which::Mesh, "rel8", 16384),
            Outcome::Unreachable(_)
        ));
        assert!(matches!(
            outcome(Which::Mesh, "rel16", 65536),
            Outcome::Done(_)
        ));
        // the band is 1000 wide
        assert!(matches!(
            outcome(Which::Banded, "rel8", 4096),
            Outcome::Unreachable(_)
        ));
        assert!(matches!(
            outcome(Which::Banded, "rel16", 65536),
            Outcome::Done(_)
        ));
        // random targets are up to n away, and abs16 addresses 65,535 nodes
        assert!(matches!(
            outcome(Which::Random, "rel16", 16384),
            Outcome::Done(_)
        ));
        assert!(matches!(
            outcome(Which::Random, "rel16", 1 << 17),
            Outcome::Unreachable(_)
        ));
        assert!(matches!(
            outcome(Which::Random, "abs16", 1 << 16),
            Outcome::Unreachable(_)
        ));
    }
}
