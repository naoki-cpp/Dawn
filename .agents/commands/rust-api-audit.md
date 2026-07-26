# /rust-api-audit — Audit changed pub items against the Rust API Guidelines

Run this before opening any PR that adds or changes a `pub` item in a Rust
crate (new type, new constructor, changed error enum, new crate). The Testing
Rules in AI_DEVELOPMENT_GUIDE.md make this audit mandatory; PRs #82 and #83
established the pattern.

Reference: https://rust-lang.github.io/api-guidelines/checklist.html

---

## Steps

### Step 1: Enumerate the changed public surface

```bash
git diff main...HEAD -- 'crates/**/*.rs' | grep -E '^\+.*\bpub\b' | grep -v 'pub(crate)' | grep -v 'pub(super)'
```

Also run, when the branch may break existing callers:

```bash
cargo semver-checks check-release --baseline-rev main
```

List every new/changed `pub` type, function, trait, and enum variant. If the
list is empty, report "no public surface change" and stop.

### Step 2: Check the focus categories

These four categories are the ones that actually regress in this codebase.
Check every item from Step 1 against each:

**C-DEBUG — public types implement `Debug`**
- Every new pub struct/enum derives or implements `Debug`.
- Spot-check: `rg 'pub struct|pub enum' <changed files>` and confirm a
  `#[derive(...Debug...)]` (or manual impl) on each.

**C-VALIDATE — constructors reject invariant-breaking input**
- `new()` / `from_*()` / builder `build()` on the changed types: can they be
  handed input that violates a domain invariant (negative HP, zero-length
  velocity where forbidden, duplicate IDs — FBD-005)?
- If yes, they must return `Result`/`Option` or clamp with documented
  semantics — not silently construct a broken value.

**C-GOOD-ERR — error variants carry context**
- New/changed error enums: does each variant carry enough to diagnose without
  a debugger (the offending ID, the limit that was exceeded, the state name)?
- Unit variants like `InvalidCommand` with no payload are the smell.

**C-CRATE-DOC / C-EXAMPLE — new crates get crate-level docs with an example**
- A new crate needs `//!` crate docs in `lib.rs` including at least one
  runnable example (doctest).
- A significantly reworked module deserves a module-level `//!` summary.

### Step 3: Secondary sweep (cheap, catch-all)

Quick pass over the remaining checklist categories most likely to matter:
- **C-CONV**: method names follow `as_`/`to_`/`into_` conventions.
- **C-GETTER**: getters are `field()`, not `get_field()`.
- **C-COMMON-TRAITS**: would `Clone`, `Copy`, `PartialEq`, `Default` be
  natural on the new types? Derive the ones that are obviously right; do not
  force the rest.
- **C-STRUCT-PRIVATE**: new structs default to private fields with accessors
  unless they are plain data carriers (the HullComp deepening in #84 is the
  precedent for retrofitting privacy).

### Step 4: Fix or record

- Fix violations directly on the branch — most are one-line derives or a
  `Result` return.
- Anything **deliberately skipped** goes in the PR description, named and
  justified (e.g. "C-EXAMPLE skipped: crate is an internal binary, no
  library consumers"). Silent omission is the failure mode this skill
  exists to prevent.

### Step 5: Verify

```bash
cargo fmt --all -- --check
cargo test --workspace   # doctests from C-EXAMPLE run here
```

## Report format

```
## Rust API audit
Public surface: N items changed (list)
C-DEBUG:    OK / fixed (list) / skipped (why)
C-VALIDATE: OK / fixed / skipped (why)
C-GOOD-ERR: OK / fixed / skipped (why)
C-CRATE-DOC/C-EXAMPLE: OK / fixed / n.a. (no new crate)
Secondary sweep: <anything touched>
Deliberate skips for the PR description: <lines to paste>
```
