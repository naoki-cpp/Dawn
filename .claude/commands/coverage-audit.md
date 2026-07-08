# /coverage-audit — Measure test coverage and close the gaps that matter

Run this periodically (like `/architecture-review`), after a feature phase
lands, or when a new logic-bearing file has accumulated changes without a
test module. It is NOT a per-PR gate — the instrumented rebuild takes
minutes, and chasing a percentage on every PR produces filler tests. The
audit's value is finding the files whose failure mode is invisible to
manual play testing.

Established by PR #112 (apply_event.rs 59%→95%, coordinates.rs 60%→89%,
protocol.rs 62%→94%).

---

## Step 1: Measure

```bash
cargo llvm-cov --workspace --summary-only -- --skip wire_schema_doc_is_up_to_date
```

- Requires `cargo-llvm-cov` + the `llvm-tools` rustup component (both
  already installed on the dev machine).
- `--skip wire_schema_doc_is_up_to_date`: known CRLF/LF checkout flake,
  unrelated to coverage.
- For per-line detail on a specific file:
  `cargo llvm-cov report --show-missing-lines`.

## Step 2: Interpret — not every 0% is a gap

**Intentionally uncovered (do not chase):**

- Binary entry points and process wiring: `dawn-simulation/src/main.rs`,
  `serve/{runtime,single,cluster,aoi_delivery}.rs`, `dawn-sector-node/src/
  {main,runtime,config,data_loader}.rs`, `dawn-actor/src/ws_server.rs`,
  `bench.rs`. These are covered by manual/hardware verification (8D-5) per
  `docs/architecture/architecture-review-server.md` — unit tests here would
  mock away everything they actually do.
- Defensive debug-only branches (e.g. `debug_assert_missing_anchor`): they
  exist to fail loudly on a data-integrity bug during development, not to
  pass in a test.

**High-value gaps (chase these first):** files whose bugs only surface in
situations manual testing never exercises —

1. **Event replay** (`node/apply_event.rs` and anything INV-002-critical):
   a broken replay arm corrupts state only after a server restart.
2. **Wire conversion** (`dawn-actor/src/protocol.rs`): a broken
   serialization/parse arm only surfaces as a client-side symptom, often a
   silently-dropped message.
3. Any logic-bearing file with **no test module at all** (a `mod tests`
   grep is faster than reading the coverage table).

Rule of thumb: logic-bearing files below ~80% lines deserve a look; the
two categories above deserve a look regardless of percentage.

## Step 3: Write the tests

- One test per uncovered match arm / variant, named for the guarantee
  (e.g. `ship_undocked_event_replay_clears_docked_state`), not the
  mechanism.
- For replay arms, also test the **idempotence/no-op path** where the
  event can legitimately arrive against already-reconstructed state
  (`restore_from` replays the full post-snapshot tail — see
  `ship_spawned_event_replay_is_a_no_op_for_an_already_reconstructed_ship`).
- For wire conversion, test both directions where they exist, the
  `skip_serializing_if` omissions (absent key, not `null`), and that
  internal events return `None` — the "not forwarded" list is a contract
  too.
- **Precision/scale trap** (learned in coordinates.rs): match the fixture's
  scale to the precision the accessor actually guarantees. A small expected
  delta can silently vanish — not crash, just quietly round to `0` or some
  other misleadingly clean wrong value — if the numeric representation's
  precision at the fixture's chosen scale is coarser than the delta being
  tested. This applies to any representation (f32, fixed-point, or whatever
  replaces it later), not just f32 specifically.

## Step 4: Re-measure and report

```bash
cargo llvm-cov --workspace --summary-only -- --skip wire_schema_doc_is_up_to_date
```

Plus the usual gates: `cargo fmt --all -- --check`,
`cargo test --workspace`, `cargo clippy --workspace -- -D warnings`.

The PR description states, per file: before% → after% (lines), what was
uncovered (which arms/variants), and anything **deliberately left
uncovered with the reason** (same rule as `/rust-api-audit`: silent
omission is the failure mode this skill exists to prevent). Anything
noticed but out of scope (a suspicious arm, a doc mismatch) gets filed,
not fixed inline.
