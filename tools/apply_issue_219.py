from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}")
    file.write_text(text.replace(old, new, 1))


replace_once(
    "crates/dawn-sector/src/transit.rs",
    '''pub fn run_runtime_tick<S, F>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_appended: F,
) -> RuntimeTickOutput
where
    S: EventStore,
    F: FnOnce(&mut SimulationNode<S>, &crate::node::TickResult),
{
    let events_before = node.total_event_count() as u64;
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    after_events_appended(node, &result);
    raft.tick();
    let pending_auto_jumps = node.drain_pending_auto_jumps();
    let completed_warps = node.drain_completed_warps();
    let events = node
        .event_store()
        .iter_from(events_before)
        .map(|record| record.event.clone())
        .collect();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}
''',
    '''/// Execute the authoritative server frame pipeline.
///
/// Ordering is deliberately centralized here for every runtime adapter:
/// committed Raft entries -> simulation Tick -> Event collection ->
/// replication hook -> Raft clock advancement -> auto-jump proposal ->
/// transient warp-output drain.
pub fn run_runtime_tick<S, F>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
    after_events_collected: F,
) -> RuntimeTickOutput
where
    S: EventStore,
    F: FnOnce(&mut SimulationNode<S>, &crate::node::TickResult, &[DomainEvent]),
{
    let events_before = node.total_event_count() as u64;
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    let events: Vec<_> = node
        .event_store()
        .iter_from(events_before)
        .map(|record| record.event.clone())
        .collect();

    // Replication must observe the newly appended Event tail before the
    // consensus clock advances. All runtime paths supply their publisher here.
    after_events_collected(node, &result, &events);
    raft.tick();

    // Auto-jump is a simulation transient, not adapter-owned Tick ordering.
    // Drain and propose it here so actor, clustered serve, and production Node
    // paths all complete the same-frame handoff.
    let pending_auto_jumps = node.drain_pending_auto_jumps();
    for &(ship_id, gate_id) in &pending_auto_jumps {
        let _ = propose_auto_jump(node, raft, ship_id, gate_id);
    }
    let completed_warps = node.drain_completed_warps();

    RuntimeTickOutput {
        tick_result: result,
        events,
        pending_auto_jumps,
        completed_warps,
    }
}
''',
)

replace_once(
    "crates/dawn-sector/src/transit.rs",
    '''pub fn step_cluster_node<S: EventStore>(
    node: &mut SimulationNode<S>,
    raft: &RaftActorHandle,
    committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    lock_commands: &[dawn_core::LockOnCommand],
) -> crate::node::TickResult {
    apply_committed_raft_entries(node, raft, committed_rx);
    let result = node.tick_with_lock_commands(lock_commands);
    raft.tick();
    result
}

''',
    "",
)

replace_once(
    "crates/dawn-simulation/src/serve/runtime.rs",
    '''                |_, _| {},
''',
    '''                |_, _, _| {},
''',
)
replace_once(
    "crates/dawn-simulation/src/serve/runtime.rs",
    '''    propose_auto_jumps(ctx.nodes, ctx.rafts, &tick_outputs);

''',
    "",
)
replace_once(
    "crates/dawn-simulation/src/serve/runtime.rs",
    '''fn propose_auto_jumps(
    nodes: &mut [SimulationNode],
    rafts: &[RaftActorHandle],
    tick_outputs: &[transit::RuntimeTickOutput],
) {
    for (i, output) in tick_outputs.iter().enumerate() {
        for (ship_id, gate_id) in &output.pending_auto_jumps {
            if let Some(to) =
                transit::propose_auto_jump(&mut nodes[i], &rafts[i], *ship_id, *gate_id)
            {
                println!(
                    "  [Server] Auto-jump proposed: ship #{} gate #{} (S{} -> S{})",
                    ship_id.raw(),
                    gate_id.0,
                    i,
                    to.0
                );
            }
        }
    }
}

''',
    "",
)

replace_once(
    "crates/dawn-simulation/src/sector_simulator_actor.rs",
    '''                        |node, _| {
                            replication.publish_new_events(node.sector_id(), node.event_store());
                        },
''',
    '''                        |node, _, _| {
                            replication.publish_new_events(node.sector_id(), node.event_store());
                        },
''',
)

replace_once(
    "crates/dawn-sector-node/src/runtime.rs",
    '''    pub(crate) fn run_frame<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let event_cursor = node.total_event_count() as u64;
        let (lock_commands, pending_jumps) = self.collect_player_commands(node);

        self.propose_player_jumps(node, raft, pending_jumps);
        self.propose_auto_jumps(node, raft);

        transit::step_cluster_node(node, raft, committed_rx, &lock_commands);

        let new_events = self.collect_new_events(node, event_cursor);
        self.outbound_replication
            .publish_new_events(self.sector_id, node.event_store());

        let jumped_ships = self.jumped_ships(&new_events);
        self.deliver_frames(node, &new_events, &jumped_ships);
    }
''',
    '''    pub(crate) fn run_frame<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
        committed_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let (lock_commands, pending_jumps) = self.collect_player_commands(node);
        self.propose_player_jumps(node, raft, pending_jumps);

        let sector_id = self.sector_id;
        let outbound_replication = &mut self.outbound_replication;
        let output = transit::run_runtime_tick(
            node,
            raft,
            committed_rx,
            &lock_commands,
            |node, _, _| {
                outbound_replication.publish_new_events(sector_id, node.event_store());
            },
        );

        self.log_auto_jumps(&output.pending_auto_jumps);
        let jumped_ships = self.jumped_ships(&output.events);
        self.deliver_frames(
            node,
            &output.events,
            &output.completed_warps,
            &jumped_ships,
        );
    }
''',
)

replace_once(
    "crates/dawn-sector-node/src/runtime.rs",
    '''    fn propose_auto_jumps<S: EventStore>(
        &self,
        node: &mut SimulationNode<S>,
        raft: &RaftActorHandle,
    ) {
        for (ship_id, gate_id) in node.drain_pending_auto_jumps() {
            if let Some(to) = transit::propose_auto_jump(node, raft, ship_id, gate_id) {
                println!(
                    "[Node] Auto-jump proposed: ship #{} gate #{} (-> S{})",
                    ship_id.raw(),
                    gate_id.0,
                    to.0
                );
            }
        }
    }

    fn collect_new_events<S: EventStore>(
        &self,
        node: &SimulationNode<S>,
        event_cursor: u64,
    ) -> Vec<DomainEvent> {
        node.event_store()
            .iter_from(event_cursor)
            .map(|r| r.event.clone())
            .collect()
    }

''',
    '''    fn log_auto_jumps(&self, auto_jumps: &[(ShipId, dawn_core::JumpGateId)]) {
        for (ship_id, gate_id) in auto_jumps {
            println!(
                "[Node] Auto-jump proposed: ship #{} gate #{}",
                ship_id.raw(),
                gate_id.0
            );
        }
    }

''',
)

replace_once(
    "crates/dawn-sector-node/src/runtime.rs",
    '''    fn deliver_frames<S: EventStore>(
        &mut self,
        node: &mut SimulationNode<S>,
        new_events: &[DomainEvent],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        let grid = aoi::CellGrid::build(self.aoi_cell_size, node.ship_absolute_positions());
        let warp_arrivals = node.drain_completed_warps();
        let aoi_delivery = &mut self.aoi_delivery;
''',
    '''    fn deliver_frames<S: EventStore>(
        &mut self,
        node: &SimulationNode<S>,
        new_events: &[DomainEvent],
        warp_arrivals: &[ShipId],
        jumped_ships: &HashMap<ShipId, SectorId>,
    ) {
        let grid = aoi::CellGrid::build(self.aoi_cell_size, node.ship_absolute_positions());
        let aoi_delivery = &mut self.aoi_delivery;
''',
)
replace_once(
    "crates/dawn-sector-node/src/runtime.rs",
    '''            aoi_delivery.deliver_frame(&mut sink, node, observer, curr, new_events, &warp_arrivals)
''',
    '''            aoi_delivery.deliver_frame(&mut sink, node, observer, curr, new_events, warp_arrivals)
''',
)

replace_once(
    "crates/dawn-sector/src/node/warp.rs",
    '''    pub fn drain_completed_warps(&mut self) -> Vec<ShipId> {
        std::mem::take(&mut self.completed_warps)
    }

''',
    '''    pub fn drain_completed_warps(&mut self) -> Vec<ShipId> {
        std::mem::take(&mut self.completed_warps)
    }

    #[cfg(test)]
    pub(crate) fn queue_runtime_transients_for_test(
        &mut self,
        auto_jump: (ShipId, JumpGateId),
        completed_warp: ShipId,
    ) {
        self.pending_auto_jumps.push(auto_jump);
        self.completed_warps.push(completed_warp);
    }

''',
)

with Path("crates/dawn-sector/src/transit/tests.rs").open("a") as file:
    file.write(
        r'''

#[test]
fn runtime_tick_owns_replication_raft_and_transient_order() {
    let mut node = mem_node();
    let gate = *node.jump_gate(JumpGateId(0)).expect("Sector 0 has Gate 0");
    let near_gate_abs = [
        gate.abs_m[0] - (gate.activation_radius * 0.5),
        gate.abs_m[1],
        gate.abs_m[2],
    ];
    let player_id = node.next_player_id();
    let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
    node.set_spawn_anchor_abs(ship, near_gate_abs);
    node.queue_runtime_transients_for_test((ship, JumpGateId(0)), ship);

    let event_cursor = node.total_event_count() as u64;
    let (raft, mut raft_messages) = raft_handle();
    let (_committed_tx, mut committed_rx) = mpsc::unbounded_channel();
    let mut replication_hook_called = false;

    let output = run_runtime_tick(
        &mut node,
        &raft,
        &mut committed_rx,
        &[],
        |node, _, events| {
            replication_hook_called = true;
            assert!(
                raft_messages.try_recv().is_err(),
                "replication hook must run before raft.tick()"
            );
            let collected = node.event_store().iter_from(event_cursor).count();
            assert_eq!(events.len(), collected, "hook receives the collected Event tail");
        },
    );

    assert!(replication_hook_called);
    assert_eq!(output.pending_auto_jumps, vec![(ship, JumpGateId(0))]);
    assert_eq!(output.completed_warps, vec![ship]);
    assert!(node.drain_pending_auto_jumps().is_empty());
    assert!(node.drain_completed_warps().is_empty());

    assert!(matches!(
        raft_messages.try_recv(),
        Ok(dawn_consensus::RaftActorMessage::TickElapsed)
    ));
    assert!(matches!(
        decode_proposed_transit(&mut raft_messages),
        TransitOp::Request {
            ship_id,
            gate_id: Some(JumpGateId(0)),
            ..
        } if ship_id == ship
    ));
}
'''
    )

replace_once(
    "docs/architecture/tick-model.md",
    '''         Both the actor path and the clustered serve path share
         `transit::run_runtime_tick` (7.5 apply -> node.tick -> Step 9 hook -> raft.tick
         -> transient outputs). `serve::runtime::run_cluster_runtime_tick` additionally
         handles clustered-serve auto-jump / ownership handoff / AoI delivery / scoped
         InitialState resend. `transit::step_cluster_node` is a thin entry point that
         drains transients for callers such as `dawn-sector-node`.
''',
    '''         Actor-backed simulation, clustered serve, and the production Sector Node
         all call `transit::run_runtime_tick`. It is the sole authoritative frame
         pipeline: Step 7.5 apply -> node.tick -> collect the Event tail -> Step 9
         replication hook -> raft.tick -> drain and propose auto-jumps -> drain
         completed-warp outputs. Runtime adapters only perform command/session,
         transport, ownership-handoff, Redirect, and AoI delivery work around that
         common output contract.
''',
)
replace_once(
    "docs/architecture/tick-model.md",
    '''         Shared via `transit::run_runtime_tick()` for actor / clustered serve;
         `dawn-sector-node` runs it via `transit::step_cluster_node()`.
''',
    '''         Shared via `transit::run_runtime_tick()` for actor-backed simulation,
         clustered serve, and the production `dawn-sector-node` runtime.
''',
)
replace_once(
    "docs/architecture/tick-model.md",
    '''`run_phase4_server()` (single node) / `run_cluster_server()` (3-node Raft) in
`main.rs` drive the loop via a fixed-interval `tokio::time::interval`
(100 ms/tick). `SimulationNode::tick_with_lock_commands()` itself is
synchronous; the caller's interval controls pacing.
''',
    '''`run_phase4_server()` (single node), `run_cluster_server()` (3-node Raft),
and the production `dawn-sector-node` process drive their loops via a
fixed-interval `tokio::time::interval` (100 ms/tick). Every server path enters
`transit::run_runtime_tick()` for the authoritative frame order;
`SimulationNode::tick_with_lock_commands()` remains synchronous and each
process adapter controls only pacing and transport/session-specific work.
''',
)

print("Issue #219 migration applied")
