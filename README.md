# rustlink

Narrow typed links for Rust: 1-, 2-, 4- or 8-byte offsets that replace 8-byte pointers inside
pointer-dense data structures. It adapts the "links narrower than pointers" mechanism of the
[Goose language](https://github.com/aardappel/goose) to Rust and ships with a benchmark suite that
measures where narrow links pay.

## What it provides

* Four link types, all `Copy`, 1-byte aligned and stored little-endian:
  * `Abs` / `OptAbs`: an element index. The width bounds the *region size*.
  * `Rel` / `OptRel`: a signed element delta from the element that stores the link. The width
    bounds the *link distance*, so a region can be arbitrarily large when links are local.
  * `Opt*` forms reserve a value for null. Every width (`u8`, `u16`, `u32`, `u64`) works with
    every form.
* `Region`: bounds-checked resolution. A link applied to the wrong region, a corrupted link or a
  dangling link yields `None` or `Err`, never undefined behavior.
* `Arena<T>`: an append-only array whose indices never change, so links inside it never dangle.
* A frozen, position-independent blob: `Arena::freeze` writes a header plus the raw elements, and
  `BlobRef::verify` checks the header, the exact length, the element size, a schema id and every
  link before returning a zero-copy view.
* `rustlink-collections`: `LinkedList`, `BTreeMap`/`BTreeSet` and `HashMap`/`HashSet` whose nodes
  live in a `Vec` and point at each other with `OptAbs` links of a width you choose. All are
  `#![no_std]` with `alloc` and contain no `unsafe`.
* No `unsafe` code, `#![no_std]` with `alloc`, no feature flags.

## Reach

Links count elements of `T`, not bytes.

| Width | `Abs`: elements addressable | `Rel`: delta range |
|---|---|---|
| `u8` | 256 | -128 to 127 |
| `u16` | 65,536 | -32,768 to 32,767 |
| `u32` | 4,294,967,296 | -2^31 to 2^31 - 1 |
| `u64` | 2^64 | -2^63 to 2^63 - 1 |

`OptAbs` addresses one element fewer than `Abs` of the same width. `OptRel` cannot point an element
at itself (delta 0 is null).

## Example

```rust
use rustlink::{Arena, BlobRef, OptAbs, Region, impl_links};
use zerocopy::byteorder::little_endian::U32;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

// A tree node: a key and two optional links of 2 bytes each. The node is 8 bytes; the same node
// with two 8-byte pointers is 24.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy)]
#[repr(C)]
struct Node {
    key: U32,
    left: OptAbs<Node, u16>,
    right: OptAbs<Node, u16>,
}
impl_links!(Node, schema = 0x4558_414D_0001, links = [left, right]);

let mut arena = Arena::new();
let leaf = arena.push(Node { key: U32::new(1), left: OptAbs::NULL, right: OptAbs::NULL });
arena.push(Node { key: U32::new(2), left: OptAbs::new(Some(leaf))?, right: OptAbs::NULL });
assert_eq!(size_of::<Node>(), 8);

// Links are resolved against the arena with a bounds check.
let root = arena.cursor(1).unwrap();
let left = root.follow(root.get().left).unwrap();
assert_eq!(left.get().key.get(), 1);

// A frozen image holds no addresses. Verify it when loading it back.
let image = arena.freeze();
let view = BlobRef::<Node>::verify(&image)?;
assert_eq!(view.len(), 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Node types must be `Unaligned` to be frozen: use `zerocopy`'s little-endian integer types for
scalar fields. Links into another region are not range-checked by `verify`; they stay
bounds-checked whenever they are followed.

## When it pays

Measured on an Intel Core Ultra 7 270K Plus (processes pinned to cpu2) with binaries built by rustc 1.99.0-nightly from git 327e31f. `bench/results.md` has every cell and `bench/environment.txt` says how the numbers were taken. Each figure is the median of five repetitions in one process, from the fastest of the sweeps that ran with fewer than two other CPUs busy. "Best narrow variant" is the `abs*`/`rel*` variant with the lowest heap bytes per node that fits, the faster one among equals. `u32` is the hand-written `u32` index variant and `box` the pointer variant, both at the same N. Heap is bytes per node (per vertex for the graphs); times are ns per lookup, visit or edge.

| Scenario | Workload and size | Best narrow variant | Heap B/node: narrow, `u32`, `box` | Traverse ns: narrow, `u32` | `box` time / `u32` time |
|---|---|---|---|---|---|
| Pointer-dense nodes, good locality | `bst-bulk` at 2^22 | `abs32` | 12, 12, 24 | 150.8, 152.5 | 1.69 |
| Pointer-dense nodes, poor locality | `bst-random` at 2^20 | `abs32` | 12, 12, 24 | 187.6, 189.5 | 2.28 |
| Expression trees built bottom-up | `ast` at 2^22 | `abs32` | 12, 12, 24 | 10.2, 10.1 | 1.31 |
| Graph edge lists, local (B per vertex, ns per edge) | `graph-linked-seq` at 2^20 | `rel8` | 24, 36, 72 | 16.1, 13.0 | about 1.4 (†) |
| Graph edge lists, scattered (B per vertex, ns per edge) | `graph-linked-shuf` at 2^20 | `abs32` (no narrower variant fits) | 36, 36, 72 | 27.4, 25.8 | about 1.3 (†) |

(†) The pointer variant of the two linked-list workloads was fixed after the sweeps: it allocated each vertex's edges one vertex at a time, whatever the slot order, so it was neither scattered in `graph-linked-shuf` nor laid out forwards in `graph-linked-seq`. The machine was too busy afterwards for a valid measurement, so those 14 `box` cells are `skip` in the report. The ratios come from paired runs of `box` and `u32` within a minute of each other, the median of 24 rounds (interquartile ranges 1.28 to 1.63 and 1.21 to 1.43). They are less certain than the other rows; `bench/environment.txt` has the evidence and the command that fills the cells on an idle machine.

* **Against pointers, 32-bit links halve the heap of tree nodes and win where memory is the bottleneck.** Tree nodes shrink from 24 to 12 bytes; lookups are 1.7x faster in `bst-bulk` at 2^22 and 2.3x faster in `bst-random` at 2^20, and expression-tree visits are 1.3x faster.
* **On linked edge lists narrow links save memory; their speed advantage over pointers is small and does not last.** The heap per vertex shrinks from 72 bytes (pointers) to 36 (32-bit links), 28 (`rel16`) and 24 (`rel8`). In paired runs the pointer variant takes 1.3x to 1.4x the `u32` time at 2^20 but 0.8x to 0.9x at 2^22, in both layouts (†); on sequential lists at 2^22, `rel8` and `rel16` take 1.5x the `u32` time, so they are slower than pointers there.
* **Against hand-written `u32` indices, in the first record's workloads, 32-bit links save nothing and cost between -14% and +17% traversal time** (N of 2^14 and up). `abs32` is faster in balanced, bulk-built trees (0.86x to 0.99x of `u32`), mixed in randomly built trees (0.92x to 1.17x), level in expression trees (0.98x to 1.05x) and slower in CSR and linked-list loops (1.04x to 1.17x): decoding a typed link is not free in a tight loop, and in the link-dense sweeps of the next section it costs 1.2x to 2.4x. `rel32` costs more than `abs32`: 0.98x to 1.04x on expression trees, up to 1.22x on `bst-bulk` and up to 1.36x on linked lists.
* **Links narrower than 32 bits pay only where links are local.** In the first record's workloads `rel8` and `rel16` are the only sub-32-bit variants that reach large sizes, and only on `graph-linked-seq`, where a vertex's edges are consecutive and every link spans one slot: 24 and 28 bytes per vertex instead of 36 (1.5x and 1.3x smaller), at 1.25x and 1.28x the traversal time of `u32` at 2^20 and 1.5x at 2^22. Elsewhere they are `n/a`: on trees `abs16` and `rel16` stop at 2^14 nodes and `rel8` does not fit even 2^10 (in `bst-bulk` at 2^10 the longest link spans 512 elements, though 99.3% span at most 127); on shuffled edge lists `abs16` and `rel16` stop at 2^12 and `rel8` never fits. A fixed-width link must fit the longest link, and the few long links near a tree's root decide the width. Variable-width (varint) links, the optional layer 4 of the design, target exactly this shape. The node graphs of the next section show the same rule with more room: `rel16` fits a mesh and a banded graph at every size, and a random graph only up to 2^15 nodes.
* **A verified blob loads about four times faster than the arena rebuilds.** At 2^22 nodes `BlobRef::verify` takes 1.33 ns per node (`abs32`; 1.12 for `rel32`) against 5.41 ns (4.95) to rebuild the arena from its plan (`blob-load`). Lookups through the verified view cost 151.2 ns (`abs32`) against 150.8 ns through the arena (`bst-bulk`); for `rel32` 168.2 ns against 158.6 ns, 6% apart and within the noise.
* **The layer-4 gate passes.** The `ast-bytes` prototype (variable-size records, LEB128 links) uses 3.26 bytes per node against 12 for the best layer-2 variant (3.68x less heap) and traverses in 0.81x to 0.84x of its time. Layer 4 is not built; the gate table, from the report, is:

| N | best L2 variant | its heap B/node | ast-bytes heap B/node | memory ratio | its traverse ns | ast-bytes traverse ns | time ratio | verdict |
|---|---|---:|---:|---:|---:|---:|---:|---|
| 2^20 | abs32 | 12.0 | 3.3 | 3.68 | 9.5 | 7.9 | 0.84 | pass |
| 2^22 | abs32 | 12.0 | 3.3 | 3.68 | 10.2 | 8.3 | 0.81 | pass |
| 2^24 | rel32 | 12.0 | 3.3 | 3.68 | 10.4 | 8.5 | 0.81 | pass |

Rule: proceed to layer 4 only if `ast-bytes` uses at least 1.5x less heap than the best layer-2 variant at no more than 1.1x its traversal time, at N of 2^20 and up.

Established elsewhere: replacing 64-bit pointers with 32-bit offsets is a proven win where pointers dominate the heap. V8's pointer compression reduced V8 heap size by up to 43%, and HotSpot's compressed oops exist for the same reason. Those results belong to those projects, not to this crate.

### Node graphs and B-trees

A second record, `bench/decisive/`, measures structures in which links are most of each node, against the baselines people write: `Rc` nodes with `Weak` links, `usize` and `u32` indices, and `std::collections::BTreeSet`. A graph node is a `u32` payload and four links. `ngraph-mesh` (a grid: neighbours one slot and one row away), `ngraph-banded` (four random links within 1,000 slots) and `ngraph-random` (four random links anywhere) are swept in index order; `ngraph-chase` walks the random graph one dependent step at a time. The B-trees (`btree-64`, `btree-256`) are packed, bulk-loaded and static, with 64- or 256-byte nodes, so link width sets the fanout. The predictions were written before the first measurement; `docs/superpowers/specs/2026-10-23-decisive-workloads-design.md` lists them with their outcome.

Heap bytes are exact. Every time is a ratio between two variants taken inside one sweep, so both saw the same machine, and each figure is the median over the calm sweeps: 14 to 16 of the 37 repetitions of each workload's matrix, built from git 5649039. The machine was shared and never quiet; `bench/decisive/environment.txt` defines calm, gives the interquartile ranges and shows how the load moves each ratio. Taking all 37 repetitions instead of the calm ones moves 11 of the 55 figures quoted in this section by more than 0.05, all in cells near a cache boundary (the mesh at 2^22, the chases at 2^18 and 2^20, the B-trees at 2^22): eight move toward 1 and three (the 2^18 chases of `Rc` and `usize`, and `BTreeSet` in `btree-64`) away from it. None reverses a comparison made here, and `bench/decisive/ratios.tsv` holds both medians for every cell.

| Heap bytes per node | `Rc` + `Weak` | `usize` | `u32`, `abs32`, `rel32` | `abs16`, `rel16` | `rel8` |
|---|---:|---:|---:|---:|---:|
| graph node (payload and four links) | 64 | 40 | 20 | 12 | 8 |

| Heap bytes per key at 2^22 keys | `BTreeSet` | `box` nodes | `usize` | `u32` | `abs32` | `abs16` |
|---|---:|---:|---:|---:|---:|---:|
| 64-byte nodes | 5.82 | 6.00 | 5.33 | 4.88 | 4.72 | 4.56 (at 2^18) |
| 256-byte nodes | 5.82 | 4.40 | 4.27 | 4.19 | 4.16 | 4.13 (at 2^20) |

Time relative to hand-written `u32` indices; below 1 is faster, `n/a` means the links do not fit. Sweeps are per edge and chases per step; 2^16 nodes of `u32` are 1.3 MB and 2^22 are 84 MB.

| Graph time / `u32` time | mesh 2^16 | mesh 2^22 | banded 2^22 | random 2^22 | chase 2^18 | chase 2^22 |
|---|---:|---:|---:|---:|---:|---:|
| `Rc` + `Weak` | 1.92 | 3.51 | 1.86 | 1.76 | 3.63 | 1.07 |
| `usize` indices | 1.12 | 1.89 | 1.52 | 1.11 | 2.32 | 1.04 |
| `abs32` | 1.66 | 2.05 | 1.23 | 1.39 | 1.00 | 0.98 |
| `rel32` | 2.22 | 2.38 | 1.40 | 1.75 | 1.04 | 1.00 |
| `rel16` | 2.17 | 1.74 | 0.83 | n/a | n/a | n/a |

| B-tree lookup time / `u32` time | 64-byte nodes, 2^18 | 64-byte nodes, 2^22 | 256-byte nodes, 2^18 | 256-byte nodes, 2^22 |
|---|---:|---:|---:|---:|
| `BTreeSet` | 1.90 | 1.63 | 2.21 | 2.28 |
| `box` nodes | 1.07 | 1.12 | 1.07 | 1.06 |
| `usize` | 1.09 | 1.21 | 1.09 | 1.18 |
| `abs32` | 1.08 | 0.94 | 1.06 | 1.09 |
| `abs16` | 1.07 | n/a | 1.04 | n/a |

* **Against `Rc` + `Weak`, 32-bit links are 3.2x smaller and faster from 2^16 nodes up.** A node takes 20 bytes instead of 64 (12 with `rel16`, 5.3x smaller). `abs32` takes 0.27x to 0.92x of the `Rc` time from 2^16 to 2^22 nodes: 0.58x on the mesh, 0.65x on the banded graph, 0.80x on the random graph and 0.92x on the dependent chase at 2^22, and 0.28x on the chase at 2^18. Below 2^16 the only loss is on a small mesh (`abs32` takes 1.08x to 1.19x the `Rc` time at 2^10 to 2^14). `rel32` and `rel16` are slower than `Rc` on small graphs (up to 1.6x at 2^10) and on the 2^16 mesh (1.16x and 1.12x), and `rel32` ties it on the random graph at 2^22. Hand-written `usize` indices also beat `Rc` everywhere (0.30x to 0.99x).
* **Against hand-written `u32` indices the memory is the same and the price is the link decode, which a loop that waits for memory does not pay.** In sweeps of 2^16 nodes and up, 32-bit links take 1.23x to 2.38x the `u32` time. In dependent chases they take 0.95x to 1.08x, and in B-tree lookups `abs32` takes 0.94x to 1.09x. The probes `abs32-idx` and `rel16-idx` decode with the public `Link::target` and index the slice themselves, with no `Cursor`, and are about as fast as the `Cursor` variants or slower (mesh at 2^22: 2.24x against 2.05x for `abs32`, 1.72x against 1.74x for `rel16`), so `Cursor` is not the cost. What remains is the typed decode: its checks, its arithmetic, and whatever it stops the compiler doing to the plain `u32` loop. They were not separated.
* **16-bit relative links beat `u32` on time as well as memory in large local graphs.** On the banded graph `rel16` takes 12 bytes per node instead of 20 and 0.80x (2^20) and 0.83x (2^22) of the `u32` time; no load band of the record puts it above 0.85x. It starts to win between 2^18 (1.46x) and 2^20; below that, and on the mesh at any size, it takes 1.5x to 2.4x the `u32` time.
* **Links narrower than 32 bits fit only local graphs.** `rel16` fits the mesh and the banded graph at every size and the random graph (and so the chase) up to 2^15 nodes, tested at 2^15 and 2^15 + 1,024. `abs16` stops at 2^14 nodes in every graph, `rel8` fits the mesh only up to 2^12 and never the banded or random graph. In the B-trees `abs16` reaches 2^18 keys (64-byte nodes) and 2^20 keys (256-byte nodes).
* **A dependent chase separates the variants where their footprints straddle a cache level, and not in DRAM.** At 2^18 the chase takes 3.6x (`Rc`) and 2.3x (`usize`) the `u32` time, at 2^20 1.3x and 1.2x, and at 2^22 1.07x and 1.04x. The 2^20 ratios fall as the machine gets busier (1.09x and 1.07x on the 17 sweeps whose reference cells read at least twice their quiet values), so a quiet machine probably reads higher; that was not measured.
* **In a B-tree link width barely matters.** `u32` is 8% to 17% faster than `usize` indices (0.83x to 0.92x of the time) and 2% to 9% smaller; `abs32` is as fast as `u32` within 9% (0.94x to 1.09x) and 1% to 3% smaller; `abs16` saves 2% to 6% more up to its reach, at 0.97x to 1.07x the time. The packed trees beat `BTreeSet` by 1.6x to 2.3x in lookup time and 1.2x to 1.4x in memory per key, but `BTreeSet` is a dynamic structure, and these trees cannot be modified.

### Dynamic containers

A third record, `bench/containers/`, measures the `rustlink-collections` types against the
standard library's dynamic containers: `BTreeMap`/`BTreeSet`, `HashMap`/`HashSet` and
`LinkedList`, with insertion and removal, not the packed static trees of the previous section.
Seven workloads build a structure and then look up, scan or churn (remove half the keys and put
them back). The variants are `std`, `std-fnv` (std's `HashMap` with the same FNV-1a hasher as the
rustlink map), `u16` and `u32`; the number is the link width. Sizes run from 2^10 to 2^20 entries.

Heap bytes are requested bytes per entry after the build; times are per lookup, visit or operation
from `CLOCK_THREAD_CPUTIME_ID`. Each figure is the median of 14 complete sweeps on one pinned
E-core (cpu23, L1d 32 KiB, L2 4 MiB, L3 36 MiB), with paired ratios taken inside each sweep. The
machine was shared and never quiet, so ratios below 1 mean rustlink is faster and cells at 2^20
carry the widest uncertainty; `bench/containers/environment.txt` gives the interquartile ranges,
the load and the merge rule. Absolute times are not comparable with the earlier records, which
ran on a P-core.

| Heap bytes per entry at 2^20 | `std` | `u32` links | `u16` links |
|---|---:|---:|---:|
| `BTreeMap<u32, u64>` | 20.7 | 29.2 | 29.0 (2^18) |
| `HashMap<u32, u64>` | 34.0 | 25.0 | 21.0 (2^14) |
| `LinkedList<u64>` | 24.0 | 16.0 | 16.0 (2^14) |

The `u16` column is shown at the largest size it can address: a B+tree's links address nodes
(about 65,000 entries at this fill), while a hash map's and a list's address one slot per entry
(65,535). `std-fnv` uses the same hasher and has the same 34.0 bytes per entry as `std`.

Time relative to `std`; below 1 is faster. The hash rows use `std-fnv` as the baseline, so the hash
function cancels and the comparison is the container layout. CPU time, median of 14 per-sweep
ratios.

| Workload / `std` | 2^14 | 2^16 | 2^18 | 2^20 |
|---|---:|---:|---:|---:|
| `btree-lookup`, `u32` | 0.87 | 0.96 | 1.06 | 1.23 (IQR 1.03-1.30) |
| `btree-scan`, `u32` | 0.98 | 1.00 | 1.29 | 0.95 |
| `btree-churn`, `u32` | 1.10 | 1.24 | 1.36 | 1.75 (IQR 1.62-1.88) |
| `hash-lookup`, `u32` / `std-fnv` | 0.96 | 1.00 | 1.28 | 1.07 |
| `hash-scan`, `u32` / `std-fnv` | 0.44 | 0.36 | 0.34 | 0.28 |
| `hash-churn`, `u32` / `std-fnv` | 0.96 | 0.91 | 1.38 | 0.67 |
| `list-scan`, `u32` | 1.58 | 1.37 | 0.49 | 0.96 |

* **A dynamic B+tree does not save memory when values are large; the arena overhead is larger than
  the links it replaces.** The tree takes 29.2 bytes per `u32` key and `u64` value against 20.7
  for `std::collections::BTreeMap`, and 16-bit links save only 0.2 bytes because the links are a
  small part of each entry. A leaf stores its entries in a `Vec` with its full 16-entry capacity,
  each node is a 48-byte enum slot, and the node arena grows geometrically; std's B+tree allocates
  fixed-size nodes and packs them more tightly. The same implementation is, however, faster than
  std while it fits in cache: lookups take 0.87x std's time at 2^14 and 0.96x at 2^16, tie at
  2^18, and 1.23x at 2^20. Builds cost 1.18x to 1.73x and churn 1.10x to 1.75x. Scans are a tie
  within 0.95x to 1.29x.
* **The hash map is the memory winner and its scan is three times faster.** 25 bytes per entry
  against 34, or 21 with 16-bit links, because entries are a flat `Vec` and buckets are 16-bit
  indices. Scanning visits a contiguous slot array: 0.28x to 0.44x of std's time with the same
  hasher, and 0.29x against std's default SipHash. Churn is 0.67x to 1.38x and lookup 0.96x to
  1.28x with the same hasher, so the container's routing is not faster than hashbrown's; the
  0.50x to 0.71x win against std's default map is mostly FNV against SipHash.
* **The linked list halves the memory and its build is ten times faster; its scan depends on
  cache.** 16 bytes per node against 24, and `push_back` into one `Vec` costs 0.09x to 0.10x of
  std's one-allocation-per-node build. At 2^14 and 2^16, where both lists fit in L2, link decoding
  makes the scan 1.4x to 1.6x slower. At 2^18 the contiguous 4 MiB array beats pointer chasing
  (0.49x), and at 2^20 the two are within noise (0.96x, IQR 0.85-1.08) while the memory saving
  remains.
* **Width matters less than expected.** A 16-bit B+tree saves 0.2 bytes per entry and a 16-bit list
  saves nothing (`u64` plus two `u16` links pads to the same 16 bytes as two `u32` links); the
  16-bit hash map saves 4 bytes per entry but cannot hold more than 65,535 entries. The 16-bit
  B+tree is 0.99x to 1.06x the `u32` time, the 16-bit hash map 0.69x to 1.00x, and the 16-bit list
  1.12x to 1.14x, so the width is not a reliable time lever after it fits.

## Benchmarks

The `rustlink-bench` crate compares pointers, `Rc` nodes, `std::collections::BTreeSet`,
hand-written `usize` and `u32` indices, every rustlink width and link form on trees, an expression
tree, graphs, link-dense node graphs, B-trees and blobs, and `rustlink-collections` against
`std::collections` on linked lists, ordered maps and hash maps.

```text
cargo build --release --bins
target/release/sweep --out my.jsonl --md my.md   # whole matrix, into your own files
target/release/sweep --dry-run                   # show the cells and the CPU it would pin to
target/release/sweep --report-only               # re-render bench/results.md from bench/results.jsonl
target/release/rlb list                          # workloads, their largest size and variants
target/release/sweep --workloads ngraph-mesh,ngraph-banded,ngraph-random,ngraph-chase,btree-64,btree-256 \
    --out my-decisive.jsonl --md my-decisive.md  # the node graphs and B-trees alone
target/release/sweep --workloads btree-lookup,btree-scan,btree-churn,hash-lookup,hash-scan,hash-churn,list-scan \
    --sizes 10,12,14,16,18,20 --out my-containers.jsonl --md my-containers.md
```

`bench/results.md` is the first recorded sweep (the nine original workloads, 405 cells: the trees, the expression tree, the graphs and the blobs), and `bench/environment.txt` says how it was taken. `bench/decisive/` records the node graphs and B-trees (364 cells), which were measured in repeated sweeps and judged on ratios (`bench/decisive/environment.txt`). `bench/containers/` records the seven dynamic-container workloads (144 cells, 129 done and 15 skipped for width), with 14 complete sweeps, a merged results file, every replicate and a ratios table (`bench/containers/environment.txt`). All 22 workloads are registered, so a plain `sweep` runs 913 cells; without `--out` and `--md` it appends them to the first record's `bench/results.jsonl` and rewrites `bench/results.md`, whose report keeps the last measurement of every cell. Pass `--out` and `--md` for your own runs, and `--workloads` to pick a record's workloads.

Each cell runs in its own process pinned to one CPU with `taskset`, so peak RSS is clean. Leave
the machine idle while a sweep runs, and check that it is first: `uptime` should show a load
average near zero, and a per-core view (`top`, then `1`) should show the pinned core idle. The
sweep pins to the core with the highest `cpuinfo_max_freq`; `--cpu N` chooses another, so pass an
idle one if that core is busy.

### Reading the numbers

* Heap bytes per node are *requested* bytes, so allocator overhead is excluded; peak RSS includes it. For the container workloads the unit is one entry, not one node, and `std`'s containers report a node size of 0 because their internal layouts are private.
* Build times of the `Vec` and `Arena` variants include first-touch page faults of freshly allocated memory, while the `box` variants may reuse memory freed by earlier repetitions, so build times can favour `box` slightly. The `box` build in `graph-linked-*` allocates every edge in slot order and then walks the edge plan to link the chains (shuffled in `graph-linked-shuf`), so do not read that build time as an allocation cost.
* The narrow variants traverse through `Cursor`, which carries a slice and an index, while the hand-written baselines pass a single index. A gap between `u32` and `abs32` is therefore the cost of the bounds-checked API, not of the link width.
* A cell marked `n/a` is not a failure: that variant's links cannot represent the structure at that size. `rel*` links need every link distance to fit the width, and `abs16` fits at most 65,535 elements (an `OptAbs` reserves one value for null); the report's notes give the measured link distances for every `n/a` cell. A cell marked `skip` was not measured, and its note says why (the `box` cells of `graph-linked-seq` and `graph-linked-shuf`, or a 16-bit container at a size its links cannot address).
* The `±` is the MAD of five repetitions in one process. It leaves out the variation between processes (page layout) and between times, which is larger: the usable measurements of one cell typically differ by 2% to 4% from 2^14 to 2^24 nodes, but by 8% (median) and 17% (90th percentile) at 2^20, and by up to 28% for single cells (`bench/environment.txt`). Treat differences under 10% as ties, differences under 20% at 2^20 as unresolved, and cells up to 2^12 nodes, which take microseconds per repetition, as indicative only. A cell is the fastest of its usable measurements, which averages 2% (4% at 2^20) below their median.
* Background load matters: cells of 2^18 nodes and more ran about 3% slower with two other CPUs busy and about 12% slower with four to six busy, and 2^20 about twice as much. The count of busy CPUs is only a proxy: the same count cost far more later on 2026-10-23, when other sessions loaded the memory system (`bench/environment.txt`). Measure a reference cell, such as `u32` of the same workload and size, next to anything you want to trust.
* The CPU tables read `CLOCK_THREAD_CPUTIME_ID` around each build and traversal. They exclude time the process was descheduled, so use them when other work shares the machine; the wall table is the same work measured with `Instant`.
* One machine, one pinned core.
