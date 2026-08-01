//! Area-of-Interest session adapters for serve loops.
//!
//! Spatial-index construction, observer visible-set resolution, Enter/Leave
//! changes, event filtering, and ordered frame delivery all live in
//! `dawn_sector::aoi_frame::AoiFrame`. This module keeps only the runtime-owned
//! session loop, Sector routing, and the orphan-rule sink adapter for
//! `ws_server::PlayerSession`.

use crate::ws_server;
use dawn_core::{DomainEvent, PlayerId, ShipId};
use dawn_sector::aoi::{AoiSink, Observer};
use dawn_sector::aoi_frame::AoiFrame;
use dawn_sector::node::SimulationNode;
use dawn_wire::ServerMessage;
use std::collections::{HashMap, HashSet};

pub(crate) struct AoiDelivery {
    cell_size: f64,
    frames: Vec<AoiFrame>,
}

impl AoiDelivery {
    pub(crate) fn new(cell_size: f64) -> Self {
        Self {
            cell_size,
            frames: vec![AoiFrame::new(cell_size)],
        }
    }

    pub(crate) fn seed_single_player(
        &mut self,
        node: &SimulationNode,
        player_id: PlayerId,
        ship_id: ShipId,
    ) {
        self.frames[0].seed_observer(node, Observer { player_id, ship_id });
    }

    pub(crate) fn seed_cluster_player(
        &mut self,
        nodes: &[SimulationNode],
        sector: usize,
        player_id: PlayerId,
        ship_id: ShipId,
    ) {
        self.ensure_frame_count(nodes.len());
        self.frames[sector].seed_observer(&nodes[sector], Observer { player_id, ship_id });
    }

    pub(crate) fn deliver_single_sector(
        &mut self,
        node: &SimulationNode,
        sessions: &mut Vec<ws_server::PlayerSession>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) {
        deliver_single_sessions(
            &mut self.frames[0],
            node,
            sessions,
            new_events,
            warp_arrivals,
        );
    }

    pub(crate) fn deliver_cluster_sectors(
        &mut self,
        nodes: &[SimulationNode],
        sessions: &mut Vec<ws_server::PlayerSession>,
        player_sector: &HashMap<PlayerId, usize>,
        new_events_by_sector: &[Vec<DomainEvent>],
        warp_arrivals_by_sector: &[Vec<ShipId>],
        reseed_players: &HashSet<PlayerId>,
    ) {
        self.ensure_frame_count(nodes.len());
        deliver_cluster_sessions(
            &mut self.frames,
            nodes,
            sessions,
            player_sector,
            new_events_by_sector,
            warp_arrivals_by_sector,
            reseed_players,
        );
    }

    fn ensure_frame_count(&mut self, count: usize) {
        let cell_size = self.cell_size;
        self.frames.resize_with(count, || AoiFrame::new(cell_size));
    }
}

trait RuntimeAoiSession {
    fn player_id(&self) -> PlayerId;
    fn ship_id(&self) -> ShipId;

    fn deliver(
        &mut self,
        frame: &mut AoiFrame,
        node: &SimulationNode,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool;
}

fn deliver_single_sessions<T: RuntimeAoiSession>(
    frame: &mut AoiFrame,
    node: &SimulationNode,
    sessions: &mut Vec<T>,
    new_events: &[DomainEvent],
    warp_arrivals: &[ShipId],
) {
    frame.rebuild(node);
    sessions.retain_mut(|session| session.deliver(frame, node, new_events, warp_arrivals));

    let live: HashSet<PlayerId> = sessions.iter().map(RuntimeAoiSession::player_id).collect();
    frame.retain_players(|player_id| live.contains(&player_id));
}

fn deliver_cluster_sessions<T: RuntimeAoiSession>(
    frames: &mut [AoiFrame],
    nodes: &[SimulationNode],
    sessions: &mut Vec<T>,
    player_sector: &HashMap<PlayerId, usize>,
    new_events_by_sector: &[Vec<DomainEvent>],
    warp_arrivals_by_sector: &[Vec<ShipId>],
    reseed_players: &HashSet<PlayerId>,
) {
    for (frame, node) in frames.iter_mut().zip(nodes) {
        frame.rebuild(node);
    }

    sessions.retain_mut(|session| {
        let player_id = session.player_id();
        let sector = *player_sector.get(&player_id).unwrap_or(&0);
        let observer = Observer {
            player_id,
            ship_id: session.ship_id(),
        };

        if reseed_players.contains(&player_id) {
            frames[sector].seed_observer_from_index(&nodes[sector], observer);
            return true;
        }

        session.deliver(
            &mut frames[sector],
            &nodes[sector],
            &new_events_by_sector[sector],
            &warp_arrivals_by_sector[sector],
        )
    });

    let live: HashSet<PlayerId> = sessions.iter().map(RuntimeAoiSession::player_id).collect();
    for (sector, frame) in frames.iter_mut().enumerate() {
        frame.retain_players(|player_id| {
            live.contains(&player_id) && player_sector.get(&player_id) == Some(&sector)
        });
    }
}

impl RuntimeAoiSession for ws_server::PlayerSession {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn ship_id(&self) -> ShipId {
        self.ship_id
    }

    fn deliver(
        &mut self,
        frame: &mut AoiFrame,
        node: &SimulationNode,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
    ) -> bool {
        let observer = Observer {
            player_id: self.player_id,
            ship_id: self.ship_id,
        };
        let mut sink = SessionSink(self);
        frame.deliver_observer(&mut sink, node, observer, new_events, warp_arrivals)
    }
}

/// Adapts a `ws_server::PlayerSession` to `AoiSink` (orphan-rule workaround:
/// the concrete session type and the trait live in different crates).
struct SessionSink<'a>(&'a mut ws_server::PlayerSession);

impl AoiSink for SessionSink<'_> {
    fn send_events(&mut self, events: &[DomainEvent]) -> bool {
        self.0.send_events(events)
    }

    fn send_message(&mut self, msg: &ServerMessage) -> bool {
        self.0.send_message(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, Velocity};
    use dawn_sector::ship_types::SHIP_TYPE_NPC_FRIGATE;

    const CELL_SIZE: f64 = 100.0;
    const PLAYER: PlayerId = PlayerId(1);

    #[derive(Debug, PartialEq, Eq)]
    enum Sent {
        Enter(u64),
        Leave(u64),
        Events(usize),
    }

    struct FakeSession {
        player_id: PlayerId,
        ship_id: ShipId,
        sent: Vec<Sent>,
    }

    impl FakeSession {
        fn new(ship_id: ShipId) -> Self {
            Self {
                player_id: PLAYER,
                ship_id,
                sent: Vec::new(),
            }
        }
    }

    impl RuntimeAoiSession for FakeSession {
        fn player_id(&self) -> PlayerId {
            self.player_id
        }

        fn ship_id(&self) -> ShipId {
            self.ship_id
        }

        fn deliver(
            &mut self,
            frame: &mut AoiFrame,
            node: &SimulationNode,
            new_events: &[DomainEvent],
            warp_arrivals: &[ShipId],
        ) -> bool {
            let observer = Observer {
                player_id: self.player_id,
                ship_id: self.ship_id,
            };
            frame.deliver_observer(self, node, observer, new_events, warp_arrivals)
        }
    }

    impl AoiSink for FakeSession {
        fn send_events(&mut self, events: &[DomainEvent]) -> bool {
            self.sent.push(Sent::Events(events.len()));
            true
        }

        fn send_message(&mut self, message: &ServerMessage) -> bool {
            match message {
                ServerMessage::AoiEnter(ship) => self.sent.push(Sent::Enter(ship.ship_id)),
                ServerMessage::AoiLeave { ship_id } => self.sent.push(Sent::Leave(*ship_id)),
                _ => {}
            }
            true
        }
    }

    fn empty_node(sector_id: SectorId) -> SimulationNode {
        SimulationNode::new(
            NodeId(99),
            sector_id,
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        )
    }

    fn initial_node(sector_id: SectorId) -> (SimulationNode, ShipId, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(7),
            sector_id,
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        );
        let own = node.spawn_ship(SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let leaving = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        (node, own, leaving)
    }

    fn next_node(sector_id: SectorId) -> (SimulationNode, ShipId, ShipId, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(7),
            sector_id,
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        );
        let own = node.spawn_ship(SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let leaving = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let entering = node.spawn_ship(
            SHIP_TYPE_NPC_FRIGATE,
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        (node, own, leaving, entering)
    }

    #[test]
    fn single_and_cluster_runtime_adapters_emit_the_same_ordered_frame() {
        let (single_initial, single_own, single_leaving) = initial_node(SectorId(0));
        let (single_next, next_own, next_leaving, single_entering) = next_node(SectorId(0));
        assert_eq!(single_own, next_own);
        assert_eq!(single_leaving, next_leaving);

        let mut single = AoiDelivery::new(CELL_SIZE);
        single.seed_single_player(&single_initial, PLAYER, single_own);
        let mut single_sessions = vec![FakeSession::new(single_own)];
        deliver_single_sessions(
            &mut single.frames[0],
            &single_next,
            &mut single_sessions,
            &[],
            &[],
        );

        let (cluster_initial, cluster_own, cluster_leaving) = initial_node(SectorId(1));
        let (cluster_next, cluster_next_own, cluster_next_leaving, cluster_entering) =
            next_node(SectorId(1));
        assert_eq!(cluster_own, cluster_next_own);
        assert_eq!(cluster_leaving, cluster_next_leaving);

        let initial_nodes = vec![empty_node(SectorId(0)), cluster_initial];
        let next_nodes = vec![empty_node(SectorId(0)), cluster_next];
        let mut cluster = AoiDelivery::new(CELL_SIZE);
        cluster.seed_cluster_player(&initial_nodes, 1, PLAYER, cluster_own);
        let mut cluster_sessions = vec![FakeSession::new(cluster_own)];
        let player_sector = HashMap::from([(PLAYER, 1)]);
        let empty_events = vec![Vec::new(), Vec::new()];
        let empty_warps = vec![Vec::new(), Vec::new()];
        deliver_cluster_sessions(
            &mut cluster.frames,
            &next_nodes,
            &mut cluster_sessions,
            &player_sector,
            &empty_events,
            &empty_warps,
            &HashSet::new(),
        );

        let expected = vec![
            Sent::Enter(single_entering.raw()),
            Sent::Leave(single_leaving.raw()),
            Sent::Events(0),
        ];
        assert_eq!(single_entering, cluster_entering);
        assert_eq!(single_sessions[0].sent, expected);
        assert_eq!(cluster_sessions[0].sent, single_sessions[0].sent);
    }

    #[test]
    fn cluster_handoff_reseeds_the_destination_frame_before_delivery() {
        let (source, own, _) = initial_node(SectorId(0));
        let (_, destination_own, _, _) = next_node(SectorId(1));
        assert_eq!(own, destination_own);

        let initial_nodes = vec![source, empty_node(SectorId(1))];
        let (destination, _, _, _) = next_node(SectorId(1));
        let destination_nodes = vec![empty_node(SectorId(0)), destination];
        let mut delivery = AoiDelivery::new(CELL_SIZE);
        delivery.seed_cluster_player(&initial_nodes, 0, PLAYER, own);

        let mut sessions = vec![FakeSession::new(own)];
        let player_sector = HashMap::from([(PLAYER, 1)]);
        let empty_events = vec![Vec::new(), Vec::new()];
        let empty_warps = vec![Vec::new(), Vec::new()];
        let reseed = HashSet::from([PLAYER]);
        deliver_cluster_sessions(
            &mut delivery.frames,
            &destination_nodes,
            &mut sessions,
            &player_sector,
            &empty_events,
            &empty_warps,
            &reseed,
        );
        assert!(sessions[0].sent.is_empty());

        deliver_cluster_sessions(
            &mut delivery.frames,
            &destination_nodes,
            &mut sessions,
            &player_sector,
            &empty_events,
            &empty_warps,
            &HashSet::new(),
        );
        assert_eq!(sessions[0].sent, vec![Sent::Events(0)]);
    }
}
