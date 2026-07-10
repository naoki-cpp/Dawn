# Third-Party Licenses

Dawn is currently a private, unpublished project (`publish = false` in
`Cargo.toml`), so nothing here is a distribution obligation yet. This file
exists so the one dependency with a non-permissive license doesn't get
rediscovered from scratch when the project is eventually released.

## MPL-2.0 — godot-rust (gdext)

`crates/dawn-client-gdext` depends on the `godot` crate (godot-rust /
gdext) — specifically `godot`, `godot-core`, `godot-ffi`, and
`godot-macros` — which are licensed under the
[Mozilla Public License 2.0](https://www.mozilla.org/en-US/MPL/2.0/).

Source: https://github.com/godot-rust/gdext

### What MPL-2.0 requires

MPL-2.0 is a **weak, file-level copyleft** license — it applies to the
licensed files themselves, not to any larger work that merely links against
or depends on them:

- Dawn's own code (including `dawn-client-gdext`, which only *uses* the
  `godot` crate's public API and does not modify its source) is **not**
  subject to MPL-2.0 and can stay under whatever license Dawn chooses.
- Obligations only trigger on **distribution**, and only for the
  MPL-licensed files:
  - If Dawn is ever distributed (even as a compiled binary), the source of
    the `godot`/`godot-core`/`godot-ffi`/`godot-macros` crates must remain
    reasonably available to recipients. Pointing to the upstream repo above
    is sufficient — Dawn does not need to bundle that source itself.
  - Distributed builds should credit the dependency and its license (this
    file, or an equivalent NOTICE, serves that purpose).
- If godot-rust's own source is ever **forked or patched** (not currently
  the case), the modified files specifically would need their source
  released under MPL-2.0. Sticking to `#[func]`/`#[class]`/etc. extension
  points instead of patching upstream avoids this entirely.

### Action items before a public release

- [ ] Confirm no godot-rust source has been forked/patched (still true as
      of ADR-0040)
- [ ] Include this file (or a generated equivalent, e.g. via
      `cargo deny list` / `cargo license`) in the distributed build
- [ ] Regenerate the full third-party notice covering *all* dependencies at
      that point — this file only calls out the one non-permissive license;
      `deny.toml`'s `[licenses] allow` list is the source of truth for
      what's permitted.
