//! Regenerate `docs/architecture/wire-protocol.schema.json` from
//! [`dawn_actor::protocol::event_json_schema`].
//!
//! Run with `cargo run -p dawn-actor --example gen_wire_schema` after
//! changing `EventJson` (or any type it references). The
//! `wire_schema_doc_is_up_to_date` test in `protocol.rs` fails the build if
//! this file is stale, so CI catches a forgotten regeneration.

use std::path::PathBuf;

fn main() {
    let schema = dawn_actor::protocol::event_json_schema();
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes");

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/architecture/wire-protocol.schema.json");
    std::fs::write(&out_path, format!("{json}\n")).expect("write schema file");

    println!("wrote {}", out_path.display());
}
