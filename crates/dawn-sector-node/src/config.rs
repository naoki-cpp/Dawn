//! Static node configuration loaded from a TOML file.
//!
//! Each physical node reads its own config (e.g. `config/node-0.toml`) that
//! describes which sector it owns, its own listen addresses, and how to reach
//! every peer node for Raft consensus and log replication.

use serde::Deserialize;
use std::net::SocketAddr;

/// Top-level node configuration.
#[derive(Deserialize)]
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
}

fn default_npc_ships() -> usize {
    20
}
fn default_pop_cap() -> usize {
    500
}

/// One peer's network endpoints.
#[derive(Deserialize, Clone)]
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
    toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("cannot parse config file '{}': {}", path, e))
}
