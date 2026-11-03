# Windows playtest releases

Distribution uses GitHub Releases only. No website or download server is required.

## Release procedure

1. Merge and verify CI on the commit that matches the playtest server.
2. Create and push a new tag, for example `playtest-2026.11.03.1`.
3. The Windows playtest release workflow builds a release GDExtension and exports
   an isolated client using the pinned Godot version. It creates a **draft
   prerelease** with the Windows ZIP and SHA256 checksum.
4. Download that ZIP on a clean Windows machine, extract it, launch Dawn.exe,
   and test connecting, movement, gate jump and reconnect against the server.
5. Check the bundled notices, then publish the draft. Share its Release URL,
   server URL and invite code (the code privately).

For local packaging, with Python 3.11+ and the pinned Rust toolchain on Windows:

```powershell
python scripts/package_playtest.py --version playtest-2026.11.03.1
```

The package is written to `client/export/` (ignored). Client resources come from
**HEAD**, not uncommitted edits; commit runtime changes before packaging. Dirty
Rust source/manifests are rejected so the DLL agrees with the recorded commit. The
first build downloads the pinned Godot editor and export templates and checks
their SHA512 sums. Local caches, tests, editor addons, credentials and local
endpoint overrides are not copied. Server URL and invite are entered at launch.

The workflow deliberately does not publish automatically: a build/export result
is not proof of successful GPU rendering or a real server connection. Use a new
version tag for fixes; do not replace already distributed binaries under a tag.
Unsigned builds are expected for this initial playtest; code signing and an
auto-updater are out of scope.
