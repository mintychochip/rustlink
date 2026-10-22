//! Graph workloads: `graph-csr`, `graph-linked-seq` and `graph-linked-shuf`.
//!
//! All three run a breadth-first search from vertex 0 over the same random graph (out-degree 1 to
//! 7, average 4, so about `4 * n` edges) and differ only in how adjacency is stored. `n` is the
//! number of vertices; heap and build figures are normalised per vertex.

use rustlink::{Abs, Arena, OptAbs, OptRel, Region, Width};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use crate::measure::{Params, Pass, run_reps};
use crate::rng::Rng;
use crate::workload::{Limit, Outcome, Reach, Workload};

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(GraphCsr),
        Box::new(GraphLinked { shuffled: false }),
        Box::new(GraphLinked { shuffled: true }),
    ]
}

/// Marker type for vertex ids stored as typed links.
pub struct Vertex;

/// A random graph in flat form: per-vertex out-degrees and all targets in vertex order.
pub struct Graph {
    /// Out-degree of each vertex.
    pub degrees: Vec<u8>,
    /// Every edge target, grouped by source vertex.
    pub targets: Vec<u32>,
}

impl Graph {
    /// Generates a graph with `vertices` vertices. Targets may repeat and may be self loops.
    pub fn generate(vertices: usize, seed: u64) -> Graph {
        let mut rng = Rng::new(seed);
        let mut degrees = Vec::with_capacity(vertices);
        let mut targets = Vec::with_capacity(vertices * 4);
        for _ in 0..vertices {
            let degree = 1 + rng.below(7) as u8;
            degrees.push(degree);
            for _ in 0..degree {
                targets.push(rng.below(vertices as u64) as u32);
            }
        }
        Graph { degrees, targets }
    }

    /// Number of vertices.
    pub fn vertices(&self) -> usize {
        self.degrees.len()
    }

    /// Number of edges.
    pub fn edges(&self) -> usize {
        self.targets.len()
    }
}

/// Neighbour enumeration, the only thing BFS needs from a representation.
trait Adjacency {
    fn for_each_neighbor(&self, vertex: usize, f: impl FnMut(usize));
}

fn bfs<A: Adjacency>(adjacency: &A, vertices: usize) -> Pass {
    if vertices == 0 {
        return Pass {
            checksum: 0,
            units: 0,
        };
    }
    let mut dist = vec![u32::MAX; vertices];
    let mut queue: Vec<u32> = Vec::with_capacity(vertices);
    dist[0] = 0;
    queue.push(0);
    let (mut head, mut edges) = (0usize, 0u64);
    while head < queue.len() {
        let u = queue[head] as usize;
        head += 1;
        let next = dist[u] + 1;
        adjacency.for_each_neighbor(u, |w| {
            edges += 1;
            if dist[w] == u32::MAX {
                dist[w] = next;
                queue.push(w as u32);
            }
        });
    }
    let checksum = dist
        .iter()
        .filter(|&&d| d != u32::MAX)
        .fold(0u64, |acc, &d| {
            acc.wrapping_add(u64::from(d)).wrapping_add(1)
        });
    Pass {
        checksum,
        units: edges,
    }
}

// ---------------------------------------------------------------------------------------------
// graph-csr
// ---------------------------------------------------------------------------------------------

/// Compressed sparse row storage.
trait CsrStore: Adjacency + Sized {
    /// Bytes of one edge entry.
    const ENTRY_BYTES: usize;
    fn build(graph: &Graph) -> Self;
}

fn offsets_of(graph: &Graph) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(graph.degrees.iter().scan(0usize, |sum, &degree| {
        *sum += usize::from(degree);
        Some(*sum)
    }))
}

struct CsrUsize {
    offsets: Vec<usize>,
    targets: Vec<usize>,
}

impl Adjacency for CsrUsize {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        for &target in &self.targets[self.offsets[vertex]..self.offsets[vertex + 1]] {
            f(target);
        }
    }
}

impl CsrStore for CsrUsize {
    const ENTRY_BYTES: usize = size_of::<usize>();

    fn build(graph: &Graph) -> Self {
        CsrUsize {
            offsets: offsets_of(graph).collect(),
            targets: graph.targets.iter().map(|&t| t as usize).collect(),
        }
    }
}

struct CsrU32 {
    offsets: Vec<u32>,
    targets: Vec<u32>,
}

impl Adjacency for CsrU32 {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        let range = self.offsets[vertex] as usize..self.offsets[vertex + 1] as usize;
        for &target in &self.targets[range] {
            f(target as usize);
        }
    }
}

impl CsrStore for CsrU32 {
    const ENTRY_BYTES: usize = size_of::<u32>();

    fn build(graph: &Graph) -> Self {
        CsrU32 {
            offsets: offsets_of(graph).map(|o| o as u32).collect(),
            targets: graph.targets.clone(),
        }
    }
}

/// Control: typed vertex ids must cost the same as plain `u32`.
struct CsrAbs32 {
    offsets: Vec<u32>,
    targets: Vec<Abs<Vertex, u32>>,
}

impl Adjacency for CsrAbs32 {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        let range = self.offsets[vertex] as usize..self.offsets[vertex + 1] as usize;
        for target in &self.targets[range] {
            f(target.index());
        }
    }
}

impl CsrStore for CsrAbs32 {
    const ENTRY_BYTES: usize = size_of::<Abs<Vertex, u32>>();

    fn build(graph: &Graph) -> Self {
        CsrAbs32 {
            offsets: offsets_of(graph).map(|o| o as u32).collect(),
            targets: graph
                .targets
                .iter()
                .map(|&t| Abs::new(t as usize).expect("vertex ids fit in u32"))
                .collect(),
        }
    }
}

fn run_csr<S: CsrStore>(params: &Params, graph: &Graph) -> Result<Outcome, String> {
    let vertices = graph.vertices();
    Ok(Outcome::from_reps(
        run_reps(
            params,
            vertices as u64,
            || S::build(graph),
            |store| bfs(store, vertices),
        ),
        S::ENTRY_BYTES,
        "edge",
        None,
    ))
}

/// BFS over CSR arrays. Variants: `usize`, `u32`, `abs32` (typed ids, a control for `u32`).
pub struct GraphCsr;

impl Workload for GraphCsr {
    fn name(&self) -> &'static str {
        "graph-csr"
    }

    fn variants(&self) -> &'static [&'static str] {
        &["usize", "u32", "abs32"]
    }

    fn max_n(&self) -> usize {
        1 << 22
    }

    fn footprint_per_n(&self) -> usize {
        192
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let graph = Graph::generate(params.n, params.seed);
        match variant {
            "usize" => run_csr::<CsrUsize>(params, &graph),
            "u32" => run_csr::<CsrU32>(params, &graph),
            "abs32" => run_csr::<CsrAbs32>(params, &graph),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// graph-linked-*
// ---------------------------------------------------------------------------------------------

/// The linked edge lists of a graph, independent of link representation. Edges live in "slots";
/// every vertex's edges form a chain through `next`.
pub struct EdgePlan {
    /// First slot of each vertex's chain.
    pub heads: Vec<Option<u32>>,
    /// `(target vertex, next slot in the chain)` for every slot.
    pub edges: Vec<(u32, Option<u32>)>,
}

impl EdgePlan {
    /// Lays the graph's edges out in slots. Sequential: a vertex's edges are consecutive slots,
    /// so every `next` is the following slot. Shuffled: slots are a random permutation, so
    /// `next` distances are random.
    pub fn new(graph: &Graph, shuffled: bool, seed: u64) -> EdgePlan {
        let slot_count = graph.edges();
        let mut slot_of: Vec<u32> = (0..slot_count as u32).collect();
        if shuffled {
            Rng::new(seed ^ 0xD6E8_FEB8_6659_FD93).shuffle(&mut slot_of);
        }
        let mut heads = vec![None; graph.vertices()];
        let mut edges = vec![(0u32, None); slot_count];
        let mut logical = 0usize;
        for (vertex, &degree) in graph.degrees.iter().enumerate() {
            let mut previous: Option<usize> = None;
            for k in 0..usize::from(degree) {
                let slot = slot_of[logical + k] as usize;
                edges[slot].0 = graph.targets[logical + k];
                match previous {
                    None => heads[vertex] = Some(slot as u32),
                    Some(p) => edges[p].1 = Some(slot as u32),
                }
                previous = Some(slot);
            }
            logical += usize::from(degree);
        }
        EdgePlan { heads, edges }
    }

    /// Distances `next slot - slot` of every chain link.
    pub fn reach(&self) -> Reach {
        Reach::from_deltas(
            self.edges
                .iter()
                .enumerate()
                .filter_map(|(slot, (_, next))| next.map(|n| i64::from(n) - slot as i64)),
        )
    }
}

/// Per-vertex singly linked edge lists in one representation.
trait LinkedStore: Adjacency + Sized {
    /// `size_of` one edge node.
    const EDGE_BYTES: usize;
    /// What the `next` links can address (counted in edges).
    const LIMIT: Limit;
    fn build(plan: &EdgePlan) -> Self;
}

struct BoxEdge {
    to: u32,
    next: Option<Box<BoxEdge>>,
}

struct LinkedBox {
    heads: Vec<Option<Box<BoxEdge>>>,
}

impl Drop for LinkedBox {
    /// Drops chains iteratively so a long chain cannot overflow the call stack.
    fn drop(&mut self) {
        for head in self.heads.drain(..) {
            let mut current = head;
            while let Some(mut edge) = current {
                current = edge.next.take();
            }
        }
    }
}

impl Adjacency for LinkedBox {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        let mut edge = self.heads[vertex].as_deref();
        while let Some(e) = edge {
            f(e.to as usize);
            edge = e.next.as_deref();
        }
    }
}

impl LinkedStore for LinkedBox {
    const EDGE_BYTES: usize = size_of::<BoxEdge>();
    const LIMIT: Limit = Limit::None;

    fn build(plan: &EdgePlan) -> Self {
        let mut chain: Vec<u32> = Vec::new();
        let heads = plan
            .heads
            .iter()
            .map(|&head| {
                chain.clear();
                let mut current = head;
                while let Some(slot) = current {
                    chain.push(slot);
                    current = plan.edges[slot as usize].1;
                }
                let mut next = None;
                for &slot in chain.iter().rev() {
                    next = Some(Box::new(BoxEdge {
                        to: plan.edges[slot as usize].0,
                        next,
                    }));
                }
                next
            })
            .collect();
        LinkedBox { heads }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct UsizeEdge {
    to: u32,
    next: usize,
}

struct LinkedUsize {
    heads: Vec<usize>,
    edges: Vec<UsizeEdge>,
}

impl Adjacency for LinkedUsize {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        let mut at = self.heads[vertex];
        while at != usize::MAX {
            let edge = self.edges[at];
            f(edge.to as usize);
            at = edge.next;
        }
    }
}

impl LinkedStore for LinkedUsize {
    const EDGE_BYTES: usize = size_of::<UsizeEdge>();
    const LIMIT: Limit = Limit::None;

    fn build(plan: &EdgePlan) -> Self {
        LinkedUsize {
            heads: plan
                .heads
                .iter()
                .map(|h| h.map_or(usize::MAX, |s| s as usize))
                .collect(),
            edges: plan
                .edges
                .iter()
                .map(|&(to, next)| UsizeEdge {
                    to,
                    next: next.map_or(usize::MAX, |s| s as usize),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct U32Edge {
    to: u32,
    next: u32,
}

/// The hand-written baseline: 8-byte edges with `u32` links.
struct LinkedU32 {
    heads: Vec<u32>,
    edges: Vec<U32Edge>,
}

impl Adjacency for LinkedU32 {
    fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
        let mut at = self.heads[vertex];
        while at != u32::MAX {
            let edge = self.edges[at as usize];
            f(edge.to as usize);
            at = edge.next;
        }
    }
}

impl LinkedStore for LinkedU32 {
    const EDGE_BYTES: usize = size_of::<U32Edge>();
    const LIMIT: Limit = Limit::None;

    fn build(plan: &EdgePlan) -> Self {
        LinkedU32 {
            heads: plan.heads.iter().map(|h| h.unwrap_or(u32::MAX)).collect(),
            edges: plan
                .edges
                .iter()
                .map(|&(to, next)| U32Edge {
                    to,
                    next: next.unwrap_or(u32::MAX),
                })
                .collect(),
        }
    }
}

macro_rules! linked_narrow {
    (
        $(#[$doc:meta])* $store:ident, $edge:ident, $link:ident, $w:ty,
        $limit:expr, |$owner:ident, $next:ident| $make:expr
    ) => {
        /// One edge: a typed target vertex and a narrow link to the next edge of the chain.
        #[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
        #[repr(C)]
        struct $edge {
            to: Abs<Vertex, u32>,
            next: $link<$edge, $w>,
        }

        $(#[$doc])*
        struct $store {
            heads: Vec<OptAbs<$edge, u32>>,
            edges: Arena<$edge>,
        }

        impl Adjacency for $store {
            fn for_each_neighbor(&self, vertex: usize, mut f: impl FnMut(usize)) {
                let mut cursor = self.heads[vertex].get().and_then(|slot| self.edges.cursor(slot));
                while let Some(at) = cursor {
                    let edge = at.get();
                    f(edge.to.index());
                    cursor = at.follow(edge.next);
                }
            }
        }

        impl LinkedStore for $store {
            const EDGE_BYTES: usize = size_of::<$edge>();
            const LIMIT: Limit = $limit;

            fn build(plan: &EdgePlan) -> Self {
                let mut edges = Arena::with_capacity(plan.edges.len());
                for (slot, &(to, next)) in plan.edges.iter().enumerate() {
                    let ($owner, $next) = (slot, next.map(|s| s as usize));
                    edges.push($edge {
                        to: Abs::new(to as usize).expect("vertex ids fit in u32"),
                        next: $make.expect("reach checked by the workload"),
                    });
                }
                let heads = plan
                    .heads
                    .iter()
                    .map(|h| OptAbs::new(h.map(|s| s as usize)).expect("edge slots fit in u32"))
                    .collect();
                $store { heads, edges }
            }
        }
    };
}

linked_narrow! {
    /// `next` is `OptAbs<_, u32>`: 8-byte edges.
    LinkedAbs32, AbsEdge32, OptAbs, u32,
    Limit::Abs { max_elements: <u32 as Width>::MAX },
    |_owner, next| OptAbs::new(next)
}

linked_narrow! {
    /// `next` is `OptAbs<_, u16>`: 6-byte edges, at most 65,535 edges.
    LinkedAbs16, AbsEdge16, OptAbs, u16,
    Limit::Abs { max_elements: <u16 as Width>::MAX },
    |_owner, next| OptAbs::new(next)
}

linked_narrow! {
    /// `next` is `OptRel<_, u32>`: 8-byte edges.
    LinkedRel32, RelEdge32, OptRel, u32,
    Limit::Rel { max_delta: <u32 as Width>::SIGNED_MAX },
    |owner, next| OptRel::new(owner, next)
}

linked_narrow! {
    /// `next` is `OptRel<_, u16>`: 6-byte edges.
    LinkedRel16, RelEdge16, OptRel, u16,
    Limit::Rel { max_delta: <u16 as Width>::SIGNED_MAX },
    |owner, next| OptRel::new(owner, next)
}

linked_narrow! {
    /// `next` is `OptRel<_, u8>`: 5-byte edges.
    LinkedRel8, RelEdge8, OptRel, u8,
    Limit::Rel { max_delta: <u8 as Width>::SIGNED_MAX },
    |owner, next| OptRel::new(owner, next)
}

fn run_linked<S: LinkedStore>(
    params: &Params,
    vertices: usize,
    plan: &EdgePlan,
    reach: &Reach,
) -> Result<Outcome, String> {
    if !S::LIMIT.admits(plan.edges.len(), reach) {
        return Ok(Outcome::Unreachable(*reach));
    }
    Ok(Outcome::from_reps(
        run_reps(
            params,
            vertices as u64,
            || S::build(plan),
            |store| bfs(store, vertices),
        ),
        S::EDGE_BYTES,
        "edge",
        Some(*reach),
    ))
}

/// BFS over per-vertex linked edge lists. `shuffled` selects worst-case slot locality.
pub struct GraphLinked {
    shuffled: bool,
}

impl Workload for GraphLinked {
    fn name(&self) -> &'static str {
        if self.shuffled {
            "graph-linked-shuf"
        } else {
            "graph-linked-seq"
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        &[
            "box", "usize", "u32", "abs32", "abs16", "rel32", "rel16", "rel8",
        ]
    }

    fn max_n(&self) -> usize {
        1 << 22
    }

    fn footprint_per_n(&self) -> usize {
        192
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let graph = Graph::generate(params.n, params.seed);
        let plan = EdgePlan::new(&graph, self.shuffled, params.seed);
        let reach = plan.reach();
        let v = graph.vertices();
        match variant {
            "box" => run_linked::<LinkedBox>(params, v, &plan, &reach),
            "usize" => run_linked::<LinkedUsize>(params, v, &plan, &reach),
            "u32" => run_linked::<LinkedU32>(params, v, &plan, &reach),
            "abs32" => run_linked::<LinkedAbs32>(params, v, &plan, &reach),
            "abs16" => run_linked::<LinkedAbs16>(params, v, &plan, &reach),
            "rel32" => run_linked::<LinkedRel32>(params, v, &plan, &reach),
            "rel16" => run_linked::<LinkedRel16>(params, v, &plan, &reach),
            "rel8" => run_linked::<LinkedRel8>(params, v, &plan, &reach),
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
            seed: 5,
            budget: std::time::Duration::from_secs(60),
        }
    }

    fn done(outcome: Outcome) -> Metrics {
        match outcome {
            Outcome::Done(metrics) => metrics,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// BFS written directly over the flat graph, independent of every representation.
    fn reference(graph: &Graph) -> Pass {
        let n = graph.vertices();
        let mut starts = vec![0usize; n + 1];
        for v in 0..n {
            starts[v + 1] = starts[v] + usize::from(graph.degrees[v]);
        }
        let mut dist = vec![u32::MAX; n];
        let mut queue = std::collections::VecDeque::from([0usize]);
        dist[0] = 0;
        let mut edges = 0u64;
        while let Some(u) = queue.pop_front() {
            for &t in &graph.targets[starts[u]..starts[u + 1]] {
                edges += 1;
                if dist[t as usize] == u32::MAX {
                    dist[t as usize] = dist[u] + 1;
                    queue.push_back(t as usize);
                }
            }
        }
        let checksum = dist
            .iter()
            .filter(|&&d| d != u32::MAX)
            .fold(0u64, |acc, &d| {
                acc.wrapping_add(u64::from(d)).wrapping_add(1)
            });
        Pass {
            checksum,
            units: edges,
        }
    }

    #[test]
    fn generator_is_deterministic_and_has_the_documented_shape() {
        let a = Graph::generate(2000, 9);
        let b = Graph::generate(2000, 9);
        assert_eq!(
            (a.degrees.clone(), a.targets.clone()),
            (b.degrees, b.targets)
        );
        assert!(a.degrees.iter().all(|&d| (1..=7).contains(&d)));
        assert!(a.targets.iter().all(|&t| (t as usize) < 2000));
        let average = a.edges() as f64 / a.vertices() as f64;
        assert!((3.5..4.5).contains(&average), "average degree {average}");
        assert_eq!(
            a.edges(),
            a.degrees.iter().map(|&d| usize::from(d)).sum::<usize>()
        );
        assert_ne!(a.targets, Graph::generate(2000, 10).targets);
    }

    #[test]
    fn sequential_plans_chain_to_the_next_slot_and_shuffled_plans_are_permutations() {
        let graph = Graph::generate(500, 3);
        let seq = EdgePlan::new(&graph, false, 3);
        for (slot, (_, next)) in seq.edges.iter().enumerate() {
            if let Some(n) = next {
                assert_eq!(*n as usize, slot + 1);
            }
        }
        assert_eq!(seq.reach().max_abs_delta, 1);

        let shuf = EdgePlan::new(&graph, true, 3);
        let again = EdgePlan::new(&graph, true, 3);
        assert_eq!(shuf.heads, again.heads);
        assert_eq!(shuf.edges, again.edges);
        // Every slot is reachable exactly once by following the chains from the heads.
        let mut seen = vec![0u32; shuf.edges.len()];
        for head in &shuf.heads {
            let mut current = *head;
            while let Some(slot) = current {
                seen[slot as usize] += 1;
                current = shuf.edges[slot as usize].1;
            }
        }
        assert!(seen.iter().all(|&c| c == 1));
        assert!(shuf.reach().max_abs_delta > 1000);
    }

    #[test]
    fn csr_variants_match_the_reference_bfs() {
        for n in [1024, 4096] {
            let graph = Graph::generate(n, 5);
            let expected = reference(&graph);
            assert!(
                expected.checksum > n as u64,
                "BFS should reach most vertices"
            );
            assert_eq!(bfs(&CsrUsize::build(&graph), n), expected);
            assert_eq!(bfs(&CsrU32::build(&graph), n), expected);
            assert_eq!(bfs(&CsrAbs32::build(&graph), n), expected);
        }
    }

    #[test]
    fn linked_variants_match_the_reference_bfs_for_both_layouts() {
        let n = 1024;
        let graph = Graph::generate(n, 5);
        let expected = reference(&graph);
        for shuffled in [false, true] {
            let plan = EdgePlan::new(&graph, shuffled, 5);
            assert_eq!(bfs(&LinkedBox::build(&plan), n), expected);
            assert_eq!(bfs(&LinkedUsize::build(&plan), n), expected);
            assert_eq!(bfs(&LinkedU32::build(&plan), n), expected);
            assert_eq!(bfs(&LinkedAbs32::build(&plan), n), expected);
            assert_eq!(bfs(&LinkedRel32::build(&plan), n), expected);
            if LinkedAbs16::LIMIT.admits(plan.edges.len(), &plan.reach()) {
                assert_eq!(bfs(&LinkedAbs16::build(&plan), n), expected);
            }
            if LinkedRel16::LIMIT.admits(plan.edges.len(), &plan.reach()) {
                assert_eq!(bfs(&LinkedRel16::build(&plan), n), expected);
            }
            if !shuffled {
                assert_eq!(bfs(&LinkedRel8::build(&plan), n), expected);
            }
        }
    }

    #[test]
    fn edge_node_sizes_match_the_documented_layouts() {
        assert_eq!(LinkedBox::EDGE_BYTES, 16);
        assert_eq!(LinkedUsize::EDGE_BYTES, 16);
        assert_eq!(LinkedU32::EDGE_BYTES, 8);
        assert_eq!(LinkedAbs32::EDGE_BYTES, 8);
        assert_eq!(LinkedRel32::EDGE_BYTES, 8);
        assert_eq!(LinkedAbs16::EDGE_BYTES, 6);
        assert_eq!(LinkedRel16::EDGE_BYTES, 6);
        assert_eq!(LinkedRel8::EDGE_BYTES, 5);
        assert_eq!(
            CsrUsize::ENTRY_BYTES + CsrU32::ENTRY_BYTES + CsrAbs32::ENTRY_BYTES,
            16
        );
    }

    #[test]
    fn csr_workload_reports_every_variant() {
        let sizes: Vec<(&str, usize)> = GraphCsr
            .variants()
            .iter()
            .map(|&v| {
                let m = done(GraphCsr.run(v, &params(1024)).unwrap());
                assert_eq!(m.unit, "edge");
                assert!(m.traverse.median > 0.0 && m.build.median > 0.0);
                (v, m.node_bytes)
            })
            .collect();
        assert_eq!(sizes, [("usize", 8), ("u32", 4), ("abs32", 4)]);
    }

    #[test]
    fn sequential_layout_fits_every_variant_including_rel8() {
        let workload = GraphLinked { shuffled: false };
        assert_eq!(workload.name(), "graph-linked-seq");
        for &variant in workload.variants() {
            let m = done(workload.run(variant, &params(1024)).unwrap());
            assert_eq!(m.unit, "edge");
            assert!(m.traverse.median > 0.0, "{variant}");
        }
    }

    #[test]
    fn abs16_depends_on_the_edge_count() {
        let workload = GraphLinked { shuffled: false };
        assert!(matches!(
            workload.run("abs16", &params(1024)).unwrap(),
            Outcome::Done(_)
        ));
        assert!(matches!(
            workload.run("abs16", &params(1 << 15)).unwrap(), // about 131,000 edges
            Outcome::Unreachable(_)
        ));
    }

    #[test]
    fn shuffled_layout_defeats_narrow_relative_links() {
        let workload = GraphLinked { shuffled: true };
        assert_eq!(workload.name(), "graph-linked-shuf");
        for variant in ["rel8", "rel16"] {
            assert!(matches!(
                workload.run(variant, &params(1 << 14)).unwrap(),
                Outcome::Unreachable(_)
            ));
        }
        assert!(matches!(
            workload.run("rel32", &params(1 << 14)).unwrap(),
            Outcome::Done(_)
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
    fn dropping_a_long_boxed_chain_does_not_overflow_the_stack() {
        let mut next = None;
        for to in 0..200_000u32 {
            next = Some(Box::new(BoxEdge { to, next }));
        }
        drop(LinkedBox { heads: vec![next] });
    }

    #[test]
    fn registers_three_workloads_with_the_documented_limits() {
        let all = workloads();
        let names: Vec<&str> = all.iter().map(|w| w.name()).collect();
        assert_eq!(
            names,
            ["graph-csr", "graph-linked-seq", "graph-linked-shuf"]
        );
        assert!(
            all.iter()
                .all(|w| w.max_n() == 1 << 22 && w.footprint_per_n() == 192)
        );
    }

    #[test]
    fn a_single_vertex_graph_is_handled() {
        let graph = Graph::generate(1, 1);
        assert_eq!(bfs(&CsrU32::build(&graph), 1).checksum, 1);
        assert_eq!(
            bfs(&CsrU32::build(&Graph::generate(0, 1)), 0),
            Pass {
                checksum: 0,
                units: 0
            }
        );
    }
}
