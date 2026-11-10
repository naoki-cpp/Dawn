# Godot Client Testing (GdUnit4)

> Canonical detail behind the "Godot client tests" section of
> AI_DEVELOPMENT_GUIDE.md. The guide keeps only the policy summary and a link
> here (same rationale as ADR-0030 — setup steps are only needed when
> touching `client/`).

## Setup

The editor and CI use Godot **4.7.2-stable**, pinned in `.godot-version`.
`dawn-client-gdext` retains the `api-4-6` API baseline and the extension's
`compatibility_minimum = "4.6"`: updating the runtime does not require exposing
new engine APIs. Rebuild the extension before testing the updated engine.

Python 3.10+ and Cargo are required. `scripts/godot_setup.py` owns preparation;
the PowerShell/Bash scripts are thin entry points. Set `PYTHON` to an explicit
interpreter path if Python is not available under its normal command name.
Preparation validates the pinned Godot version (SHA512 on download), installs
the pinned GdUnit4 addon when absent, validates/applies runner log patches,
builds `dawn-client-gdext`, then waits for `--headless --editor --import`.
The import log is `client/.godot-test-logs/import.log`; a nonzero exit or
script/import error in that log prevents readiness and test execution.

Editor addons under `client/addons/` are `.gitignore`d; the runtime
`lens_effects` addon is vendored with its license. Local setup and CI both
install the version in `.gdunit4-version` automatically. An existing addon with
a different version is rejected, not overwritten: move it aside and rerun.
The plugin enablement in `project.godot` is already committed.

Tests live under `client/test/` as `<target-file>_test.gd`
(e.g. `client/test/main_test.gd`).

**Getting the Godot binary**: the repository does not vendor Godot itself
(uv/pyenv style — `.godot-version` pins the version and each machine fetches
it individually).

```bash
bash scripts/setup-godot.sh        # prepare everything through build + import
# Windows PowerShell:
scripts/setup-godot.ps1
bash scripts/setup-godot.sh --run-tests
scripts/setup-godot.ps1 -RunTests
python3 -m unittest discover -s scripts/tests -p 'test_godot_setup.py'
```

`--print` / `-PrintPath` only resolves the binary path, with no preparation.
`--skip-gdunit` / `-SkipGdUnit` installs/validates Godot only and does not claim
test readiness; it cannot be combined with test execution.
`scripts/install-gdunit.sh` remains an addon-only compatibility entry point.

## GitHub Actions

The `godot` job in `.github/workflows/rust-ci.yml` tests the preparer, calls
the same setup entry point as local development, and separately runs
the full GdUnit4 suite under `xvfb`. GdUnit4's normal CLI runner opens a
window, so `xvfb-run` provides the display server on the Ubuntu runner; plain
`--headless` is intentionally not used.

The job installs Mesa's lavapipe software Vulkan driver and explicitly selects
its ICD using `VK_DRIVER_FILES`. A `vulkaninfo --summary` preflight under xvfb
checks the driver before running tests. The solar compositor pixel tests need
Forward+ / RenderingDevice; an automatic fallback to OpenGL Compatibility would
render no compute effect and is not a valid replacement for GPU test coverage.

## Running from the CLI

Run GdUnit4 with the fetched Godot binary (working directory: `client/`):

```bash
cd client
GODOT_BIN="$(bash ../scripts/setup-godot.sh --print)"
bash addons/gdUnit4/runtest.sh --godot_binary "$GODOT_BIN" -a test
```

On Windows, prefer the setup script so it also creates the Godot user log
directory and applies the pinned-version GdUnit4 compatibility patches:

```powershell
scripts/setup-godot.ps1 -RunTests
```

> **Pinned runner patches:** the shared preparer redirects both runner and
> copy-helper logs to `client/.godot-test-logs/`. Patches accept LF/CRLF and
> already-patched files, but reject unknown shapes before writing any file.
> The runner's invalid `--remote-debug tcp://127.0.0.1:0` argument is removed.
> GdUnit4 6.1.3 already fixes the older FileAccess/plugin/monitor issues;
> their obsolete compatibility patches have been removed. A version upgrade
> must update and test the patch expectations, not silently skip them.
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

## Godot 4.7.2 upgrade validation

Validated on Windows on 2026-10-04:

- The official editor archive passed SHA512 verification.
- The Windows setup script was verified through a fresh download and its
  `-RunTests` entry point (220 tests passed). The sh setup was checked with
  Git Bash on Windows; Linux/macOS execution remains covered by CI/platform
  testing rather than this local run.
- The rebuilt GDExtension loaded with API 4.6 and runtime 4.7.2.
- Rust client-core/GDExtension tests passed (120 tests).
- GdUnit4 6.1.3 passed all 220 tests in both headless and Vulkan Forward+
  runs (21 suites, no failures or skipped tests).
- The main scene ran for 120 frames under Vulkan Forward+ on Intel UHD
  Graphics and exited successfully without startup or shader errors.
- Six `!is_inside_tree()` diagnostics in `main_test.gd` also occur under
  4.6.3 with the same fixtures; they are pre-existing test diagnostics.

Connected visual playtesting is still required before declaring the upgrade
fully verified: compare normal flight/camera movement, warp departure/arrival
and celestial parallax, docking/undocking and HUD state, and star/planet/marker
visibility against 4.6.3. The automated runs do not establish frame-time or
image-quality improvements. HDR output and new material features are separate
follow-up work.

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

## Client connection lifecycle tests

`dawn-client-core::ConnectionLifecycle` owns phase, retry/deadline/log timing,
and Welcome gating. Pure Rust tests cover initial/reconnect/Redirect budgets,
repeated open/close without Welcome, cancellation, and invalid elapsed time.
`connection_test.gd` and `connection_dialog_test.gd` use the production connection
and lifecycle with `test/fixtures/connection_socket.gd` as the socket adapter;
they do not maintain a FakeConnection policy or modify lifecycle flags.
They verify transport effects, credential cleanup/pinning, and UI state.

## Fixed Lighting Review

Rendering ownership and approximation limits are documented in
[Celestial Rendering](../architecture/celestial-rendering.md); architectural
rationale remains in [ADR-0053](../adr/ADR-0053-celestial-presentation-boundary.md).

Open `client/scenes/lighting_study.tscn` and run that scene for an offline visual
comparison. It uses the production environment and ship scene with deterministic
rock fixtures; it does not connect to a server or add gameplay asteroids.

With the configured Godot executable, run:

```text
godot --path client res://scenes/lighting_study.tscn -- --capture
```

This waits for procedural textures and settled frames, writes
`.tmp/lighting-study.png`, and exits. Compare the same framing and exposure when
adjusting background, hull/engine materials, fill lighting, glare or light shafts.
`ship_material_test.gd` verifies that scene materials are actually bound, the
hull remains lit/non-emissive, and the engine retains HDR emission.

The stellar-source tests check projected-area flux and finite-disc variance.
Dim-source GPU readback checks the normalized lens PSF's absolute integrated
energy (before saturation), radius-squared scaling, and invariance under squeeze
and resolution changes. Projection tests cover FOV and unclamped source radius;
the bounded depth-occlusion footprint is tested independently. These checks do
not assert a particular number of white pixels for every stellar distance.

## WorldSession test boundary

The server-driven mutation surface is `ClientState::apply`: production
`ServerMessageOutcome::dispatch` converts decoded wire values to `ClientFact`
and applies them through `ClientState` to `WorldSessionState`, while pure Rust
tests call the same surface directly. GdUnit tests
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
