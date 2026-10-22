# Changelog

## 0.1.0 (unreleased)

### Added

- `rustlink`: narrow typed links `Abs`, `OptAbs`, `Rel` and `OptRel` at `u8`, `u16`, `u32` and `u64`; bounds-checked resolution through `Region` and `Cursor`; the append-only `Arena`; `Links` and `impl_links!`; position-independent blobs with `Arena::freeze` and `BlobRef::verify`. `#![no_std]`, no `unsafe`.
- `rustlink-bench`: pointer, `usize`, `u32` and rustlink variants of binary search trees, an expression tree, graphs and blobs; the `rlb` one-cell runner; the `sweep` matrix driver; a Markdown report with the layer-4 gate table. The first full sweep has not been recorded yet.
