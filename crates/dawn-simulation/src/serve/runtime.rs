//! Serve runtime orchestration shared by clustered serve loops.

use super::{AoiDelivery, AOI_CELL_SIZE};
use crate::ws_server;
use dawn_consensus::RaftActorHandle;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::node::SimulationNode;
use dawn_sector::transit;
use dawn_wire::{InitialStateWire, ServerMessage};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

/// Run one clustered serve tick and deliver the resulting client frames.
///
/// The shared runtime tick owns simulation, replication-hook, Raft-clock, and
/// transient-output ordering. This adapter only performs jump ownership handoff,
/// AOI delivery, and scoped InitialState resend for players that changed Sector.
pub(crate) struct ClusterRuntimeTickContext<'a> {
    pub(crate) nodes: &'a mut [SimulationNode],
    pub(crate) rafts: &'a [RaftActorHandle],
    pub(crate) committed_rxs: &'a mut [mpsc::UnboundedReceiver<Vec<u8>>],
    pub(crate) sessions: &'a mut Vec<ws_server::PlayerSession>,
    pub(crate) player_sector: &'a mut HashMap<PlayerId, usize>,
    pub(crate) ship_player: &'a HashMap<ShipId, PlayerId>,
    pub(crate) aoi_delivery: &'a mut AoiDelivery,
}

pub(crate) fn run_cluster_runtime_tick(
    ctx: ClusterRuntimeTickContext<'_>,
    lock_commands: &[Vec<dawn_core::LockOnCommand>],
) -> Vec<transit::RuntimeTickOutput> {
    let tick_outputs: Vec<_> = (0..ctx.nodes.len())
        .map(|i| {
            transit::run_runtime_tick(
                &mut ctx.nodes[i],
                &ctx.rafts[i],
                &mut ctx.committed_rxs[i],
                &lock_commands[i],
                |_, _, _| {},
            )
        })
        .collect();

    let warp_arrivals_by_sector: Vec<Vec<ShipId>> = tick_outputs
        .iter()
        .map(|output| output.completed_warps.clone())
        .collect();
    let events_by_sector: Vec<Vec<DomainEvent>> = tick_outputs
        .iter()
        .map(|output| output.events.clone())
        .collect();

    let handoff = apply_jump_handoffs(
        ctx.nodes,
        ctx.player_sector,
        ctx.ship_player,
        &events_by_sector,
    );
    deliver_cluster_frames(
        ctx.nodes,
        ctx.sessions,
        ctx.player_sector,
        ctx.aoi_delivery,
        &events_by_sector,
        &warp_arrivals_by_sector,
        &handoff,
    );
    let failed_players = resend_jump_initial_state(ctx.nodes, ctx.sessions, &handoff);
    for player_id in failed_players {
        ctx.player_sector.remove(&player_id);
    }
    tick_outputs
}

struct JumpHandoff {
    jumped_players: Vec<(PlayerId, usize)>,
    own_events: HashMap<PlayerId, Vec<DomainEvent>>,
}

fn apply_jump_handoffs(
    nodes: &mut [SimulationNode],
    player_sector: &mut HashMap<PlayerId, usize>,
    ship_player: &HashMap<ShipId, PlayerId>,
    events_by_sector: &[Vec<DomainEvent>],
) -> JumpHandoff {
    let mut jumped_players: Vec<(PlayerId, usize)> = Vec::new();
    let mut own_events: HashMap<PlayerId, Vec<DomainEvent>> = HashMap::new();

    for sector_events in events_by_sector {
        for event in sector_events {
            match event {
                DomainEvent::JumpGateUsed(e) => {
                    if let Some(&player_id) = ship_player.get(&e.ship_id) {
                        let dest = e.to_sector.0 as usize;
                        nodes[dest].adopt_player_ship(e.ship_id, player_id);
                        player_sector.insert(player_id, dest);
                        jumped_players.push((player_id, dest));
                        own_events.entry(player_id).or_default().push(event.clone());
                        println!(
                            "  [Server] {player_id:?} ship #{} now owned by Sector {dest}",
                            e.ship_id.raw()
                        );
                    }
                }
                DomainEvent::StarSystemChanged(e) => {
                    if let Some(&player_id) = ship_player.get(&e.ship_id) {
                        own_events.entry(player_id).or_default().push(event.clone());
                    }
                }
                _ => {}
            }
        }
    }

    JumpHandoff {
        jumped_players,
        own_events,
    }
}

fn deliver_cluster_frames(
    nodes: &[SimulationNode],
    sessions: &mut Vec<ws_server::PlayerSession>,
    player_sector: &HashMap<PlayerId, usize>,
    aoi_delivery: &mut AoiDelivery,
    events_by_sector: &[Vec<DomainEvent>],
    warp_arrivals_by_sector: &[Vec<ShipId>],
    handoff: &JumpHandoff,
) {
    let jumped_ids: HashSet<PlayerId> = handoff
        .jumped_players
        .iter()
        .map(|(player_id, _)| *player_id)
        .collect();

    aoi_delivery.deliver_cluster_sectors(
        nodes,
        sessions,
        player_sector,
        events_by_sector,
        warp_arrivals_by_sector,
        &jumped_ids,
    );
}

trait JumpHandoffSession {
    fn player_id(&self) -> PlayerId;
    fn ship_id(&self) -> ShipId;
    fn send_events(&mut self, events: &[DomainEvent]);
    fn send_initial_state(&mut self, initial_state: InitialStateWire);
}

impl JumpHandoffSession for ws_server::PlayerSession {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn ship_id(&self) -> ShipId {
        self.ship_id
    }

    fn send_events(&mut self, events: &[DomainEvent]) {
        let _ = ws_server::PlayerSession::send_events(self, events);
    }

    fn send_initial_state(&mut self, initial_state: InitialStateWire) {
        let _ = self.send_message(&ServerMessage::InitialState(initial_state));
    }
}

fn resend_jump_initial_state<T: JumpHandoffSession>(
    nodes: &[SimulationNode],
    sessions: &mut Vec<T>,
    handoff: &JumpHandoff,
) -> Vec<PlayerId> {
    let destinations: HashMap<PlayerId, usize> = handoff.jumped_players.iter().copied().collect();
    let mut failed_players = Vec::new();

    sessions.retain_mut(|session| {
        let player_id = session.player_id();
        let Some(&dest) = destinations.get(&player_id) else {
            return true;
        };

        let initial_state = match nodes[dest]
            .build_initial_state_for_observer(session.ship_id(), AOI_CELL_SIZE)
        {
            Ok(initial_state) => initial_state,
            Err(error) => {
                eprintln!(
                    "[Server] post-transit handoff failed for {player_id:?} in Sector {dest}: {error}"
                );
                failed_players.push(player_id);
                return false;
            }
        };

        if let Some(events) = handoff.own_events.get(&player_id) {
            session.send_events(events);
        }
        session.send_initial_state(initial_state);
        true
    });

    failed_players
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, Velocity};
    use dawn_sector::ship_types::SHIP_TYPE_NPC_FRIGATE;

    struct FakeSession {
        player_id: PlayerId,
        ship_id: ShipId,
        initial_state_ship_ids: Vec<Vec<u64>>,
    }

    impl FakeSession {
        fn new(player_id: PlayerId, ship_id: ShipId) -> Self {
            Self {
                player_id,
                ship_id,
                initial_state_ship_ids: Vec::new(),
            }
        }
    }

    impl JumpHandoffSession for FakeSession {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn ship_id(&self) -> ShipId {
            self.ship_id
        }

        fn send_events(&mut self, _events: &[DomainEvent]) {}

        fn send_initial_state(&mut self, initial_state: InitialStateWire) {
            self.initial_state_ship_ids.push(
                initial_state
                    .ships
                    .into_iter()
                    .map(|ship| ship.ship_id)
                    .collect(),
            );
        }
    }

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        )
    }

    fn jump_handoff(player_id: PlayerId, dest: usize) -> JumpHandoff {
        JumpHandoff {
            jumped_players: vec![(player_id, dest)],
            own_events: HashMap::new(),
        }
    }

    #[test]
    fn post_transit_handoff_drops_a_session_with_a_missing_observer() {
        let player_id = PlayerId(1);
        let missing = ShipId::new(NodeId(9), 999);
        let nodes = vec![node()];
        let mut sessions = vec![FakeSession::new(player_id, missing)];

        let failed = resend_jump_initial_state(&nodes, &mut sessions, &jump_handoff(player_id, 0));

        assert_eq!(failed, vec![player_id]);
        assert!(sessions.is_empty());
    }

    #[test]
    fn post_transit_handoff_keeps_valid_observer_state_scoped() {
        let player_id = PlayerId(1);
        let mut destination = node();
        let observer =
            destination.spawn_ship(SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let far = destination.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(100_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let nodes = vec![destination];
        let mut sessions = vec![FakeSession::new(player_id, observer)];

        let failed = resend_jump_initial_state(&nodes, &mut sessions, &jump_handoff(player_id, 0));

        assert!(failed.is_empty());
        assert_eq!(sessions.len(), 1);
        let ids = &sessions[0].initial_state_ship_ids[0];
        assert!(ids.contains(&observer.raw()));
        assert!(!ids.contains(&far.raw()));
    }
}
