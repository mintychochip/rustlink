//! Model-based tests for the B+tree.

use super::map::BTreeMap;
use super::set::BTreeSet;
use super::*;
use crate::error::Full;
use std::collections::{BTreeMap as StdMap, BTreeSet as StdSet};
use std::fmt::Debug;
use std::rc::Rc;
use std::vec::Vec;

/// Small deterministic generator; the crate has no RNG dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// Panics unless every structural invariant holds and the leaves hold `len` sorted entries.
fn check<K, V, W>(map: &BTreeMap<K, V, W>)
where
    K: Ord + Clone + Debug,
    V: Debug,
    W: Width,
{
    if map.len == 0 {
        assert!(map.root.get().is_none());
        assert!(map.first.get().is_none());
        assert!(map.last.get().is_none());
        return;
    }
    assert!(map.root.get().is_some());

    let mut at = map.first.get().expect("a non-empty tree has a first leaf");
    let mut previous = None;
    let mut keys: Vec<K> = Vec::new();
    let mut entries = 0;
    let mut leaves = 0;
    loop {
        let leaf = map.as_leaf(at);
        let is_root = map.root.get() == Some(at);
        assert!(leaf.entries.len() <= LEAF_CAP);
        if !is_root || leaves > 0 {
            assert!(
                leaf.entries.len() >= MIN_LEAF,
                "leaf {at} has {} entries",
                leaf.entries.len()
            );
        }
        for pair in leaf.entries.windows(2) {
            assert!(pair[0].0 < pair[1].0, "leaf {at} is not sorted");
        }
        keys.extend(leaf.entries.iter().map(|(key, _)| key.clone()));
        entries += leaf.entries.len();
        leaves += 1;
        assert_eq!(leaf.prev.get(), previous, "leaf {at} has a bad prev link");
        previous = Some(at);
        match leaf.next.get() {
            Some(next) => at = next,
            None => break,
        }
    }
    assert_eq!(previous, map.last.get());
    assert_eq!(entries, map.len);
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));

    let mut visited = 0;
    let root = map.root.get().expect("the root exists");
    let (depth, _) = check_node(map, root, true, &mut visited);
    assert!(depth <= MAX_HEIGHT);

    let mut seen = vec![false; map.nodes.len()];
    let mut free = map.free.get();
    let mut free_count = 0;
    while let Some(index) = free {
        assert!(!seen[index], "the free list repeats node {index}");
        seen[index] = true;
        free_count += 1;
        free = match &map.nodes[index] {
            Node::Free(next) => next.get(),
            _ => panic!("the free list led to a live node"),
        };
    }
    assert_eq!(free_count, map.free_count);
}

/// Returns the depth from `index` to its leaves and the smallest key below it.
fn check_node<K, V, W>(
    map: &BTreeMap<K, V, W>,
    index: usize,
    is_root: bool,
    visited: &mut usize,
) -> (usize, K)
where
    K: Ord + Clone + Debug,
    V: Debug,
    W: Width,
{
    *visited += 1;
    assert!(*visited <= map.nodes.len());
    match &map.nodes[index] {
        Node::Leaf(leaf) => (0, leaf.entries[0].0.clone()),
        Node::Inner(inner) => {
            assert_eq!(inner.keys.len() + 1, inner.kids.len());
            assert!(inner.keys.len() <= INNER_CAP);
            if is_root {
                assert!(inner.keys.len() <= INNER_CAP);
            } else {
                assert!(
                    inner.keys.len() >= MIN_INNER,
                    "inner {index} has {} separators",
                    inner.keys.len()
                );
            }
            assert!(
                inner.keys.windows(2).all(|pair| pair[0] < pair[1]),
                "inner {index} is not sorted"
            );
            let mut depth = None;
            let mut smallest = None;
            for (position, kid) in inner.kids.iter().enumerate() {
                let kid = kid.get().expect("a child link is live");
                let (kid_depth, min) = check_node(map, kid, false, visited);
                match depth {
                    None => {
                        depth = Some(kid_depth);
                        smallest = Some(min.clone());
                    }
                    Some(expected) => assert_eq!(kid_depth, expected, "leaves are uneven"),
                }
                if position > 0 {
                    assert_eq!(
                        inner.keys[position - 1],
                        min,
                        "inner {index} separator does not match its child"
                    );
                }
            }
            (
                depth.expect("an inner node has children") + 1,
                smallest.unwrap(),
            )
        }
        Node::Free(_) => panic!("a live link led to a free node"),
    }
}

fn assert_matches<K, V, W>(map: &BTreeMap<K, V, W>, model: &StdMap<K, V>)
where
    K: Ord + Clone + Debug,
    V: PartialEq + Debug,
    W: Width,
{
    check(map);
    assert_eq!(map.len(), model.len());
    assert_eq!(
        map.iter().map(|(k, v)| (k.clone(), v)).collect::<Vec<_>>(),
        model
            .iter()
            .map(|(k, v)| (k.clone(), v))
            .collect::<Vec<_>>()
    );
    assert_eq!(map.first_key_value(), model.first_key_value());
    assert_eq!(map.last_key_value(), model.last_key_value());
    let mut backward: Vec<(&K, &V)> = map.iter().rev().collect();
    backward.reverse();
    assert_eq!(backward, map.iter().collect::<Vec<_>>());
}

fn drive<W: Width>(steps: usize, key_space: u64) {
    let mut map: BTreeMap<u32, u32, W> = BTreeMap::new();
    let mut model: StdMap<u32, u32> = StdMap::new();
    let mut rng = Lcg(0x5EED);
    for step in 0..steps {
        let key = (rng.next() % key_space) as u32;
        match rng.next() % 4 {
            0..=2 => {
                let value = step as u32;
                assert_eq!(map.insert(key, value), model.insert(key, value));
            }
            _ => assert_eq!(map.remove(&key), model.remove(&key)),
        }
        if step.is_multiple_of(61) {
            assert_matches(&map, &model);
        }
    }
    assert_matches(&map, &model);
}

#[test]
fn random_inserts_and_removals_match_std_with_three_widths() {
    drive::<u16>(4_000, 600);
    drive::<u32>(8_000, 1_500);
    drive::<u64>(2_000, 300);
}

#[test]
fn ascending_and_descending_builds_remove_cleanly() {
    for ascending in [true, false] {
        let mut map: BTreeMap<u32, u32, u32> = BTreeMap::new();
        let keys: Vec<u32> = if ascending {
            (0..3_000).collect()
        } else {
            (0..3_000).rev().collect()
        };
        for &key in &keys {
            assert_eq!(map.insert(key, key * 2), None);
        }
        check(&map);

        let mut model: StdMap<u32, u32> = keys.iter().map(|&key| (key, key * 2)).collect();
        for (removed, &key) in keys.iter().enumerate() {
            if key.is_multiple_of(2) {
                assert_eq!(map.remove(&key), Some(key * 2));
                model.remove(&key);
            }
            if removed.is_multiple_of(100) {
                assert_matches(&map, &model);
            }
        }
        assert_matches(&map, &model);
        for &key in keys.iter().filter(|key| **key % 2 == 1) {
            assert_eq!(map.remove(&key), Some(key * 2));
            model.remove(&key);
        }
        assert!(map.is_empty());
        check(&map);
        assert_eq!(map.iter().next(), None);
    }
}

#[test]
fn pop_first_and_pop_last_match_std_from_both_ends() {
    let mut map: BTreeMap<u32, u32, u16> = (0..1_000).map(|key| (key, key)).collect();
    let mut model: StdMap<u32, u32> = (0..1_000).map(|key| (key, key)).collect();
    for turn in 0..1_000usize {
        if turn.is_multiple_of(2) {
            assert_eq!(map.pop_first(), model.pop_first());
        } else {
            assert_eq!(map.pop_last(), model.pop_last());
        }
        if turn.is_multiple_of(37) {
            check(&map);
        }
    }
    assert!(map.is_empty());
    check(&map);
}

#[test]
fn the_iterator_meets_in_the_middle_without_repeating_or_skipping() {
    let map: BTreeMap<u32, u32, u32> = (0..500).map(|key| (key, key * 3)).collect();
    let mut iter = map.iter();
    let (mut front, mut back) = (Vec::new(), Vec::new());
    let mut rng = Lcg(7);
    loop {
        assert_eq!(iter.len(), map.len() - front.len() - back.len());
        if rng.next().is_multiple_of(2) {
            match iter.next() {
                Some(entry) => front.push((*entry.0, *entry.1)),
                None => break,
            }
        } else {
            match iter.next_back() {
                Some(entry) => back.push((*entry.0, *entry.1)),
                None => break,
            }
        }
    }
    back.reverse();
    front.extend(back);
    assert_eq!(
        front,
        (0..500).map(|key| (key, key * 3)).collect::<Vec<_>>()
    );
}

#[test]
fn borrowed_keys_need_no_allocation() {
    let mut map: BTreeMap<String, u32, u16> = BTreeMap::new();
    map.insert("alpha".to_owned(), 1);
    map.insert("beta".to_owned(), 2);
    assert_eq!(map.get("alpha"), Some(&1));
    *map.get_mut("beta").unwrap() = 20;
    assert_eq!(map.remove("alpha"), Some(1));
    assert_eq!(map.len(), 1);
    check(&map);
}

#[test]
fn an_8_bit_tree_returns_full_and_stays_usable() {
    // 255 nodes at 16 entries per leaf is a few thousand entries; insert until the arena fills.
    let mut map: BTreeMap<u32, u32, u8> = BTreeMap::new();
    let mut accepted = Vec::new();
    let mut refused = None;
    for key in 0..100_000 {
        match map.try_insert(key, key) {
            Ok(previous) => {
                assert_eq!(previous, None);
                accepted.push(key);
            }
            Err(Full((key, value))) => {
                assert_eq!((key, value), (key, key));
                refused = Some(key);
                break;
            }
        }
    }
    assert!(refused.is_some(), "the 8-bit arena never filled");
    check(&map);
    for &key in &accepted {
        assert_eq!(map.get(&key), Some(&key));
    }
    assert_eq!(map.get(&refused.unwrap()), None);

    // Removing entries frees nodes; either outcome of the retry keeps the tree consistent.
    for &key in accepted.iter().take(300) {
        assert_eq!(map.remove(&key), Some(key));
    }
    check(&map);
    let mut inserted = 0;
    for key in 0..2_000 {
        if map.try_insert(key, key).is_ok() {
            inserted += 1;
        }
    }
    assert!(inserted > 0);
    check(&map);
    let keys: Vec<u32> = map.keys().copied().collect();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn clear_clone_equality_and_owned_iteration_drop_each_value_once() {
    let marker = Rc::new(());
    let mut map: BTreeMap<u32, Rc<()>, u16> = BTreeMap::new();
    for key in 0..200 {
        map.insert(key, Rc::clone(&marker));
    }
    assert_eq!(Rc::strong_count(&marker), 201);

    let clone = map.clone();
    assert_eq!(clone, map);
    assert_eq!(Rc::strong_count(&marker), 401);
    drop(clone);
    assert_eq!(Rc::strong_count(&marker), 201);

    let mut iter = map.clone().into_iter();
    assert_eq!(iter.len(), 200);
    assert!(iter.next().is_some());
    assert_eq!(iter.len(), 199);
    drop(iter);
    assert_eq!(Rc::strong_count(&marker), 201);

    map.clear();
    assert!(map.is_empty());
    check(&map);
    assert_eq!(Rc::strong_count(&marker), 1);
}

#[test]
fn the_set_matches_std_under_insert_and_remove() {
    let mut set: BTreeSet<u32, u16> = BTreeSet::new();
    let mut model: StdSet<u32> = StdSet::new();
    let mut rng = Lcg(99);
    for step in 0..4_000usize {
        let key = (rng.next() % 700) as u32;
        if rng.next().is_multiple_of(3) {
            assert_eq!(set.remove(&key), model.remove(&key));
        } else {
            assert_eq!(set.insert(key), model.insert(key));
        }
        if step.is_multiple_of(53) {
            assert_eq!(set.len(), model.len());
            assert_eq!(
                set.iter().copied().collect::<Vec<_>>(),
                model.iter().copied().collect::<Vec<_>>()
            );
            assert_eq!(set.first(), model.first());
            assert_eq!(set.last(), model.last());
        }
    }
    assert_eq!(set, set.clone());
    let owned: Vec<u32> = set.clone().into_iter().collect();
    assert_eq!(owned, model.iter().copied().collect::<Vec<_>>());
    let popped: Vec<u32> = std::iter::from_fn(|| set.pop_first()).collect();
    assert_eq!(popped, model.iter().copied().collect::<Vec<_>>());
    assert!(set.is_empty());
}

#[test]
fn the_set_borrows_queries() {
    let set: BTreeSet<String, u16> = ["one", "two"].into_iter().map(str::to_owned).collect();
    assert!(set.contains("one"));
    assert!(!set.contains("three"));
}

#[test]
fn a_single_entry_tree_is_a_leaf_root() {
    let mut map: BTreeMap<u8, u8, u8> = BTreeMap::new();
    map.insert(1, 2);
    check(&map);
    assert_eq!(map.remove(&1), Some(2));
    check(&map);
    assert!(map.is_empty());
    assert_eq!(map.remove(&1), None);
}
