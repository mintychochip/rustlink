//! Set workloads: `rustlink-collections` `BTreeSet`/`HashSet` against std.
//!
//! Each workload builds a set of `n` shuffled distinct keys, then looks up, scans or churns
//! (remove half, insert them back). Sets store only keys, so these isolate the container overhead
//! that the map workloads hide behind an 8-byte value.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeSet as StdBTreeSet, HashSet as StdHashSet};
use std::hash::BuildHasher;

use rustlink::Width;
use rustlink_collections::{BTreeMap, BTreeSet, FnvBuildHasher, HashMap, HashSet};

use crate::containers::{inputs, timed};
use crate::measure::{Params, Pass};
use crate::workload::{Outcome, Workload};

const TREE_VARIANTS: &[&str] = &["std", "u16", "u32"];
const HASH_VARIANTS: &[&str] = &["std", "std-fnv", "u16", "u32"];
const MAX_N: usize = 1 << 20;
/// A u16 B+tree holds about 5.7M keys at 128-entry nodes; 2^22 fits and 2^23 does not.
const TREE_16_MAX_N: usize = 1 << 22;
/// A u16 hash set addresses one entry slot per key.
const HASH_16_MAX_N: u64 = 65_535;

pub fn workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(TreeSetWorkload { op: Op::Lookup }),
        Box::new(TreeSetWorkload { op: Op::Scan }),
        Box::new(TreeSetWorkload { op: Op::Churn }),
        Box::new(HashSetWorkload { op: Op::Lookup }),
        Box::new(HashSetWorkload { op: Op::Scan }),
        Box::new(HashSetWorkload { op: Op::Churn }),
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

fn build_std_tree(keys: &[u32]) -> StdBTreeSet<u32> {
    let mut set = StdBTreeSet::new();
    for &key in keys {
        set.insert(key);
    }
    set
}

fn build_rust_tree<W: Width>(keys: &[u32]) -> BTreeSet<u32, W> {
    let mut set = BTreeSet::new();
    for &key in keys {
        set.insert(key);
    }
    set
}

fn build_std_hash<S: BuildHasher + Default>(keys: &[u32]) -> StdHashSet<u32, S> {
    let mut set = StdHashSet::with_capacity_and_hasher(keys.len(), S::default());
    for &key in keys {
        set.insert(key);
    }
    set
}

fn build_rust_hash<W: Width>(keys: &[u32]) -> HashSet<u32, W> {
    let mut set = HashSet::with_capacity(keys.len());
    for &key in keys {
        set.insert(key);
    }
    set
}

fn lookup_std_tree(set: &StdBTreeSet<u32>, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|key| set.contains(key)).count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

fn lookup_rust_tree<W: Width>(set: &BTreeSet<u32, W>, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|key| set.contains(key)).count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

fn lookup_std_hash<S: BuildHasher>(set: &StdHashSet<u32, S>, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|key| set.contains(key)).count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

fn lookup_rust_hash<W: Width, S: BuildHasher>(set: &HashSet<u32, W, S>, probes: &[u32]) -> Pass {
    let hits = probes.iter().filter(|key| set.contains(key)).count() as u64;
    Pass {
        checksum: hits,
        units: probes.len() as u64,
    }
}

fn scan_keys<'a>(keys: impl Iterator<Item = &'a u32>) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for key in keys {
        checksum = checksum.wrapping_add(u64::from(*key));
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_std_tree(set: &mut StdBTreeSet<u32>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        checksum = checksum.wrapping_add(u64::from(set.remove(&key)) * u64::from(key));
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        set.insert(key);
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_rust_tree<W: Width>(set: &mut BTreeSet<u32, W>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        checksum = checksum.wrapping_add(u64::from(set.remove(&key)) * u64::from(key));
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        set.insert(key);
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_std_hash<S: BuildHasher>(set: &mut StdHashSet<u32, S>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        checksum = checksum.wrapping_add(u64::from(set.remove(&key)) * u64::from(key));
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        set.insert(key);
        units += 1;
    }
    Pass { checksum, units }
}

fn churn_rust_hash<W: Width, S: BuildHasher>(set: &mut HashSet<u32, W, S>, keys: &[u32]) -> Pass {
    let mut checksum = 0u64;
    let mut units = 0u64;
    for &key in keys.iter().step_by(2) {
        checksum = checksum.wrapping_add(u64::from(set.remove(&key)) * u64::from(key));
        units += 1;
    }
    for &key in keys.iter().step_by(2) {
        set.insert(key);
        units += 1;
    }
    Pass { checksum, units }
}

pub struct TreeSetWorkload {
    op: Op,
}

impl TreeSetWorkload {
    fn std(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_std_tree(keys),
                |set| lookup_std_tree(set, probes),
                self.op.unit(),
                0,
            ),
            Op::Scan => timed(
                params,
                || build_std_tree(keys),
                |set| scan_keys(set.iter()),
                self.op.unit(),
                0,
            ),
            Op::Churn => timed(
                params,
                || build_std_tree(keys),
                |set| churn_std_tree(set, keys),
                self.op.unit(),
                0,
            ),
        }
    }

    fn rust<W: Width>(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        let node_bytes = BTreeMap::<u32, (), W>::node_bytes();
        match self.op {
            Op::Lookup => timed(
                params,
                || build_rust_tree::<W>(keys),
                |set| lookup_rust_tree(set, probes),
                self.op.unit(),
                node_bytes,
            ),
            Op::Scan => timed(
                params,
                || build_rust_tree::<W>(keys),
                |set| scan_keys(set.iter()),
                self.op.unit(),
                node_bytes,
            ),
            Op::Churn => timed(
                params,
                || build_rust_tree::<W>(keys),
                |set| churn_rust_tree(set, keys),
                self.op.unit(),
                node_bytes,
            ),
        }
    }
}

impl Workload for TreeSetWorkload {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Lookup => "btreeset-lookup",
            Op::Scan => "btreeset-scan",
            Op::Churn => "btreeset-churn",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        TREE_VARIANTS
    }

    fn max_n(&self) -> usize {
        MAX_N
    }

    fn footprint_per_n(&self) -> usize {
        32
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (keys, probes) = inputs(params.n, params.seed);
        match variant {
            "std" => Ok(self.std(params, &keys, &probes)),
            "u16" => {
                if params.n > TREE_16_MAX_N {
                    return Ok(Outcome::Skipped(
                        "u16 links address 65,535 B+tree nodes".to_owned(),
                    ));
                }
                Ok(self.rust::<u16>(params, &keys, &probes))
            }
            "u32" => Ok(self.rust::<u32>(params, &keys, &probes)),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

pub struct HashSetWorkload {
    op: Op,
}

impl HashSetWorkload {
    fn std<S: BuildHasher + Default>(
        &self,
        params: &Params,
        keys: &[u32],
        probes: &[u32],
    ) -> Outcome {
        match self.op {
            Op::Lookup => timed(
                params,
                || build_std_hash::<S>(keys),
                |set| lookup_std_hash(set, probes),
                self.op.unit(),
                0,
            ),
            Op::Scan => timed(
                params,
                || build_std_hash::<S>(keys),
                |set| scan_keys(set.iter()),
                self.op.unit(),
                0,
            ),
            Op::Churn => timed(
                params,
                || build_std_hash::<S>(keys),
                |set| churn_std_hash(set, keys),
                self.op.unit(),
                0,
            ),
        }
    }

    fn rust<W: Width>(&self, params: &Params, keys: &[u32], probes: &[u32]) -> Outcome {
        let node_bytes = HashMap::<u32, (), W>::node_bytes();
        match self.op {
            Op::Lookup => timed(
                params,
                || build_rust_hash::<W>(keys),
                |set| lookup_rust_hash(set, probes),
                self.op.unit(),
                node_bytes,
            ),
            Op::Scan => timed(
                params,
                || build_rust_hash::<W>(keys),
                |set| scan_keys(set.iter()),
                self.op.unit(),
                node_bytes,
            ),
            Op::Churn => timed(
                params,
                || build_rust_hash::<W>(keys),
                |set| churn_rust_hash(set, keys),
                self.op.unit(),
                node_bytes,
            ),
        }
    }
}

impl Workload for HashSetWorkload {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Lookup => "hashset-lookup",
            Op::Scan => "hashset-scan",
            Op::Churn => "hashset-churn",
        }
    }

    fn variants(&self) -> &'static [&'static str] {
        HASH_VARIANTS
    }

    fn max_n(&self) -> usize {
        MAX_N
    }

    fn footprint_per_n(&self) -> usize {
        32
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let (keys, probes) = inputs(params.n, params.seed);
        match variant {
            "std" => Ok(self.std::<RandomState>(params, &keys, &probes)),
            "std-fnv" => Ok(self.std::<FnvBuildHasher>(params, &keys, &probes)),
            "u16" => {
                if params.n as u64 > HASH_16_MAX_N {
                    return Ok(Outcome::Skipped(
                        "u16 links address 65,535 entry slots".to_owned(),
                    ));
                }
                Ok(self.rust::<u16>(params, &keys, &probes))
            }
            "u32" => Ok(self.rust::<u32>(params, &keys, &probes)),
            other => Err(format!("unknown variant `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::Outcome;

    #[test]
    fn every_set_workload_runs_every_variant_at_1024() {
        let params = Params::new(1_024);
        for workload in workloads() {
            for &variant in workload.variants() {
                match workload.run(variant, &params) {
                    Ok(Outcome::Done(metrics)) => assert!(metrics.traverse.median > 0.0),
                    other => panic!("{} {variant}: {other:?}", workload.name()),
                }
            }
        }
    }

    #[test]
    fn std_and_rustlink_sets_agree_on_the_checksums() {
        let (keys, probes) = inputs(1_000, 21);
        let std_tree = build_std_tree(&keys);
        let rust_tree = build_rust_tree::<u32>(&keys);
        assert_eq!(
            lookup_std_tree(&std_tree, &probes).checksum,
            lookup_rust_tree(&rust_tree, &probes).checksum
        );
        assert_eq!(
            scan_keys(std_tree.iter()).checksum,
            scan_keys(rust_tree.iter()).checksum
        );
        let mut a = std_tree;
        let mut b = rust_tree;
        assert_eq!(
            churn_std_tree(&mut a, &keys).checksum,
            churn_rust_tree(&mut b, &keys).checksum
        );

        let std_hash = build_std_hash::<FnvBuildHasher>(&keys);
        let rust_hash = build_rust_hash::<u32>(&keys);
        assert_eq!(
            lookup_std_hash(&std_hash, &probes).checksum,
            lookup_rust_hash(&rust_hash, &probes).checksum
        );
        assert_eq!(
            scan_keys(std_hash.iter()).checksum,
            scan_keys(rust_hash.iter()).checksum
        );
        let mut a = std_hash;
        let mut b = rust_hash;
        assert_eq!(
            churn_std_hash(&mut a, &keys).checksum,
            churn_rust_hash(&mut b, &keys).checksum
        );
    }

    #[test]
    fn a_u16_hash_set_skips_sizes_past_its_slots() {
        let params = Params::new(65_536);
        let outcome = HashSetWorkload { op: Op::Lookup }
            .run("u16", &params)
            .unwrap();
        assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
    }
}
