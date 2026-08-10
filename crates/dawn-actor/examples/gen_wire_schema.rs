//! Regenerate the checked-in wire-protocol schema files from
//! [`dawn_protocol::server_fact_json_schema`] (server -> client),
//! [`dawn_protocol::client_request_json_schema`] (Sector client ->
//! server), and [`dawn_protocol::market_command_wire_json_schema`]
//! (Market client -> server).
//!
//! Run with `cargo run -p dawn-actor --example gen_wire_schema` after
//! changing any of those types (or any type they reference). The
//! `wire_schema_doc_is_up_to_date` test in `dawn-protocol/src/lib.rs` fails the
//! build if a file is stale, so CI catches a forgotten regeneration.

use std::path::PathBuf;

fn write_schema(schema: &schemars::Schema, relative_path: &str) {
    let json = serde_json::to_string_pretty(schema).expect("schema serializes");
    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    std::fs::write(&out_path, format!("{json}\n")).expect("write schema file");
    println!("wrote {}", out_path.display());
}

fn main() {
    write_schema(
        &dawn_protocol::server_fact_json_schema(),
        "../../docs/architecture/wire-protocol.schema.json",
    );
    write_schema(
        &dawn_protocol::client_request_json_schema(),
        "../../docs/architecture/wire-protocol-commands.schema.json",
    );
    write_schema(
        &dawn_protocol::market_command_wire_json_schema(),
        "../../docs/architecture/wire-protocol-market.schema.json",
    );
}
