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

## Outcome

Measured on 2026-10-23 on the shared machine of the first record, in 37 repetitions of each workload's whole matrix. Ratios are taken inside one sweep and judged on the 14 to 16 calm sweeps per workload; `bench/decisive/environment.txt` defines calm, and `bench/decisive/ratios.tsv` has the interquartile ranges. The predictions above are unedited.

| # | Verdict | Evidence |
|---|---|---|
| P1 | held | 222 done node-graph cells: 64, 40, 20, 12 and 8 bytes per node exactly (worst deviation 0). |
| P2 | half: held on `ngraph-random`, falsified on `ngraph-chase` | `rc` / `u32` at 2^20: 2.15 (interquartile range 2.12 to 2.49) on the random sweep; 1.29 (1.21 to 1.40) on the chase, below the 1.5 line. The chase ratio grows as the machine calms (1.09 in the heaviest load band, 1.51 on the 4 calmest sweeps); extrapolated linearly to a quiet machine it is 1.41, still short of 2. |
| P3 | falsified, on the high side | Mesh at 2^22: `rel16` / `u32` = 1.74 (1.70 to 1.87), above 1.3 (1.41 in the heaviest load band, 1.79 in the calmest). The cost is not memory: the ratio is already 2.17 at 2^16, where the `u32` graph fits the L2. The reading rule's other trigger fired: `ngraph-banded` `rel16` takes 0.80 (2^20) and 0.83 (2^22) of the `u32` time, in every load band. |
| P4 | not decided | `usize` / `u32` at 2^20 = 1.22 (1.17 to 1.31): above the 1.1 falsification line, below the predicted 1.3. It is 1.07 in the heaviest load band and 1.45 on the 4 calmest sweeps. At 2^18, a size the prediction did not name, it is 2.32 (1.76 to 3.10). |
| P5 | held | The reach matrix matches at every size of the grid. Direct runs: `rel16` on `ngraph-random` fits 2^15 nodes and not 2^15 + 1,024. |
| P6 | not falsified; two numeric bounds missed | `u32` against `usize`. 256-byte nodes: memory 1.7% less (within 5%), time 0.91 at 2^18 and 2^20 (within 10%) but 0.85 (0.82 to 0.91) at 2^22, outside (0.91, inside, on all 37 sweeps). 64-byte nodes: memory 8.6% less (within 15%), time 0.92 at 2^18 and 2^20 (predicted 10% to 25% faster: 8% is short) and 0.83 at 2^22 (within). No cell reaches 1.3x faster, and memory never differs by 20%. |
| P7 | held on the medians, borderline at one cell | `abs32` / `u32` is 0.94 to 1.09 over 2^16 to 2^22. 256-byte nodes at 2^22: 1.09 (0.97 to 1.20), and 1.21 on the 6 calmest sweeps. |
| P8 | falsified on time, held on memory | `BTreeSet` / `u32` lookup time is 1.63 to 2.28 over 2^16 to 2^22, against the 1.5 limit. Memory per key: 5.82 bytes against 4.72 (`abs32`, 64-byte nodes, 1.23x) and 4.16 (256-byte nodes, 1.40x), within 1.5x. |

What follows, in the terms of "How to read the outcome":

* P3 failed upward, so narrow links are not free in time: where a loop does not wait for memory, 32-bit links take 1.23x to 2.38x the `u32` time in sweeps of 2^16 nodes and up. In dependent chases (0.95x to 1.08x) and B-tree lookups (0.94x to 1.09x) the decode does not show. The probes `abs32-idx` and `rel16-idx`, which decode with `Link::target` and index the slice without `Cursor`, are about as fast as the `Cursor` variants or slower, so they rule out `Cursor` as the cost. They still run the full checked decode, so what remains was not separated: the checks inside the decode (range, null, bounds), its arithmetic, and what it stops the compiler doing to the plain `u32` loop (vectorisation was not inspected). The value of an unchecked path is untested; a probe that reads the raw stored integer is the experiment that would separate the checks from the rest.
* The time argument exists in one regime: a large (2^20 nodes and up) local graph swept in order, where `rel16` reads 40% fewer bytes per node than `u32` and takes 0.80x to 0.83x of its time. It is absent in cache and absent on the mesh.
* P6 held in substance: B-trees are not a place for narrow links. `u32` against `usize` saves 2% to 9% of memory and 8% to 17% of lookup time; `abs16` saves 2% to 6% more, up to its reach.
* P8's miss is not a verdict on `BTreeSet`: it is dynamic, and these trees are static and packed to capacity.

Deviations from the note's own plan: no separate paired runner was written. A workload's sweep takes 15 to 20 seconds, so the ratio between two variants inside one sweep is already a paired measurement; the sweeps were repeated 37 times with canary cells around each, and the judged sample is the calm subset (the first record's busy-core selection rule could not be met on this machine). Two probe variants (`abs32-idx`, `rel16-idx`) were added to the node graphs after the first look at the mesh, to separate the cost of `Cursor` from the cost of the decode. Layer 4 is unaffected: its gate (3.68x less heap at 0.81x to 0.84x of the time) rests on one workload, the expression tree, and part of its saving comes from variable-size records, not from varint links.
