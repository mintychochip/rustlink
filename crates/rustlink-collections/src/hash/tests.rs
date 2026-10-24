//! Model-based tests for the chained hash map and set.

use super::map::HashMap;
use super::set::HashSet;
use super::*;
use crate::error::Full;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap as StdMap, HashSet as StdSet};
use std::fmt::Debug;
use std::hash::Hash;
use std::rc::Rc;
use std::vec::Vec;

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

fn check<K, V, W, S>(map: &HashMap<K, V, W, S>)
where
    K: Eq + Hash + Default + Clone + Debug,
    V: Default + PartialEq + Debug,
    W: Width,
    S: core::hash::BuildHasher,
{
    assert_eq!(map.entries.len(), map.live.len());
    assert_eq!(map.live.iter().filter(|live| **live).count(), map.len);
    assert_eq!(
        map.live.iter().filter(|live| !**live).count(),
        map.free_count
    );
    assert!(map.buckets.len().is_power_of_two() || map.buckets.is_empty());
    if map.len > 0 {
        assert!(
            map.len * LOAD_DEN <= map.buckets.len() * LOAD_NUM,
            "load factor exceeded"
        );
    }

    let mut seen = vec![false; map.entries.len()];
    let mut chained = 0;
    for head in &map.buckets {
        let mut at = head.get();
        while let Some(index) = at {
            assert!(index < map.entries.len());
            assert!(map.live[index], "a bucket chain reached a free slot");
            assert!(!seen[index], "an entry appears in two chains");
            seen[index] = true;
            chained += 1;
            at = map.entries[index].next.get();
        }
    }
    assert_eq!(chained, map.len, "some live entries are unreachable");

    let mut free_seen = vec![false; map.entries.len()];
    let mut at = map.free.get();
    let mut free = 0;
    while let Some(index) = at {
        assert!(!map.live[index], "the free list reached a live slot");
        assert!(!free_seen[index], "the free list repeats a slot");
        free_seen[index] = true;
        free += 1;
        at = map.entries[index].next.get();
    }
    assert_eq!(free, map.free_count);

    for (index, live) in map.live.iter().enumerate() {
        if !*live {
            assert_eq!(map.entries[index].key, K::default());
            assert_eq!(map.entries[index].value, V::default());
        }
    }

    // Every live entry chains from the bucket its key hashes to.
    for (index, live) in map.live.iter().enumerate() {
        if !*live {
            continue;
        }
        let bucket =
            HashMap::<K, V, W, S>::bucket_of(map.hash(&map.entries[index].key), map.buckets.len());
        let mut at = map.buckets[bucket].get();
        let mut found = false;
        while let Some(entry) = at {
            if entry == index {
                found = true;
                break;
            }
            at = map.entries[entry].next.get();
        }
        assert!(found, "live entry {index} is not in its bucket");
    }

    assert_eq!(map.iter().count(), map.len);
}

fn assert_matches<K, V, W, S>(map: &HashMap<K, V, W, S>, model: &StdMap<K, V>)
where
    K: Eq + Hash + Default + Clone + Debug + Ord,
    V: Default + PartialEq + Debug,
    W: Width,
    S: core::hash::BuildHasher,
{
    check(map);
    assert_eq!(map.len(), model.len());
    for (key, value) in map.iter() {
        assert_eq!(model.get(key), Some(value));
    }
    for (key, value) in model {
        assert_eq!(map.get(key), Some(value));
    }
    let mut actual: Vec<(&K, &V)> = map.iter().collect();
    actual.sort_by(|a, b| a.0.cmp(b.0));
    let mut expected: Vec<(&K, &V)> = model.iter().collect();
    expected.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(actual, expected);
}

fn drive<W: Width>(steps: usize, key_space: u64) {
    let mut map: HashMap<u64, u64, W> = HashMap::new();
    let mut model: StdMap<u64, u64> = StdMap::new();
    let mut rng = Lcg(0xC0FFEE);
    for step in 0..steps {
        let key = rng.next() % key_space;
        match rng.next() % 4 {
            0..=2 => {
                let value = step as u64;
                assert_eq!(map.insert(key, value), model.insert(key, value));
            }
            _ => assert_eq!(map.remove(&key), model.remove(&key)),
        }
        if step.is_multiple_of(71) {
            assert_matches(&map, &model);
        }
    }
    assert_matches(&map, &model);
}

#[test]
fn random_inserts_and_removals_match_std_with_three_widths() {
    drive::<u16>(6_000, 900);
    drive::<u32>(12_000, 2_500);
    drive::<u64>(2_000, 200);
}

#[test]
fn growth_and_shrinking_slot_use_stay_consistent() {
    let mut map: HashMap<u32, u32, u32> = HashMap::new();
    let mut model: StdMap<u32, u32> = StdMap::new();
    for key in 0..5_000u32 {
        assert_eq!(map.insert(key, key * 2), model.insert(key, key * 2));
    }
    assert_matches(&map, &model);
    for key in 0..5_000u32 {
        if key.is_multiple_of(2) {
            assert_eq!(map.remove(&key), model.remove(&key));
        }
    }
    assert_matches(&map, &model);
    for key in 5_000..9_000 {
        assert_eq!(map.insert(key, key), model.insert(key, key));
    }
    assert_matches(&map, &model);
    for key in (0..9_000).step_by(3) {
        assert_eq!(map.remove(&key), model.remove(&key));
    }
    assert_matches(&map, &model);
}

#[test]
fn borrowed_keys_and_a_custom_hasher_work() {
    let mut map: HashMap<String, u32, u16, RandomState> = HashMap::with_hasher(RandomState::new());
    map.insert("alpha".to_owned(), 1);
    map.insert("beta".to_owned(), 2);
    assert_eq!(map.get("alpha"), Some(&1));
    assert_eq!(map.remove("beta"), Some(2));
    check(&map);
}

#[test]
fn an_8_bit_map_returns_full_and_recovers_after_removal() {
    let mut map: HashMap<u32, u32, u8> = HashMap::new();
    let mut accepted = Vec::new();
    let mut refused = None;
    for key in 0..10_000 {
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
    assert_eq!(accepted.len(), 255);
    assert!(refused.is_some());
    check(&map);
    for &key in &accepted {
        assert_eq!(map.get(&key), Some(&key));
    }
    for &key in accepted.iter().take(100) {
        assert_eq!(map.remove(&key), Some(key));
    }
    check(&map);
    for key in 0..100 {
        map.try_insert(key, key).unwrap();
    }
    check(&map);
    assert_eq!(map.len(), 255);
}

#[test]
fn clear_clone_equality_and_owned_iteration_drop_each_value_once() {
    let marker = Rc::new(());
    let mut map: HashMap<u32, Rc<()>, u16> = HashMap::new();
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
    let mut set: HashSet<u32, u16> = HashSet::new();
    let mut model: StdSet<u32> = StdSet::new();
    let mut rng = Lcg(1234);
    for step in 0..5_000usize {
        let key = (rng.next() % 800) as u32;
        if rng.next().is_multiple_of(3) {
            assert_eq!(set.remove(&key), model.remove(&key));
        } else {
            assert_eq!(set.insert(key), model.insert(key));
        }
        if step.is_multiple_of(83) {
            assert_eq!(set.len(), model.len());
            let mut actual: Vec<u32> = set.iter().copied().collect();
            let mut expected: Vec<u32> = model.iter().copied().collect();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected);
        }
    }
    assert_eq!(set, set.clone());
    let owned: Vec<u32> = set.clone().into_iter().collect();
    let mut actual = owned;
    let mut expected: Vec<u32> = model.iter().copied().collect();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn mutable_visits_and_retain_match_the_model() {
    let mut map: HashMap<u32, u64, u16> = (0..500).map(|k| (k, u64::from(k))).collect();
    map.for_each_mut(|key, value| *value = value.wrapping_add(u64::from(*key)));
    for (_, value) in map.iter_mut() {
        *value = value.wrapping_add(1);
    }
    for key in 0..500 {
        assert_eq!(map.get(&key), Some(&(u64::from(key) * 2 + 1)));
    }
    let before = map.len();
    map.retain(|key, _| key % 2 == 0);
    assert_eq!(map.len(), before.div_ceil(2));
    assert_eq!(map.len(), 250);
    check(&map);
    for key in 0..500 {
        assert_eq!(map.get(&key).is_some(), key % 2 == 0);
    }
    map.shrink_to_fit();
    check(&map);
    assert_eq!(map.len(), 250);
}
