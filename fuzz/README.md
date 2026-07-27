# Dawn fuzz targets

These targets exercise Dawn's untrusted postcard WebSocket boundaries.

## Prerequisites

- a nightly Rust toolchain
- `cargo-fuzz` (`cargo install cargo-fuzz --locked`)
- a C++ compiler supported by libFuzzer

## Run locally

```bash
cargo fuzz run decode_client_message
cargo fuzz run decode_server_message
```

Successful decodes are re-encoded and decoded again. A panic, sanitizer finding,
or failure to round-trip is treated as a fuzzing failure.

GitHub Actions runs each target for 30 seconds when the harness or `dawn-wire`
changes, and for five minutes in the weekly scheduled run. Crash artifacts are
uploaded from `fuzz/artifacts/`.
