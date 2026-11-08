# Rustlink formalization

Lean 4 model of `crates/rustlink`, built bottom-up. No Mathlib;
arithmetic proofs use core `omega`/`decide`.

## Layers

1. **Width** (`Rustlink/Width.lean`): `BYTES`, `MAX = 2^(8·BYTES) - 1`,
   `SIGNED_MIN = -2^(8·BYTES - 1)`, `SIGNED_MAX = 2^(8·BYTES - 1) - 1`.
   Abs/OptAbs/Rel/OptRel are `Link W` subtypes over one `Width` (no `T/W`
   generics in Lean; link payloads are `Nat`/`Int`).

2. **Link** (`Rustlink/Link.lean`): `Target` + `target`/`check` mirror
   `crates/rustlink/src/link.rs:13-36,97-247`. Encodings:
   `Abs(idx) = idx`, `OptAbs(idx+1)` with `0 = null`,
   `Rel(delta)`, `OptRel(delta)` with `0 = null`.

3. **Region** (`Rustlink/Region.lean`): `follow` is sound —
   any `Some (i, _)` satisfies `i < len`.

4. **Arena** (`Rustlink/Arena.lean`): `push` (fresh index, earlier
   elements unmoved) plus `clear` invalidation (old indices idle).

5. **Blob** (`Rustlink/Blob.lean`): `verifyFull` in short-circuit order
   (magic, version, elem size, schema, overflow, exact length) with a
   per-element list conjunction (`links : Nat → List (Link w)`);
   `verifyFull = true ⇒ allLinksOk` and `tryFollow` never errors.
   Byte-for-byte decoding beyond the header fields is not modeled.

6. **Maximums** (`Rustlink/Maximums.lean`): bridges tying the maxima to
   constructors — `absConstructor` accepts iff `i < MAX + 1`,
   `optAbsConstructor` accepts iff `i < MAX`, targets round-trip —
   plus the closed-form table below.

7. **Collections** (`Rustlink/Collections.lean`): list/hash cap `MAX`,
   B-tree upper bound `128 * MAX` (unreachable; measured ~5.7M for `u16`),
   pure-link saving `8 / bytes`.

8. **Memory** (`Rustlink/Memory.lean`): arithmetic size model.
   Link `size = BYTES`, `align = 1`; node `payload + k * bytes`.
   Nodes are modeled as `List Field` with `size = sum`, `padding = 0`
   when every `align = 1` (BST/graph instances via `nodeFields`).
   Heap `n * size` requested bytes (counting allocator,
   `rustlink-bench/src/alloc.rs`); `Vec`-doubling monotonicity.

9. **Stability** (`Rustlink/Concurrency.lean`): `follow`/`tryFollow`/
   `check` are `append`-stable — a hit in `xs` stays a hit in
   `xs ++ [x]` and `xs ++ tail`, and `check` stays `true`.
   This is the reader-writer refinement: an old snapshot's answers
   for in-range indices never change under `push`.

## What is intentionally not modeled (yet)

- `zerocopy` byte-for-byte endianness (only the decoded `Nat`/`Int`).
- `usize::MAX` saturation (assumes 64-bit `usize` = `2^64 - 1`).
- Element payload layout — nodes are opaque `payload: Nat`.
- `Send`/`Sync`, atomics, and `Vec` capacity growth bounds.
  `Send`/`Sync` are rustc auto-trait results (not modeled);
  the model proves snapshot/append stability of `follow`/`tryFollow`/
  `check` plus the arithmetic size model.

## Theoretical maximums (theorems in `Rustlink/Maximums.lean`)

| Width | Hold `Abs` cap | Hold `OptAbs` cap | `Rel` span | `node_limit` |
|-------|---------------|-------------------|------------|--------------|
| `u8` | 256 | 255 | 256 | 255 |
| `u16` | 65,536 | 65,535 | 65,536 | 65,535 |
| `u32` | 4,294,967,296 | 4,294,967,295 | 4,294,967,296 | 4,294,967,295 |
| `u64` | 2^64 | 2^64 - 1 | 2^64 | 2^64 - 1 |

## Verification

```text
cd lean && lake build   # 12 jobs, zero sorry
```
