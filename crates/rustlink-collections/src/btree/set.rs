//! The set half of the B+tree.

use core::borrow::Borrow;
use core::fmt;
use core::iter::FusedIterator;

use rustlink::Width;

use super::map;
use crate::error::{Full, MESSAGE};

/// A set of keys in order, backed by a [`BTreeMap`](super::BTreeMap) with `()` values.
///
/// ```
/// use rustlink_collections::BTreeSet;
///
/// let mut set: BTreeSet<u32, u16> = BTreeSet::new();
/// assert!(set.insert(2));
/// assert!(!set.insert(2));
/// assert!(set.contains(&2));
/// assert_eq!(set.iter().copied().collect::<Vec<_>>(), [2]);
/// ```
#[derive(Clone)]
pub struct BTreeSet<T, W: Width = u32> {
    map: map::BTreeMap<T, (), W>,
}

impl<T, W: Width> BTreeSet<T, W> {
    /// An empty set. Does not allocate.
    pub const fn new() -> Self {
        BTreeSet {
            map: map::BTreeMap::new(),
        }
    }

    /// An empty set with room for about `capacity` keys.
    pub fn with_capacity(capacity: usize) -> Self {
        BTreeSet {
            map: map::BTreeMap::with_capacity(capacity),
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

    /// Removes every key. Keeps the node allocation.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// The smallest key.
    pub fn first(&self) -> Option<&T> {
        self.iter().next()
    }

    /// The largest key.
    pub fn last(&self) -> Option<&T> {
        self.iter().next_back()
    }

    /// An iterator over the keys in order.
    pub fn iter(&self) -> Iter<'_, T, W> {
        Iter {
            iter: self.map.iter(),
        }
    }
}

impl<T: Ord + Clone, W: Width> BTreeSet<T, W> {
    /// Adds `value`, or returns it inside [`Full`] if the width cannot address another node.
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
    /// If `W` cannot address another node. Use [`try_insert`](Self::try_insert) to handle it.
    pub fn insert(&mut self, value: T) -> bool {
        self.try_insert(value).expect(MESSAGE)
    }

    /// Removes `value`; returns true if it was present.
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.map.remove(value).is_some()
    }

    /// True if the set holds `value`.
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.map.contains_key(value)
    }

    /// Removes and returns the smallest key.
    pub fn pop_first(&mut self) -> Option<T> {
        self.map.pop_first().map(|(key, ())| key)
    }

    /// Removes and returns the largest key.
    pub fn pop_last(&mut self) -> Option<T> {
        self.map.pop_last().map(|(key, ())| key)
    }
}

impl<T, W: Width> Default for BTreeSet<T, W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug, W: Width> fmt::Debug for BTreeSet<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T: PartialEq, W: Width> PartialEq for BTreeSet<T, W> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T: Eq, W: Width> Eq for BTreeSet<T, W> {}

impl<T: Ord + Clone, W: Width> Extend<T> for BTreeSet<T, W> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.insert(value);
        }
    }
}

impl<T: Ord + Clone, W: Width> FromIterator<T> for BTreeSet<T, W> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut set = Self::with_capacity(iter.size_hint().0);
        set.extend(iter);
        set
    }
}

/// An iterator over the keys of a [`BTreeSet`], created by [`BTreeSet::iter`].
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

impl<T, W: Width> DoubleEndedIterator for Iter<'_, T, W> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back().map(|(key, ())| key)
    }
}

impl<T, W: Width> ExactSizeIterator for Iter<'_, T, W> {}
impl<T, W: Width> FusedIterator for Iter<'_, T, W> {}

impl<'a, T, W: Width> IntoIterator for &'a BTreeSet<T, W> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T, W>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An owning iterator over the keys of a [`BTreeSet`], front to back.
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

impl<T, W: Width> IntoIterator for BTreeSet<T, W> {
    type Item = T;
    type IntoIter = IntoIter<T, W>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            iter: self.map.into_iter(),
        }
    }
}
