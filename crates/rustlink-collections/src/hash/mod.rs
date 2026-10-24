//! A chained hash map and set stored in one arena and linked by narrow indices.
//!
//! Buckets hold `W`-wide links to entries; an entry chains to the next entry in its bucket.
//! Entries live in one `Vec` and removed slots are reused, so an insert never allocates a node
//! and a removal never moves the remaining entries. The default hasher is FNV-1a, which is fast
//! but not collision-resistant; pass your own [`BuildHasher`] when that matters.

mod map;
mod set;

pub use map::HashMap;
pub use set::HashSet;

use core::hash::{BuildHasherDefault, Hasher};

use rustlink::{OptAbs, Width};

/// FNV-1a over the bytes `Hasher::write` receives.
#[derive(Clone, Copy, Debug)]
pub struct FnvHasher {
    state: u64,
}

impl Default for FnvHasher {
    fn default() -> Self {
        FnvHasher {
            state: Self::OFFSET,
        }
    }
}

impl FnvHasher {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
}

impl Hasher for FnvHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut state = self.state;
        for &byte in bytes {
            state ^= u64::from(byte);
            state = state.wrapping_mul(Self::PRIME);
        }
        self.state = state;
    }

    fn finish(&self) -> u64 {
        self.state
    }
}

/// [`BuildHasher`](core::hash::BuildHasher) for [`FnvHasher`], the default.
pub type FnvBuildHasher = BuildHasherDefault<FnvHasher>;

type Link<K, V, W> = OptAbs<Entry<K, V, W>, W>;

pub(crate) fn link<K, V, W: Width>(index: usize) -> Link<K, V, W> {
    OptAbs::new(Some(index)).expect("entry index is inside the width's range")
}

/// One entry slot. A free slot keeps its `next` as the free-list link and its key and value
/// defaulted.
#[derive(Clone)]
pub(crate) struct Entry<K, V, W: Width> {
    pub(crate) key: K,
    pub(crate) value: V,
    pub(crate) next: Link<K, V, W>,
}

/// Buckets in a table that has not grown yet.
pub(crate) const MIN_BUCKETS: usize = 8;
/// Grow when `len + 1` would exceed three quarters of the bucket count.
pub(crate) const LOAD_NUM: usize = 3;
pub(crate) const LOAD_DEN: usize = 4;

#[cfg(test)]
mod tests;
