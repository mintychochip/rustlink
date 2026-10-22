# rustlink: narrow typed links

Design spec, 2026-10-22. Status: draft for user review.

## 1. Purpose and scope

`rustlink` is a Rust library that replaces 8-byte pointers inside pointer-dense data structures with narrow typed links of 1, 2, 4 or 8 bytes. It adapts the "links narrower than pointers" mechanism of the Goose language ([`aardappel/goose`](https://github.com/aardappel/goose), spec section 3.9) to Rust, and ships with a benchmark suite whose results decide whether a fourth, byte-addressed layer is worth building.

| Layer | Content | Status |
|---|---|---|
| L1 | `Width`, `Abs` / `OptAbs` / `Rel` / `OptRel` links, `Link` trait, `LinkError` | v1 |
| L2 | `Region` trait, `Cursor`, append-only `Arena<T>` | v1 |
| L3 | `Arena::freeze`, `BlobRef::verify`, `Links` trait, `impl_links!` | v1 |
| Bench | `rustlink-bench` crate: runner, sweep driver, workloads, report | v1 |
| L4 | Byte-addressed regions: varint (LEB128) links, variable-size records | gated on benchmark results (section 10) |

Toolchain: stable Rust features only (the machine default is nightly 1.99, but no nightly feature is used), edition 2024, `rust-version = "1.88"` (the minimum of the dev-dependency `proptest` 1.12).

### Decisions already made with the user

1. Approach C: layered and benchmark-gated. Both link forms and both containers ship in v1; the byte layer (L4) follows only if the benchmarks justify it.
2. Resolution is always `(region, owner index, link)`; links count elements, not bytes.
3. Safety comes from bounds-checked resolution, not from compile-time root proofs or branded lifetimes.
4. No hand-written `unsafe` is planned (the single contingency is in section 12, risk 1).
5. Blobs are built on `zerocopy` 0.8; benchmarks run on this machine with per-process isolation.
6. Layer-4 gate default: at least 1.5x less memory than the best L2 variant at no more than 1.1x its traversal time on the `ast` workload at N >= 2^20; the final go/no-go is the user's decision with the numbers.

## 2. Evidence base

- **Proven: 64-bit to 32-bit links.** V8 pointer compression reduced heap size by up to 43% and Chrome renderer memory by up to 20% on desktop; tagged values were about 70% of the V8 heap ([V8 blog](https://v8.dev/blog/pointer-compression)). HotSpot reports that an LP64 heap can need about 1.5x the memory of an ILP32 heap because of pointer size, and compressed oops (32-bit offsets scaled by 8) address up to about 32 GB ([Oracle](https://docs.oracle.com/en/java/javase/17/vm/java-hotspot-virtual-machine-performance-enhancements.html)).
- **Narrow offsets in bounded regions are standard practice.** SQLite stores 2-byte cell offsets inside a b-tree page ([file format](https://www.sqlite.org/fileformat2.html)). FlatBuffers uses `uint32` offsets for objects and `uint16` for vtable entries ([internals](https://flatbuffers.dev/internals/)).
- **Goose's own benchmarks do not isolate narrow links.** Its `sexp`, `interp` and `bintrees` sources use only `<u32>` links. Its [summary](https://github.com/aardappel/goose/blob/master/bench/summary.md) files `sexp`, `interp` and `records` under "compact representation" (variant-sized nodes, inline text); `records` has no links; the CSR lead over C++/Rust is credited to check-free traversal. No Goose benchmark uses `u8` or `u16` links.
- **Varint has a cliff.** Goose measured LEB128 beating the best branchless format by about 3x above roughly 70-75% single-byte values and losing by up to about 3x below ([Goose spec, Appendix B](https://github.com/aardappel/goose/blob/master/docs/goose_spec.md)).
- **Decode cost matters.** V8 found that decode must be one add against a base held in a register; recomputing the base on every load hurt performance.
- **Rust prior art.** [`rkyv::rel_ptr`](https://docs.rs/rkyv/latest/rkyv/rel_ptr/index.html) offers self-relative pointers with i8 to u64 offsets and fallible conversion, aimed at archives. [`compact_arena`](https://docs.rs/compact_arena/latest/compact_arena/) offers branded `Idx8`/`Idx16`/`Idx32` indices, is not position-independent, and shipped a critical branding flaw ([RUSTSEC-2019-0015](https://rustsec.org/advisories/RUSTSEC-2019-0015.html)). This is not an exhaustive crates.io survey.
- **Unmeasured, to be answered by this project's benchmarks:** whether `u8`/`u16` links pay on realistic structures, the benefit of element-scaled offsets, and the cost of bounds-checked resolution.

## 3. Architecture

### 3.1 Goose mapping

| Goose | rustlink | Difference |
|---|---|---|
| pool-relative `T&<uN in pool>` | `Abs<T, W>` / `OptAbs<T, W>` | element index instead of byte offset plus one; any `Region` can be the pool |
| self-relative `T&<uN>` | `Rel<T, W>` / `OptRel<T, W>` | element delta from the owner element's index instead of a byte offset from the field address |
| optional `T&<uN>?`, 0 is null | `OptAbs` / `OptRel` | `OptAbs` stores index plus one; `OptRel` reserves delta 0 |
| compile-time same-root proof | bounds-checked resolution | no compile-time proof |
| values holding self-relative links cannot be copied | not needed | the owner index is explicit at resolution, so a copied node cannot silently mis-resolve |
| `to_bytes` / `from_bytes` with verification | `Arena::freeze` / `BlobRef::verify` | adds a schema id to the header |
| `T&<varint>`, variable-size records, inline strings | L4 (gated) | not in v1 |

### 3.2 Workspace layout

```
rustlink/
  Cargo.toml                       # workspace, resolver "3", publish = false
  crates/rustlink/                 # the library (L1-L3)
    src/{lib,width,link,error,region,arena,blob,links}.rs
    tests/                         # integration, property and tamper tests; support/ holds shared fixtures
  crates/rustlink-bench/           # benchmarks (publish = false)
    src/bin/{rlb,sweep}.rs
    src/{alloc,blob_load,graph,measure,plan,proc,registry,report,rng,stats,workload}.rs
    src/{trees,variants}/
    tests/smoke.rs                 # end-to-end test of rlb and sweep
  bench/                           # generated results (results.jsonl, results.md)
  docs/superpowers/specs/          # this document
  README.md  CHANGELOG.md
```

### 3.3 Dependencies

- `rustlink` (runtime): `zerocopy = { version = "0.8", features = ["derive"] }`. The crate is `#![no_std]` with `extern crate alloc`; errors implement `core::error::Error`.
- `rustlink` (dev): `proptest = "1"`.
- `rustlink-bench`: `rustlink` and `zerocopy` only. Random numbers use an in-crate xorshift64*, JSON is written by hand, and page faults are read from `/proc/self/stat`.

### 3.4 Safety stance

The public API is entirely safe. The crate contains no hand-written `unsafe`; byte-to-struct casts go through `zerocopy`, whose derives reject padding at compile time. Safety never depends on verification: every resolution is bounds-checked against the region it is applied to, so a link used on the wrong region, a corrupted blob, or a dangling link yields `None` or `Err`, never undefined behavior. A link applied to the wrong region of the same `T` can yield a valid but wrong element; Goose permits the same outcome ("type-safe reuse").

## 4. L1: links

### 4.1 `Width`

```rust
mod sealed { pub trait Sealed {} }

/// Unsigned width of a stored offset. Sealed: implemented for u8, u16, u32, u64 only.
pub trait Width: sealed::Sealed + Copy + 'static {
    const BYTES: usize;          // 1, 2, 4, 8
    const MAX: u64;              // 2^(8*BYTES) - 1
    const SIGNED_MIN: i64;       // -2^(8*BYTES - 1)
    const SIGNED_MAX: i64;       //  2^(8*BYTES - 1) - 1
    const ZERO: Self::Le;        // the all-zero storage value, so `OptAbs::NULL` can be a const
    /// Alignment-1 little-endian storage: `u8` or zerocopy's
    /// `byteorder::little_endian::{U16, U32, U64}`.
    type Le: Copy + Eq + core::hash::Hash + core::fmt::Debug
        + zerocopy::FromBytes + zerocopy::IntoBytes + zerocopy::Immutable
        + zerocopy::KnownLayout + zerocopy::Unaligned;
    fn encode(raw: u64) -> Self::Le;          // low BYTES bytes; caller guarantees raw <= MAX
    fn decode(le: Self::Le) -> u64;
    fn encode_signed(v: i64) -> Self::Le;     // two's complement; caller guarantees range
    fn decode_signed(le: Self::Le) -> i64;    // sign-extended
}
```

### 4.2 Link types

```rust
pub struct Abs<T, W: Width>(W::Le, PhantomData<fn() -> T>);     // stores index
pub struct OptAbs<T, W: Width>(W::Le, PhantomData<fn() -> T>);  // stores index + 1, 0 is null
pub struct Rel<T, W: Width>(W::Le, PhantomData<fn() -> T>);     // stores target - owner
pub struct OptRel<T, W: Width>(W::Le, PhantomData<fn() -> T>);  // same, delta 0 is null
```

Invariants (compile-time asserted for every width and any `T`):

- `size_of` equals `W::BYTES` and `align_of` equals 1.
- All four implement `Copy`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Debug` by hand, with no bounds on `T`. `OptAbs` and `OptRel` implement `Default` as null.
- All four implement the zerocopy traits `FromBytes`, `IntoBytes`, `Immutable`, `KnownLayout`, `Unaligned`, so node structs containing them can derive the same traits.

| Type | Stored value | Null | `new` fails with |
|---|---|---|---|
| `Abs` | `index` | none | `OutOfRange` if `index > W::MAX` |
| `OptAbs` | `index + 1` | 0 | `OutOfRange` if `Some(index)` and `index + 1 > W::MAX` |
| `Rel` | two's-complement `target - owner` | none (delta 0 is the owner itself) | `OutOfRange` if the delta is outside `[SIGNED_MIN, SIGNED_MAX]` |
| `OptRel` | same as `Rel` | delta 0 | `OutOfRange` as `Rel`; `NullCollision` if `Some(target)` and `target == owner` |

Constructors and decoders:

```rust
impl<T, W: Width> Abs<T, W> {
    pub fn new(index: usize) -> Result<Self, LinkError>;
    pub fn index(self) -> usize;                       // saturates to usize::MAX if the stored value exceeds usize
}
impl<T, W: Width> OptAbs<T, W> {
    pub const NULL: Self;
    pub fn new(index: Option<usize>) -> Result<Self, LinkError>;
    pub fn get(self) -> Option<usize>;                 // None is null
}
impl<T, W: Width> Rel<T, W> {
    pub fn new(owner: usize, target: usize) -> Result<Self, LinkError>;   // computed in i128
    pub fn delta(self) -> i64;
}
impl<T, W: Width> OptRel<T, W> {
    pub const NULL: Self;
    pub fn new(owner: usize, target: Option<usize>) -> Result<Self, LinkError>;
    pub fn delta(self) -> Option<i64>;                 // None is null
}
```

Reach, in elements of `T`:

| Width | `Abs`: elements addressable | `Rel`: delta range |
|---|---|---|
| u8 | 256 | -128 to 127 |
| u16 | 65,536 | -32,768 to 32,767 |
| u32 | 4,294,967,296 | -2^31 to 2^31 - 1 |
| u64 | 2^64 | -2^63 to 2^63 - 1 |

`OptAbs` addresses one element fewer than `Abs` of the same width. `Abs` width bounds the region size; `Rel` width bounds link distance, so a region can be arbitrarily large when links are local.

### 4.3 The `Link` trait

```rust
pub enum Target { Null, Index(usize), Invalid }

pub trait Link<T>: Copy {
    /// Decodes to a target index. `owner` is ignored by `Abs` and `OptAbs`.
    /// `Invalid` means the arithmetic left the range 0..=usize::MAX (only `Rel` forms).
    fn target(self, owner: usize) -> Target;
    /// Provided method. True if null, or if the target is a valid index into a region of `len` elements.
    fn check(self, owner: usize, len: usize) -> bool {
        match self.target(owner) {
            Target::Null => true,
            Target::Index(i) => i < len,
            Target::Invalid => false,
        }
    }
}
```

`Rel::target` computes `owner as i128 + delta as i128` and returns `Invalid` outside `0..=usize::MAX`. `Link<T>` is implemented for `Abs<T, W>`, `OptAbs<T, W>`, `Rel<T, W>`, `OptRel<T, W>` for every `W`.

### 4.4 Errors

```rust
#[non_exhaustive]
pub enum LinkError {
    OutOfRange { value: i128, min: i128, max: i128 },
    NullCollision,
}
```

`LinkError` implements `Display` and `core::error::Error`.

## 5. L2: regions and arena

### 5.1 `Region` and `Cursor`

```rust
pub struct Dangling { pub owner: usize }   // Display + core::error::Error

pub trait Region<T> {
    fn as_slice(&self) -> &[T];
    fn len(&self) -> usize;                                    // provided
    fn get(&self, index: usize) -> Option<&T>;                 // provided
    /// None if the link is null, invalid, or its target is outside the region.
    fn follow<L: Link<T>>(&self, owner: usize, link: L) -> Option<(usize, &T)>;      // provided
    /// Ok(None) for null; Err(Dangling) for invalid or out-of-region targets.
    fn try_follow<L: Link<T>>(&self, owner: usize, link: L)
        -> Result<Option<(usize, &T)>, Dangling>;                                     // provided
    fn cursor(&self, index: usize) -> Option<Cursor<'_, T>>;   // provided
}

#[derive(Clone, Copy)]
pub struct Cursor<'a, T> { /* items: &'a [T], index: usize */ }
impl<'a, T> Cursor<'a, T> {
    pub fn index(&self) -> usize;
    pub fn get(&self) -> &'a T;                                // never panics: index < len by construction
    pub fn follow<L: Link<T>>(&self, link: L) -> Option<Cursor<'a, T>>;              // owner = self.index
    pub fn try_follow<L: Link<T>>(&self, link: L) -> Result<Option<Cursor<'a, T>>, Dangling>;
}
```

Resolution cost per link: decode, one add for `Rel`, one bounds check, then `base + index * size_of::<T>()`. The benchmarks measure this against hand-written baselines.

### 5.2 `Arena<T>`

```rust
pub struct Arena<T> { /* Vec<T> */ }
impl<T> Arena<T> {
    pub fn new() -> Self;
    pub fn with_capacity(n: usize) -> Self;
    pub fn push(&mut self, value: T) -> usize;                 // returns the new element's index
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T>; // patch links after construction
    pub fn clear(&mut self);
    pub fn into_vec(self) -> Vec<T>;
}
impl<T> Region<T> for Arena<T> { /* as_slice */ }
```

The arena is append-only: no `remove`, `insert`, `swap_remove`, `sort` or `truncate`, so existing indices stay stable and links held inside the arena never dangle. `clear` is allowed because it invalidates every in-arena link at once; any link held outside (for example a cross-region `Abs`) is still bounds-checked on use.

### 5.3 Cross-region links

An `Arena<T>` node may hold `Abs<U, W>` or `OptAbs<U, W>` pointing into an `Arena<U>`; resolution uses `Region<U>::follow` on the target region (the owner argument is ignored by `Abs` forms, so callers pass `0`). `Rel` forms are same-region only: the owner index and the target index must belong to the same region.

## 6. L3: blob

### 6.1 Element requirements

```rust
pub trait Links {
    const SCHEMA: u64;                                         // user-chosen layout identifier
    /// True if every same-region link in `self` is null or in range for a region of `len` elements.
    fn check_links(&self, owner: usize, len: usize) -> bool;
}
```

`impl_links!(Node, schema = 0x524C_4E4B_0001, links = [left, right])` generates the impl, calling `Link::<Node>::check` on each listed field. Cross-region link fields are left out of the list; `verify` does not range-check them, and they stay bounds-checked on every `follow` against their own region.

Blob elements must implement `FromBytes + IntoBytes + Immutable + KnownLayout + Unaligned + Links` and be non-zero-sized (a compile-time assertion). `Unaligned` means every field is a byte array, a `u8`/`i8`, a zerocopy little-endian integer type (`zerocopy::byteorder::little_endian::*`) or one of the link types, so a blob needs no alignment and is endian-portable by construction. Native-endian padding-free structs are not accepted.

### 6.2 Format

All integers little-endian. `HEADER_LEN = 32`.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `RLNKBLOB` |
| 8 | 4 | `version` = 1 |
| 12 | 4 | `elem_size` = `size_of::<T>()` |
| 16 | 8 | `schema` = `T::SCHEMA` |
| 24 | 8 | `count` = number of elements |
| 32 | `count * elem_size` | elements, back to back |

The total length must equal `32 + count * elem_size` exactly.

### 6.3 API

```rust
impl<T: IntoBytes + Immutable + Unaligned + Links> Arena<T> {
    pub fn freeze_into(&self, out: &mut Vec<u8>);              // appends header + elements
    pub fn freeze(&self) -> Vec<u8>;
}

pub struct BlobRef<'a, T> { /* items: &'a [T] */ }
impl<'a, T: FromBytes + KnownLayout + Immutable + Unaligned + Links> BlobRef<'a, T> {
    pub fn verify(bytes: &'a [u8]) -> Result<Self, VerifyError>;
}
impl<T> Region<T> for BlobRef<'_, T> { /* as_slice */ }
```

`verify` order: length at least `HEADER_LEN` (`Truncated`); magic; version; `elem_size`; `schema`; `count * elem_size` overflow (`CountOverflow`); exact total length (`Length`); cast the element bytes with `<[T]>::ref_from_bytes_with_elems` (any residual cast error maps to `Length`); then for every element `i`, `check_links(i, count)` or `BadLink { index: i }`. Verification is O(n), allocation-free, and returns a zero-copy view.

```rust
#[non_exhaustive]
pub enum VerifyError {
    Truncated,
    BadMagic,
    Version { found: u32 },
    ElemSize { found: u32, expected: u32 },
    Schema { found: u64, expected: u64 },
    CountOverflow,
    Length { expected: u64, actual: usize },
    BadLink { index: usize },
}
```

`VerifyError` implements `Display` and `core::error::Error`.

### 6.4 What verification buys

Memory safety does not depend on it (section 3.4). Verification gives fail-fast detection of corrupt or hostile data at load time, a guarantee that every same-region link resolves (the precondition for a future unchecked fast path), and detection of layout mismatches through `elem_size` and `schema`. This mirrors Goose's `from_bytes`, where tampered bytes are rejected before they become a value.

## 7. Benchmarks

### 7.1 Variants

| Name | Link representation | Role |
|---|---|---|
| `box` | `Option<Box<Node>>` | pointer baseline |
| `usize` | `Vec<Node>` with `usize` index links | machine-word arena baseline |
| `u32` | `Vec<Node>` with hand-written `u32` index links | what a Rust developer writes by hand; rustlink at u32 must match it |
| `abs32`, `abs16` | `OptAbs` for nullable links, `Abs` for always-present links (edge targets), in align-1 nodes | rustlink absolute |
| `rel32`, `rel16`, `rel8` | `OptRel` for nullable links, `Rel` for always-present links, in align-1 nodes | rustlink relative |

Variants whose links cannot represent a workload's links (`abs16` above 65,535 nodes; any `rel` whose deltas exceed the width) are not run for that cell. The cell is reported as `n/a` together with the measured link-distance distribution (fraction of links within 127 and within 32,767), which is itself a finding for the L4 decision.

### 7.2 Workloads

| Name | Description |
|---|---|
| `bst-random` | Insert N random `u32` keys into an unbalanced BST (nodes allocated in insertion order, poor locality), then N lookups (half hits). |
| `bst-bulk` | Build a perfectly balanced BST from sorted keys, children allocated before parents, then N random lookups. |
| `ast` | Build a random expression tree of at least N nodes bottom-up (leaf constant, unary negate, binary add or multiply), then evaluate it. Same node layout as the BST: `{payload: u32, a, b}`, with the operation in the high 8 bits of the payload and a 24-bit constant in the low bits. |
| `graph-csr` | Random sparse graph, N vertices, out-degree 1 to 7 (average 4), CSR arrays; BFS from vertex 0. Variants `usize`, `u32`, `abs32` (control: typed ids must match plain `u32`). |
| `graph-linked-seq` | Same graph as per-vertex linked edge lists, edges allocated consecutively per source vertex (best-case locality); BFS. Heads are `OptAbs<_, u32>` for every rustlink variant; only the `next` link varies. |
| `graph-linked-shuf` | Same, edge slots a random permutation (worst-case locality); BFS. |
| `blob-load` | rustlink variants only, on the `bst-bulk` tree. Build is rebuilding the arena from the plan; traverse is `BlobRef::verify` of the frozen image, in ns per node. |
| `blob-lookup` | rustlink variants only: random lookups through the verified blob view, to compare with `bst-bulk` lookups through the arena. |
| `ast-bytes` | Bench-only prototype for the L4 gate: the `ast` tree as variable-size records in a `Vec<u8>` (1-byte tag, LEB128 backward-offset links, inline integer constants), traversed by byte offset. Not part of the library. |

Random numbers use a fixed-seed xorshift64*. All variants of a workload consume identical key and edge sequences.

### 7.3 Sizes, measurements, environment

- Default sweep: N = 2^10, 2^12, 2^14, 2^16, 2^18, 2^20, 2^22, 2^24; `--big` adds 2^26. N is nodes for trees and vertices for graphs. Each workload declares a `max_n` (`bst-random` and the graph workloads 2^22, the others 2^24) and larger sizes are left out. With 12-byte nodes the sweep spans 12 KiB at 2^10 nodes to 192 MiB at 2^24, from L1-resident to DRAM-resident on this machine (aggregate figures when this spec was written: 24 CPUs, L2 40 MiB across 12 instances, L3 36 MiB, 62 GiB RAM, swap fully used). The report header records per-level cache sizes read from `/sys/devices/system/cpu/cpu*/cache`. A cell is skipped with a note if its estimated footprint (`n * footprint_per_n`) exceeds half of `MemAvailable`, or if the warm-up repetition predicts that the remaining repetitions exceed the time budget (`--budget-secs`, default 120).
- Per cell the runner reports: `size_of` node bytes; heap bytes requested per node (a counting global allocator, enabled only during the warm-up repetition so timed repetitions pay nothing); build ns per node and traverse ns per unit of work (a lookup, node visit or edge scan, declared per workload), each as median and median absolute deviation over at least 5 measured repetitions inside one fresh process, the first repetition being a discarded warm-up; peak RSS (`VmHWM` from `/proc/self/status`); minor and major page faults (`/proc/self/stat`).
- Isolation and noise control: one process per cell, so `VmHWM` is clean; the driver pins every process with `taskset -c <cpu>` to a CPU with the highest `cpuinfo_max_freq` (a P-core on a hybrid part; it falls back to CPU 0 with a warning if sysfs is unavailable); cells run in randomized interleaved order; `std::hint::black_box` guards all results. `perf` is not installed, so cache-miss counters are out of scope unless the user installs it.

### 7.4 Components

- `rlb` (runner): `rlb run --workload W --variant V --n N [--reps R] [--seed S] [--budget-secs B]` prints one JSON line (`measure::Measurement`); `rlb list` lists the registered workloads.
- `sweep` (driver): expands the matrix, pins and spawns `rlb`, appends to `bench/results.jsonl`, and renders `bench/results.md` (one group of tables per workload: rows are variants, columns are N, cells are traverse and build times, heap bytes, peak RSS and node size; machine info, kernel, `rustc` version and git revision in the header; the L4 gate table at the end). `--report-only` re-renders from existing results; the last measurement of a cell wins.
- Workloads implement the `Workload` trait. Each module (`trees`, `graph`, `blob_load`) exposes `workloads()`, and `registry::all()` concatenates them, so adding a workload never edits shared files.

## 8. Testing

- **Link properties** (`proptest`): for every width and form, round-trip at 0, 1, `MAX - 1`, `MAX` (`Abs`) and at `SIGNED_MIN`, -1, 0, 1, `SIGNED_MAX` (`Rel`); `new` rejects out-of-range values; `OptRel::new` rejects `Some(owner)` with `NullCollision`; `OptAbs::new(Some(W::MAX))` is rejected; `target` never panics for any owner and any raw bytes.
- **Layout**: `size_of`/`align_of` for all link types and widths; a sample node (`i32` key as little-endian integer plus two links) derives the zerocopy traits and has sizes 12, 8 and 6 bytes for u32, u16 and u8 links.
- **Arena and cursor**: append-only invariants; `follow`/`try_follow` on null, in-range, out-of-range and `Rel` underflow; cursor traversal of a hand-built tree matches the expected order.
- **Differential**: random BST operation sequences on the `box` variant and on each reachable rustlink variant give identical lookup results.
- **Blob**: freeze, verify, traverse matches arena traversal. Tamper test: on a small blob (64 nodes), set every byte offset to every one of the 255 other values; `verify` must return `Err`, or return a view whose links all resolve in range, with no panic. Truncation and trailing-byte cases return `Truncated` / `Length`. The exhaustive tamper test is skipped under Miri; a sampled variant runs everywhere.
- **Type safety**: a `compile_fail,E0277` rustdoc test on `Arena` shows that applying an `Abs<A, _>` to an `Arena<B>` does not compile. It checks the error code, not the message text, so it does not depend on compiler wording.
- **Tooling**: `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --check`; `cargo miri test -p rustlink` with `PROPTEST_CASES=16`; `cargo check -p rustlink --target wasm32-unknown-unknown` (confirms the `#![no_std]` crate builds for a non-host target; the crate has no feature flags).
- **Benchmark smoke test**: `rlb run` at N = 2^10 for every workload and variant exits 0 and prints valid JSON.

## 9. Orchestration

**Wave 0 (orchestrator, inline, sequential).** `git init`; workspace skeleton; full real implementation, with tests, of: L1 (`Width`, the four links, `Link`, `LinkError`), `Region`/`Cursor`/`Dangling`, `Arena`, `Links`/`impl_links!`, the blob (`freeze`, `verify`, `VerifyError`), and the benchmark foundation (`rng`, `stats`, `alloc`, `workload`, `measure` with `run_reps` and the JSON `Measurement`, `registry`). These are the contracts the parallel tasks build on; none is a stub. The zerocopy derive compile test on the generic link types (section 12, risk 1) is the first thing Wave 0 does.

**Wave 1 (three isolated tasks, disjoint files, in parallel).**

| Task | Owns | Delivers |
|---|---|---|
| `bench-trees` | `plan.rs`, `variants/`, `trees/` | `bst-random`, `bst-bulk`, `ast`, `ast-bytes` across all variants |
| `bench-graph` | `graph.rs` | `graph-csr`, `graph-linked-seq`, `graph-linked-shuf` |
| `bench-harness` | `proc.rs`, `report.rs`, `bin/{rlb,sweep}.rs` | runner, sweep driver, report renderer |

**Wave 2 (orchestrator, sequential).** Add `blob_load.rs` (`blob-load`, `blob-lookup`), its registry entry and `tests/smoke.rs`; run the full workspace checks including Miri and the `no_std` build; run review agents on soundness and API **before** the sweep, because builds and reviews would disturb the measurements; run the sweep with the machine otherwise idle; write `README.md` (usage, reach table, scenario guidance backed by the measured results) and `CHANGELOG.md`.

**Wave 3.** Present `bench/results.md` and the L4 gate table to the user for the go/no-go.

Tasks never edit files they do not own; any contract change goes through the orchestrator. Each task runs in its own worktree, and the orchestrator integrates.

## 10. L4: gated byte-addressed layer

Not built in v1. If the gate passes, a follow-up spec covers `VarRegion` (variable-size records in a byte buffer, sequential access) and `VarLink` (LEB128, zigzag-signed for relative links, construction-only, as in Goose section 3.6). The gate evidence is the `ast-bytes` prototype against the best L2 variant on `ast`, and the measured link-distance distributions from the `rel` cells. Default rule (adjustable by the user): proceed only if `ast-bytes` uses at least 1.5x less heap than the best L2 variant at no more than 1.1x its traversal time, at N >= 2^20.

## 11. Non-goals

Element removal and free-list reuse; concurrency primitives; an `mmap` helper (callers pass `&[u8]`); a derive macro for `Links`; varint links and variable-size records (L4); testing on big-endian hardware (portability is by construction through little-endian wrapper types, and is verified on this little-endian host only); publishing to crates.io (`publish = false`).

## 12. Risks

1. **`zerocopy` derives or trait bounds on generic link types may not compile** (associated-type field `W::Le`; bounds such as `Hash` on the storage type). Wave 0 tests this first. Fallback: relax bounds that are not needed; if a derive still fails, one audited `unsafe impl` block in `link.rs`, covered by Miri, which would be the only `unsafe` in the crate.
2. **Bounds checks may dominate traversal.** The benchmarks measure `rustlink` against the hand-written `u32` baseline. If the cost is material, a later opt-in unchecked path, usable only on a verified `BlobRef`, goes into one audited module.
3. **Sub-32-bit widths may not pay.** They are zero-cost generics, so all widths ship; the README states measured guidance only.
4. **Benchmark noise** on a CPU that is likely hybrid (P/E cores) with swap fully used. Mitigated by pinning to the fastest core, the footprint cap, interleaved order, repetitions with median and MAD, and machine info recorded in the report.
5. **Locality-dependent reach.** Random structures will overflow `Rel` u8/u16; the harness reports this instead of failing.
6. **Scope creep toward L4.** The gate and the non-goals list bound it.

## 13. Acceptance criteria

1. Section 8 tooling passes: workspace tests, clippy with `-D warnings`, `fmt --check`, Miri, the `no_std` build, the tamper test and the benchmark smoke test.
2. `sweep` completes the default matrix on this machine and writes `bench/results.md` with at least 5 repetitions per cell, machine info, link-distance distributions for unreachable cells, and the L4 gate table.
3. `README.md` documents usage, both link forms, the reach table and measured scenario guidance; `CHANGELOG.md` has the initial entry.
4. The L4 go/no-go is presented to the user with the numbers.
