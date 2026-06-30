---
scope    : Git commit message format for all contributors and AI agents
audience : Human Developer / AI Agent
update   : Only with explicit human approval
---

# Commit Convention

All commits to this repository **must** follow this convention.
AI agents must apply it without exception.

---

## Format

```
<type>(<scope>): <description>

[body]

```

### type

| Type | When to use |
|---|---|
| `feat` | New feature or user-visible behaviour |
| `fix` | Bug fix |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `docs` | Documentation only (`.md`, comments, ADR) |
| `test` | Adding or fixing tests, no production code change |
| `perf` | Performance improvement |
| `chore` | Build, deps, CI, tooling — no production code change |
| `data` | Changes to `data/*.toml` balance files only |

Use exactly one type.  Do not invent new types.

### scope

Identifies the primary area changed.  Choose the most specific scope that applies.

| Scope | Covers |
|---|---|
| `dawn-core` | `crates/dawn-core/` |
| `dawn-ecs` | `crates/dawn-ecs/` |
| `dawn-event-store` | `crates/dawn-event-store/` |
| `dawn-actor` | `crates/dawn-actor/` |
| `dawn-simulation` | `crates/dawn-simulation/` |
| `godot` | `client/` (GDScript, scenes, assets) |
| `docs` | `docs/` |
| `data` | `data/*.toml` |
| `workspace` | `Cargo.toml`, `.github/`, repo-wide changes |

Scope is **optional** when the change touches multiple areas equally and no
single scope dominates.

### description

- Written in English, imperative mood ("add", "fix", "remove" — not "added" or "adds")
- Starts with a lowercase letter
- No period at the end
- The entire first line (type + scope + description) must be **≤ 72 characters**

---

## Body

- Separated from the description by a blank line
- Wrap at **72 characters** per line
- Explain **why**, not what — the diff already shows what changed
- Reference the relevant INV-* or ADR-* when applicable

---

## Examples

```
feat(dawn-ecs): add CapacitorSystem with cycle-based cap drain

Active modules now consume cap once per cycle rather than every tick.
Introduces FittedSlot.cycle_remaining to track countdown state.
Cap shortage forces module OFF and emits ModuleDeactivated.
```

```
fix(dawn-simulation): initialize CapacitorComp on player ship spawn

Player ships were missing CapacitorComp after spawn_player_ship_at(),
causing CapacitorSystem to skip them silently each tick.
```

```
refactor(dawn-core): use centered() for SectorBounds default

Replaces cube(DEFAULT_SIZE) with centered(DEFAULT_HALF) so the spawn
origin (0,0,0) sits in the middle of the playfield rather than at a
corner surrounded by walls.
```

```
docs(adr): update ADR-0006 checklist to reflect Phase 6 completion

All Fitting, Combat, Lock-on, and Capacitor items marked done.
Test count updated to 154.
```

```
data: increase Small Railgun cap cost from 60 to 80 GJ per cycle

Playtesting shows gun-only frigate has too much cap headroom.
Raising cost to 80 makes gun+AB a meaningful trade-off.
```

---

## Anti-patterns (rejected)

```
# Too vague
fix: bug fix

# Japanese subject
feat: キャパシタ実装

# Missing scope when the change is clearly scoped
feat: add CapacitorSystem

# Past tense
feat(dawn-ecs): added CapacitorSystem

# Period at end
fix(godot): correct cap bar percentage.

# First line over 72 characters
feat(dawn-simulation): implement client-side capacitor simulation with cycle tracking in Godot HUD
```

---

## Language policy

All commit messages are written in **English only**.
Japanese may appear in code comments only when the file already uses Japanese
comments; new files must use English comments throughout.
