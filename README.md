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
* **Against hand-written `u32` indices, 32-bit links save nothing and cost between -14% and +17% traversal time** (N of 2^14 and up). `abs32` is faster in balanced, bulk-built trees (0.86x to 0.99x of `u32`), mixed in randomly built trees (0.92x to 1.17x), level in expression trees (0.98x to 1.05x) and slower in CSR and linked-list loops (1.04x to 1.17x): decoding a typed link is not free in a tight loop (a probe that skips `Cursor` and indexes the slice itself is no faster, see the next section). `rel32` costs more than `abs32`: 0.98x to 1.04x on expression trees, up to 1.22x on `bst-bulk` and up to 1.36x on linked lists.
* **Links narrower than 32 bits pay only where links are local.** `rel8` and `rel16` are the only sub-32-bit variants that reach large sizes, and only on `graph-linked-seq`, where a vertex's edges are consecutive and every link spans one slot: 24 and 28 bytes per vertex instead of 36 (1.5x and 1.3x smaller), at 1.25x and 1.28x the traversal time of `u32` at 2^20 and 1.5x at 2^22. Elsewhere they are `n/a`: on trees `abs16` and `rel16` stop at 2^14 nodes and `rel8` does not fit even 2^10 (in `bst-bulk` at 2^10 the longest link spans 512 elements, though 99.3% span at most 127); on shuffled edge lists `abs16` and `rel16` stop at 2^12 and `rel8` never fits. A fixed-width link must fit the longest link, and the few long links near a tree's root decide the width. Variable-width (varint) links, the optional layer 4 of the design, target exactly this shape.
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

Heap bytes are exact. Every time is a ratio between two variants taken inside one sweep, so both saw the same machine, and each figure is the median over the calm sweeps: 14 to 16 of 37 repetitions of the whole matrix, built from git 5649039. The machine was shared and never quiet; `bench/decisive/environment.txt` defines calm, gives the interquartile ranges and shows how the load moves each ratio.

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
* **Against hand-written `u32` indices the memory is the same and the price is the decode, which a loop that waits for memory does not pay.** In sweeps of 2^16 nodes and up, 32-bit links take 1.23x to 2.38x the `u32` time. In dependent chases they take 0.95x to 1.08x, and in B-tree lookups `abs32` takes 0.94x to 1.09x. The probes `abs32-idx` and `rel16-idx` decode with the public `Link::target` and index the slice themselves, with no `Cursor`, and are no faster than the `Cursor` variants (mesh at 2^22: 2.24x against 2.05x for `abs32`, 1.72x against 1.74x for `rel16`), so the cost is the decode and not the cursor.
* **16-bit relative links beat `u32` on time as well as memory in large local graphs.** On the banded graph `rel16` takes 12 bytes per node instead of 20 and 0.80x (2^20) and 0.83x (2^22) of the `u32` time; no load band of the record puts it above 0.85x. It starts to win between 2^18 (1.46x) and 2^20; below that, and on the mesh at any size, it takes 1.5x to 2.4x the `u32` time.
* **Links narrower than 32 bits fit only local graphs.** `rel16` fits the mesh and the banded graph at every size and the random graph (and so the chase) up to 2^15 nodes, tested at 2^15 and 2^15 + 1,024. `abs16` stops at 2^14 nodes in every graph, `rel8` fits the mesh only up to 2^12 and never the banded or random graph. In the B-trees `abs16` reaches 2^18 keys (64-byte nodes) and 2^20 keys (256-byte nodes).
* **A dependent chase separates the variants where their footprints straddle a cache level, and not in DRAM.** At 2^18 the chase takes 3.6x (`Rc`) and 2.3x (`usize`) the `u32` time, at 2^20 1.3x and 1.2x, and at 2^22 1.07x and 1.04x. The 2^20 ratios fall as the machine gets busier (1.09x and 1.07x on the 17 sweeps whose reference cells read at least twice their quiet values), so a quiet machine probably reads higher; that was not measured.
* **In a B-tree link width barely matters.** `u32` is 8% to 17% faster than `usize` indices (0.83x to 0.92x of the time) and 2% to 9% smaller; `abs32` is as fast as `u32` within 9% (0.94x to 1.09x) and 1% to 3% smaller; `abs16` saves 2% to 6% more up to its reach, at 0.97x to 1.07x the time. The packed trees beat `BTreeSet` by 1.6x to 2.3x in lookup time and 1.2x to 1.4x in memory per key, but `BTreeSet` is a dynamic structure, and these trees cannot be modified.

## Benchmarks

The `rustlink-bench` crate compares pointers, `Rc` nodes, `std::collections::BTreeSet`, hand-written
`usize` and `u32` indices, and every rustlink width and link form on trees, an expression tree,
graphs, link-dense node graphs, B-trees and blobs.

```text
cargo build --release --bins
target/release/sweep --out my.jsonl --md my.md   # whole matrix, into your own files
target/release/sweep --dry-run                   # show the cells and the CPU it would pin to
target/release/sweep --report-only               # re-render bench/results.md from bench/results.jsonl
target/release/rlb list                          # workloads, their largest size and variants
target/release/sweep --workloads ngraph-mesh,ngraph-banded,ngraph-random,ngraph-chase,btree-64,btree-256 \
    --out my-decisive.jsonl --md my-decisive.md  # the node graphs and B-trees alone
```

`bench/results.md` is the first recorded sweep (the trees, the expression tree, the graphs and the blobs), and `bench/environment.txt` says how it was taken. `bench/decisive/` records the node graphs and B-trees, which were measured in repeated sweeps and judged on ratios (`bench/decisive/environment.txt`). Without `--out` and `--md`, `sweep` appends to the first record's `bench/results.jsonl` and rewrites `bench/results.md`; the report keeps the last measurement of every cell.

Each cell runs in its own process pinned to one CPU with `taskset`, so peak RSS is clean. Leave
the machine idle while a sweep runs, and check that it is first: `uptime` should show a load
average near zero, and a per-core view (`top`, then `1`) should show the pinned core idle. The
sweep pins to the core with the highest `cpuinfo_max_freq`; `--cpu N` chooses another, so pass an
idle one if that core is busy.

### Reading the numbers

* Heap bytes per node are *requested* bytes, so allocator overhead is excluded; peak RSS includes it.
* Build times of the `Vec` and `Arena` variants include first-touch page faults of freshly allocated memory, while the `box` variants may reuse memory freed by earlier repetitions, so build times can favour `box` slightly. The `box` build in `graph-linked-*` allocates every edge in slot order and then walks the edge plan to link the chains (shuffled in `graph-linked-shuf`), so do not read that build time as an allocation cost.
* The narrow variants traverse through `Cursor`, which carries a slice and an index, while the hand-written baselines pass a single index. A gap between `u32` and `abs32` is therefore the cost of the bounds-checked API, not of the link width.
* A cell marked `n/a` is not a failure: that variant's links cannot represent the structure at that size. `rel*` links need every link distance to fit the width, and `abs16` fits at most 65,535 elements (an `OptAbs` reserves one value for null); the report's notes give the measured link distances for every `n/a` cell. A cell marked `skip` was not measured, and its note says why (the `box` cells of `graph-linked-seq` and `graph-linked-shuf`, see the note under the table above).
* The `±` is the MAD of five repetitions in one process. It leaves out the variation between processes (page layout) and between times, which is larger: the usable measurements of one cell typically differ by 2% to 4% from 2^14 to 2^24 nodes, but by 8% (median) and 17% (90th percentile) at 2^20, and by up to 28% for single cells (`bench/environment.txt`). Treat differences under 10% as ties, differences under 20% at 2^20 as unresolved, and cells up to 2^12 nodes, which take microseconds per repetition, as indicative only. A cell is the fastest of its usable measurements, which averages 2% (4% at 2^20) below their median.
* Background load matters: cells of 2^18 nodes and more ran about 3% slower with two other CPUs busy and about 12% slower with four to six busy, and 2^20 about twice as much. The count of busy CPUs is only a proxy: the same count cost far more later on 2026-10-23, when other sessions loaded the memory system (`bench/environment.txt`). Measure a reference cell, such as `u32` of the same workload and size, next to anything you want to trust.
* One machine, one pinned core.
