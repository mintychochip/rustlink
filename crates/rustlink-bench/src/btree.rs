//! B-tree workloads: `btree-64` and `btree-256`.
//!
//! A packed, bulk-loaded B+tree of `u32` keys with fixed-size nodes: leaves hold keys only, inner
//! nodes hold separators and child links. The node size is the budget (64 bytes, a cache line, or
//! 256 bytes) and the fanout follows from the link width, so narrower links buy a wider tree:
//!
//! | links | 64-byte inner node | 256-byte inner node |
//! |---|---|---|
//! | pointers, `usize` | 5 children | 21 children |
//! | `u32`, `abs32` | 8 children | 32 children |
//! | `abs16` | 11 children | 43 children |
//!
//! Leaves are identical in every variant (15 or 63 keys), so links only change the inner nodes,
//! the few percent of nodes above the leaves. Lookups are half hits.
//!
//! The variants are `std` (`BTreeSet`), `box` (boxed enum nodes), `usize` and `u32` (indices in
//! one `Vec` per level) and rustlink `abs32` and `abs16`. A leaf and an inner node are different
//! types, and so are the inner nodes of different levels, because a link is typed by the element
//! it leads to: the rustlink tree is a chain of regions, one per level, nested as
//! `InnerTier<InnerTier<..LeafTier..>>`. Relative links need the target in the same region, so
//! there is no `rel` variant.

use std::collections::BTreeSet;

use rustlink::{Arena, OptAbs, Region, Width};
use zerocopy::byteorder::little_endian::{U16, U32};

use crate::measure::{Params, Pass, run_reps};
use crate::rng::Rng;
use crate::workload::{Limit, Outcome, Reach, Workload};

/// The tallest tree the rustlink variants are instantiated for.
const MAX_HEIGHT: usize = 9;

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![Box::new(BTree(Size::N64)), Box::new(BTree(Size::N256))]
}

/// The size of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 64 bytes.
    N64,
    /// 256 bytes.
    N256,
}

/// `n` random distinct keys, sorted (fewer than `n` if the generator repeats itself).
pub fn keys(n: usize, seed: u64) -> Vec<u32> {
    let mut rng = Rng::new(seed);
    let mut keys: Vec<u32> = (0..n).map(|_| rng.next_u32()).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// One probe per key, alternately a present key and a random one.
fn probes(keys: &[u32], seed: u64) -> Vec<u32> {
    let mut rng = Rng::new(seed ^ 0x5EED_B7EE);
    (0..keys.len())
        .map(|i| {
            if i % 2 == 0 {
                keys[rng.below(keys.len() as u64) as usize]
            } else {
                rng.next_u32()
            }
        })
        .collect()
}

/// One level of inner nodes. The children of a node are consecutive nodes of the level below.
#[derive(Default)]
struct Level {
    /// Every node's separator keys, one after another.
    sep: Vec<u32>,
    /// Where each node's separators start in `sep`, and one past the last.
    sep_start: Vec<u32>,
    /// The index of each node's first child.
    kid_first: Vec<u32>,
    /// How many children each node has.
    kid_count: Vec<u16>,
}

impl Level {
    fn len(&self) -> usize {
        self.kid_first.len()
    }

    fn separators(&self, node: usize) -> &[u32] {
        &self.sep[self.sep_start[node] as usize..self.sep_start[node + 1] as usize]
    }
}

/// A tree described independently of any link representation: the leaves with their keys, and the
/// levels of inner nodes above them, bottom first. The last level has one node, the root; a tree
/// of one leaf has no levels. Child `j` of a node holds the keys from separator `j - 1` up to
/// separator `j`.
pub struct Shape {
    leaf_keys: Vec<u32>,
    leaf_start: Vec<u32>,
    levels: Vec<Level>,
}

impl Shape {
    /// A packed tree over the sorted distinct `keys`: leaves of `leaf_cap` keys, inner nodes of
    /// `inner_keys` separators, every node full except the last of its level.
    pub fn bulk(keys: &[u32], leaf_cap: usize, inner_keys: usize) -> Shape {
        assert!(leaf_cap >= 1 && inner_keys >= 1);
        let fanout = inner_keys + 1;
        let leaf_count = keys.len().div_ceil(leaf_cap).max(1);
        let leaf_start: Vec<u32> = (0..=leaf_count)
            .map(|i| (i * leaf_cap).min(keys.len()) as u32)
            .collect();
        // the smallest key under each node of the level being grouped
        let mut mins: Vec<u32> = (0..leaf_count)
            .map(|i| keys.get(i * leaf_cap).copied().unwrap_or(0))
            .collect();
        let mut levels = Vec::new();
        let mut width = leaf_count;
        while width > 1 {
            let nodes = width.div_ceil(fanout);
            let mut level = Level::default();
            let mut next = Vec::with_capacity(nodes);
            for node in 0..nodes {
                let first = node * fanout;
                let count = (width - first).min(fanout);
                level.kid_first.push(first as u32);
                level.kid_count.push(count as u16);
                level.sep_start.push(level.sep.len() as u32);
                level.sep.extend_from_slice(&mins[first + 1..first + count]);
                next.push(mins[first]);
            }
            level.sep_start.push(level.sep.len() as u32);
            levels.push(level);
            mins = next;
            width = nodes;
        }
        Shape {
            leaf_keys: keys.to_vec(),
            leaf_start,
            levels,
        }
    }

    /// Number of inner levels.
    fn height(&self) -> usize {
        self.levels.len()
    }

    fn leaf_count(&self) -> usize {
        self.leaf_start.len() - 1
    }

    fn leaf(&self, index: usize) -> &[u32] {
        &self.leaf_keys[self.leaf_start[index] as usize..self.leaf_start[index + 1] as usize]
    }

    /// The most nodes any one level has: what a region's link width must be able to address.
    fn max_tier(&self) -> usize {
        self.levels
            .iter()
            .map(Level::len)
            .chain([self.leaf_count()])
            .max()
            .unwrap_or(0)
    }

    /// Distances `child index - parent index` of every link, each in its own level.
    fn reach(&self) -> Reach {
        Reach::from_deltas(self.levels.iter().flat_map(|level| {
            (0..level.len()).flat_map(move |node| {
                (0..level.kid_count[node] as usize)
                    .map(move |j| i64::from(level.kid_first[node]) + j as i64 - node as i64)
            })
        }))
    }
}

/// A tree that answers membership.
trait Lookup {
    fn contains(&self, key: u32) -> bool;
}

fn lookups(tree: &dyn Lookup, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|&&key| tree.contains(key)).count();
    Pass {
        checksum: hits as u64,
        units: probes.len() as u64,
    }
}

/// `std::collections::BTreeSet`, built from the sorted keys.
struct StdSet(BTreeSet<u32>);

impl Lookup for StdSet {
    fn contains(&self, key: u32) -> bool {
        self.0.contains(&key)
    }
}

/// A link held as a plain index.
trait Index: Copy {
    const NIL: Self;
    fn from_index(index: usize) -> Self;
    fn index(self) -> usize;
}

impl Index for u32 {
    const NIL: u32 = u32::MAX;

    fn from_index(index: usize) -> u32 {
        index as u32
    }

    fn index(self) -> usize {
        self as usize
    }
}

impl Index for usize {
    const NIL: usize = usize::MAX;

    fn from_index(index: usize) -> usize {
        index
    }

    fn index(self) -> usize {
        self
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PlainLeaf<const KL: usize> {
    len: u16,
    keys: [u32; KL],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PlainInner<L: Index, const K: usize, const F: usize> {
    len: u16,
    keys: [u32; K],
    kids: [L; F],
}

/// Hand-written indices: one `Vec` of leaves and one `Vec` of inner nodes per level.
struct PlainTree<L: Index, const KL: usize, const K: usize, const F: usize> {
    leaves: Vec<PlainLeaf<KL>>,
    levels: Vec<Vec<PlainInner<L, K, F>>>,
}

impl<L: Index, const KL: usize, const K: usize, const F: usize> PlainTree<L, KL, K, F> {
    fn build(shape: &Shape) -> Self {
        let mut leaves = Vec::with_capacity(shape.leaf_count());
        for i in 0..shape.leaf_count() {
            let keys = shape.leaf(i);
            leaves.push(PlainLeaf {
                len: keys.len() as u16,
                keys: std::array::from_fn(|j| keys.get(j).copied().unwrap_or(0)),
            });
        }
        let levels = shape
            .levels
            .iter()
            .map(|level| {
                let mut nodes = Vec::with_capacity(level.len());
                for node in 0..level.len() {
                    let separators = level.separators(node);
                    let (first, count) = (
                        level.kid_first[node] as usize,
                        level.kid_count[node] as usize,
                    );
                    nodes.push(PlainInner {
                        len: separators.len() as u16,
                        keys: std::array::from_fn(|j| separators.get(j).copied().unwrap_or(0)),
                        kids: std::array::from_fn(|j| {
                            if j < count {
                                L::from_index(first + j)
                            } else {
                                L::NIL
                            }
                        }),
                    });
                }
                nodes
            })
            .collect();
        PlainTree { leaves, levels }
    }
}

impl<L: Index, const KL: usize, const K: usize, const F: usize> Lookup for PlainTree<L, KL, K, F> {
    fn contains(&self, key: u32) -> bool {
        let mut node = 0usize;
        for level in self.levels.iter().rev() {
            let inner = &level[node];
            let child = inner.keys[..inner.len as usize].partition_point(|&s| s <= key);
            node = inner.kids[child].index();
        }
        let leaf = &self.leaves[node];
        leaf.keys[..leaf.len as usize].binary_search(&key).is_ok()
    }
}

#[repr(C)]
struct BoxLeaf<const KL: usize> {
    len: u16,
    keys: [u32; KL],
}

#[repr(C)]
struct BoxInner<const KL: usize, const K: usize, const F: usize> {
    len: u16,
    keys: [u32; K],
    kids: [Option<Box<BoxNode<KL, K, F>>>; F],
}

/// What a developer writes for a pointer B-tree: nodes in boxes, an enum for the two kinds.
enum BoxNode<const KL: usize, const K: usize, const F: usize> {
    Leaf(BoxLeaf<KL>),
    Inner(BoxInner<KL, K, F>),
}

struct BoxTree<const KL: usize, const K: usize, const F: usize> {
    root: Option<Box<BoxNode<KL, K, F>>>,
}

impl<const KL: usize, const K: usize, const F: usize> BoxTree<KL, K, F> {
    fn build(shape: &Shape) -> Self {
        let mut below: Vec<Option<Box<BoxNode<KL, K, F>>>> = (0..shape.leaf_count())
            .map(|i| {
                let keys = shape.leaf(i);
                Some(Box::new(BoxNode::Leaf(BoxLeaf {
                    len: keys.len() as u16,
                    keys: std::array::from_fn(|j| keys.get(j).copied().unwrap_or(0)),
                })))
            })
            .collect();
        for level in &shape.levels {
            let mut here = Vec::with_capacity(level.len());
            for node in 0..level.len() {
                let separators = level.separators(node);
                let (first, count) = (
                    level.kid_first[node] as usize,
                    level.kid_count[node] as usize,
                );
                here.push(Some(Box::new(BoxNode::Inner(BoxInner {
                    len: separators.len() as u16,
                    keys: std::array::from_fn(|j| separators.get(j).copied().unwrap_or(0)),
                    kids: std::array::from_fn(|j| {
                        if j < count {
                            below[first + j].take()
                        } else {
                            None
                        }
                    }),
                }))));
            }
            below = here;
        }
        BoxTree {
            root: below.pop().flatten(),
        }
    }
}

impl<const KL: usize, const K: usize, const F: usize> Lookup for BoxTree<KL, K, F> {
    fn contains(&self, key: u32) -> bool {
        let Some(mut node) = self.root.as_deref() else {
            return false;
        };
        loop {
            match node {
                BoxNode::Leaf(leaf) => {
                    return leaf.keys[..leaf.len as usize].binary_search(&key).is_ok();
                }
                BoxNode::Inner(inner) => {
                    let child = inner.keys[..inner.len as usize].partition_point(|&s| s <= key);
                    match inner.kids[child].as_deref() {
                        Some(next) => node = next,
                        None => return false,
                    }
                }
            }
        }
    }
}

/// A leaf: a little-endian count and keys, 1-byte aligned.
struct LeafNode<const KL: usize> {
    len: U16,
    keys: [U32; KL],
}

/// An inner node whose children are `C`s in another region.
struct InnerNode<C, W: Width, const K: usize, const F: usize> {
    len: U16,
    keys: [U32; K],
    kids: [OptAbs<C, W>; F],
}

/// One level of the rustlink tree: its own region, and the tier below it.
trait Tier {
    type Node;

    fn arena(&self) -> &Arena<Self::Node>;
    /// Whether `key` is under `node`, the element at `index` of this tier.
    fn contains_in(&self, node: &Self::Node, index: usize, key: u32) -> bool;
}

trait BuildTier: Tier + Sized {
    /// The tier of inner level `level` (0 for the leaves) of `shape`, with the tiers below it.
    fn build(shape: &Shape, level: usize) -> Self;
}

struct LeafTier<const KL: usize> {
    leaves: Arena<LeafNode<KL>>,
}

impl<const KL: usize> Tier for LeafTier<KL> {
    type Node = LeafNode<KL>;

    fn arena(&self) -> &Arena<LeafNode<KL>> {
        &self.leaves
    }

    fn contains_in(&self, node: &LeafNode<KL>, _index: usize, key: u32) -> bool {
        node.keys[..node.len.get() as usize]
            .binary_search_by(|k| k.get().cmp(&key))
            .is_ok()
    }
}

impl<const KL: usize> BuildTier for LeafTier<KL> {
    fn build(shape: &Shape, _level: usize) -> Self {
        let mut leaves = Arena::with_capacity(shape.leaf_count());
        for i in 0..shape.leaf_count() {
            let keys = shape.leaf(i);
            leaves.push(LeafNode {
                len: U16::new(keys.len() as u16),
                keys: std::array::from_fn(|j| U32::new(keys.get(j).copied().unwrap_or(0))),
            });
        }
        LeafTier { leaves }
    }
}

struct InnerTier<B: Tier, W: Width, const K: usize, const F: usize> {
    nodes: Arena<InnerNode<B::Node, W, K, F>>,
    below: B,
}

impl<B: Tier, W: Width, const K: usize, const F: usize> Tier for InnerTier<B, W, K, F> {
    type Node = InnerNode<B::Node, W, K, F>;

    fn arena(&self) -> &Arena<Self::Node> {
        &self.nodes
    }

    fn contains_in(&self, node: &Self::Node, index: usize, key: u32) -> bool {
        let separators = &node.keys[..node.len.get() as usize];
        let child = separators.partition_point(|s| s.get() <= key);
        match self.below.arena().follow(index, node.kids[child]) {
            Some((below_index, below_node)) => self.below.contains_in(below_node, below_index, key),
            None => false,
        }
    }
}

impl<B: BuildTier, W: Width, const K: usize, const F: usize> BuildTier for InnerTier<B, W, K, F> {
    fn build(shape: &Shape, level: usize) -> Self {
        let below = B::build(shape, level - 1);
        let level = &shape.levels[level - 1];
        let mut nodes = Arena::with_capacity(level.len());
        for node in 0..level.len() {
            let separators = level.separators(node);
            let (first, count) = (
                level.kid_first[node] as usize,
                level.kid_count[node] as usize,
            );
            nodes.push(InnerNode {
                len: U16::new(separators.len() as u16),
                keys: std::array::from_fn(|j| U32::new(separators.get(j).copied().unwrap_or(0))),
                kids: std::array::from_fn(|j| {
                    if j < count {
                        OptAbs::new(Some(first + j)).expect("tier sizes checked")
                    } else {
                        OptAbs::NULL
                    }
                }),
            });
        }
        InnerTier { nodes, below }
    }
}

/// A tree of tiers, entered at the single node of the top one.
struct Rooted<T: Tier> {
    top: T,
}

impl<T: Tier> Lookup for Rooted<T> {
    fn contains(&self, key: u32) -> bool {
        match self.top.arena().get(0) {
            Some(root) => self.top.contains_in(root, 0, key),
            None => false,
        }
    }
}

fn rooted<T: BuildTier + 'static>(shape: &Shape) -> Box<dyn Lookup> {
    Box::new(Rooted {
        top: T::build(shape, shape.height()),
    })
}

/// A function building the rustlink tree of any height up to [`MAX_HEIGHT`]: the nested tier
/// types differ by height, so each height is its own instantiation.
macro_rules! tiers {
    ($name:ident, $leaf:ty, $w:ty, $k:literal, $f:literal) => {
        fn $name(shape: &Shape) -> Box<dyn Lookup> {
            type T0 = $leaf;
            type T1 = InnerTier<T0, $w, $k, $f>;
            type T2 = InnerTier<T1, $w, $k, $f>;
            type T3 = InnerTier<T2, $w, $k, $f>;
            type T4 = InnerTier<T3, $w, $k, $f>;
            type T5 = InnerTier<T4, $w, $k, $f>;
            type T6 = InnerTier<T5, $w, $k, $f>;
            type T7 = InnerTier<T6, $w, $k, $f>;
            type T8 = InnerTier<T7, $w, $k, $f>;
            type T9 = InnerTier<T8, $w, $k, $f>;
            match shape.height() {
                0 => rooted::<T0>(shape),
                1 => rooted::<T1>(shape),
                2 => rooted::<T2>(shape),
                3 => rooted::<T3>(shape),
                4 => rooted::<T4>(shape),
                5 => rooted::<T5>(shape),
                6 => rooted::<T6>(shape),
                7 => rooted::<T7>(shape),
                8 => rooted::<T8>(shape),
                9 => rooted::<T9>(shape),
                height => unreachable!("a tree of height {height} was checked against MAX_HEIGHT"),
            }
        }
    };
}

tiers!(abs32_64, LeafTier<15>, u32, 7, 8);
tiers!(abs16_64, LeafTier<15>, u16, 10, 11);
tiers!(abs32_256, LeafTier<63>, u32, 31, 32);
tiers!(abs16_256, LeafTier<63>, u16, 42, 43);

/// The link representations of the B-tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Std,
    Boxed,
    Usize,
    U32,
    Abs32,
    Abs16,
}

impl Variant {
    #[cfg(test)]
    const ALL: [Variant; 6] = [
        Variant::Std,
        Variant::Boxed,
        Variant::Usize,
        Variant::U32,
        Variant::Abs32,
        Variant::Abs16,
    ];

    fn parse(name: &str) -> Option<Variant> {
        Some(match name {
            "std" => Variant::Std,
            "box" => Variant::Boxed,
            "usize" => Variant::Usize,
            "u32" => Variant::U32,
            "abs32" => Variant::Abs32,
            "abs16" => Variant::Abs16,
            _ => return None,
        })
    }

    /// Keys per leaf and per inner node when a node is `size` bytes; `None` for `std`.
    fn capacities(self, size: Size) -> Option<(usize, usize)> {
        let leaf = match size {
            Size::N64 => 15,
            Size::N256 => 63,
        };
        let inner = match (self, size) {
            (Variant::Std, _) => return None,
            (Variant::Boxed | Variant::Usize, Size::N64) => 4,
            (Variant::Boxed | Variant::Usize, Size::N256) => 20,
            (Variant::U32 | Variant::Abs32, Size::N64) => 7,
            (Variant::U32 | Variant::Abs32, Size::N256) => 31,
            (Variant::Abs16, Size::N64) => 10,
            (Variant::Abs16, Size::N256) => 42,
        };
        Some((leaf, inner))
    }

    fn limit(self) -> Limit {
        match self {
            Variant::Abs16 => Limit::Abs {
                max_elements: <u16 as Width>::MAX,
            },
            _ => Limit::None,
        }
    }

    /// `size_of` one inner node in bytes (one allocation for `box`), 0 for `std`, whose nodes are
    /// private.
    fn node_bytes(self, size: Size) -> usize {
        match (self, size) {
            (Variant::Std, _) => 0,
            (Variant::Boxed, Size::N64) => size_of::<BoxNode<15, 4, 5>>(),
            (Variant::Boxed, Size::N256) => size_of::<BoxNode<63, 20, 21>>(),
            (Variant::Usize, Size::N64) => size_of::<PlainInner<usize, 4, 5>>(),
            (Variant::Usize, Size::N256) => size_of::<PlainInner<usize, 20, 21>>(),
            (Variant::U32, Size::N64) => size_of::<PlainInner<u32, 7, 8>>(),
            (Variant::U32, Size::N256) => size_of::<PlainInner<u32, 31, 32>>(),
            (Variant::Abs32, Size::N64) => size_of::<InnerNode<LeafNode<15>, u32, 7, 8>>(),
            (Variant::Abs32, Size::N256) => size_of::<InnerNode<LeafNode<63>, u32, 31, 32>>(),
            (Variant::Abs16, Size::N64) => size_of::<InnerNode<LeafNode<15>, u16, 10, 11>>(),
            (Variant::Abs16, Size::N256) => size_of::<InnerNode<LeafNode<63>, u16, 42, 43>>(),
        }
    }

    /// The tree over `keys`; `shape` is that tree's shape for the variant's capacities, `None`
    /// only for `std`.
    fn build(self, size: Size, shape: Option<&Shape>, keys: &[u32]) -> Box<dyn Lookup> {
        let shape = || shape.expect("every variant but std has a shape");
        match (self, size) {
            (Variant::Std, _) => Box::new(StdSet(keys.iter().copied().collect())),
            (Variant::Boxed, Size::N64) => Box::new(BoxTree::<15, 4, 5>::build(shape())),
            (Variant::Boxed, Size::N256) => Box::new(BoxTree::<63, 20, 21>::build(shape())),
            (Variant::Usize, Size::N64) => Box::new(PlainTree::<usize, 15, 4, 5>::build(shape())),
            (Variant::Usize, Size::N256) => {
                Box::new(PlainTree::<usize, 63, 20, 21>::build(shape()))
            }
            (Variant::U32, Size::N64) => Box::new(PlainTree::<u32, 15, 7, 8>::build(shape())),
            (Variant::U32, Size::N256) => Box::new(PlainTree::<u32, 63, 31, 32>::build(shape())),
            (Variant::Abs32, Size::N64) => abs32_64(shape()),
            (Variant::Abs32, Size::N256) => abs32_256(shape()),
            (Variant::Abs16, Size::N64) => abs16_64(shape()),
            (Variant::Abs16, Size::N256) => abs16_256(shape()),
        }
    }
}

/// A B-tree of one node size in every link representation, probed with lookups.
pub struct BTree(pub Size);

impl Workload for BTree {
    fn name(&self) -> &'static str {
        match self.0 {
            Size::N64 => "btree-64",
            Size::N256 => "btree-256",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        &["std", "box", "usize", "u32", "abs32", "abs16"]
    }

    fn max_n(&self) -> usize {
        1 << 22
    }

    fn footprint_per_n(&self) -> usize {
        64
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let variant =
            Variant::parse(variant).ok_or_else(|| format!("unknown variant `{variant}`"))?;
        let keys = keys(params.n, params.seed);
        let probes = probes(&keys, params.seed);
        let shape = variant
            .capacities(self.0)
            .map(|(leaf, inner)| Shape::bulk(&keys, leaf, inner));
        let mut reach = Reach::default();
        if let Some(shape) = &shape {
            if shape.height() > MAX_HEIGHT {
                return Err(format!(
                    "a tree of height {} is taller than the {MAX_HEIGHT} supported",
                    shape.height()
                ));
            }
            reach = shape.reach();
            if !variant.limit().admits(shape.max_tier(), &reach) {
                return Ok(Outcome::Unreachable(reach));
            }
        }
        let result = run_reps(
            params,
            keys.len() as u64,
            || variant.build(self.0, shape.as_ref(), &keys),
            |tree| lookups(&**tree, &probes),
        );
        Ok(Outcome::from_reps(
            result,
            variant.node_bytes(self.0),
            "lookup",
            Some(reach),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::measure::Params;
    use crate::workload::{Outcome, Workload};

    /// (keys per leaf, keys per inner node) of every node size and link width.
    const CAPACITIES: [(usize, usize); 6] =
        [(15, 4), (15, 7), (15, 10), (63, 20), (63, 31), (63, 42)];
    const COUNTS: [usize; 9] = [0, 1, 14, 15, 16, 120, 121, 1000, 5000];
    const SIZES: [Size; 2] = [Size::N64, Size::N256];

    /// Distinct sorted keys with gaps, so that `key + 1` is a miss.
    fn spread(count: usize) -> Vec<u32> {
        (0..count as u32).map(|i| i * 3 + 1).collect()
    }

    /// Every key, its neighbours and the extremes.
    fn probes_for(keys: &[u32]) -> Vec<u32> {
        let mut probes = vec![0, 1, 2, u32::MAX];
        for &key in keys {
            probes.extend([key - 1, key, key + 1]);
        }
        probes
    }

    /// A lookup that descends the shape itself, independent of every representation.
    fn shape_contains(shape: &Shape, key: u32) -> bool {
        let mut node = 0usize;
        for level in shape.levels.iter().rev() {
            let separators = level.separators(node);
            let child = separators.partition_point(|&s| s <= key);
            node = level.kid_first[node] as usize + child;
        }
        shape.leaf(node).binary_search(&key).is_ok()
    }

    fn built(variant: Variant, size: Size, keys: &[u32]) -> Box<dyn Lookup> {
        let shape = variant
            .capacities(size)
            .map(|(leaf, inner)| Shape::bulk(keys, leaf, inner));
        variant.build(size, shape.as_ref(), keys)
    }

    #[test]
    fn levels_shrink_by_the_fanout_until_a_single_root() {
        let shape = Shape::bulk(&spread(1000), 15, 7); // 67 leaves, 8 children per inner node
        assert_eq!(shape.leaf_count(), 67);
        let sizes: Vec<usize> = shape.levels.iter().map(Level::len).collect();
        assert_eq!(sizes, [9, 2, 1]);
        assert_eq!(shape.height(), 3);
        assert_eq!(shape.max_tier(), 67);
    }

    #[test]
    fn a_descent_through_the_shape_finds_exactly_the_keys() {
        for (leaf, inner) in CAPACITIES {
            for count in COUNTS {
                let keys = spread(count);
                let shape = Shape::bulk(&keys, leaf, inner);
                for key in probes_for(&keys) {
                    assert_eq!(
                        shape_contains(&shape, key),
                        keys.binary_search(&key).is_ok(),
                        "{count} keys, capacities {leaf}/{inner}, key {key}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_variant_finds_exactly_the_keys() {
        for size in SIZES {
            for variant in Variant::ALL {
                for count in COUNTS {
                    let keys = spread(count);
                    let tree = built(variant, size, &keys);
                    for key in probes_for(&keys) {
                        assert_eq!(
                            tree.contains(key),
                            keys.binary_search(&key).is_ok(),
                            "{variant:?} {size:?} {count} keys, key {key}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nodes_fit_their_budget_and_use_most_of_it() {
        fn fits<T>(budget: usize) {
            let bytes = size_of::<T>();
            assert!(
                bytes <= budget && bytes + 8 > budget,
                "{} is {bytes} bytes of {budget}",
                std::any::type_name::<T>()
            );
        }
        fits::<LeafNode<15>>(64);
        fits::<PlainLeaf<15>>(64);
        fits::<BoxLeaf<15>>(64);
        fits::<InnerNode<LeafNode<15>, u32, 7, 8>>(64);
        fits::<InnerNode<LeafNode<15>, u16, 10, 11>>(64);
        fits::<PlainInner<u32, 7, 8>>(64);
        fits::<PlainInner<usize, 4, 5>>(64);
        fits::<BoxInner<15, 4, 5>>(64);

        fits::<LeafNode<63>>(256);
        fits::<PlainLeaf<63>>(256);
        fits::<BoxLeaf<63>>(256);
        fits::<InnerNode<LeafNode<63>, u32, 31, 32>>(256);
        fits::<InnerNode<LeafNode<63>, u16, 42, 43>>(256);
        fits::<PlainInner<u32, 31, 32>>(256);
        fits::<PlainInner<usize, 20, 21>>(256);
        fits::<BoxInner<63, 20, 21>>(256);
    }

    #[test]
    fn abs16_cannot_address_a_tier_of_more_than_65535_nodes() {
        let outcome = |n| {
            let params = Params {
                n,
                reps: 2,
                seed: 1,
                budget: Duration::from_secs(60),
            };
            BTree(Size::N64).run("abs16", &params).unwrap()
        };
        assert!(matches!(outcome(1 << 16), Outcome::Done(_)));
        // 2^20 random keys fill about 70,000 leaves of 15
        assert!(matches!(outcome(1 << 20), Outcome::Unreachable(_)));
    }
}
