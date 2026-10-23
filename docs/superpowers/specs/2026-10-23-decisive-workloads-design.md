# Decisive workloads: link-dense node graphs, pointer chasing and B-trees

Design note, 2026-10-23. Extends section 7 of `2026-10-22-rustlink-design.md`. The predictions below were written before any of these workloads was measured.

## Why

The first sweep (405 cells) used 12-byte nodes with two links. That cannot tell "narrow links help" from "this structure is link-light", and its pointer baseline (`Option<Box<_>>`) is not what people write for graphs. It found that 32-bit links halve memory against pointers, save nothing against hand-written `u32` indices and cost 0% to 17% traversal time, and that 8- and 16-bit links fit only local structures. Four questions are left:

1. Where links are most of each node, and local, do 8- and 16-bit links win on time as well as memory?
2. What do the baselines people write cost: an `Rc` node graph, `std::collections::BTreeSet`?
3. Does a latency-bound dependent chase, unlike an independent sweep, separate the variants by cache footprint?
4. In a B-tree, where link width sets the fanout of cache-line nodes, does it matter?

## Workloads

All use the first sweep's methodology: seed 1, 6 repetitions (1 discarded), one process per cell pinned to one CPU, heap bytes counted by the global allocator. Sizes are 2^10 to 2^22 in steps of 2^2 unless noted.

**Node graphs.** Every node is `{ payload: u32, 4 links }`, links are null where a neighbour is missing, and every variant consumes the same plan. Variants: `rc` (`Rc` nodes with `Weak` links, the safe idiom for cyclic graphs), `usize`, `u32` (hand-written indices, what petgraph does), `abs32`, `abs16`, `rel32`, `rel16`, `rel8` (rustlink, traversed through `Cursor::follow`).

| Workload | Links | Operation (unit) |
|---|---|---|
| `ngraph-mesh` | 4 neighbours on a `w x h` grid (distance 1 and `w`) | sweep: sum of every neighbour's payload, nodes in index order (edge) |
| `ngraph-banded` | 4 random targets within 1000 of the node | sweep (edge) |
| `ngraph-random` | 4 uniformly random targets | sweep (edge) |
| `ngraph-chase` | as `ngraph-random` | a dependent walk of `n` steps: the next link is chosen from the payload read at the current node (step) |

**B-tree.** A packed, bulk-loaded B+tree of `u32` keys with fixed-size nodes, so fanout follows link width: leaves hold keys only, inner nodes hold separators and child links. Lookups, half hits. Variants: `std` (`BTreeSet`), `box` (an enum of boxed nodes), `usize`, `u32`, `abs32`, `abs16`. Relative links do not apply: a child lives in another region, and `Rel` links are same-region only. `btree-64` uses 64-byte nodes (a cache line, where link width decides fanout: 5, 8 and 11 children for 8-, 4- and 2-byte links) and `btree-256` 256-byte nodes (21, 32 and 43). Sizes up to 2^22 keys.

## Predictions

| # | Prediction | Falsified if |
|---|---|---|
| P1 | Heap bytes per node: `rc` 64, `usize` 40, `u32`, `abs32` and `rel32` 20, `abs16` and `rel16` 12, `rel8` 8 (exact, asserted by a test). | any differs by 0.5% or more |
| P2 | `rc` is heavy: at 2^20 on `ngraph-random` and `ngraph-chase` it takes at least 2x the `u32` time. | below 1.5x |
| P3 | Narrow links save memory, not time, in a link-dense local graph: on `ngraph-mesh` at 2^22 `rel16` takes 0.9x to 1.3x the `u32` time (the sweep is bound by decoding links, not by bandwidth). | below 0.9x (narrow links buy speed) or above 1.3x |
| P4 | Footprint decides a dependent chase: on `ngraph-chase` at 2^20, `usize` (40 MB, over the 36 MB L3) takes at least 1.3x the `u32` time (20 MB, inside it). | below 1.1x |
| P5 | Reach: `rel16` fits `ngraph-mesh` and `ngraph-banded` at every size, `rel8` fits the mesh only up to 2^12, and on `ngraph-random` `rel16` fits only up to 2^15 nodes. | any differs |
| P6 | Link width barely moves a B-tree: at 256-byte nodes `u32` memory per key is within 5% of `usize` and lookups within 10%; at 64-byte nodes lookups are 10% to 25% faster with `u32` than with `usize`, memory within 15%. | `u32` is 1.3x faster at 256 bytes, or memory differs by 20% or more |
| P7 | `abs32` is as fast as `u32` in a B-tree (within 10%): the bounds checks are amortised over a node scan. | outside 10% |
| P8 | `std::BTreeSet` is within 0.7x to 1.5x of the rustlink B-trees in lookup time, and within 1.5x in memory per key. | outside |

## How to read the outcome

If P3 holds, the case for narrow links is memory, capacity and I/O, and an unchecked fast path would be the lever for time. If any link-dense workload shows narrow links at 0.9x or less, the library has a time argument as well. If P6 holds, B-trees are not a place for narrow links: links are a minority of their bytes, and typed single-region arenas force a per-level type for the leaf and inner split, which is a cost of its own.

## Measurement

The machine is shared and its background load changed during the first sweep's session. Heap bytes are exact. For time, each cell is measured in repeated full sweeps under the first record's selection rule, with the load monitor running, and the headline ratios are also measured in paired rounds, every variant of one cell within the same minute, because a ratio of simultaneous measurements survives slowly changing load better than a ratio of separate ones. Both are reported with their spread. A prediction is judged on the paired ratios when the sweeps' selection rule cannot be met.

## Non-goals

No mutation workloads (the arena is append-only), no `Rel` variant of the B-tree, no `dyn` node graphs, no changes to the library.
