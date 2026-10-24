//! The map half of the chained hash table.

use alloc::vec;
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::hash::{BuildHasher, Hash};
use core::iter::FusedIterator;
use core::mem;

use rustlink::{OptAbs, Width};

use super::{Entry, FnvBuildHasher, LOAD_DEN, LOAD_NUM, Link, MIN_BUCKETS, link};
use crate::error::{Full, MESSAGE};
use crate::limit::node_limit;

/// A hash map whose entries live in one `Vec` and chain through `W`-wide links.
///
/// An entry slot is `size_of::<K>() + size_of::<V>() + W` bytes plus one liveness byte; the
/// bucket table holds one `W`-byte link per bucket at a three-quarter load factor. Removed slots
/// go on a free list and are reused, so `insert` does not allocate a node for every entry.
///
/// The default hasher is FNV-1a. It is fast but not collision-resistant; use
/// [`with_hasher`](Self::with_hasher) for an adversarial input.
///
/// Free slots must hold a default key and value, so `K: Default` and `V: Default` are required
/// by insertion and removal. `W` bounds the number of live entries plus free slots: a `u16` map
/// addresses 65,535 slots.
///
/// ```
/// use rustlink_collections::HashMap;
///
/// let mut map: HashMap<u32, &str, u16> = HashMap::new();
/// assert_eq!(map.insert(2, "two"), None);
/// assert_eq!(map.insert(1, "one"), None);
/// assert_eq!(map.get(&2), Some(&"two"));
/// assert_eq!(map.remove(&1), Some("one"));
/// assert!(map.is_empty() == false);
/// ```
#[derive(Clone)]
pub struct HashMap<K, V, W: Width = u32, S = FnvBuildHasher> {
    pub(crate) buckets: Vec<Link<K, V, W>>,
    pub(crate) entries: Vec<Entry<K, V, W>>,
    pub(crate) live: Vec<bool>,
    pub(crate) free: Link<K, V, W>,
    pub(crate) free_count: usize,
    pub(crate) len: usize,
    hasher: S,
}

impl<K, V, W: Width> HashMap<K, V, W> {
    /// An empty map with the default FNV hasher. Does not allocate.
    pub fn new() -> Self {
        Self::with_hasher(FnvBuildHasher::default())
    }

    /// An empty map with room for `capacity` entries and the default FNV hasher.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, FnvBuildHasher::default())
    }
}

impl<K, V, W: Width, S> HashMap<K, V, W, S> {
    /// An empty map using `hasher`.
    pub fn with_hasher(hasher: S) -> Self {
        HashMap {
            buckets: Vec::new(),
            entries: Vec::new(),
            live: Vec::new(),
            free: OptAbs::NULL,
            free_count: 0,
            len: 0,
            hasher,
        }
    }

    /// An empty map with room for `capacity` entries.
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        let entries = Vec::with_capacity(capacity);
        let buckets = if capacity == 0 {
            0
        } else {
            capacity
                .saturating_mul(LOAD_DEN)
                .div_ceil(LOAD_NUM)
                .next_power_of_two()
                .max(MIN_BUCKETS)
        };
        HashMap {
            buckets: vec![OptAbs::NULL; buckets],
            entries,
            live: Vec::with_capacity(capacity),
            free: OptAbs::NULL,
            free_count: 0,
            len: 0,
            hasher,
        }
    }

    /// One entry slot plus its liveness byte, excluding the bucket table.
    pub const fn node_bytes() -> usize {
        core::mem::size_of::<Entry<K, V, W>>() + core::mem::size_of::<bool>()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes every entry. Keeps the allocations.
    pub fn clear(&mut self) {
        self.buckets.clear();
        self.entries.clear();
        self.live.clear();
        self.free = OptAbs::NULL;
        self.free_count = 0;
        self.len = 0;
    }

    pub(crate) fn hash<Q>(&self, key: &Q) -> u64
    where
        K: Borrow<Q>,
        Q: Hash + ?Sized,
        S: BuildHasher,
    {
        self.hasher.hash_one(key)
    }

    pub(crate) fn bucket_of(hash: u64, buckets: usize) -> usize {
        hash as usize & (buckets - 1)
    }

    /// Grows the bucket table when the next entry would exceed the load factor.
    fn ensure_buckets(&mut self)
    where
        K: Hash,
        S: BuildHasher,
    {
        let needed = (self.len + 1) * LOAD_DEN > self.buckets.len() * LOAD_NUM;
        if self.buckets.is_empty() {
            self.rehash(MIN_BUCKETS);
        } else if needed {
            self.rehash(self.buckets.len() * 2);
        }
    }

    /// Rebuilds the chains into a table of `buckets` buckets.
    fn rehash(&mut self, buckets: usize)
    where
        K: Hash,
        S: BuildHasher,
    {
        let mut table = vec![OptAbs::NULL; buckets];
        for index in 0..self.entries.len() {
            if !self.live[index] {
                continue;
            }
            let hash = self.hash(&self.entries[index].key);
            let bucket = Self::bucket_of(hash, buckets);
            self.entries[index].next = table[bucket];
            table[bucket] = link(index);
        }
        self.buckets = table;
    }

    /// The value for `key`.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q> + Eq,
        Q: Hash + Eq + ?Sized,
        S: BuildHasher,
    {
        if self.buckets.is_empty() {
            return None;
        }
        let bucket = Self::bucket_of(self.hash(key), self.buckets.len());
        let mut at = self.buckets[bucket].get();
        while let Some(index) = at {
            if self.live[index] && self.entries[index].key.borrow() == key {
                return Some(&self.entries[index].value);
            }
            at = self.entries[index].next.get();
        }
        None
    }

    /// The value for `key`, mutably.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q> + Eq,
        Q: Hash + Eq + ?Sized,
        S: BuildHasher,
    {
        if self.buckets.is_empty() {
            return None;
        }
        let bucket = Self::bucket_of(self.hash(key), self.buckets.len());
        let mut at = self.buckets[bucket].get();
        while let Some(index) = at {
            let next = self.entries[index].next;
            if self.live[index] && self.entries[index].key.borrow() == key {
                return Some(&mut self.entries[index].value);
            }
            at = next.get();
        }
        None
    }

    /// True if the map holds `key`.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q> + Eq,
        Q: Hash + Eq + ?Sized,
        S: BuildHasher,
    {
        self.get(key).is_some()
    }

    /// Inserts `key`, returning the previous value or the pair back inside [`Full`] when the
    /// width cannot address another slot.
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, Full<(K, V)>>
    where
        K: Eq + Hash + Default,
        V: Default,
        S: BuildHasher,
    {
        let hash = self.hash(&key);
        if !self.buckets.is_empty() {
            let bucket = Self::bucket_of(hash, self.buckets.len());
            let mut at = self.buckets[bucket].get();
            while let Some(index) = at {
                if self.live[index] && self.entries[index].key == key {
                    return Ok(Some(mem::replace(&mut self.entries[index].value, value)));
                }
                at = self.entries[index].next.get();
            }
        }
        if self.free.get().is_none() && self.entries.len() >= node_limit::<W>() {
            return Err(Full((key, value)));
        }
        self.ensure_buckets();
        let bucket = Self::bucket_of(hash, self.buckets.len());
        let index = match self.free.get() {
            Some(index) => {
                let next = self.entries[index].next;
                self.free = next;
                self.free_count -= 1;
                self.entries[index] = Entry {
                    key,
                    value,
                    next: self.buckets[bucket],
                };
                self.live[index] = true;
                index
            }
            None => {
                let index = self.entries.len();
                self.entries.push(Entry {
                    key,
                    value,
                    next: self.buckets[bucket],
                });
                self.live.push(true);
                index
            }
        };
        self.buckets[bucket] = link(index);
        self.len += 1;
        Ok(None)
    }

    /// Inserts `key`, returning the previous value.
    ///
    /// # Panics
    ///
    /// If `W` cannot address another slot. Use [`try_insert`](Self::try_insert) to handle it.
    pub fn insert(&mut self, key: K, value: V) -> Option<V>
    where
        K: Eq + Hash + Default,
        V: Default,
        S: BuildHasher,
    {
        self.try_insert(key, value).expect(MESSAGE)
    }

    /// Removes `key` and returns its value.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q> + Eq + Hash + Default,
        V: Default,
        Q: Hash + Eq + ?Sized,
        S: BuildHasher,
    {
        if self.buckets.is_empty() {
            return None;
        }
        let bucket = Self::bucket_of(self.hash(key), self.buckets.len());
        let mut previous: Option<usize> = None;
        let mut at = self.buckets[bucket].get();
        while let Some(index) = at {
            let next = self.entries[index].next;
            if self.live[index] && self.entries[index].key.borrow() == key {
                match previous {
                    Some(previous) => self.entries[previous].next = next,
                    None => self.buckets[bucket] = next,
                }
                let value = mem::take(&mut self.entries[index].value);
                let _key = mem::take(&mut self.entries[index].key);
                self.entries[index].next = self.free;
                self.live[index] = false;
                self.free = link(index);
                self.free_count += 1;
                self.len -= 1;
                return Some(value);
            }
            previous = Some(index);
            at = next.get();
        }
        None
    }

    /// Compacts live entries to the front, truncates the slot array and rebuilds the bucket
    /// table at the smallest power of two that holds the current load. Useful after many
    /// removals or a large `retain`; the next insert may reallocate.
    pub fn shrink_to_fit(&mut self)
    where
        K: Hash,
        S: BuildHasher,
    {
        let mut write = 0;
        for read in 0..self.entries.len() {
            if !self.live[read] {
                continue;
            }
            if write != read {
                self.entries.swap(write, read);
                self.live[write] = true;
                self.live[read] = false;
            }
            write += 1;
        }
        self.entries.truncate(write);
        self.live.truncate(write);
        self.free = OptAbs::NULL;
        self.free_count = 0;
        if self.len == 0 {
            self.buckets.clear();
            return;
        }
        let target = (self.len * LOAD_DEN)
            .div_ceil(LOAD_NUM)
            .next_power_of_two()
            .max(MIN_BUCKETS);
        self.rehash(target);
    }

    /// Calls `f` on every live entry with mutable access to the value.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&K, &mut V)) {
        for (index, live) in self.live.iter().enumerate() {
            if *live {
                let entry = &mut self.entries[index];
                f(&entry.key, &mut entry.value);
            }
        }
    }

    /// Keeps only the entries for which `keep` returns true, in one pass over the slot array,
    /// then rebuilds the bucket chains. Keys are hashed once, not once per removed key.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&K, &mut V) -> bool,
        K: Hash + Default,
        V: Default,
        S: BuildHasher,
    {
        for index in 0..self.entries.len() {
            if !self.live[index] {
                continue;
            }
            let entry = &mut self.entries[index];
            if keep(&entry.key, &mut entry.value) {
                continue;
            }
            self.live[index] = false;
            let _key = mem::take(&mut entry.key);
            let _value = mem::take(&mut entry.value);
            entry.next = self.free;
            self.free = link(index);
            self.free_count += 1;
            self.len -= 1;
        }
        if !self.buckets.is_empty() {
            self.rehash(self.buckets.len());
        }
    }

    /// An iterator over the entries in slot order, with mutable values.
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V, W> {
        IterMut {
            remaining: self.len,
            live: &self.live,
            entries: self.entries.iter_mut(),
            index: 0,
        }
    }

    /// An iterator over the entries in slot order.
    pub fn iter(&self) -> Iter<'_, K, V, W> {
        Iter {
            entries: &self.entries,
            live: &self.live,
            index: 0,
            remaining: self.len,
        }
    }

    /// An iterator over the keys in slot order.
    pub fn keys(&self) -> Keys<'_, K, V, W> {
        Keys { iter: self.iter() }
    }

    /// An iterator over the values in slot order.
    pub fn values(&self) -> Values<'_, K, V, W> {
        Values { iter: self.iter() }
    }
}

impl<K, V, W: Width, S: Default> Default for HashMap<K, V, W, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<K: fmt::Debug, V: fmt::Debug, W: Width, S> fmt::Debug for HashMap<K, V, W, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V, W, S> PartialEq for HashMap<K, V, W, S>
where
    K: Eq + Hash,
    V: PartialEq,
    W: Width,
    S: BuildHasher,
{
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self
                .iter()
                .all(|(key, value)| other.get(key).is_some_and(|other| other == value))
    }
}

impl<K: Eq + Hash, V: Eq, W: Width, S: BuildHasher> Eq for HashMap<K, V, W, S> {}

impl<K, V, W, S> Extend<(K, V)> for HashMap<K, V, W, S>
where
    K: Eq + Hash + Default,
    V: Default,
    W: Width,
    S: BuildHasher,
{
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<K, V, W, S> FromIterator<(K, V)> for HashMap<K, V, W, S>
where
    K: Eq + Hash + Default,
    V: Default,
    W: Width,
    S: BuildHasher + Default,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut map = Self::with_capacity_and_hasher(iter.size_hint().0, S::default());
        map.extend(iter);
        map
    }
}

/// An iterator over the entries of a [`HashMap`], created by [`HashMap::iter`].
pub struct Iter<'a, K, V, W: Width> {
    entries: &'a [Entry<K, V, W>],
    live: &'a [bool],
    index: usize,
    remaining: usize,
}

impl<K, V, W: Width> Clone for Iter<'_, K, V, W> {
    fn clone(&self) -> Self {
        Iter { ..*self }
    }
}

impl<'a, K, V, W: Width> Iterator for Iter<'a, K, V, W> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.entries.len() {
            let index = self.index;
            self.index += 1;
            if self.live[index] {
                self.remaining -= 1;
                return Some((&self.entries[index].key, &self.entries[index].value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, W: Width> ExactSizeIterator for Iter<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Iter<'_, K, V, W> {}

impl<'a, K, V, W: Width, S> IntoIterator for &'a HashMap<K, V, W, S> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, W>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the keys of a [`HashMap`], created by [`HashMap::keys`].
pub struct Keys<'a, K, V, W: Width> {
    iter: Iter<'a, K, V, W>,
}

impl<K, V, W: Width> Clone for Keys<'_, K, V, W> {
    fn clone(&self) -> Self {
        Keys {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, K, V, W: Width> Iterator for Keys<'a, K, V, W> {
    type Item = &'a K;

    fn next(&mut self) -> Option<&'a K> {
        self.iter.next().map(|(key, _)| key)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K, V, W: Width> ExactSizeIterator for Keys<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Keys<'_, K, V, W> {}

/// An iterator over the values of a [`HashMap`], created by [`HashMap::values`].
pub struct Values<'a, K, V, W: Width> {
    iter: Iter<'a, K, V, W>,
}

impl<K, V, W: Width> Clone for Values<'_, K, V, W> {
    fn clone(&self) -> Self {
        Values {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, K, V, W: Width> Iterator for Values<'a, K, V, W> {
    type Item = &'a V;

    fn next(&mut self) -> Option<&'a V> {
        self.iter.next().map(|(_, value)| value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K, V, W: Width> ExactSizeIterator for Values<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Values<'_, K, V, W> {}

/// An iterator over the entries of a [`HashMap`] with mutable values, created by
/// [`HashMap::iter_mut`].
pub struct IterMut<'a, K, V, W: Width> {
    entries: core::slice::IterMut<'a, Entry<K, V, W>>,
    live: &'a [bool],
    index: usize,
    remaining: usize,
}

impl<'a, K, V, W: Width> Iterator for IterMut<'a, K, V, W> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.entries.next()?;
            let index = self.index;
            self.index += 1;
            if self.live[index] {
                self.remaining -= 1;
                return Some((&entry.key, &mut entry.value));
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, W: Width> ExactSizeIterator for IterMut<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for IterMut<'_, K, V, W> {}

impl<'a, K, V, W: Width, S> IntoIterator for &'a mut HashMap<K, V, W, S> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V, W>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// An owning iterator over the entries of a [`HashMap`], in slot order.
pub struct IntoIter<K, V, W: Width> {
    entries: alloc::vec::IntoIter<Entry<K, V, W>>,
    live: alloc::vec::IntoIter<bool>,
    remaining: usize,
}

impl<K, V, W: Width> Iterator for IntoIter<K, V, W> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        loop {
            let entry = self.entries.next()?;
            let live = self.live.next()?;
            if live {
                self.remaining -= 1;
                return Some((entry.key, entry.value));
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, W: Width> ExactSizeIterator for IntoIter<K, V, W> {}
impl<K, V, W: Width> FusedIterator for IntoIter<K, V, W> {}

impl<K, V, W: Width, S> IntoIterator for HashMap<K, V, W, S> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V, W>;

    fn into_iter(self) -> Self::IntoIter {
        let remaining = self.len;
        IntoIter {
            entries: self.entries.into_iter(),
            live: self.live.into_iter(),
            remaining,
        }
    }
}
