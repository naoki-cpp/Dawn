# OWASP Top 10 (2021) mapped to Dawn

Dawn is a binary-protocol (WebSocket + JSON lines) game server, not a web
app. This file records which categories apply, how they translate, and which
are deliberately out of scope — so a review neither skips a real risk nor
wastes time on web-only categories.

File names below are baseline-era orientation, not truth — discover the
current surface with SKILL.md's Step 1 commands. What is durable here is the
*category mapping* (what each OWASP item means for this architecture), which
only changes if the architecture itself changes (a web admin panel, an HTTP
API, outbound requests).

| OWASP | Applies? | Dawn translation |
|---|---|---|
| A01 Broken Access Control | **Yes — highest value** | `owns_ship` / `can_use_station` / active-ship resolution before any state mutation. The `*_owned` handler suffix is the convention marker. |
| A02 Cryptographic Failures | Deferred | No TLS/auth by documented decision (LAN prototype). In scope only at public-release trigger. |
| A03 Injection | **Yes** | SQL (rusqlite in `station_inventory_db.rs`) + any client string reaching a path/format!/log line. GDScript client strings are also untrusted server-side. |
| A04 Insecure Design | **Yes (resource exhaustion)** | Client-driven allocation/loop counts, missing WebSocket frame caps, unbounded collections in `ClientCommandJson`. |
| A05 Security Misconfiguration | Partially | Library-default limits used implicitly (the `WebSocketConfig` finding is this category as much as A04). No config files with secrets today. |
| A06 Vulnerable Components | **Yes — automated** | Covered by CI: `cargo audit` (RUSTSEC) + `cargo deny` run on every PR. Manual review not needed; just confirm CI is green. |
| A07 Identification/Auth Failures | Deferred | Same as A02 — no auth layer exists by decision. Session resume tokens (`hello_resume.rs`) are the one live surface: they gate reconnection, so check they can't be trivially guessed/replayed once auth matters. |
| A08 Software & Data Integrity | **Yes** | "Clients send intents and IDs, never authoritative values." Quantities, costs, positions, damage are server-computed. Event-sourcing replay (INV-001) is the integrity backbone — a client value written into an event corrupts history permanently. |
| A09 Logging/Monitoring Failures | Low priority | `eprintln!`-level logging exists. No security-event alerting; acceptable for LAN prototype. Don't file findings here yet. |
| A10 SSRF | No | Server makes no outbound requests driven by client input. Re-check only if a feature adds client-supplied URLs (e.g. avatar fetch). |

## Web-only categories that never apply

XSS, CSRF, clickjacking, session fixation, open redirects: there is no
browser origin model in a Godot client speaking a custom protocol. If a
future web-based client or admin panel appears, revisit.

## Event-sourcing-specific concern (no OWASP number)

A validation bypass here is worse than in a CRUD app: events are append-only
facts (INV-006 — events cannot be rejected once emitted) and replay
reconstructs state from them. Any finding where unvalidated client data
reaches an **event emit** should be graded one severity level higher than the
same flaw in transient state, because the corruption survives restarts and
propagates through replication.
