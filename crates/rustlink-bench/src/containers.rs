//! Dynamic container workloads: `rustlink-collections` against the standard library.
//!
//! Seven workloads measure the containers the same way the tree benchmarks measure nodes: the
//! build inserts `n` entries in shuffled key order and the traversal does `n` units of work.
//!
//! | Workload | Build | Traversal | Unit |
//! |---|---|---|---|
//! | `btree-lookup`, `hash-lookup` | insert `n` entries | one probe per entry, half present | lookup |
//! | `btree-scan`, `hash-scan` | insert `n` entries | visit every entry | visit |
//! | `btree-churn`, `hash-churn` | insert `n` entries | remove half, insert them back | op |
//! | `list-scan` | `push_back` `n` values | visit every value | visit |
//!
//! Map variants are `std` (and `std-fnv` for the hash map), `u16` and `u32`; the number is the
//! width of the rustlink links. `u16` cells that cannot address the structure are `skipped`:
//! a B+tree's links address nodes (about one per eight entries), while a hash map's and a linked
//! list's address one slot per entry.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap as StdBTreeMap, HashMap as StdHashMap, LinkedList as StdList};
use std::hash::BuildHasher;

use rustlink::Width;
use rustlink_collections::{BTreeMap, FnvBuildHasher, HashMap, LinkedList};

use crate::measure::{Params, Pass, run_reps};
use crate::rng::Rng;
use crate::workload::{Outcome, Workload};

const MAP_VARIANTS: &[&str] = &["std", "u16", "u32"];
const HASH_VARIANTS: &[&str] = &["std", "std-fnv", "u16", "u32"];
const LIST_VARIANTS: &[&str] = &["std", "u16", "u32"];

/// The workload size limit: 2^20 entries keeps a churn pass under the default budget.
const MAX_N: usize = 1 << 20;
/// A `u16` B+tree addresses 65,535 nodes; random insertion keeps leaves about two-thirds full,
/// so 2^18 entries need roughly 37,000 nodes and 2^19 need about 74,000.
const BTREE_16_MAX_N: usize = 1 << 18;
/// A `u16` hash map or list addresses 65,535 entry slots.
const SLOT_16_MAX_N: u64 = 65_535;

/// The workloads this module provides.
pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(BTreeMapWorkload { op: Op::Lookup }),
        Box::new(BTreeMapWorkload { op: Op::Scan }),
        Box::new(BTreeMapWorkload { op: Op::Churn }),
        Box::new(HashMapWorkload { op: Op::Lookup }),
        Box::new(HashMapWorkload { op: Op::Scan }),
        Box::new(HashMapWorkload { op: Op::Churn }),
        Box::new(ListWorkload),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Lookup,
    Scan,
    Churn,
}

impl Op {
    fn unit(self) -> &'static str {
        match self {
            Op::Lookup => "lookup",
            Op::Scan => "visit",
            Op::Churn => "op",
        }
    }
}

/// `n` shuffled distinct keys and `n` probes, half present and half random.
fn inputs(n: usize, seed: u64) -> (Vec<u32>, Vec<u32>) {
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut keys: Vec<u32> = (0..n as u32).collect();
    Rng::new(seed ^ 0xA5A5_5A5A_1234_5678).shuffle(&mut keys);
    let mut rng = Rng::new(seed ^ 0x5EED_1234_9E37_79B9);
    let probes = (0..n)
        .map(|i| {
            if i.is_multiple_of(2) {
                keys[rng.below(n as u64) as usize]
            } else {
                rng.next_u32() % (2 * n as u32)
            }
        })
        .collect();
    (keys, probes)
}

fn value(key: u32) -> u64 {
    u64::from(key) * 2 + 1
}

fn timed<M>(
    params: &Params,
    build: impl FnMut() -> M,
    traverse: impl FnMut(&mut M) -> Pass,
    unit: &'static str,
    node_bytes: usize,
) -> Outcome {
    Outcome::from_reps(
        run_reps(params, params.n as u64, build, traverse),
        node_bytes,
        unit,
        None,
    )
}

fn build_std_btree(keys: &[u32]) -> StdBTreeMap<u32, u64> {
    let mut map = StdBTreeMap::new();
    for &key in keys {
        map.insert(key, value(key));
    }
    map
}

fn build_rust_btree<W: Width>(keys: &[u32]) -> BTreeMap<u32, u64, W> {
    let mut map = BTreeMap::with_capacity(keys.len());
    for &key in keys {
        map.insert(key, value(key));
    }
    map
}

fn build_std_hash<S: BuildHasher + Default>(keys: &[u32]) -> StdHashMap<u32, u64, S> {
    let mut map = StdHashMap::with_capacity_and_hasher(keys.len(), S::default());
    for &key in keys {
        map.insert(key, value(key));
    }
    map
}

fn build_rust_hash<W: Width>(keys: &[u32]) -> HashMap<u32, u64, W> {
    let mut map = HashMap::with_capacity(keys.len());
    for &key in keys {
        map.insert(key, value(key));
    }
    map
}

fn build_std_list(n: usize) -> StdList<u64> {
    let mut list = StdList::new();
    for value in 0..n as u64 {
        list.push_back(value);
    }
    list
}

fn build_rust_list<W: Width>(n: usize) -> LinkedList<u64, W> {
    let mut list = LinkedList::with_capacity(n);
    for value in 0..n as u64 {
        list.push_back(value);
    }
    list
}

fn lookup_std_btree(map: &StdBTreeMap<u32, u64>, probes: &[u32]) -> Pass {
    let mut checksum = 0u64;
    for &key in probes {
        if let Some(value) = map.get(&key) {
            checksum = checksum.wrapping_add(*value);
        }
    }
    Pass {
        checksum,
        units: probes.len() as u64,
    }
}

fn lookup_rust_btree<W: Width>(map: &BTreeMap<u32, u64, W>, probes: &[u32]) -> Pass {
    let mut checksum = 0u64;
    for &key in probes {
        if let Some(value) = map.get(&key) {
            checksum = checksum.wrapping_add(*value);
        }
    }
    Pass {
        checksum,
        units: probes.len() as u64,
    }
}

fn lookup_std_hash<S: BuildHasher>(map: &StdHashMap<u32, u64, S>, probes: &[u32]) -> Pass {
    let mut checksum = 0u64;
    for &key in probes {
        if let Some(value) = map.get(&key) {
            checksum = checksum.wrapping_add(*value);
        }
    }
    Pass {
        checksum,
        units: probes.len() as u64,
    }
}

fn lookup_rust_hash<W: Width, S: BuildHasher>(
    map: &HashMap<u32, u64, W, S>,
    probes: &[u32],
) -> Pass {
    let mut checksum = 0u64;
    for &key in probes {
        if let Some(value) = map.get(&key) {
            checksum = checksum.wrapping_add(*value);
        }
    }
    Pass {
        checksum,
        units: probes.len() as u64,
    }
}

fn scan_pairs(pairs: impl Iterator<Item = (u32, u64)>) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for (key, value) in pairs {
        checksum = checksum.wrapping_add(u64::from(key)).wrapping_add(value);
        units += 1;
    }
    Pass { checksum, units }
}

fn scan_std_btree(map: &StdBTreeMap<u32, u64>) -> Pass {
    scan_pairs(map.iter().map(|(key, value)| (*key, *value)))
}

fn scan_rust_btree<W: Width>(map: &BTreeMap<u32, u64, W>) -> Pass {
    scan_pairs(map.iter().map(|(key, value)| (*key, *value)))
}

fn scan_std_hash<S: BuildHasher>(map: &StdHashMap<u32, u64, S>) -> Pass {
    scan_pairs(map.iter().map(|(key, value)| (*key, *value)))
}

fn scan_rust_hash<W: Width, S: BuildHasher>(map: &HashMap<u32, u64, W, S>) -> Pass {
    scan_pairs(map.iter().map(|(key, value)| (*key, *value)))
}

fn scan_values<'a>(values: impl Iterator<Item = &'a u64>) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for value in values {
        checksum = checksum.wrapping_add(*value);
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_std_btree(map: &mut StdBTreeMap<u32, u64>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        if let Some(value) = map.remove(&key) {
            checksum = checksum.wrapping_add(value);
        }
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        map.insert(key, value(key).wrapping_add(1));
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_rust_btree<W: Width>(map: &mut BTreeMap<u32, u64, W>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        if let Some(value) = map.remove(&key) {
            checksum = checksum.wrapping_add(value);
        }
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        map.insert(key, value(key).wrapping_add(1));
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_std_hash<S: BuildHasher>(map: &mut StdHashMap<u32, u64, S>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        if let Some(value) = map.remove(&key) {
            checksum = checksum.wrapping_add(value);
        }
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        map.insert(key, value(key).wrapping_add(1));
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_rust_hash<W: Width, S: BuildHasher>(
    map: &mut HashMap<u32, u64, W, S>,
    keys: &[u32],
) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        if let Some(value) = map.remove(&key) {
            checksum = checksum.wrapping_add(value);
        }
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        map.insert(key, value(key).wrapping_add(1));
        units += 1;
    }
    Pass { checksum, units }
}

/// The B+tree map workloads.
pub struct BTreeMapWorkload {
    op: Op,
}

impl BTreeMapWorkload {
    fn run_std(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_std_btree(keys),
                |map| lookup_std_btree(map, probes),
                self.op.unit(),
                0,
            ),
            Op::Scan => timed(
                params,
                || build_std_btree(keys),
                |map| scan_std_btree(map),
                self.op.unit(),
                0,
            ),
            Op::Churn => timed(
                params,
                || build_std_btree(keys),
                |map| churn_std_btree(map, keys),
                self.op.unit(),
                0,
            ),
        }
    }

    fn run_rust<W: Width>(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_rust_btree::<W>(keys),
                |map| lookup_rust_btree(map, probes),
                self.op.unit(),
                BTreeMap::<u32, u64, W>::node_bytes(),
            ),
            Op::Scan => timed(
                params,
                || build_rust_btree::<W>(keys),
                |map| scan_rust_btree(map),
                self.op.unit(),
                BTreeMap::<u32, u64, W>::node_bytes(),
            ),
            Op::Churn => timed(
                params,
                || build_rust_btree::<W>(keys),
                |map| churn_rust_btree(map, keys),
                self.op.unit(),
                BTreeMap::<u32, u64, W>::node_bytes(),
            ),
        }
    }
}

impl Workload for BTreeMapWorkload {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Lookup => "btree-lookup",
            Op::Scan => "btree-scan",
            Op::Churn => "btree-churn",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        MAP_VARIANTS
    }

    fn max_n(&self) -> usize {
        MAX_N
    }

    fn footprint_per_n(&self) -> usize {
        64
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (keys, probes) = inputs(params.n, params.seed);
        match variant {
            "std" => Ok(self.run_std(params, &keys, &probes)),
            "u16" => {
                if params.n > BTREE_16_MAX_N {
                    return Ok(Outcome::Skipped(
                        "u16 links address 65,535 nodes, about 65,000 entries at this fill"
                            .to_owned(),
                    ));
                }
                Ok(self.run_rust::<u16>(params, &keys, &probes))
            }
            "u32" => Ok(self.run_rust::<u32>(params, &keys, &probes)),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

/// The chained hash map workloads.
pub struct HashMapWorkload {
    op: Op,
}

impl HashMapWorkload {
    fn run_std<S: BuildHasher + Default>(
        &self,
        params: &Params,
        keys: &[u32],
        probes: &[u32],
    ) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_std_hash::<S>(keys),
                |map| lookup_std_hash(map, probes),
                self.op.unit(),
                0,
            ),
            Op::Scan => timed(
                params,
                || build_std_hash::<S>(keys),
                |map| scan_std_hash(map),
                self.op.unit(),
                0,
            ),
            Op::Churn => timed(
                params,
                || build_std_hash::<S>(keys),
                |map| churn_std_hash(map, keys),
                self.op.unit(),
                0,
            ),
        }
    }

    fn run_rust<W: Width>(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_rust_hash::<W>(keys),
                |map| lookup_rust_hash(map, probes),
                self.op.unit(),
                HashMap::<u32, u64, W>::node_bytes(),
            ),
            Op::Scan => timed(
                params,
                || build_rust_hash::<W>(keys),
                |map| scan_rust_hash(map),
                self.op.unit(),
                HashMap::<u32, u64, W>::node_bytes(),
            ),
            Op::Churn => timed(
                params,
                || build_rust_hash::<W>(keys),
                |map| churn_rust_hash(map, keys),
                self.op.unit(),
                HashMap::<u32, u64, W>::node_bytes(),
            ),
        }
    }
}

impl Workload for HashMapWorkload {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Lookup => "hash-lookup",
            Op::Scan => "hash-scan",
            Op::Churn => "hash-churn",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        HASH_VARIANTS
    }

    fn max_n(&self) -> usize {
        MAX_N
    }

    fn footprint_per_n(&self) -> usize {
        64
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (keys, probes) = inputs(params.n, params.seed);
        match variant {
            "std" => Ok(self.run_std::<RandomState>(params, &keys, &probes)),
            "std-fnv" => Ok(self.run_std::<FnvBuildHasher>(params, &keys, &probes)),
            "u16" => {
                if params.n as u64 > SLOT_16_MAX_N {
                    return Ok(Outcome::Skipped(
                        "u16 links address 65,535 entry slots".to_owned(),
                    ));
                }
                Ok(self.run_rust::<u16>(params, &keys, &probes))
            }
            "u32" => Ok(self.run_rust::<u32>(params, &keys, &probes)),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

/// `list-scan`: build a list and visit every value.
pub struct ListWorkload;

impl Workload for ListWorkload {
    fn name(&self) -> &'static str {
        "list-scan"
    }

    fn variants(&self) -> &'static [&'static str] {
        LIST_VARIANTS
    }

    fn max_n(&self) -> usize {
        MAX_N
    }

    fn footprint_per_n(&self) -> usize {
        32
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        match variant {
            "std" => Ok(timed(
                params,
                || build_std_list(params.n),
                |list| scan_values(list.iter()),
                "visit",
                0,
            )),
            "u16" => {
                if params.n as u64 > SLOT_16_MAX_N {
                    return Ok(Outcome::Skipped(
                        "u16 links address 65,535 nodes".to_owned(),
                    ));
                }
                Ok(timed(
                    params,
                    || build_rust_list::<u16>(params.n),
                    |list| scan_values(list.iter()),
                    "visit",
                    LinkedList::<u64, u16>::node_bytes(),
                ))
            }
            "u32" => Ok(timed(
                params,
                || build_rust_list::<u32>(params.n),
                |list| scan_values(list.iter()),
                "visit",
                LinkedList::<u64, u32>::node_bytes(),
            )),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::Outcome;

    #[test]
    fn inputs_are_distinct_and_probes_are_half_hits() {
        let (keys, probes) = inputs(1_000, 9);
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..1_000).collect::<Vec<u32>>());
        assert_eq!(probes.len(), 1_000);
    }

    #[test]
    fn every_container_workload_runs_every_variant_at_1024() {
        let params = Params::new(1_024);
        for workload in workloads() {
            for &variant in workload.variants() {
                match workload.run(variant, &params) {
                    Ok(Outcome::Done(metrics)) => {
                        assert!(metrics.traverse.median > 0.0);
                    }
                    other => panic!("{} {variant}: {other:?}", workload.name()),
                }
            }
        }
    }

    #[test]
    fn std_and_rustlink_agree_on_the_checksums() {
        let (keys, probes) = inputs(1_000, 11);
        let std_btree = build_std_btree(&keys);
        let rust_btree = build_rust_btree::<u32>(&keys);
        assert_eq!(
            lookup_std_btree(&std_btree, &probes).checksum,
            lookup_rust_btree(&rust_btree, &probes).checksum
        );
        assert_eq!(
            scan_std_btree(&std_btree).checksum,
            scan_rust_btree(&rust_btree).checksum
        );
        let mut std_churn = std_btree;
        let mut rust_churn = rust_btree;
        assert_eq!(
            churn_std_btree(&mut std_churn, &keys).checksum,
            churn_rust_btree(&mut rust_churn, &keys).checksum
        );

        let std_hash = build_std_hash::<FnvBuildHasher>(&keys);
        let rust_hash = build_rust_hash::<u32>(&keys);
        assert_eq!(
            lookup_std_hash(&std_hash, &probes).checksum,
            lookup_rust_hash(&rust_hash, &probes).checksum
        );
        assert_eq!(
            scan_std_hash(&std_hash).checksum,
            scan_rust_hash(&rust_hash).checksum
        );
        let mut std_churn = std_hash;
        let mut rust_churn = rust_hash;
        assert_eq!(
            churn_std_hash(&mut std_churn, &keys).checksum,
            churn_rust_hash(&mut rust_churn, &keys).checksum
        );

        assert_eq!(
            scan_values(build_std_list(1_000).iter()).checksum,
            scan_values(build_rust_list::<u16>(1_000).iter()).checksum
        );
    }

    #[test]
    fn sizes_beyond_a_u16_trees_node_limit_are_skipped_without_building() {
        let params = Params::new(BTREE_16_MAX_N + 1);
        let outcome = BTreeMapWorkload { op: Op::Lookup }
            .run("u16", &params)
            .unwrap();
        assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");

        let params = Params::new(65_536);
        for workload in [
            Box::new(HashMapWorkload { op: Op::Lookup }) as Box<dyn Workload>,
            Box::new(ListWorkload) as Box<dyn Workload>,
        ] {
            let outcome = workload.run("u16", &params).unwrap();
            assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
        }
    }
}
