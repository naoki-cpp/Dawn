---
scope    : Record of performance baseline values taken before Phase 7
audience : AI Agent / Human Developer
update   : Re-run the benchmarks at phase completion and append results
related  : ../architecture/tick-model.md, roadmap.md
---

# Benchmark Baseline

Baseline values for quantifying the overhead introduced by Raft (Phase 7)
and later work.
Measurement command: `cargo run -p dawn-simulation --bin simulate --release`

## As of Phase 6 completion (2026-06-11, commit 450aa8a)

Environment: Windows 11 Pro / development machine (absolute values are
machine-dependent; always compare on the same machine).

### Phase 1 benchmark — single node, 10,000 ships, 100 ticks

| Metric | Value |
|---|---|
| spawn | 10,000 ships / 12.4 ms |
| tick min | 83,316 µs |
| tick mean | 87,485 µs |
| tick p95 | 92,506 µs |
| tick max | 96,823 µs |
| SLA (≤16,000 µs) | ✗ FAIL |

Note: run-to-run variance is large (a same-day rerun recorded a mean of
131,851 µs). Run multiple times and compare trends, not single values.

### Phase 2 demo — 3 nodes × 1,000 ships, 20 ticks

| Metric | Value |
|---|---|
| spawn | 3,000 ships / 65 ms |
| 20 ticks × 3 nodes | 73 ms |
| replicated | 3,000 events |

### Phase 3 demo — persistence round-trip, 100 ships, 10 ticks

Confirmed that the snapshot save / restore / event-replay round-trip
completes.

## Phase 7 (ADR-0014 Raft Consensus)

Measurement command:
`cargo test -p dawn-simulation --release transit_latency_benchmark -- --ignored --nocapture`

### Sector Transit latency (local ECS operations only)

| Metric | Value |
|---|---|
| propose_transit + export_transit + import_transit (avg, 1,000 iterations) | 10.664 µs |

Note: this value covers only the ECS operations on `SimulationNode`
(setting TransitState / extracting the snapshot / restoring into the ECS).
It excludes the latency until the proposal commits into the Raft log
(election/heartbeat run on tick-driven timers, INV-005). Raft commit latency
is on the order of `heartbeat_interval` (3 ticks in cluster.rs); the ECS
operation cost itself is negligible.

## Known issues (as of Phase 6 — separate from the baseline itself)

1. **Phase 1 SLA FAIL**: mean 87 ms at 10,000 ships (target 16 ms). The
   tick got heavier when Phase 6 added the Capacitor / Lock / Combat / Bot
   systems. Meeting the 10,000-ship SLA is Phase 8 scope (Anti-TiDi /
   Spatial Index), so missing it here is expected.

The staleness in the Phase 2 / Phase 3 pass criteria (which assumed the
pre-ADR-0008 per-tick event model) was fixed in commit 4408ca8. Both now
PASS.
