//! The set half of the chained hash table.

use core::borrow::Borrow;
use core::fmt;
use core::hash::{BuildHasher, Hash};
use core::iter::FusedIterator;

use rustlink::Width;

use super::map;
use crate::error::{Full, MESSAGE};

/// A hash set backed by a [`HashMap`](super::HashMap) with `()` values.
///
/// Free slots hold a default key, so insertion and removal require `T: Default`. `W` bounds the
/// number of live entries plus free slots. Iteration order is slot order, not insertion order.
///
/// ```
/// use rustlink_collections::HashSet;
///
/// let mut set: HashSet<u32, u16> = HashSet::new();
/// assert!(set.insert(2));
/// assert!(!set.insert(2));
/// assert!(set.contains(&2));
/// assert!(set.remove(&2));
/// assert!(set.is_empty());
/// ```
#[derive(Clone)]
pub struct HashSet<T, W: Width = u32, S = super::FnvBuildHasher> {
    map: map::HashMap<T, (), W, S>,
}

impl<T, W: Width> HashSet<T, W> {
    /// An empty set with the default FNV hasher. Does not allocate.
    pub fn new() -> Self {
        HashSet {
            map: map::HashMap::new(),
        }
    }

    /// An empty set with room for `capacity` keys and the default FNV hasher.
    pub fn with_capacity(capacity: usize) -> Self {
        HashSet {
            map: map::HashMap::with_capacity(capacity),
        }
    }
}

impl<T, W: Width, S> HashSet<T, W, S> {
    /// An empty set using `hasher`.
    pub fn with_hasher(hasher: S) -> Self {
        HashSet {
            map: map::HashMap::with_hasher(hasher),
        }
    }

    /// An empty set with room for `capacity` keys.
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        HashSet {
            map: map::HashMap::with_capacity_and_hasher(capacity, hasher),
        }
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if the set has no keys.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Removes every key. Keeps the allocations.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// An iterator over the keys in slot order.
    pub fn iter(&self) -> Iter<'_, T, W> {
        Iter {
            iter: self.map.iter(),
        }
    }
}

impl<T, W: Width, S> HashSet<T, W, S>
where
    T: Eq + Hash + Default,
    S: BuildHasher,
{
    /// Adds `value`, or returns it inside [`Full`] if the width cannot address another slot.
    pub fn try_insert(&mut self, value: T) -> Result<bool, Full<T>> {
        match self.map.try_insert(value, ()) {
            Ok(previous) => Ok(previous.is_none()),
            Err(Full((value, ()))) => Err(Full(value)),
        }
    }

    /// Adds `value`; returns true if it was absent.
    ///
    /// # Panics
    ///
    /// If `W` cannot address another slot. Use [`try_insert`](Self::try_insert) to handle it.
    pub fn insert(&mut self, value: T) -> bool {
        self.try_insert(value).expect(MESSAGE)
    }

    /// Removes `value`; returns true if it was present.
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.remove(value).is_some()
    }
}

impl<T, W: Width, S> HashSet<T, W, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    /// True if the set holds `value`.
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(value)
    }
}

impl<T, W: Width, S: Default> Default for HashSet<T, W, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<T: fmt::Debug, W: Width, S> fmt::Debug for HashSet<T, W, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T, W, S> PartialEq for HashSet<T, W, S>
where
    T: Eq + Hash,
    W: Width,
    S: BuildHasher,
{
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|key| other.contains(key))
    }
}

impl<T: Eq + Hash, W: Width, S: BuildHasher> Eq for HashSet<T, W, S> {}

impl<T, W, S> Extend<T> for HashSet<T, W, S>
where
    T: Eq + Hash + Default,
    W: Width,
    S: BuildHasher,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.insert(value);
        }
    }
}

impl<T, W, S> FromIterator<T> for HashSet<T, W, S>
where
    T: Eq + Hash + Default,
    W: Width,
    S: BuildHasher + Default,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut set = Self::with_capacity_and_hasher(iter.size_hint().0, S::default());
        set.extend(iter);
        set
    }
}

/// An iterator over the keys of a [`HashSet`], created by [`HashSet::iter`].
pub struct Iter<'a, T, W: Width> {
    iter: map::Iter<'a, T, (), W>,
}

impl<T, W: Width> Clone for Iter<'_, T, W> {
    fn clone(&self) -> Self {
        Iter {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, T, W: Width> Iterator for Iter<'a, T, W> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        self.iter.next().map(|(key, ())| key)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T, W: Width> ExactSizeIterator for Iter<'_, T, W> {}
impl<T, W: Width> FusedIterator for Iter<'_, T, W> {}

impl<'a, T, W: Width, S> IntoIterator for &'a HashSet<T, W, S> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T, W>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An owning iterator over the keys of a [`HashSet`], in slot order.
pub struct IntoIter<T, W: Width> {
    iter: map::IntoIter<T, (), W>,
}

impl<T, W: Width> Iterator for IntoIter<T, W> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.iter.next().map(|(key, ())| key)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T, W: Width> ExactSizeIterator for IntoIter<T, W> {}
impl<T, W: Width> FusedIterator for IntoIter<T, W> {}

impl<T, W: Width, S> IntoIterator for HashSet<T, W, S> {
    type Item = T;
    type IntoIter = IntoIter<T, W>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            iter: self.map.into_iter(),
        }
    }
}
