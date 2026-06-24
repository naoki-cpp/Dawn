# Dawn

[![Rust CI](https://github.com/naoki-cpp/Dawn/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/naoki-cpp/Dawn/actions/workflows/rust-ci.yml)

An event-sourced, distributed space-combat sandbox: a Rust simulation server with a Godot 4 client, built to scale a single shard across physical nodes via Raft-replicated sector transit.

## Layout
- **Server** — Cargo workspace in `crates/`: `dawn-core` (domain types/events), `dawn-ecs` (systems), `dawn-sector` (game logic), `dawn-simulation` / `dawn-sector-node` (server binaries), `dawn-consensus` (Raft), `dawn-replication` (log gossip).
- **Client** — Godot 4 / GDScript in `client/`.
- **Docs** — decisions in `docs/adr/`; contributor entry point is `AI_AGENT_BRIEF.md`.

## Build & run
```bash
cargo test --workspace                                   # build + test the server
cargo run -p dawn-simulation --bin simulate -- --serve   # run the server, then open the Godot client
```

Client tests (GdUnit4): see `docs/process/godot-client-testing.md`.
