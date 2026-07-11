//! Regenerate the checked-in wire-protocol schema files from
//! [`dawn_actor::protocol::event_wire_json_schema`] (server -> client) and
//! [`dawn_actor::protocol::client_command_wire_json_schema`] (client -> server).
//!
//! Run with `cargo run -p dawn-actor --example gen_wire_schema` after
//! changing `EventWire` / `ClientCommandWire` (or any type either
//! references). The `wire_schema_doc_is_up_to_date` test in `protocol.rs`
//! fails the build if either file is stale, so CI catches a forgotten
//! regeneration.

use std::path::PathBuf;

fn write_schema(schema: &schemars::schema::RootSchema, relative_path: &str) {
    let json = serde_json::to_string_pretty(schema).expect("schema serializes");
    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    std::fs::write(&out_path, format!("{json}\n")).expect("write schema file");
    println!("wrote {}", out_path.display());
}

fn main() {
    write_schema(
        &dawn_actor::protocol::event_wire_json_schema(),
        "../../docs/architecture/wire-protocol.schema.json",
    );
    write_schema(
        &dawn_actor::protocol::client_command_wire_json_schema(),
        "../../docs/architecture/wire-protocol-commands.schema.json",
    );
}
