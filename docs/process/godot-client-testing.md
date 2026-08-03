# Godot Client Testing (GdUnit4)

> Canonical detail behind the "Godot client tests" section of
> AI_DEVELOPMENT_GUIDE.md. The guide keeps only the policy summary and a link
> here (same rationale as ADR-0030 — setup steps are only needed when
> touching `client/`).

## Setup

`client/addons/` is `.gitignore`d. Local developers may install addons from
the Godot editor's AssetLib, while CI restores the pinned GdUnit4 release with
`scripts/install-gdunit.sh`. **On first local setup, search for "GdUnit4"
in the editor's AssetLib tab, install it, and enable the plugin in
`project.godot`** (`enabled=PackedStringArray("res://addons/gdUnit4/plugin.cfg")`
is already committed; only the addon body needs per-machine installation).

Tests live under `client/test/` as `<target-file>_test.gd`
(e.g. `client/test/main_test.gd`).

**Getting the Godot binary**: the repository does not vendor Godot itself
(uv/pyenv style — `.godot-version` pins the version and each machine fetches
it individually).

```bash
scripts/setup-godot.sh             # fetch the pinned version into .tools/godot/ with SHA512 verification
# Linux/CI:
bash scripts/install-gdunit.sh      # restore the GdUnit4 version from .gdunit4-version
# Windows PowerShell:
scripts/setup-godot.ps1
scripts/setup-godot.sh --run-tests
scripts/setup-godot.ps1 -RunTests
```

## GitHub Actions

The `godot` job in `.github/workflows/rust-ci.yml` builds
`dawn-client-gdext`, installs the pinned Godot and GdUnit4 versions, and runs
the full GdUnit4 suite under `xvfb`. GdUnit4's normal CLI runner opens a
window, so `xvfb-run` provides the display server on the Ubuntu runner; plain
`--headless` is intentionally not used.

## Running from the CLI

Run GdUnit4 with the fetched Godot binary (working directory: `client/`):

```bash
cd client
GODOT_BIN="$(../scripts/setup-godot.sh --print)"
bash addons/gdUnit4/runtest.sh --godot_binary "$GODOT_BIN" -a test
```

On Windows, prefer the setup script so it also creates the Godot user log
directory and applies the pinned-version GdUnit4 compatibility patches:

```powershell
scripts/setup-godot.ps1 -RunTests
```

> **Compatibility patches (GdUnit4 v6.1.3 × Godot 4.6.x)**: the setup
> scripts keep the following patches idempotent for local AssetLib installs
> and the CI copy of the addon (re-apply after any reinstall if running the
> test runner directly):
>   - `addons/gdUnit4/src/core/GdUnitFileAccess.gd:199`:
>     `file.get_as_text(true)` → `file.get_as_text()`
>   - `addons/gdUnit4/plugin.gd:17`: add the second argument `false`
>     (default value) to
>     `ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")`
>   - `addons/gdUnit4/src/monitor/GodotGdErrorMonitor.gd`: guard the
>     `FileAccess.open()` calls in `collect_full_logs()` and
>     `_collect_log_entries()` before calling `seek()`, and initialize `_eof` to
>     `0`. Godot 4.6 can start the monitor before the configured log file
>     exists, which otherwise causes `Cannot call method 'seek' on a null value`.
>     `scripts/setup-godot.ps1` and `scripts/setup-godot.sh` apply this patch
>     automatically and idempotently.
> **Known issue on display-less/headless sandboxes (e.g. AI agent
> environments with no GPU/window server)**: `runtest.sh`/`runtest.cmd`'s
> default invocation of `GdUnitCmdTool.gd` does **not** pass `--headless` --
> it expects a real window. `GdUnitCmdTool.gd` itself actively refuses to
> run under plain `--headless` (prints "Headless mode is not supported!" and
> exits) unless `--ignoreHeadlessMode` is also passed. On a machine with no
> display, the tool's attempt to open a window segfaults (SIGSEGV) instead
> of printing that message -- this looks identical to a real engine/addon
> crash and is easy to misdiagnose as GdExtension or addon corruption (ADR-0041
> lost significant time to exactly this before finding the real cause).
> If GdUnit4 crashes with a native SIGSEGV backtrace ("no debug info in
> PE/COFF executable" / similar) on every test file including ones that were
> passing before, check this first, before suspecting the addon install,
> `.godot` cache, or your own code changes. Bypass `runtest.sh`/`.cmd` and
> invoke Godot directly with both flags:
> ```bash
> cd client
> "$GODOT_BIN" --headless --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd -a test --ignoreHeadlessMode
> ```
> (`--headless` before `-s`, `--ignoreHeadlessMode` after the script path/args
> -- it's parsed by `GdUnitCmdTool.gd` itself, not the engine.) UI-interaction
> tests won't receive real `InputEvents` in this mode (same caveat the tool's
> own warning states), but this project's client tests are already restricted
> to scene-tree-free pure logic (see "What is testable vs out of scope"
> below), so this doesn't lose coverage here.

## What is testable vs out of scope

Unlike the server side (Rust crates), **not all client code can be tested**.

```
Testable (pure functions/logic with no scene-tree dependency):
  - coordinate conversion, ray/distance math, computations over arrays and
    dictionaries
  - e.g. WorldSpace.to_godot_components() / _ray_point_distance() / _spectral_color() /
    _compute_warp_snap_pos_core() (see client/test/main_test.gd)
  - instantiating a script with .new() without adding it to the scene tree
    never calls _ready(), so functions that avoid @onready variables are
    safe to test

Not testable / out of scope (left to visual checks in the Godot editor):
  - HUD construction/updates, input handling, marker (node) creation, the
    picking loop itself
  - anything depending on @onready scene-tree path references
  - WebSocket communication (the live connection part of connection.gd)
  -> matches the areas marked "needs Godot editor verification" as C-1/C-3
     in docs/architecture/architecture-review/client.md
```

**When adding or extracting a new pure function into `main.gd` etc., include
its test in the same change.** Conversely, when changing scene-tree-dependent
logic, state in the PR description what was verified in the Godot editor in
place of a test (for AI sessions that cannot run the editor, state that fact
and the recommended manual verification steps).

Typed client fixtures must not recreate wire JSON/Dictionary shapes. Prefer
debug-only typed record factories for focused UI tests and
`ServerMessageDecoder.test_outcome()` when testing the real binary inbound path.

## WorldSession test boundary

`WorldSession` has one server-driven mutation surface in both production and
tests: `ServerMessageOutcome::dispatch`, which converts decoded wire values to
`WorldSessionUpdate` before applying them to `WorldSessionState`. GdUnit tests
must not add or use public `WorldSession` setters for health, locks, ship
lifecycle, system, server ticks, or dock/loadout state.

Use the following split:

- GdUnit dispatches `ServerMessageDecoder.test_outcome()` fixtures when it must
  verify the binary inbound wiring, then reads the same `WorldSession`
  accessors used by production presentation code.
- Pure ordering, validation, and transition details are tested directly on
  `WorldSessionState` in `dawn-client-core` Rust tests.
- The only public mutable Godot operations retained on `WorldSession` are
  connection/session `reset()` and client-owned clock prediction through
  `advance_client_ticks()`.
