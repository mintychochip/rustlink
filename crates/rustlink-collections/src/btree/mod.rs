//! A B+tree map and set stored in one arena and linked by narrow indices.
//!
//! Leaves hold the entries and are chained in key order; inner nodes hold separator keys and
//! child links. Every node lives in one `Vec` and children are addressed with `W`-wide
//! [`OptAbs`] links, so `W` bounds the number of *nodes*, not entries: with 16-entry leaves a
//! `u16` tree addresses roughly a million entries. A removed node's slot is reused.
//!
//! Unlike `std::collections::BTreeMap`, keys and values are moved with `Vec::remove`, so `K` need
//! not be `Default`; keys are cloned into inner-node separators, so `K: Clone`.

mod map;
mod set;

pub use map::BTreeMap;
pub use set::BTreeSet;

use core::borrow::Borrow;
use rustlink::{OptAbs, Width};

/// Maximum entries in a leaf. A full leaf splits into two halves of `MIN_LEAF`.
const LEAF_CAP: usize = 16;
/// Maximum separators in an inner node; it then has `INNER_CAP + 1` children.
const INNER_CAP: usize = 16;
/// Minimum live entries or separators in a non-root node.
const MIN_LEAF: usize = LEAF_CAP / 2;
const MIN_INNER: usize = INNER_CAP / 2;
/// Deepest path a tree with a 32-bit link width can reach, with room to spare.
const MAX_HEIGHT: usize = 16;

pub(crate) type Link<K, V, W> = OptAbs<Node<K, V, W>, W>;

pub(crate) fn link<K, V, W: Width>(index: usize) -> Link<K, V, W> {
    OptAbs::new(Some(index)).expect("node index is inside the width's range")
}

#[derive(Clone)]
pub(crate) enum Node<K, V, W: Width> {
    Leaf(Leaf<K, V, W>),
    Inner(Inner<K, V, W>),
    /// A slot on the free list. Values have been dropped.
    Free(Link<K, V, W>),
}

#[derive(Clone)]
pub(crate) struct Leaf<K, V, W: Width> {
    pub(crate) entries: alloc::vec::Vec<(K, V)>,
    pub(crate) prev: Link<K, V, W>,
    pub(crate) next: Link<K, V, W>,
}

#[derive(Clone)]
pub(crate) struct Inner<K, V, W: Width> {
    /// `keys[i]` is the first key of `kids[i + 1]`; there is no key for `kids[0]`.
    pub(crate) keys: alloc::vec::Vec<K>,
    pub(crate) kids: alloc::vec::Vec<Link<K, V, W>>,
}

/// Where the search went: one `(inner node, child position)` per level.
pub(crate) struct Path {
    pub(crate) levels: [(usize, usize); MAX_HEIGHT],
    pub(crate) len: usize,
}

impl Path {
    pub(crate) fn new() -> Self {
        Path {
            levels: [(0, 0); MAX_HEIGHT],
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, node: usize, child: usize) {
        self.levels[self.len] = (node, child);
        self.len += 1;
    }

    pub(crate) fn top(&self) -> (usize, usize) {
        self.levels[self.len - 1]
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

/// `keys[i]` is the first key of `kids[i + 1]`, so a probe belongs to the first child whose
/// separator is greater.
pub(crate) fn child_pos<K, Q>(keys: &[K], key: &Q) -> usize
where
    K: Borrow<Q> + Ord,
    Q: Ord + ?Sized,
{
    keys.partition_point(|separator| separator.borrow() <= key)
}

#[cfg(test)]
mod tests;
