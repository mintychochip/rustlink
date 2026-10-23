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

| Scenario | Workload and size | Best narrow variant | Heap B/node: narrow, `u32`, `box` | Traverse ns: narrow, `u32`, `box` |
|---|---|---|---|---|
| Pointer-dense nodes, good locality | `bst-bulk` at 2^22 | `abs32` | 12, 12, 24 | 150.8, 152.5, 258.1 |
| Pointer-dense nodes, poor locality | `bst-random` at 2^20 | `abs32` | 12, 12, 24 | 187.6, 189.5, 431.3 |
| Expression trees built bottom-up | `ast` at 2^22 | `abs32` | 12, 12, 24 | 10.2, 10.1, 13.3 |
| Graph edge lists, local | `graph-linked-seq` at 2^20 | `rel8` | 24, 36, 72 | 16.1, 13.0, 22.0 |
| Graph edge lists, scattered | `graph-linked-shuf` at 2^20 | `abs32` (no narrower variant fits) | 36, 36, 72 | 27.4, 25.8, 21.6 (†) |

(†) `box` is not scattered here: it allocates each vertex's edges one after another whatever the slot permutation (22.0 ns in `graph-linked-seq`, 21.6 in `graph-linked-shuf`), so it shows what an allocator gives, not the worst case the index variants are put through.

* **Against pointers, 32-bit links halve the heap and win where memory is the bottleneck.** Tree nodes shrink from 24 to 12 bytes; lookups are 1.7x faster in `bst-bulk` at 2^22 and 2.3x faster in `bst-random` at 2^20, and expression-tree visits are 1.3x faster. Linked edge lists shrink from 72 to between 24 and 36 bytes per vertex and are scanned 1.4x (`rel8`) to 1.6x (`abs32`) faster in `graph-linked-seq`.
* **Against hand-written `u32` indices, 32-bit links save nothing and cost between -14% and +17% traversal time** (N of 2^14 and up). `abs32` is faster in balanced, bulk-built trees (0.86x to 0.99x of `u32`), mixed in randomly built trees (0.92x to 1.17x), level in expression trees (0.98x to 1.05x) and slower in CSR and linked-list loops (1.04x to 1.17x): the bounds-checked `Cursor` is not free in a tight pointer-chasing loop. `rel32` costs more than `abs32`: 0.98x to 1.04x on expression trees, up to 1.22x on `bst-bulk` and up to 1.36x on linked lists.
* **Links narrower than 32 bits pay only where links are local.** `rel8` and `rel16` are the only sub-32-bit variants that reach large sizes, and only on `graph-linked-seq`, where a vertex's edges are consecutive and every link spans one slot: 24 and 28 bytes per vertex instead of 36 (1.5x and 1.3x smaller), at 1.25x and 1.29x the traversal time of `u32` at 2^20. Elsewhere they are `n/a`: on trees `abs16` and `rel16` stop at 2^14 nodes and `rel8` does not fit even 2^10 (in `bst-bulk` at 2^10 the longest link spans 512 elements, though 99.3% span at most 127); on shuffled edge lists `abs16` and `rel16` stop at 2^12 and `rel8` never fits. A fixed-width link must fit the longest link, and the few long links near a tree's root decide the width. Variable-width (varint) links, the optional layer 4 of the design, target exactly this shape.
* **A verified blob loads about four times faster than the arena rebuilds.** At 2^22 nodes `BlobRef::verify` takes 1.33 ns per node (`abs32`; 1.12 for `rel32`) against 5.41 ns (4.95) to rebuild the arena from its plan (`blob-load`). Lookups through the verified view cost 151.2 ns (`abs32`) against 150.8 ns through the arena (`bst-bulk`); for `rel32` 168.2 ns against 158.6 ns.
* **The layer-4 gate passes.** The `ast-bytes` prototype (variable-size records, LEB128 links) uses 3.26 bytes per node against 12 for the best layer-2 variant (3.68x less heap) and traverses in 0.81x to 0.84x of its time. Layer 4 is not built; the gate table, from the report, is:

| N | best L2 variant | its heap B/node | ast-bytes heap B/node | memory ratio | its traverse ns | ast-bytes traverse ns | time ratio | verdict |
|---|---|---:|---:|---:|---:|---:|---:|---|
| 2^20 | abs32 | 12.0 | 3.3 | 3.68 | 9.5 | 7.9 | 0.84 | pass |
| 2^22 | abs32 | 12.0 | 3.3 | 3.68 | 10.2 | 8.3 | 0.81 | pass |
| 2^24 | rel32 | 12.0 | 3.3 | 3.68 | 10.4 | 8.5 | 0.81 | pass |

Rule: proceed to layer 4 only if `ast-bytes` uses at least 1.5x less heap than the best layer-2 variant at no more than 1.1x its traversal time, at N of 2^20 and up.

Established elsewhere: replacing 64-bit pointers with 32-bit offsets is a proven win where pointers dominate the heap. V8's pointer compression reduced V8 heap size by up to 43%, and HotSpot's compressed oops exist for the same reason. Those results belong to those projects, not to this crate.

## Benchmarks

The `rustlink-bench` crate compares pointers, hand-written `usize` and `u32` indices, and every
rustlink width and link form on trees, an expression tree, graphs and blobs.

```text
cargo build --release --bins
target/release/sweep --out my.jsonl --md my.md   # whole matrix, into your own files
target/release/sweep --dry-run                   # show the cells and the CPU it would pin to
target/release/sweep --report-only               # re-render bench/results.md from bench/results.jsonl
target/release/rlb list                          # workloads, their largest size and variants
```

`bench/results.md` is the recorded sweep, and `bench/environment.txt` says how it was taken. Without `--out` and `--md`, `sweep` appends to the recorded `bench/results.jsonl` and rewrites `bench/results.md`; the report keeps the last measurement of every cell.

Each cell runs in its own process pinned to one CPU with `taskset`, so peak RSS is clean. Leave
the machine idle while a sweep runs, and check that it is first: `uptime` should show a load
average near zero, and a per-core view (`top`, then `1`) should show the pinned core idle. The
sweep pins to the core with the highest `cpuinfo_max_freq`; `--cpu N` chooses another, so pass an
idle one if that core is busy.

### Reading the numbers

* Heap bytes per node are *requested* bytes, so allocator overhead is excluded; peak RSS includes it.
* Build times of the `Vec` and `Arena` variants include first-touch page faults of freshly allocated memory, while the `box` variants may reuse memory freed by earlier repetitions, so build times can favour `box` slightly. The `box` build in `graph-linked-*` also walks the edge plan, which is shuffled in `graph-linked-shuf`; do not read that build time as an allocation cost.
* The narrow variants traverse through `Cursor`, which carries a slice and an index, while the hand-written baselines pass a single index. A gap between `u32` and `abs32` is therefore the cost of the bounds-checked API, not of the link width.
* A cell marked `n/a` is not a failure: that variant's links cannot represent the structure at that size. `rel*` links need every link distance to fit the width, and `abs16` fits at most 65,535 elements (an `OptAbs` reserves one value for null); the report's notes give the measured link distances for every `n/a` cell.
* The `±` is the MAD of five repetitions in one process. It leaves out the variation between processes (page layout) and between times, which is larger: the usable measurements of one cell differed by a median of 2% (2^14 to 2^18), 8% (2^20) and 4% (2^22 and 2^24), and more when other work was running (`bench/environment.txt`). Treat differences under 10% as ties, and cells up to 2^12 nodes, which take microseconds per repetition, as indicative only.
* Background load matters: cells of 2^18 nodes and more ran about 3% slower with two other CPUs busy and about 12% slower with four to six busy, and 2^20 about twice as much.
* One machine, one pinned core.
