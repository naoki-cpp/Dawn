# Godot Client Testing (GdUnit4)

> Canonical detail behind the "Godot client tests" section of
> AI_DEVELOPMENT_GUIDE.md. The guide keeps only the policy summary and a link
> here (same rationale as ADR-0030 — setup steps are only needed when
> touching `client/`).

## Setup

`client/addons/` is `.gitignore`d (each developer installs addons locally
from the Godot editor's AssetLib), so **on first setup, search for "GdUnit4"
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
# Windows PowerShell:
scripts/setup-godot.ps1
scripts/setup-godot.sh --run-tests
scripts/setup-godot.ps1 -RunTests
```

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

> **Known compatibility issue (GdUnit4 v6.1.3 × Godot 4.6.x)**: GdUnit4
> v6.1.3 (the AssetLib release) does not handle Godot 4.6's breaking changes
> (removal of the `skip_cr` argument from `FileAccess.get_as_text()`, and
> removal of the `debug/gdscript/warnings/exclude_addons` setting; upstream
> issue GD-1004 — fixed on master but not in this tag), so CLI runs fail
> out of the box. Because `client/addons/` is `.gitignore`d (per-machine
> local install), **apply these two manual patches locally right after
> installing from AssetLib** (re-apply after any reinstall):
>   - `addons/gdUnit4/src/core/GdUnitFileAccess.gd:199`:
>     `file.get_as_text(true)` → `file.get_as_text()`
>   - `addons/gdUnit4/plugin.gd:17`: add the second argument `false`
>     (default value) to
>     `ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")`
> Once GdUnit4 ships a 4.6-compatible release, these patches become
> unnecessary.

## What is testable vs out of scope

Unlike the server side (Rust crates), **not all client code can be tested**.

```
Testable (pure functions/logic with no scene-tree dependency):
  - coordinate conversion, ray/distance math, computations over arrays and
    dictionaries
  - e.g. _server_to_godot_pos() / _ray_point_distance() / _spectral_color() /
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
     in docs/architecture/architecture-review-client.md
```

**When adding or extracting a new pure function into `main.gd` etc., include
its test in the same change.** Conversely, when changing scene-tree-dependent
logic, state in the PR description what was verified in the Godot editor in
place of a test (for AI sessions that cannot run the editor, state that fact
and the recommended manual verification steps).
