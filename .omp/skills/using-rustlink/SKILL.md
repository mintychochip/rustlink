---
name: using-rustlink
description: Use when choosing or implementing Rust pointer-dense structures with rustlink or no_std collections with rustlink-collections, when comparing BTreeMap/HashMap/LinkedList against std, or when choosing link widths, Arena/blob layouts, and capacity limits.
---

# Using rustlink

Core rule: replace 8-byte pointers with narrow indices only when links are a large share of a
node. The decode has a cost; narrow links win memory and often locality, not every loop.

## Decide

| Situation | Choose |
|---|---|
| Pointer-dense tree/graph, stable node indices, no mutation of the arena | `Arena<T>` + `Abs`/`Rel`; `u16` if the region/distance fits, otherwise `u32` |
| Frozen/zero-copy image | `Arena::freeze` + `BlobRef::verify`; nodes must be `Unaligned` |
| `no_std` + `alloc`, ordered map/set | `rustlink_collections::BTreeMap`/`BTreeSet` |
| std available, read-mostly ordered map/set | rustlink `BTreeMap`/`BTreeSet` for memory and lookup; std if build time dominates or you need `entry`/`split_off`/`append` |
| Hash map: memory or scans matter, values are not tiny | rustlink `HashMap`; it wins at 8-byte values |
| Hash map: lookup latency or untrusted keys | std `HashMap`; same-hasher lookup is slower at 2^18+, and FNV is not DoS-resistant |
| Hash set of small keys | std `HashSet` unless scan speed outweighs memory; chaining costs ~17 B/key vs ~10 |
| Linked list, append/ends only | rustlink `LinkedList`; contiguous builds get slice-fast scan |
| Linked list, random access or middle removal | `Vec`/`VecDeque`; the list falls back to link-following |

**Width limits:** a `u16` B+tree addresses 65,535 nodes (~5.7M entries at 128-entry nodes). A
`u16` hash map/set/list addresses one slot per element: **65,535 entries maximum**. Default to
`u32` unless the cap is proven safe. `Full` is returned by `try_*`; plain methods panic.

## Quick API

```rust
use rustlink::{Arena, BlobRef, OptAbs, Region, impl_links};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
#[repr(C)]
struct Node { key: U32, left: OptAbs<Node, u16>, right: OptAbs<Node, u16> }
impl_links!(Node, schema = 1, links = [left, right]);

let mut arena = Arena::new();
let left = arena.push(Node { key: U32::new(1), left: OptAbs::NULL, right: OptAbs::NULL });
arena.push(Node { key: U32::new(2), left: OptAbs::new(Some(left)).unwrap(), right: OptAbs::NULL });
let root = arena.cursor(1).unwrap();
let child = root.follow(root.get().left).unwrap();
let blob = arena.freeze();
let view = BlobRef::<Node>::verify(&blob).unwrap();
```

`Abs` is an index; `Rel` is a signed delta from the owning element. `Opt*` reserves null;
`OptRel` cannot point at its owner (delta 0 is null). Choose a width by the maximum region size
(`Abs`) or maximum link distance (`Rel`).

```rust
use rustlink_collections::{BTreeMap, HashMap, LinkedList};

let mut map: BTreeMap<u32, u64, u32> = BTreeMap::with_capacity(1 << 20);
assert!(map.try_insert(1, 10).is_ok()); // Err(Full((K, V))) at the width limit
map.get(&1); map.range(10..20); map.for_each_mut(|_, v| *v += 1);
map.shrink_to_fit();                    // read-mostly compaction

let mut hash: HashMap<u32, u64, u32> = HashMap::with_capacity(1 << 20);
hash.iter_mut().for_each(|(_, v)| *v += 1);
hash.retain(|k, _| k % 2 == 0);         // one pass, rehashes survivors
hash.shrink_to_fit();                   // compacts dropped slots

let mut list: LinkedList<u64, u16> = LinkedList::with_capacity(65_535);
list.push_back(1);
assert!(list.is_contiguous());          // iter/into_iter use a slice fast path
```

Bounds: `BTreeMap` needs `K: Clone`; `HashMap` needs `K: Default` and `V: Default`; `LinkedList`
has no extra bounds. `BTreeSet`/`HashSet` wrap the maps. `BTreeMap` has no `iter_mut`; use
`for_each_mut` or per-key `get_mut`.

## Performance rules

- B+tree leaves/inner nodes are **128 entries**. That was the key optimization: at 2^20 entries
  the map uses 18.4 vs std's 20.7 B/entry and lookup is 0.65× std; build is 1.14×. Do not shrink
  nodes without re-running the differential suite at 8-entry nodes.
- Keep lists contiguous (`push_back`, removals from the back). Contiguous scan is 0.13–0.63× std;
  after `push_front` or middle removal it follows links and loses the fast path.
- Hash map: 25 vs 34 B/entry at 8-byte values, scan 0.34–0.45×, but same-hasher lookup 0.80–1.22×.
  Do not attribute the win over std defaults to layout; it is mostly FNV vs SipHash.
- Hash set: 17 vs 10 B/key; only its scan wins. Use std if memory matters.
- `shrink_to_fit`, `range` and `for_each_mut` are the high-value APIs for read-mostly or bulk
  work. `retain` wins when removing almost everything, loses when removing half.
- Do not compare std `collect()` (bulk-load) with rustlink explicit inserts; use the same loop.

## Common mistakes

| Mistake | Fix |
|---|---|
| "Narrow links are always faster" | Decode costs; compare per workload, memory first |
| Using `u16` for >65,535 hash/list entries | `try_*` returns `Full`; plain methods panic |
| Using a linked list as a general sequence | `Vec`/`VecDeque`; list only for ends/contiguous builds |
| Expecting `HashSet` to save memory | It does not for small keys; see 17 vs 10 B/key |
| Assuming std `BTreeSet` build is comparable | `collect()` bulk-loads a dense tree; use explicit inserts |
| Thinking `BTreeMap` lacks `with_capacity` | It has it, as do `HashMap`, `HashSet` and `LinkedList` |
| Looking for `iter_mut` on `BTreeMap` | Use `for_each_mut` |

Benchmark records and method: `bench/containers/results.md`, `bench/sets/results.md`, and each
record's `environment.txt`.
