---
name: security-check
description: OWASP-informed security review of Dawn's untrusted-input paths (wire protocol, command validation, SQL, resource limits). Use whenever the user asks about security, SQL injection, input validation, OWASP, vulnerability review, DoS/resource-exhaustion concerns, or after changes to the wire protocol crate, any SQL/rusqlite code, or any command handler. Also run it when a new client-facing command or wire message type lands, even if the user only says "check the new command is safe".
---

# /security-check — OWASP-informed input-path security review

Reviews every path where untrusted client input enters the Dawn server, from
the network frame up to the point it mutates game state. Uses OWASP
categories as vocabulary, adapted for a non-HTTP binary-protocol game server
(most web-centric OWASP items — XSS, CSRF, session fixation — do not apply
here and are deliberately out of scope).

> **This is an analysis skill. It changes no code.** Findings that need fixing
> become their own PRs, or get filed in
> `docs/architecture/security-review.md` with a decision + trigger, mirroring
> how `/architecture-review` files issues. Fixes and review stay separate so
> the review diff is docs-only.

**Nothing in this file is a source of truth about the code.** File names,
function names, and patterns below are discovery *starting points*, correct
as of the baseline date. Always re-derive the actual surface with the
discovery commands in each step — the codebase moves, and this skill must
not rot with it. If a discovery command comes up empty (a file was renamed,
a pattern changed), that is itself worth a minute of investigation, not a
"step passed".

Arguments (optional):
- `sql` = SQL layer only (Step 2)
- `wire` = wire protocol / deserialization only (Steps 1, 5)
- a file path or command name = scope the review to input paths that reach it
- omitted = full review (all steps)

## Known scope boundaries (do not re-flag)

- **No TLS / no authentication** is a documented decision for the LAN-only
  prototype (`docs/architecture/architecture-review/server-pending.md`
  「採らない方針」). Do not report it as a finding. It becomes in-scope only
  when public release preparation starts. (If that pending entry is gone,
  the decision may have been reversed — check before assuming either way.)
- Client-side code (GDScript or client-side Rust crates) is not a trust
  boundary — the server must stay safe against a fully malicious client, so
  client-side validation counts for UX only, never as a mitigation.

## Process

### Step 0: Diff against the standing findings doc

Read `docs/architecture/security-review.md` (current state — entry points,
verified-clean list, open findings; if it doesn't exist yet, seed it from
[references/baseline.md](references/baseline.md)) and skim
`docs/architecture/security-review-completed.md` (append-only log of what
was already fixed, so you don't waste time re-discovering a resolved
finding). Then check what changed since the review date recorded in
`security-review.md`'s front matter:

```bash
git log --oneline --since=<last-review-date> -- crates/
```

Commits touching the wire protocol, command handlers, or persistence are
where new findings live. Spend effort proportional to the diff, but always
spot-check at least one previously-green item per category — regressions
happen in "safe" files too, and the doc's clean-list is only as good as its
last verification.

### Step 1: Enumerate the input surface — by discovery, not memory

Re-derive the set of entry points for untrusted bytes each run:

```bash
# Network accept / frame handling (WebSocket or any future transport)
grep -rln "accept_async\|WebSocketConfig\|TcpListener" crates/ --include='*.rs'
# Wire-format parsing of client input
grep -rln "ClientCommandJson\|client_command_from_json\|parse_client_command" crates/ --include='*.rs'
# Command dispatch into game logic
grep -rln "apply_client_command" crates/ --include='*.rs'
```

Compare the result against the entry-point table in
`docs/architecture/security-review.md`. **A new entry point that isn't in
the doc is itself a finding** (undocumented attack surface), independent of
whether its code is safe. Baseline-era layout, for orientation only:
network I/O lived in `dawn-actor` (`ws_server.rs`, `protocol/`), dispatch in
`dawn-sector`'s node modules.

The trust rule: anything a handler reads from a client command value is
untrusted; anything the server computed itself (registry lookups, constants,
tick counters) is trusted.

### Step 2: SQL (OWASP A03 — injection)

Discover the SQL surface — never assume it's still one file:

```bash
grep -rln "rusqlite\|sqlx" crates/ --include='*.rs'
```

Then inspect every `execute` / `prepare` / `query_row` / `query_map` call in
each hit:

- Values arrive via parameter binding (`params![]`, positional `?N`, named
  `:name`) — never `format!` or string concatenation building SQL text.
  `grep -n "format!" <file>` near SQL calls is the fast smell test.
- Table and column names are never derived from variable input. Mappings
  from client-facing identifiers to column names must be a closed `match`
  over a server-side enum, not a string passthrough.
- If SQL appears in a file the standing doc doesn't list, the SQL surface
  grew: review it fully and add it to the doc.

### Step 3: Access control (OWASP A01)

Discover the handlers that mutate player-owned state:

```bash
grep -rn "fn .*_owned" crates/dawn-sector/src/ --include='*.rs'
```

(The `_owned` suffix is the project convention for "player-initiated,
ownership-checked". If handlers stop following that convention, enumerate
them from the command-dispatch match arms found in Step 1 instead.)

Every handler must verify ownership **before** mutating state. The
established patterns, strongest first:

1. Resolving the ship from the player's own server-side state (e.g. an
   active-ship map) — the client never names the ship at all.
2. An explicit `owns_ship(player_id, ship_id)`-style check on a
   client-supplied ID.
3. Station operations additionally check station-usability/docked-state on
   **both** the player and the ship — a ship docked elsewhere must not be
   operable through the station the player is docked at.

Check every handler added or modified since the last review against this.
The failure mode to hunt: an early-return validation that covers one ID,
while a later code path (an event emit, a followup command) uses a
*different* client-supplied ID the validation never touched.

### Step 4: Data integrity (OWASP A08)

Clients send **intents and IDs, never authoritative values**. Scan the wire
command type definitions (found in Step 1) for fields that look like
quantities, prices, coordinates, damage, or ranges, and trace each to where
it's consumed. A violation looks like: a client-supplied number written into
state or — worse, because replay makes it permanent — into an emitted event,
without server-side recomputation. Costs must be server-side constants;
movement commands express targets, and the physics/tick systems own the
resulting state. (See the event-sourcing severity rule in
[references/owasp-map.md](references/owasp-map.md).)

**Numeric well-formedness, not just ownership.** A value can be the *right
kind* of thing (an authorized player moving their own ship) and still be a
*malformed* thing. Any client-supplied floating-point value that feeds
physics, geometry, or a shared simulation is a well-formedness hole if
nothing checks it's finite:

```bash
grep -rn ": f32\|: f64" <wire command type file>
```

For each hit, trace whether the value is used in arithmetic before any
`is_finite()`/`is_nan()`/range check. NaN/Infinity propagate through
addition, multiplication, and comparisons in ways that corrupt shared state
silently (no panic, no rejected command — just poisoned physics or a
navigation system that never resolves). This matters more here than in a
typical app because of event-sourcing replay: a NaN written into a
`Position` and then evented is now permanent history, not a transient bug.
This is a distinct check from "is this value in a plausible range" (also
worth asking, but secondary) — a value can be perfectly finite and still be
nonsensical (negative velocity magnitude); finiteness is the floor, not the
ceiling.

### Step 5: Resource exhaustion (OWASP A04)

- The client-to-server wire command types must stay scalar-only: no
  `Vec<T>`, no client-supplied count that drives a server-side loop or
  allocation. Check with `grep -n "Vec<" <wire command type file>` — any hit
  in a client-to-server type needs an explicit length cap at parse time and
  a note in the standing doc.
- Network accept path: message/frame size limits should be explicit
  configuration, not library defaults. Check the standing doc for the
  current state of this finding before re-reporting it.
- Per-message batching (e.g. line-splitting a frame into many commands):
  bounded by the frame cap is acceptable for the LAN prototype; note it,
  don't over-engineer rate limiting.
- **Every queue between "client sent it" and "server processed it" needs a
  bound.** A frame-size cap only bounds one message; it says nothing about
  how many messages a client can queue up before the server drains them.
  Discover the internal channels a connection feeds:
  ```bash
  grep -rn "unbounded_channel\|channel::<\|VecDeque::new\|mpsc::channel" crates/ --include='*.rs'
  ```
  For each hit that a client connection can push into (a per-session command
  queue, an inbound-message buffer), check: is it `bounded` with a size that
  causes backpressure or disconnection when full, or `unbounded` with no
  cap? An unbounded queue fed by network input is a memory-exhaustion vector
  even when every individual message is small and well-formed — the frame
  cap and the queue-depth cap are two different limits and both are needed.
  Also check whether one connection's queue can starve tick processing for
  *other* connections (a single greedy client consuming disproportionate
  server time per tick), which is a fairness failure even without exhausting
  memory.

### Step 6: Report and record

Report per category with the same discipline as `/architecture-review`:

```
### A03 SQL injection
OK — parameterized throughout (N call sites checked, files: ...)
```

or

```
### A01 access control
Finding — <file>:<line>: <what>
  Exploitable: yes/no + why (note mitigating checks found upstream)
  Severity: high / medium / low (LAN-prototype context)
```

Then update the two living docs, split by purpose the same way
`architecture-review/server.md`/`server-completed.md` are:

- **`docs/architecture/security-review.md`** (current state — entry points,
  verified-clean list, open findings): front-matter `date` → today, one-line
  summary of what changed. New findings get an ID (`SEC-N`, next number,
  never renumber), root cause, and a decision (Fix / Defer with trigger /
  Accept with reason) — the same issue discipline as `/architecture-review`.
  Keep the entry-point table and the verified-clean list current: they are
  what make the next diff-based review cheap. Record what was verified
  clean, not just problems.
- **`docs/architecture/security-review-completed.md`** (append-only audit
  log): when a finding is fixed, move its full write-up here dated, and
  leave nothing but the `security-review.md` finding entry deleted (not a
  strikethrough pointer — unlike `architecture-review`'s issue IDs, nothing
  in code comments cites a `SEC-N` ID, so there's no cross-reference to keep
  resolvable). Never delete or rewrite past entries in this file.

Every finding must say whether it is **actually exploitable by a malicious
LAN client today** or theoretical hardening. Severity is judged in the
LAN-prototype context; re-grade against public-internet assumptions only
when the release trigger fires.

## Reference files

- [references/owasp-map.md](references/owasp-map.md) — which OWASP Top 10
  categories apply to this codebase, which don't and why, plus the
  event-sourcing severity rule. Read when deciding whether an unusual
  finding fits a category, or when the user asks "what about OWASP
  category X?"
- [references/baseline.md](references/baseline.md) — the 2026-07-10 initial
  review's findings, frozen as history. Only used to seed
  `docs/architecture/security-review.md` when it doesn't exist; after that
  the living doc wins wherever they disagree.
