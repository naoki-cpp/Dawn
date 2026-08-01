//! Static node configuration loaded from a TOML file.
//!
//! Each physical node reads its own config (e.g. `config/node-0.toml`) that
//! describes which sector it owns, its own listen addresses, and how to reach
//! every peer node for Raft consensus and log replication.

use dawn_core::entity::MAX_GODOT_COMPATIBLE_NODE_ID;
use serde::Deserialize;
use std::net::SocketAddr;

/// Top-level node configuration.
#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    /// The unique node identifier (matches `RaftState::self_id`).
    pub node_id: u8,
    /// The Sector this node is authoritative for.
    pub sector_id: u8,
    /// WebSocket address the Godot clients connect to.
    pub ws_addr: SocketAddr,
    /// TCP address for incoming Raft RPC messages.
    pub raft_addr: SocketAddr,
    /// TCP address for incoming replication gossip frames.
    pub repl_addr: SocketAddr,
    /// How many NPC ships to spawn at startup.
    #[serde(default = "default_npc_ships")]
    pub npc_ships: usize,
    /// Population backstop per sector (ADR-0018).
    #[serde(default = "default_pop_cap")]
    pub pop_cap: usize,
    /// Every other node in the cluster.
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    /// Path to this node's hot event log (ADR-0017 two-tier log). Created on
    /// first run; reopened (and replayed past the snapshot, if any) on
    /// restart.
    #[serde(default = "default_event_log_path")]
    pub event_log_path: String,
    /// Path to the latest authoritative snapshot (ADR-0017 §5-C). Overwritten
    /// on every checkpoint.
    #[serde(default = "default_snapshot_path")]
    pub snapshot_path: String,
    /// Path to the append-only cold archive that compaction migrates
    /// snapshotted-and-confirmed segments into.
    #[serde(default = "default_cold_path")]
    pub cold_path: String,
    /// Logical ticks between checkpoints (snapshot + hot-log compaction).
    /// Driven by the logical tick, not wall-clock time (INV-005/FBD-003), so
    /// checkpointing stays deterministic and replay-stable.
    #[serde(default = "default_checkpoint_interval_ticks")]
    pub checkpoint_interval_ticks: u64,
    /// Path to the SQLite database backing Station inventory (ADR-0038).
    /// Independent of the event log / snapshot lifecycle -- durable on its
    /// own, and reopened as-is on restart.
    #[serde(default = "default_station_inventory_db_path")]
    pub station_inventory_db_path: String,
}

fn default_npc_ships() -> usize {
    20
}
fn default_pop_cap() -> usize {
    500
}
fn default_event_log_path() -> String {
    "data/events.log".to_string()
}
fn default_snapshot_path() -> String {
    "data/snapshot.bin".to_string()
}
fn default_cold_path() -> String {
    "data/cold.log".to_string()
}
fn default_checkpoint_interval_ticks() -> u64 {
    600
}
fn default_station_inventory_db_path() -> String {
    "data/station_inventory.sqlite3".to_string()
}

/// One peer's network endpoints.
#[derive(Debug, Deserialize, Clone)]
pub struct PeerConfig {
    pub node_id: u8,
    /// Raft TCP address of this peer.
    pub raft_addr: SocketAddr,
    /// Replication TCP address of this peer.
    pub repl_addr: SocketAddr,
    /// WebSocket address — used for the client Redirect message when a player
    /// jumps to this peer's sector (see `serve.rs`).
    pub ws_addr: SocketAddr,
}

/// Load a [`NodeConfig`] from a TOML file.
pub fn load(path: &str) -> anyhow::Result<NodeConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read config file '{}': {}", path, e))?;
    let config: NodeConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("cannot parse config file '{}': {}", path, e))?;
    validate_godot_node_ids(&config)?;
    Ok(config)
}

/// The current Godot client stores packed entity IDs in its signed 64-bit
/// `int`, while [`dawn_core::EntityId`] reserves the upper eight bits for the
/// node ID. Keep the deployment in the range that preserves the full 56-bit
/// counter without setting the sign bit. The domain/wire format still supports
/// all `u8` node IDs for future clients with a wider ID projection.
fn validate_godot_node_ids(config: &NodeConfig) -> anyhow::Result<()> {
    if config.node_id > MAX_GODOT_COMPATIBLE_NODE_ID {
        anyhow::bail!(
            "node_id {} exceeds the Godot-compatible maximum {}; use a wider client ID projection before assigning this node ID",
            config.node_id,
            MAX_GODOT_COMPATIBLE_NODE_ID
        );
    }
    for peer in &config.peers {
        if peer.node_id > MAX_GODOT_COMPATIBLE_NODE_ID {
            anyhow::bail!(
                "peer node_id {} exceeds the Godot-compatible maximum {}; use a wider client ID projection before assigning this node ID",
                peer.node_id,
                MAX_GODOT_COMPATIBLE_NODE_ID
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(node_id: u8, peer_node_id: u8) -> NodeConfig {
        NodeConfig {
            node_id,
            sector_id: 0,
            ws_addr: "127.0.0.1:7878".parse().unwrap(),
            raft_addr: "127.0.0.1:7879".parse().unwrap(),
            repl_addr: "127.0.0.1:7880".parse().unwrap(),
            npc_ships: 0,
            pop_cap: 1,
            peers: vec![PeerConfig {
                node_id: peer_node_id,
                raft_addr: "127.0.0.1:7881".parse().unwrap(),
                repl_addr: "127.0.0.1:7882".parse().unwrap(),
                ws_addr: "127.0.0.1:7883".parse().unwrap(),
            }],
            event_log_path: String::new(),
            snapshot_path: String::new(),
            cold_path: String::new(),
            checkpoint_interval_ticks: 1,
            station_inventory_db_path: String::new(),
        }
    }

    #[test]
    fn accepts_the_documented_godot_compatible_maximum() {
        assert!(validate_godot_node_ids(&config(
            MAX_GODOT_COMPATIBLE_NODE_ID,
            MAX_GODOT_COMPATIBLE_NODE_ID
        ))
        .is_ok());
    }

    #[test]
    fn rejects_local_or_peer_node_ids_that_set_the_sign_bit() {
        let first_incompatible = MAX_GODOT_COMPATIBLE_NODE_ID + 1;
        assert!(validate_godot_node_ids(&config(first_incompatible, 1)).is_err());
        assert!(validate_godot_node_ids(&config(1, first_incompatible)).is_err());
    }
}
