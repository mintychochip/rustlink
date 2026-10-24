//! The map half of the B+tree.

use alloc::vec;
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::iter::FusedIterator;
use core::mem;

use rustlink::{OptAbs, Width};

use super::{
    INNER_CAP, Inner, LEAF_CAP, Leaf, Link, MIN_INNER, MIN_LEAF, Node, Path, child_pos, link,
};
use crate::error::{Full, MESSAGE};
use crate::limit::node_limit;

/// A map from keys to values in key order.
///
/// A node takes `Vec<(K, V)>` plus two `W`-byte links for a leaf, or one `K` per separator plus
/// `W` bytes per child for an inner node. The tree is a B+tree: every entry is in a leaf, leaves
/// are chained in key order, and inner nodes only route. Lookups therefore touch fewer bytes per
/// level than a map that stores values in inner nodes too, and a full scan follows one link per
/// leaf.
///
/// ```
/// use rustlink_collections::BTreeMap;
///
/// let mut map: BTreeMap<u32, &str, u16> = BTreeMap::new();
/// assert_eq!(map.insert(2, "two"), None);
/// assert_eq!(map.insert(1, "one"), None);
/// assert_eq!(map.get(&2), Some(&"two"));
/// assert_eq!(map.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [1, 2]);
/// assert_eq!(map.remove(&1), Some("one"));
/// ```
#[derive(Clone)]
pub struct BTreeMap<K, V, W: Width = u32> {
    pub(crate) nodes: Vec<Node<K, V, W>>,
    pub(crate) root: Link<K, V, W>,
    pub(crate) first: Link<K, V, W>,
    pub(crate) last: Link<K, V, W>,
    pub(crate) free: Link<K, V, W>,
    pub(crate) free_count: usize,
    pub(crate) len: usize,
}

impl<K, V, W: Width> BTreeMap<K, V, W> {
    /// An empty map. Does not allocate.
    pub const fn new() -> Self {
        BTreeMap {
            nodes: Vec::new(),
            root: OptAbs::NULL,
            first: OptAbs::NULL,
            last: OptAbs::NULL,
            free: OptAbs::NULL,
            free_count: 0,
            len: 0,
        }
    }

    /// An empty map with room for about `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        let nodes = capacity.div_ceil(MIN_LEAF).max(1);
        BTreeMap {
            nodes: Vec::with_capacity(nodes),
            ..Self::new()
        }
    }

    /// `size_of` one node slot: the enum with its leaf or inner variant, including the `Vec`
    /// headers and links but not the entry, key or child buffers they point at.
    pub const fn node_bytes() -> usize {
        core::mem::size_of::<Node<K, V, W>>()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes every entry. Keeps the node allocation.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = OptAbs::NULL;
        self.first = OptAbs::NULL;
        self.last = OptAbs::NULL;
        self.free = OptAbs::NULL;
        self.free_count = 0;
        self.len = 0;
    }

    pub(crate) fn as_leaf(&self, index: usize) -> &Leaf<K, V, W> {
        match &self.nodes[index] {
            Node::Leaf(leaf) => leaf,
            _ => unreachable!("a leaf link led to a non-leaf node"),
        }
    }

    pub(crate) fn as_leaf_mut(&mut self, index: usize) -> &mut Leaf<K, V, W> {
        match &mut self.nodes[index] {
            Node::Leaf(leaf) => leaf,
            _ => unreachable!("a leaf link led to a non-leaf node"),
        }
    }

    pub(crate) fn as_inner(&self, index: usize) -> &Inner<K, V, W> {
        match &self.nodes[index] {
            Node::Inner(inner) => inner,
            _ => unreachable!("an inner link led to a non-inner node"),
        }
    }

    pub(crate) fn as_inner_mut(&mut self, index: usize) -> &mut Inner<K, V, W> {
        match &mut self.nodes[index] {
            Node::Inner(inner) => inner,
            _ => unreachable!("an inner link led to a non-inner node"),
        }
    }

    pub(crate) fn leaf_len(&self, index: usize) -> usize {
        self.as_leaf(index).entries.len()
    }

    pub(crate) fn inner_len(&self, index: usize) -> usize {
        self.as_inner(index).keys.len()
    }

    /// True if `extra` new nodes can be allocated without exceeding the link width.
    fn ensure_capacity(&self, extra: usize) -> bool {
        let spare = node_limit::<W>().saturating_sub(self.nodes.len());
        self.free_count.saturating_add(spare) >= extra
    }

    fn alloc(&mut self, node: Node<K, V, W>) -> Link<K, V, W> {
        match self.free.get() {
            Some(index) => {
                let next = match &self.nodes[index] {
                    Node::Free(next) => *next,
                    _ => unreachable!("the free list led to a live node"),
                };
                self.free = next;
                self.free_count -= 1;
                self.nodes[index] = node;
                link(index)
            }
            None => {
                let index = self.nodes.len();
                self.nodes.push(node);
                link(index)
            }
        }
    }

    fn free_node(&mut self, index: usize) {
        let old = mem::replace(&mut self.nodes[index], Node::Free(self.free));
        debug_assert!(matches!(old, Node::Leaf(_) | Node::Inner(_)));
        drop(old);
        self.free = link(index);
        self.free_count += 1;
    }

    fn alloc_leaf(
        &mut self,
        entries: Vec<(K, V)>,
        prev: Link<K, V, W>,
        next: Link<K, V, W>,
    ) -> Link<K, V, W> {
        self.alloc(Node::Leaf(Leaf {
            entries,
            prev,
            next,
        }))
    }

    fn alloc_inner(&mut self, keys: Vec<K>, kids: Vec<Link<K, V, W>>) -> Link<K, V, W> {
        self.alloc(Node::Inner(Inner { keys, kids }))
    }

    /// The entry for `key`.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        let mut index = self.root.get()?;
        loop {
            match &self.nodes[index] {
                Node::Inner(inner) => {
                    index = inner.kids[child_pos(&inner.keys, key)].get()?;
                }
                Node::Leaf(leaf) => {
                    let at = leaf
                        .entries
                        .binary_search_by(|(k, _)| k.borrow().cmp(key))
                        .ok()?;
                    return Some(&leaf.entries[at].1);
                }
                Node::Free(_) => unreachable!("a live link led to a free node"),
            }
        }
    }

    /// The entry for `key`, mutably.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        let mut index = self.root.get()?;
        loop {
            match &self.nodes[index] {
                Node::Inner(inner) => {
                    index = inner.kids[child_pos(&inner.keys, key)].get()?;
                }
                Node::Leaf(leaf) => {
                    let at = leaf
                        .entries
                        .binary_search_by(|(k, _)| k.borrow().cmp(key))
                        .ok()?;
                    return match &mut self.nodes[index] {
                        Node::Leaf(leaf) => Some(&mut leaf.entries[at].1),
                        _ => unreachable!(),
                    };
                }
                Node::Free(_) => unreachable!("a live link led to a free node"),
            }
        }
    }

    /// True if the map holds `key`.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        self.get(key).is_some()
    }

    /// The first entry in key order.
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        let leaf = self.as_leaf(self.first.get()?);
        let (key, value) = leaf.entries.first()?;
        Some((key, value))
    }

    /// The last entry in key order.
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        let leaf = self.as_leaf(self.last.get()?);
        let (key, value) = leaf.entries.last()?;
        Some((key, value))
    }

    /// Inserts `key` and returns the previous value, or the pair back inside [`Full`] if the
    /// width cannot address another node.
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, Full<(K, V)>>
    where
        K: Ord + Clone,
    {
        if self.root.get().is_none() {
            let leaf = self.alloc_leaf(Vec::new(), OptAbs::NULL, OptAbs::NULL);
            self.root = leaf;
            self.first = leaf;
            self.last = leaf;
        }

        let mut path = Path::new();
        let mut index = self.root.get().expect("the root exists");
        loop {
            match &self.nodes[index] {
                Node::Inner(inner) => {
                    let at = child_pos(&inner.keys, &key);
                    path.push(index, at);
                    index = inner.kids[at].get().expect("a child link is live");
                }
                Node::Leaf(leaf) => {
                    if let Ok(at) = leaf.entries.binary_search_by(|(k, _)| k.cmp(&key)) {
                        let leaf = self.as_leaf_mut(index);
                        return Ok(Some(mem::replace(&mut leaf.entries[at].1, value)));
                    }
                    break;
                }
                Node::Free(_) => unreachable!("a live link led to a free node"),
            }
        }

        if self.leaf_len(index) == LEAF_CAP {
            // A split may allocate one node per level plus a new root.
            if !self.ensure_capacity(path.len + 2) {
                return Err(Full((key, value)));
            }
            let (separator, right) = self.split_leaf(index);
            self.insert_separator(&path, separator, right);

            // Ancestors may have split, so rebuild the path instead of patching it.
            path.clear();
            index = self.root.get().expect("the root exists");
            while let Node::Inner(inner) = &self.nodes[index] {
                let at = child_pos(&inner.keys, &key);
                path.push(index, at);
                index = inner.kids[at].get().expect("a child link is live");
            }
        }

        let at = match &self.nodes[index] {
            Node::Leaf(leaf) => leaf
                .entries
                .binary_search_by(|(k, _)| k.cmp(&key))
                .expect_err("the key is absent"),
            _ => unreachable!(),
        };
        if at == 0 && path.len > 0 {
            // A child's first key is stored in its parent, and a leftmost child's change may
            // travel further up.
            self.set_first(&path, key.clone());
        }
        let leaf = self.as_leaf_mut(index);
        leaf.entries.insert(at, (key, value));
        self.len += 1;
        Ok(None)
    }

    /// Inserts `key` and returns the previous value.
    ///
    /// # Panics
    ///
    /// If `W` cannot address another node. Use [`try_insert`](Self::try_insert) to handle it.
    pub fn insert(&mut self, key: K, value: V) -> Option<V>
    where
        K: Ord + Clone,
    {
        self.try_insert(key, value).expect(MESSAGE)
    }

    /// Splits a full leaf in half and returns the first key of the right half and its link.
    fn split_leaf(&mut self, index: usize) -> (K, Link<K, V, W>)
    where
        K: Clone,
    {
        let next = self.as_leaf(index).next;
        let right_entries = self.as_leaf_mut(index).entries.split_off(MIN_LEAF);
        let separator = right_entries[0].0.clone();
        let right = self.alloc_leaf(right_entries, link(index), next);
        self.as_leaf_mut(index).next = right;
        match next.get() {
            Some(next) => self.as_leaf_mut(next).prev = right,
            None => self.last = right,
        }
        (separator, right)
    }

    /// Splits a full inner node around its middle separator.
    fn split_inner(&mut self, index: usize) -> (K, Link<K, V, W>) {
        let (up, right_keys, right_kids) = {
            let inner = self.as_inner_mut(index);
            let right_keys = inner.keys.split_off(MIN_INNER + 1);
            let up = inner.keys.pop().expect("the inner node overflowed");
            let right_kids = inner.kids.split_off(MIN_INNER + 1);
            (up, right_keys, right_kids)
        };
        let right = self.alloc_inner(right_keys, right_kids);
        (up, right)
    }

    /// Inserts `separator` and a new right child into the parent on top of `path`, splitting
    /// ancestors (and the root) as needed.
    fn insert_separator(&mut self, path: &Path, mut separator: K, mut right: Link<K, V, W>) {
        let mut level = path.len;
        loop {
            if level == 0 {
                let left = self.root;
                self.root = self.alloc_inner(vec![separator], vec![left, right]);
                return;
            }
            level -= 1;
            let (parent, at) = path.levels[level];
            {
                let inner = self.as_inner_mut(parent);
                inner.keys.insert(at, separator);
                inner.kids.insert(at + 1, right);
                if inner.keys.len() <= INNER_CAP {
                    return;
                }
            }
            let (up, new_right) = self.split_inner(parent);
            separator = up;
            right = new_right;
        }
    }

    /// Removes `key` and returns its value.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q> + Ord + Clone,
        Q: Ord + ?Sized,
    {
        let mut path = Path::new();
        let mut index = self.root.get()?;
        loop {
            match &self.nodes[index] {
                Node::Inner(inner) => {
                    let at = child_pos(&inner.keys, key);
                    path.push(index, at);
                    index = inner.kids[at].get()?;
                }
                Node::Leaf(_) => break,
                Node::Free(_) => unreachable!("a live link led to a free node"),
            }
        }
        let at = match &self.nodes[index] {
            Node::Leaf(leaf) => leaf
                .entries
                .binary_search_by(|(k, _)| k.borrow().cmp(key))
                .ok()?,
            _ => unreachable!(),
        };
        let value = self.as_leaf_mut(index).entries.remove(at).1;
        self.len -= 1;

        if path.len == 0 {
            if self.len == 0 {
                self.root = OptAbs::NULL;
                self.first = OptAbs::NULL;
                self.last = OptAbs::NULL;
            }
            return Some(value);
        }

        let (parent, child) = path.top();
        if self.leaf_len(index) < MIN_LEAF {
            self.rebalance_leaf(parent, child, index);
            if matches!(&self.nodes[index], Node::Leaf(_)) {
                let first = self.as_leaf(index).entries[0].0.clone();
                self.set_first(&path, first);
            }
        } else if at == 0 {
            let first = self.as_leaf(index).entries[0].0.clone();
            self.set_first(&path, first);
        }

        // A merge removes a separator from every inner node on the path. Rebalance each one,
        // bottom up; the root is allowed to underflow and is collapsed below.
        let root = self.root.get().expect("a non-empty tree has a root");
        for level in (0..path.len).rev() {
            let node = path.levels[level].0;
            if node == root {
                break;
            }
            let parent = path.levels[level - 1].0;
            let child = path.levels[level - 1].1;
            if self.inner_len(node) < MIN_INNER {
                self.rebalance_inner(parent, child, node);
            }
        }

        loop {
            let root = self.root.get().expect("a non-empty tree has a root");
            let empty = matches!(&self.nodes[root], Node::Inner(inner) if inner.keys.is_empty());
            if !empty {
                break;
            }
            let child = self.as_inner(root).kids[0];
            self.root = child;
            self.free_node(root);
        }
        Some(value)
    }

    /// Stores `first` as the separator above the node at the bottom of `path`, climbing while
    /// the node is its parent's first child (whose minimum is not stored at that level).
    fn set_first(&mut self, path: &Path, first: K) {
        for level in (0..path.len).rev() {
            let (parent, child) = path.levels[level];
            if child > 0 {
                self.as_inner_mut(parent).keys[child - 1] = first;
                return;
            }
        }
    }

    /// Makes a leaf whole again by borrowing from a sibling or merging with one.
    fn rebalance_leaf(&mut self, parent: usize, child: usize, index: usize)
    where
        K: Clone,
    {
        let kids = self.as_inner(parent).kids.len();
        if child > 0 {
            let left = self.as_inner(parent).kids[child - 1]
                .get()
                .expect("a live link");
            if self.leaf_len(left) > MIN_LEAF {
                let entry = self.as_leaf_mut(left).entries.pop().expect("non-empty");
                let first = entry.0.clone();
                self.as_leaf_mut(index).entries.insert(0, entry);
                self.as_inner_mut(parent).keys[child - 1] = first;
                return;
            }
        }
        if child + 1 < kids {
            let right = self.as_inner(parent).kids[child + 1]
                .get()
                .expect("a live link");
            if self.leaf_len(right) > MIN_LEAF {
                let entry = self.as_leaf_mut(right).entries.remove(0);
                let first = self.as_leaf(right).entries[0].0.clone();
                self.as_leaf_mut(index).entries.push(entry);
                self.as_inner_mut(parent).keys[child] = first;
                return;
            }
        }

        if child > 0 {
            let left = self.as_inner(parent).kids[child - 1]
                .get()
                .expect("a live link");
            let next = self.as_leaf(index).next;
            let moved = mem::take(&mut self.as_leaf_mut(index).entries);
            {
                let leaf = self.as_leaf_mut(left);
                leaf.entries.extend(moved);
                leaf.next = next;
            }
            match next.get() {
                Some(next) => self.as_leaf_mut(next).prev = link(left),
                None => self.last = link(left),
            }
            let inner = self.as_inner_mut(parent);
            inner.keys.remove(child - 1);
            inner.kids.remove(child);
            self.free_node(index);
        } else {
            let right = self.as_inner(parent).kids[child + 1]
                .get()
                .expect("a live link");
            let next = self.as_leaf(right).next;
            let moved = mem::take(&mut self.as_leaf_mut(right).entries);
            {
                let leaf = self.as_leaf_mut(index);
                leaf.entries.extend(moved);
                leaf.next = next;
            }
            match next.get() {
                Some(next) => self.as_leaf_mut(next).prev = link(index),
                None => self.last = link(index),
            }
            let inner = self.as_inner_mut(parent);
            inner.keys.remove(child);
            inner.kids.remove(child + 1);
            self.free_node(right);
        }
    }

    /// Makes an inner node whole again by borrowing from a sibling or merging with one.
    fn rebalance_inner(&mut self, parent: usize, child: usize, index: usize)
    where
        K: Clone,
    {
        let kids = self.as_inner(parent).kids.len();
        if child > 0 {
            let left = self.as_inner(parent).kids[child - 1]
                .get()
                .expect("a live link");
            if self.inner_len(left) > MIN_INNER {
                let up = self.as_inner_mut(left).keys.pop().expect("non-empty");
                let down = self.as_inner(parent).keys[child - 1].clone();
                let kid = self.as_inner_mut(left).kids.pop().expect("non-empty");
                {
                    let inner = self.as_inner_mut(index);
                    inner.keys.insert(0, down);
                    inner.kids.insert(0, kid);
                }
                self.as_inner_mut(parent).keys[child - 1] = up;
                return;
            }
        }
        if child + 1 < kids {
            let right = self.as_inner(parent).kids[child + 1]
                .get()
                .expect("a live link");
            if self.inner_len(right) > MIN_INNER {
                let up = self.as_inner_mut(right).keys.remove(0);
                let down = self.as_inner(parent).keys[child].clone();
                let kid = self.as_inner_mut(right).kids.remove(0);
                {
                    let inner = self.as_inner_mut(index);
                    inner.keys.push(down);
                    inner.kids.push(kid);
                }
                self.as_inner_mut(parent).keys[child] = up;
                return;
            }
        }

        if child > 0 {
            let left = self.as_inner(parent).kids[child - 1]
                .get()
                .expect("a live link");
            let up = self.as_inner_mut(parent).keys.remove(child - 1);
            let keys = mem::take(&mut self.as_inner_mut(index).keys);
            let kids = mem::take(&mut self.as_inner_mut(index).kids);
            {
                let inner = self.as_inner_mut(left);
                inner.keys.push(up);
                inner.keys.extend(keys);
                inner.kids.extend(kids);
            }
            self.as_inner_mut(parent).kids.remove(child);
            self.free_node(index);
        } else {
            let right = self.as_inner(parent).kids[child + 1]
                .get()
                .expect("a live link");
            let up = self.as_inner_mut(parent).keys.remove(child);
            let keys = mem::take(&mut self.as_inner_mut(right).keys);
            let kids = mem::take(&mut self.as_inner_mut(right).kids);
            {
                let inner = self.as_inner_mut(index);
                inner.keys.push(up);
                inner.keys.extend(keys);
                inner.kids.extend(kids);
            }
            self.as_inner_mut(parent).kids.remove(child + 1);
            self.free_node(right);
        }
    }

    /// Removes and returns the first entry.
    pub fn pop_first(&mut self) -> Option<(K, V)>
    where
        K: Ord + Clone,
    {
        let key = self.first_key_value()?.0.clone();
        self.remove(&key).map(|value| (key, value))
    }

    /// Removes and returns the last entry.
    pub fn pop_last(&mut self) -> Option<(K, V)>
    where
        K: Ord + Clone,
    {
        let key = self.last_key_value()?.0.clone();
        self.remove(&key).map(|value| (key, value))
    }

    /// An iterator over the entries in key order.
    pub fn iter(&self) -> Iter<'_, K, V, W> {
        Iter {
            map: self,
            front_leaf: self.first,
            front_index: 0,
            back_leaf: self.last,
            back_index: self.last.get().map_or(0, |index| self.leaf_len(index)),
            remaining: self.len,
        }
    }

    /// An iterator over the keys in order.
    pub fn keys(&self) -> Keys<'_, K, V, W> {
        Keys { iter: self.iter() }
    }

    /// An iterator over the values in key order.
    pub fn values(&self) -> Values<'_, K, V, W> {
        Values { iter: self.iter() }
    }
}

impl<K, V, W: Width> Default for BTreeMap<K, V, W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: fmt::Debug, V: fmt::Debug, W: Width> fmt::Debug for BTreeMap<K, V, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: PartialEq, V: PartialEq, W: Width> PartialEq for BTreeMap<K, V, W> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl<K: Eq, V: Eq, W: Width> Eq for BTreeMap<K, V, W> {}

impl<K: Ord + Clone, V, W: Width> Extend<(K, V)> for BTreeMap<K, V, W> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<K: Ord + Clone, V, W: Width> FromIterator<(K, V)> for BTreeMap<K, V, W> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut map = Self::with_capacity(iter.size_hint().0);
        map.extend(iter);
        map
    }
}

/// An iterator over the entries of a [`BTreeMap`], created by [`BTreeMap::iter`].
pub struct Iter<'a, K, V, W: Width> {
    map: &'a BTreeMap<K, V, W>,
    front_leaf: Link<K, V, W>,
    front_index: usize,
    back_leaf: Link<K, V, W>,
    back_index: usize,
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
        if self.remaining == 0 {
            return None;
        }
        loop {
            let leaf = self.map.as_leaf(self.front_leaf.get()?);
            if self.front_index < leaf.entries.len() {
                let (key, value) = &leaf.entries[self.front_index];
                self.front_index += 1;
                self.remaining -= 1;
                return Some((key, value));
            }
            self.front_leaf = leaf.next;
            self.front_index = 0;
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, W: Width> DoubleEndedIterator for Iter<'_, K, V, W> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        loop {
            let leaf = self.map.as_leaf(self.back_leaf.get()?);
            if self.back_index > 0 {
                self.back_index -= 1;
                self.remaining -= 1;
                let (key, value) = &leaf.entries[self.back_index];
                return Some((key, value));
            }
            self.back_leaf = leaf.prev;
            self.back_index = self
                .back_leaf
                .get()
                .map_or(0, |index| self.map.leaf_len(index));
        }
    }
}

impl<K, V, W: Width> ExactSizeIterator for Iter<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Iter<'_, K, V, W> {}

impl<'a, K, V, W: Width> IntoIterator for &'a BTreeMap<K, V, W> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, W>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the keys of a [`BTreeMap`], created by [`BTreeMap::keys`].
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

impl<K, V, W: Width> DoubleEndedIterator for Keys<'_, K, V, W> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back().map(|(key, _)| key)
    }
}

impl<K, V, W: Width> ExactSizeIterator for Keys<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Keys<'_, K, V, W> {}

/// An iterator over the values of a [`BTreeMap`], created by [`BTreeMap::values`].
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

impl<K, V, W: Width> DoubleEndedIterator for Values<'_, K, V, W> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back().map(|(_, value)| value)
    }
}

impl<K, V, W: Width> ExactSizeIterator for Values<'_, K, V, W> {}
impl<K, V, W: Width> FusedIterator for Values<'_, K, V, W> {}

/// An owning iterator over the entries of a [`BTreeMap`], front to back.
pub struct IntoIter<K, V, W: Width> {
    nodes: Vec<Node<K, V, W>>,
    leaf: Link<K, V, W>,
    entries: alloc::vec::IntoIter<(K, V)>,
    remaining: usize,
}

impl<K, V, W: Width> Iterator for IntoIter<K, V, W> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        loop {
            if let Some(entry) = self.entries.next() {
                self.remaining -= 1;
                return Some(entry);
            }
            let leaf = self.leaf.get()?;
            let (next, entries) = match &mut self.nodes[leaf] {
                Node::Leaf(leaf) => (leaf.next, mem::take(&mut leaf.entries)),
                _ => unreachable!("a leaf link led to a non-leaf node"),
            };
            self.leaf = next;
            self.entries = entries.into_iter();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, W: Width> ExactSizeIterator for IntoIter<K, V, W> {}
impl<K, V, W: Width> FusedIterator for IntoIter<K, V, W> {}

impl<K, V, W: Width> IntoIterator for BTreeMap<K, V, W> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V, W>;

    fn into_iter(self) -> Self::IntoIter {
        let remaining = self.len;
        IntoIter {
            nodes: self.nodes,
            leaf: self.first,
            entries: Vec::new().into_iter(),
            remaining,
        }
    }
}
