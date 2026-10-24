//! A doubly linked list stored in one `Vec` and linked by narrow indices.

use alloc::vec::Vec;
use core::fmt;
use core::iter::FusedIterator;

use rustlink::{OptAbs, Width};

use crate::error::{Full, MESSAGE};
use crate::limit::node_limit;

type Link<T, W> = OptAbs<Node<T, W>, W>;

#[derive(Clone)]
struct Node<T, W: Width> {
    value: T,
    prev: Link<T, W>,
    next: Link<T, W>,
}

/// A doubly linked list whose nodes live in one `Vec` and point at each other with `W`-wide links.
///
/// Compared with `std::collections::LinkedList`, a node takes `size_of::<T>()` plus two links
/// (4 + 8 = 12 bytes for a `u32` element and `u32` links, against 24 plus allocator overhead),
/// pushing never allocates a node, and the nodes of a list built by `push_back` are contiguous.
/// While storage order equals list order, [`iter`](Self::iter) and [`into_iter`](IntoIterator)
/// walk the slice directly instead of decoding links ([`is_contiguous`](Self::is_contiguous)).
/// Nodes are kept dense: removing one moves the last-stored node into its slot, so storage order
/// is not always list order and there is no way to hand out `&mut` references in list order
/// without `unsafe`. Use [`for_each_mut`](Self::for_each_mut) or the ends instead of `iter_mut`.
///
/// `append` and `split_off` cost O(n) rather than O(1), because nodes must move between `Vec`s.
///
/// A list with `W = u16` holds at most 65,535 elements; see [`Full`].
///
/// ```
/// use rustlink_collections::LinkedList;
///
/// let mut list: LinkedList<&str, u16> = LinkedList::new();
/// list.push_back("b");
/// list.push_front("a");
/// list.push_back("c");
/// assert_eq!(list.pop_front(), Some("a"));
/// assert_eq!(list.iter().collect::<Vec<_>>(), [&"b", &"c"]);
/// ```
#[derive(Clone)]
pub struct LinkedList<T, W: Width = u32> {
    nodes: Vec<Node<T, W>>,
    head: Link<T, W>,
    tail: Link<T, W>,
    /// True while storage order is list order, which lets iteration walk the slice directly.
    contiguous: bool,
}

impl<T, W: Width> LinkedList<T, W> {
    /// An empty list. Does not allocate.
    pub const fn new() -> Self {
        LinkedList {
            nodes: Vec::new(),
            head: OptAbs::NULL,
            tail: OptAbs::NULL,
            contiguous: true,
        }
    }

    /// An empty list with room for `capacity` nodes.
    pub fn with_capacity(capacity: usize) -> Self {
        LinkedList {
            nodes: Vec::with_capacity(capacity),
            ..Self::new()
        }
    }

    /// `size_of` one node: the value plus its two links.
    pub const fn node_bytes() -> usize {
        core::mem::size_of::<Node<T, W>>()
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True if the list has no elements.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Removes every element. Keeps the allocation.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.head = OptAbs::NULL;
        self.tail = OptAbs::NULL;
        self.contiguous = true;
    }

    /// True while the nodes are stored in list order, so [`iter`](Self::iter) can walk the slice
    /// directly instead of following links. `push_back` and `pop_back` preserve it; `push_front`,
    /// removing from the middle and appending a list that is not contiguous clear it.
    pub fn is_contiguous(&self) -> bool {
        self.contiguous
    }

    /// The first element.
    pub fn front(&self) -> Option<&T> {
        Some(&self.nodes[self.head.get()?].value)
    }

    /// The first element, mutably.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        Some(&mut self.nodes[self.head.get()?].value)
    }

    /// The last element.
    pub fn back(&self) -> Option<&T> {
        Some(&self.nodes[self.tail.get()?].value)
    }

    /// The last element, mutably.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        Some(&mut self.nodes[self.tail.get()?].value)
    }

    fn link(index: usize) -> Link<T, W> {
        OptAbs::new(Some(index)).expect("an index below the node limit is representable")
    }

    /// Appends `value`, or returns it inside [`Full`] if `W` cannot address another node.
    pub fn try_push_back(&mut self, value: T) -> Result<(), Full<T>> {
        let Ok(link) = OptAbs::new(Some(self.nodes.len())) else {
            return Err(Full(value));
        };
        let was_empty = self.nodes.is_empty();
        self.nodes.push(Node {
            value,
            prev: self.tail,
            next: OptAbs::NULL,
        });
        if was_empty {
            self.contiguous = true;
        }
        match self.tail.get() {
            Some(tail) => self.nodes[tail].next = link,
            None => self.head = link,
        }
        self.tail = link;
        Ok(())
    }

    /// Prepends `value`, or returns it inside [`Full`] if `W` cannot address another node.
    pub fn try_push_front(&mut self, value: T) -> Result<(), Full<T>> {
        let Ok(link) = OptAbs::new(Some(self.nodes.len())) else {
            return Err(Full(value));
        };
        let was_empty = self.nodes.is_empty();
        self.nodes.push(Node {
            value,
            prev: OptAbs::NULL,
            next: self.head,
        });
        self.contiguous = was_empty;
        match self.head.get() {
            Some(head) => self.nodes[head].prev = link,
            None => self.tail = link,
        }
        self.head = link;
        Ok(())
    }

    /// Appends `value`.
    ///
    /// # Panics
    ///
    /// If `W` cannot address another node. Use [`try_push_back`](Self::try_push_back) to handle it.
    pub fn push_back(&mut self, value: T) {
        self.try_push_back(value).expect(MESSAGE);
    }

    /// Prepends `value`.
    ///
    /// # Panics
    ///
    /// If `W` cannot address another node. Use [`try_push_front`](Self::try_push_front) to handle it.
    pub fn push_front(&mut self, value: T) {
        self.try_push_front(value).expect(MESSAGE);
    }

    /// Removes and returns the first element.
    pub fn pop_front(&mut self) -> Option<T> {
        let head = self.head.get()?;
        Some(self.take(head))
    }

    /// Removes and returns the last element.
    pub fn pop_back(&mut self) -> Option<T> {
        let tail = self.tail.get()?;
        Some(self.take(tail))
    }

    /// Unlinks the node at `index`, closes the gap by moving the last-stored node into it, and
    /// returns the value.
    fn take(&mut self, index: usize) -> T {
        let (prev, next) = (self.nodes[index].prev, self.nodes[index].next);
        match prev.get() {
            Some(p) => self.nodes[p].next = next,
            None => self.head = next,
        }
        match next.get() {
            Some(n) => self.nodes[n].prev = prev,
            None => self.tail = prev,
        }

        let last = self.nodes.len() - 1;
        let removed = self.nodes.swap_remove(index);
        if self.nodes.is_empty() {
            self.contiguous = true;
        } else if self.contiguous && index != last {
            self.contiguous = false;
        }
        if index != last {
            // The node that was stored last now lives at `index`; whoever pointed at `last` must
            // point at `index` instead.
            let moved = Self::link(index);
            let (prev, next) = (self.nodes[index].prev, self.nodes[index].next);
            match prev.get() {
                Some(p) => self.nodes[p].next = moved,
                None => self.head = moved,
            }
            match next.get() {
                Some(n) => self.nodes[n].prev = moved,
                None => self.tail = moved,
            }
        }
        removed.value
    }

    /// Moves every element of `other` to the back of `self`, or returns [`Full`] and changes
    /// neither list if the result would not fit in `W`.
    ///
    /// O(`other.len()`): the nodes of `other` are moved into `self`'s `Vec` and their links shifted.
    pub fn try_append(&mut self, other: &mut Self) -> Result<(), Full<()>> {
        if other.is_empty() {
            return Ok(());
        }
        let contiguous = self.contiguous && other.contiguous;
        let offset = self.nodes.len();
        if offset + other.nodes.len() > node_limit::<W>() {
            return Err(Full(()));
        }
        let shift = |link: Link<T, W>| match link.get() {
            Some(index) => Self::link(index + offset),
            None => OptAbs::NULL,
        };
        let (other_head, other_tail) = (shift(other.head), shift(other.tail));
        for mut node in other.nodes.drain(..) {
            node.prev = shift(node.prev);
            node.next = shift(node.next);
            self.nodes.push(node);
        }
        other.head = OptAbs::NULL;
        other.tail = OptAbs::NULL;
        self.contiguous = contiguous;
        other.contiguous = true;

        match self.tail.get() {
            Some(tail) => {
                self.nodes[tail].next = other_head;
                if let Some(first) = other_head.get() {
                    self.nodes[first].prev = self.tail;
                }
            }
            None => self.head = other_head,
        }
        self.tail = other_tail;
        Ok(())
    }

    /// Moves every element of `other` to the back of `self`.
    ///
    /// # Panics
    ///
    /// If the result would not fit in `W`. Use [`try_append`](Self::try_append) to handle it.
    pub fn append(&mut self, other: &mut Self) {
        self.try_append(other).expect(MESSAGE);
    }

    /// Splits the list at `at`: `self` keeps the first `at` elements and the rest is returned.
    ///
    /// O(`len - at`).
    ///
    /// # Panics
    ///
    /// If `at > len`.
    pub fn split_off(&mut self, at: usize) -> Self {
        assert!(at <= self.len(), "Cannot split off at a nonexistent index");
        let mut tail: Vec<T> = Vec::with_capacity(self.len() - at);
        while self.len() > at {
            tail.extend(self.pop_back());
        }
        tail.into_iter().rev().collect()
    }

    /// True if the list contains `value`.
    pub fn contains(&self, value: &T) -> bool
    where
        T: PartialEq,
    {
        self.iter().any(|x| x == value)
    }

    /// Calls `f` on every element in list order, with mutable access.
    ///
    /// The substitute for `iter_mut`, which cannot be written without `unsafe`.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&mut T)) {
        let mut at = self.head.get();
        while let Some(index) = at {
            let node = &mut self.nodes[index];
            f(&mut node.value);
            at = node.next.get();
        }
    }

    /// An iterator over the elements, front to back.
    pub fn iter(&self) -> Iter<'_, T, W> {
        let inner = if self.contiguous {
            Inner::Slice(self.nodes.iter())
        } else {
            Inner::Links {
                nodes: &self.nodes,
                front: self.head,
                back: self.tail,
            }
        };
        Iter {
            inner,
            remaining: self.nodes.len(),
        }
    }
}

#[cfg(test)]
impl<T, W: Width> LinkedList<T, W> {
    /// Panics unless the links form one chain through every stored node, mirrored by `prev`.
    fn check_links(&self) {
        let n = self.nodes.len();
        if n == 0 {
            assert!(self.head.get().is_none() && self.tail.get().is_none());
            return;
        }
        let mut seen = std::vec![false; n];
        let (mut previous, mut at, mut count) = (None, self.head.get(), 0);
        while let Some(index) = at {
            assert!(
                index < n && !seen[index],
                "link to {index} leaves or repeats"
            );
            seen[index] = true;
            count += 1;
            assert_eq!(self.nodes[index].prev.get(), previous);
            previous = Some(index);
            at = self.nodes[index].next.get();
        }
        assert_eq!(count, n, "the chain skips nodes");
        assert_eq!(self.tail.get(), previous);
    }
}

impl<T, W: Width> Default for LinkedList<T, W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug, W: Width> fmt::Debug for LinkedList<T, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: PartialEq, W: Width> PartialEq for LinkedList<T, W> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T: Eq, W: Width> Eq for LinkedList<T, W> {}

impl<T, W: Width> Extend<T> for LinkedList<T, W> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.push_back(value);
        }
    }
}

impl<T, W: Width> FromIterator<T> for LinkedList<T, W> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut list = Self::with_capacity(iter.size_hint().0);
        list.extend(iter);
        list
    }
}

/// An iterator over the elements of a [`LinkedList`], created by [`LinkedList::iter`].
pub struct Iter<'a, T, W: Width> {
    inner: Inner<'a, T, W>,
    remaining: usize,
}

enum Inner<'a, T, W: Width> {
    Slice(core::slice::Iter<'a, Node<T, W>>),
    Links {
        nodes: &'a [Node<T, W>],
        front: Link<T, W>,
        back: Link<T, W>,
    },
}

impl<T, W: Width> Clone for Iter<'_, T, W> {
    fn clone(&self) -> Self {
        Iter {
            inner: match &self.inner {
                Inner::Slice(iter) => Inner::Slice(iter.clone()),
                Inner::Links { nodes, front, back } => Inner::Links {
                    nodes,
                    front: *front,
                    back: *back,
                },
            },
            remaining: self.remaining,
        }
    }
}

impl<'a, T, W: Width> Iterator for Iter<'a, T, W> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        match &mut self.inner {
            Inner::Slice(iter) => iter.next().map(|node| &node.value),
            Inner::Links {
                nodes,
                front,
                back: _,
            } => {
                let node = &nodes[front.get()?];
                *front = node.next;
                Some(&node.value)
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, T, W: Width> DoubleEndedIterator for Iter<'a, T, W> {
    fn next_back(&mut self) -> Option<&'a T> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        match &mut self.inner {
            Inner::Slice(iter) => iter.next_back().map(|node| &node.value),
            Inner::Links {
                nodes,
                front: _,
                back,
            } => {
                let node = &nodes[back.get()?];
                *back = node.prev;
                Some(&node.value)
            }
        }
    }
}

impl<T, W: Width> ExactSizeIterator for Iter<'_, T, W> {}
impl<T, W: Width> FusedIterator for Iter<'_, T, W> {}

impl<'a, T, W: Width> IntoIterator for &'a LinkedList<T, W> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T, W>;

    fn into_iter(self) -> Iter<'a, T, W> {
        self.iter()
    }
}

/// An owning iterator over the elements of a [`LinkedList`], front to back.
pub struct IntoIter<T, W: Width> {
    inner: IntoInner<T, W>,
}

enum IntoInner<T, W: Width> {
    Contiguous(alloc::vec::IntoIter<Node<T, W>>),
    Links(LinkedList<T, W>),
}

impl<T, W: Width> Iterator for IntoIter<T, W> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match &mut self.inner {
            IntoInner::Contiguous(iter) => iter.next().map(|node| node.value),
            IntoInner::Links(list) => list.pop_front(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match &self.inner {
            IntoInner::Contiguous(iter) => iter.len(),
            IntoInner::Links(list) => list.len(),
        };
        (len, Some(len))
    }
}

impl<T, W: Width> DoubleEndedIterator for IntoIter<T, W> {
    fn next_back(&mut self) -> Option<T> {
        match &mut self.inner {
            IntoInner::Contiguous(iter) => iter.next_back().map(|node| node.value),
            IntoInner::Links(list) => list.pop_back(),
        }
    }
}

impl<T, W: Width> ExactSizeIterator for IntoIter<T, W> {}
impl<T, W: Width> FusedIterator for IntoIter<T, W> {}

impl<T, W: Width> IntoIterator for LinkedList<T, W> {
    type Item = T;
    type IntoIter = IntoIter<T, W>;

    fn into_iter(self) -> IntoIter<T, W> {
        if self.contiguous {
            IntoIter {
                inner: IntoInner::Contiguous(self.nodes.into_iter()),
            }
        } else {
            IntoIter {
                inner: IntoInner::Links(self),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::LinkedList as StdList;
    use std::rc::Rc;
    use std::vec::Vec;

    /// Everything observable about a list, front to back, checked from both ends.
    fn contents<T: Clone, W: Width>(list: &LinkedList<T, W>) -> Vec<T> {
        let forward: Vec<T> = list.iter().cloned().collect();
        let mut backward: Vec<T> = list.iter().rev().cloned().collect();
        backward.reverse();
        assert_eq!(forward.len(), list.len());
        assert_eq!(backward.len(), forward.len());
        list.check_links();
        forward
    }

    #[derive(Clone, Debug)]
    enum Op {
        PushFront(u16),
        PushBack(u16),
        PopFront,
        PopBack,
    }

    fn apply<W: Width>(op: &Op, list: &mut LinkedList<u16, W>, model: &mut StdList<u16>) {
        match *op {
            Op::PushFront(x) => {
                list.push_front(x);
                model.push_front(x);
            }
            Op::PushBack(x) => {
                list.push_back(x);
                model.push_back(x);
            }
            Op::PopFront => assert_eq!(list.pop_front(), model.pop_front()),
            Op::PopBack => assert_eq!(list.pop_back(), model.pop_back()),
        }
        assert_eq!(contents(list), model.iter().copied().collect::<Vec<_>>());
        assert_eq!(list.front(), model.front());
        assert_eq!(list.back(), model.back());
    }

    /// Every sequence of up to `depth` operations over a two-value alphabet. Removing a node moves
    /// the last-stored node into its place, so this walks through every neighbourhood the move can
    /// patch: the head, the tail, the node before and after it, a lone node and a pair.
    fn all_sequences(depth: usize) -> Vec<Vec<Op>> {
        let alphabet = [Op::PushFront(1), Op::PushBack(2), Op::PopFront, Op::PopBack];
        let mut all = vec![Vec::new()];
        let mut layer = vec![Vec::new()];
        for _ in 0..depth {
            let mut next = Vec::new();
            for prefix in &layer {
                for op in &alphabet {
                    let mut sequence = prefix.clone();
                    sequence.push(op.clone());
                    next.push(sequence);
                }
            }
            all.extend(next.iter().cloned());
            layer = next;
        }
        all
    }

    #[test]
    fn every_short_sequence_of_pushes_and_pops_matches_std() {
        for (n, sequence) in all_sequences(7).iter().enumerate() {
            let (mut list, mut model) = (LinkedList::<u16, u32>::new(), StdList::new());
            // Distinct values, so a wrongly relinked node cannot hide behind an equal one.
            for (i, op) in sequence.iter().enumerate() {
                let op = match *op {
                    Op::PushFront(_) => Op::PushFront((n * 8 + i) as u16),
                    Op::PushBack(_) => Op::PushBack((n * 8 + i) as u16),
                    ref other => other.clone(),
                };
                apply(&op, &mut list, &mut model);
            }
        }
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => any::<u16>().prop_map(Op::PushFront),
            3 => any::<u16>().prop_map(Op::PushBack),
            2 => Just(Op::PopFront),
            2 => Just(Op::PopBack),
        ]
    }

    macro_rules! long_sequences_match_std {
        ($name:ident, $width:ty) => {
            proptest! {
                #![proptest_config(ProptestConfig::with_cases(64))]
                #[test]
                fn $name(ops in prop::collection::vec(op_strategy(), 0..300)) {
                    let (mut list, mut model) = (LinkedList::<u16, $width>::new(), StdList::new());
                    for op in &ops {
                        apply(op, &mut list, &mut model);
                    }
                    prop_assert_eq!(list.len(), model.len());
                }
            }
        };
    }
    long_sequences_match_std!(long_sequences_match_std_with_16_bit_links, u16);
    long_sequences_match_std!(long_sequences_match_std_with_32_bit_links, u32);
    long_sequences_match_std!(long_sequences_match_std_with_64_bit_links, u64);

    #[test]
    fn a_full_list_gives_the_value_back_and_works_again_after_a_pop() {
        // An 8-bit optional link stores index + 1, so it addresses 255 nodes.
        let mut list: LinkedList<u32, u8> = LinkedList::new();
        for i in 0..255 {
            list.try_push_back(i).unwrap();
        }
        assert_eq!(list.try_push_back(1000), Err(Full(1000)));
        assert_eq!(list.try_push_front(1001), Err(Full(1001)));
        assert_eq!(list.len(), 255);
        assert_eq!(contents(&list), (0..255).collect::<Vec<_>>());

        assert_eq!(list.pop_front(), Some(0));
        list.try_push_front(7).unwrap();
        list.pop_back();
        list.try_push_back(8).unwrap();
        assert_eq!(list.len(), 255);
        assert_eq!(list.front(), Some(&7));
        assert_eq!(list.back(), Some(&8));
        contents(&list);
    }

    #[test]
    #[should_panic(expected = "cannot address another node")]
    fn push_panics_where_try_push_returns_full() {
        let mut list: LinkedList<u8, u8> = LinkedList::new();
        for i in 0..256 {
            list.push_back(i as u8);
        }
    }

    #[test]
    fn an_append_that_does_not_fit_changes_neither_list() {
        let mut a: LinkedList<u32, u8> = (0..200).collect();
        let mut b: LinkedList<u32, u8> = (1000..1100).collect();
        assert!(a.try_append(&mut b).is_err());
        assert_eq!(contents(&a), (0..200).collect::<Vec<_>>());
        assert_eq!(contents(&b), (1000..1100).collect::<Vec<_>>());

        let mut c: LinkedList<u32, u8> = (2000..2055).collect();
        a.try_append(&mut c).unwrap();
        assert_eq!(a.len(), 255);
        assert!(c.is_empty());
        contents(&c);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn append_and_split_off_match_std(
            head in prop::collection::vec(any::<u16>(), 0..60),
            ops in prop::collection::vec(op_strategy(), 0..60),
            tail in prop::collection::vec(any::<u16>(), 0..60),
            at in 0usize..130,
        ) {
            // Scramble the storage order first, so link order and storage order differ.
            let mut list: LinkedList<u16, u16> = head.iter().copied().collect();
            let mut model: StdList<u16> = head.iter().copied().collect();
            for op in &ops {
                apply(op, &mut list, &mut model);
            }
            let mut other: LinkedList<u16, u16> = tail.iter().copied().collect();
            let mut other_model: StdList<u16> = tail.iter().copied().collect();
            list.append(&mut other);
            model.append(&mut other_model);
            prop_assert!(other.is_empty());
            prop_assert_eq!(contents(&list), model.iter().copied().collect::<Vec<_>>());

            let at = at.min(model.len());
            let split = list.split_off(at);
            let split_model = model.split_off(at);
            prop_assert_eq!(contents(&list), model.iter().copied().collect::<Vec<_>>());
            prop_assert_eq!(contents(&split), split_model.iter().copied().collect::<Vec<_>>());
        }

        #[test]
        fn the_iterator_meets_in_the_middle_without_repeating_or_skipping(
            items in prop::collection::vec(any::<u16>(), 0..80),
            from_back in prop::collection::vec(any::<bool>(), 0..100),
        ) {
            let list: LinkedList<u16, u32> = items.iter().copied().collect();
            let mut iter = list.iter();
            let (mut front, mut back) = (Vec::new(), Vec::new());
            for &take_back in &from_back {
                prop_assert_eq!(iter.len(), items.len() - front.len() - back.len());
                if take_back {
                    back.extend(iter.next_back().copied());
                } else {
                    front.extend(iter.next().copied());
                }
            }
            back.reverse();
            front.extend(iter.copied());
            front.extend(back);
            prop_assert_eq!(front, items);
        }
    }

    #[test]
    fn owned_iteration_runs_from_both_ends_and_drops_the_rest_exactly_once() {
        let marker = Rc::new(());
        let list: LinkedList<Rc<()>, u16> = (0..10).map(|_| Rc::clone(&marker)).collect();
        assert_eq!(Rc::strong_count(&marker), 11);
        let mut iter = list.into_iter();
        assert!(iter.next().is_some());
        assert!(iter.next_back().is_some());
        assert_eq!(Rc::strong_count(&marker), 9);
        drop(iter);
        assert_eq!(Rc::strong_count(&marker), 1);
    }

    #[test]
    fn clear_and_drop_release_every_element_once() {
        let marker = Rc::new(());
        let mut list: LinkedList<Rc<()>, u16> = LinkedList::new();
        for _ in 0..50 {
            list.push_back(Rc::clone(&marker));
            list.push_front(Rc::clone(&marker));
        }
        for _ in 0..30 {
            list.pop_front();
            list.pop_back();
        }
        assert_eq!(Rc::strong_count(&marker), 41);
        let copy = list.clone();
        assert_eq!(Rc::strong_count(&marker), 81);
        list.clear();
        assert_eq!(Rc::strong_count(&marker), 41);
        assert!(list.is_empty());
        contents(&list);
        drop(copy);
        assert_eq!(Rc::strong_count(&marker), 1);
    }

    #[test]
    fn a_clone_is_independent_and_equality_ignores_storage_order() {
        let mut a: LinkedList<u32, u32> = (0..20).collect();
        let b = a.clone();
        a.pop_front();
        a.push_front(0);
        a.pop_back();
        a.push_back(19);
        // `a` now stores its nodes in a different order than `b`, but holds the same sequence.
        assert_eq!(a, b);
        a.push_back(99);
        assert_ne!(a, b);
        assert_eq!(b.len(), 20);
    }

    #[test]
    fn contains_and_mutation_through_front_and_back() {
        let mut list: LinkedList<u32, u16> = (1..=3).collect();
        assert!(list.contains(&2));
        assert!(!list.contains(&9));
        *list.front_mut().unwrap() = 10;
        *list.back_mut().unwrap() = 30;
        assert_eq!(contents(&list), [10, 2, 30]);
        let mut sum = 0;
        list.for_each_mut(|x| {
            *x += 1;
            sum += *x;
        });
        assert_eq!(contents(&list), [11, 3, 31]);
        assert_eq!(sum, 45);
    }
    #[test]
    fn contiguity_tracks_storage_order_and_both_iterators_agree() {
        let mut list: LinkedList<u32, u32> = (0..10).collect();
        assert!(list.is_contiguous());
        assert_eq!(
            list.iter().copied().collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
        list.push_front(99);
        assert!(!list.is_contiguous());
        assert_eq!(
            list.iter().copied().collect::<Vec<_>>(),
            [99, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        list.clear();
        assert!(list.is_contiguous());

        let mut a: LinkedList<u32, u32> = (0..5).collect();
        let mut b: LinkedList<u32, u32> = (5..10).collect();
        a.append(&mut b);
        assert!(a.is_contiguous());
        assert_eq!(
            a.iter().copied().collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
        let mut c: LinkedList<u32, u32> = (100..102).collect();
        c.push_front(99);
        a.append(&mut c);
        assert!(!a.is_contiguous());
        assert_eq!(
            a.iter().copied().collect::<Vec<_>>(),
            (0..10).chain([99, 100, 101]).collect::<Vec<_>>()
        );

        let mut list: LinkedList<u32, u32> = (0..5).collect();
        assert_eq!(list.pop_back(), Some(4));
        assert!(list.is_contiguous());
        assert_eq!(list.pop_front(), Some(0));
        assert!(!list.is_contiguous());
        assert_eq!(list.into_iter().collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn a_contiguous_owned_iterator_matches_from_both_ends() {
        let list: LinkedList<u32, u16> = (0..100).collect();
        let mut iter = list.into_iter();
        let mut front = Vec::new();
        let mut back = Vec::new();
        loop {
            assert_eq!(iter.len(), 100 - front.len() - back.len());
            if front.len() + back.len() == 100 {
                break;
            }
            front.push(iter.next().unwrap());
            if front.len() + back.len() == 100 {
                break;
            }
            back.push(iter.next_back().unwrap());
        }
        back.reverse();
        front.extend(back);
        assert_eq!(front, (0..100).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod preallocated_tests {
    use super::*;

    #[test]
    fn a_preallocated_u16_list_still_stops_at_the_width_limit() {
        let mut list: LinkedList<u32, u16> = LinkedList::with_capacity(100_000);
        for i in 0..65_535 {
            list.try_push_back(i).unwrap();
        }
        assert_eq!(list.try_push_back(7), Err(Full(7)));
        assert_eq!(list.len(), 65_535);
        assert_eq!(list.front(), Some(&0));
        assert_eq!(list.back(), Some(&65_534));
        assert!(list.is_contiguous());
    }
}
