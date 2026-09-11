# Measurements

Recorded benchmark results, target vs. reality.

## Hierarchy propagation (Phase 7)

`benches/hierarchy.rs`: transform propagation over a ~10⁶-node, depth-10 tree
(64 parents per level, fan-out 1800), 12 cores.

| Approach | Time | vs. recursive |
| --- | --- | --- |
| Serial recursive walk (`get`/`get_mut` over a children map) | 34.1 ms | 1.0× |
| Per-depth parallel pass (`World::propagate`) | 6.4 ms | **5.3×** |

Target: ≥ 3× vs. the recursive walk, scaling with cores. Met. The speedup is
sub-linear in cores because per-node access is random-access and memory-bound,
and the shallow top levels run near-serially.
