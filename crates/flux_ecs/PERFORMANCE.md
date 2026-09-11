# flux_ecs2 — deferred performance work

Improvements that are known, designed-for, and deliberately not built yet.
Each entry names its trigger — the observation that should cause the work —
so nothing here depends on remembering this file at the right moment: check
it whenever a profile or benchmark looks off.

## Command queues: byte-arena encoding and batched application

Commands are a `Vec<Command>` enum with boxed payloads: one small heap
allocation per value-carrying command and a virtual call at apply. The
zero-allocation representation writes payloads inline into a byte arena
(headers + offsets), and application batches commands by kind and by
(source, target) archetype pair into bulk range copies instead of
per-entity moves.

- Cost today: ~20–50 ns per command; noise at current rates (renderer:
  ~24 at init, ~1/frame).
- Trigger: mass structural churn — thousands of commands per frame — or the
  bulk-structural-change milestone, whichever lands first.
- The enum's discriminants and separated payloads are the shape the arena
  version consumes; `Commands`'s API does not change.

## Entity reservation: prediction → atomic reservation

`Commands::spawn` predicts ids from a per-fetch allocator snapshot and
redeems them with `Entities::alloc_specific`. Correct in the serial
executor because nothing allocates between a system's run and its apply.
The parallel executor breaks that assumption; the design there is lock-free
reservation on `Entities` (atomic cursor over the free list, negative
overflow for fresh indices), with `alloc_specific` unchanged as the
redemption primitive.

- Trigger: the parallel executor. Not before — atomics on the serial path
  are pure cost.

## Intra-system parallelism: explicit, not automatic tiling

`Query::par_for_each` splits a query's matched chunks across threads. The
design envisioned *automatic* tiles — the executor splitting any system by
chunk range — but that is unsound for opaque function-systems (one keeping
cross-row state would be run wrongly over N ranges). par_for_each's
`Fn + Sync` bound enforces per-row purity, making the split sound; it is the
mechanism every production ECS uses. Measured scaling on 12 cores: ~7.4x on
a compute-bound 1M-row kernel, ~5.8x on a bandwidth-bound one (the ceiling
is the workload's arithmetic intensity, not the executor).

- Automatic tiling would need a kernel-style system model (systems expressed
  as per-chunk functions the executor can re-invoke over ranges) — a larger
  change than the current function-system design, deferred as its own effort.
- The cost-model / over-splitting concern from the roadmap does not arise:
  splitting is opt-in per call, so a system that does not call par_for_each
  has zero parallel overhead.

## Parallel executor: refined access recomputed every wave

`run_parallel` recomputes each not-yet-run system's refined access (refresh
+ matched-archetype snapshot) at every wave boundary, since barrier commands
can create archetypes. The design calls for caching the plan keyed by
archetype generation and recomputing only on change.

- Cost today: refresh is incremental (scans only new archetypes) and
  `matched().to_vec()` per accessed component per wave; small for stable
  archetype sets.
- Trigger: profiles showing wave planning dominating on schedules with many
  systems, or workloads with continuous archetype churn (§8.3).

## Ambiguity detection: O(n²) pairs with per-node DFS

`Schedule::compile` computes reachability per system via DFS and checks
every unordered pair. Fine for hundreds of systems, recomputed only when
the schedule changes.

- Trigger: schedules with thousands of systems, or per-frame schedule
  mutation. The parallel executor's access-refined plan replaces this
  machinery wholesale.

## Small-archetype chunk waste

`CHUNK_SIZE` is 64 KiB (measured: the 16 KiB page-per-chunk layout cost a
1.49× streaming tax on 16 KiB-page hardware; 64 KiB reduces it to 1.05×).
Worst-case waste is one partial chunk per archetype — 64 KiB × archetype
count for tiny archetypes.

- Trigger: allocator peak counters showing waste dominating a
  many-archetype, few-entities world (editor/tool worlds). Known answer:
  size classes — small chunks for small archetypes.

## Query iteration: 89% of the flat-SoA baseline

`benches/`: `baseline/simple_iter_1M` (flat SoA, the absolute ceiling),
`baseline/chunk_layout_1M` (the ceiling given chunked storage),
`query/simple_iter_1M` (actual). Remaining gap is per-chunk fetch
machinery (~9 ns/chunk).

- Trigger: regression below the chunk-layout baseline in the bench, or a
  profile showing fetch overhead on real workloads.
