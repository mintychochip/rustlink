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

No full benchmark sweep has been recorded yet. It was deferred because the machine was running other heavy work, and on that machine the same cell varied by about 24% between runs (232 to 287 ns per lookup for `bst-bulk` with `u32` links at 2^20 nodes), which is larger than most of the differences the sweep is meant to resolve. Run `target/release/sweep` on a quiet machine to produce `bench/results.md`; this section should then quote its numbers.

What is already established:

* Replacing 64-bit pointers with 32-bit offsets is a proven win where pointers dominate the heap. V8's pointer compression reduced V8 heap size by up to 43%, and HotSpot's compressed oops exist for the same reason. Those results belong to those projects, not to this crate.
* Narrow relative links need locality. In a balanced binary tree built children-first (`bst-bulk`) with 4,096 nodes, 99.2% of the links span at most 127 elements, but the longest spans 2,048, so `rel8` cannot represent the tree at all (`rlb run --workload bst-bulk --variant rel8 --n 4096` reports it as `unreachable`). A fixed-width link must fit the longest link, and the few long links near the root decide the width. Variable-width (varint) links, the optional layer 4 of the design, target exactly this shape.
* `Abs` links bound the region size instead: `u16` addresses at most 65,536 elements, so `abs16` is unreachable above that.

## Benchmarks

The `rustlink-bench` crate compares pointers, hand-written `usize` and `u32` indices, and every
rustlink width and link form on trees, an expression tree, graphs and blobs.

```text
cargo build --release --bins
target/release/sweep                 # whole matrix; writes bench/results.jsonl and bench/results.md
target/release/sweep --dry-run       # show the cells and the CPU it would pin to
target/release/sweep --report-only   # re-render bench/results.md from bench/results.jsonl
target/release/rlb list              # workloads, their largest size and variants
```

Each cell runs in its own process pinned to one CPU with `taskset`, so peak RSS is clean. Leave
the machine idle while a sweep runs, and check that it is first: `uptime` should show a load
average near zero, and a per-core view (`top`, then `1`) should show the pinned core idle. The
sweep pins to the core with the highest `cpuinfo_max_freq`; `--cpu N` chooses another, so pass an
idle one if that core is busy.

### Reading the numbers

* Heap bytes per node are *requested* bytes, so allocator overhead is excluded; peak RSS includes it.
* Build times of the `Vec` and `Arena` variants include first-touch page faults of freshly allocated memory, while the `box` variants may reuse memory freed by earlier repetitions, so build times can favour `box` slightly.
* The narrow variants traverse through `Cursor`, which carries a slice and an index, while the hand-written baselines pass a single index. A gap between `u32` and `abs32` is therefore the cost of the bounds-checked API, not of the link width.
* A cell marked `n/a` is not a failure: that variant's links cannot represent the structure at that size, and the report's notes give the measured link distances.
* One machine, one pinned core. Re-run before trusting any difference smaller than the reported MAD.
