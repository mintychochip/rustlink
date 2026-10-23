# Changelog

## 0.1.0 (unreleased)

### Added

- `rustlink`: narrow typed links `Abs`, `OptAbs`, `Rel` and `OptRel` at `u8`, `u16`, `u32` and `u64`; bounds-checked resolution through `Region` and `Cursor`; the append-only `Arena`; `Links` and `impl_links!`; position-independent blobs with `Arena::freeze` and `BlobRef::verify`. `#![no_std]`, no `unsafe`.
- `rustlink-bench`: pointer, `usize`, `u32` and rustlink variants of binary search trees, an expression tree, graphs and blobs; the `rlb` one-cell runner; the `sweep` matrix driver; a Markdown report, in registry order, with the layer-4 gate table.
- `bench/`: the recorded sweep of the default matrix (405 cells): `results.md` and `results.jsonl`, `replicates.tsv` with the six sweeps behind every measured cell, `box-paired.tsv` with the paired runs of the pointer variant, and `environment.txt` on the machine, the selection rule and the noise. On trees, 32-bit rustlink links halve the heap of 64-bit pointers and are 1.3x to 2.3x faster where memory dominates; on linked lists narrow links save memory (up to 3x) but not time; sub-32-bit links fit only local structures; the layer-4 gate passes (3.68x less heap at 0.81x to 0.84x of the traversal time). The 14 `box` cells of the two linked-list graph workloads are skipped until they can be measured on an idle machine.
