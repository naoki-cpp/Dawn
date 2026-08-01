from pathlib import Path

path = Path("crates/dawn-sector-node/src/client_admission.rs")
text = path.read_text()
text = text.replace(
    "should_despawn_on_completion_failure(&handshake_identity)",
    "fresh_spawn_for_failed_handshake(&handshake_identity)",
)
text = text.replace(
    "should_despawn_on_completion_failure(&identity)",
    "fresh_spawn_for_failed_handshake(&identity)",
)
old = '''/// Whether a `HandshakeRequest::complete` failure for `identity` leaves a
/// ghost ship that must be despawned. Only a fresh spawn qualifies: a
/// resumed ship existed before this connection attempt, so its ownership
/// predates the failure and is a separate concern (ADR-0007 §2-A resume;
/// the ownership-hijack risk on resume is already tracked as a security
/// finding, not something this cleanup should also touch).
fn build_handshake_payload<S: EventStore>(
    node: &SimulationNode<S>,
    identity: &HandshakeIdentity,
    aoi_cell_size: f64,
) -> Result<HandoffPayload, MissingObserverShip> {
    node.build_handoff_payload(identity.ship_id, aoi_cell_size)
}

fn should_despawn_on_completion_failure(identity: &HandshakeIdentity) -> Option<ShipId> {
    (!identity.resumed).then_some(identity.ship_id)
}
'''
new = '''fn build_handshake_payload<S: EventStore>(
    node: &SimulationNode<S>,
    identity: &HandshakeIdentity,
    aoi_cell_size: f64,
) -> Result<HandoffPayload, MissingObserverShip> {
    node.build_handoff_payload(identity.ship_id, aoi_cell_size)
}

/// Return the fresh-spawn ship that must be removed when a handshake cannot
/// complete. A resumed ship predates this attempt and must never be removed as
/// cleanup for an admission or transport failure (ADR-0007 §2-A resume).
fn fresh_spawn_for_failed_handshake(identity: &HandshakeIdentity) -> Option<ShipId> {
    (!identity.resumed).then_some(identity.ship_id)
}
'''
if text.count(old) != 1:
    raise RuntimeError("expected misplaced cleanup rustdoc block once")
text = text.replace(old, new)
path.write_text(text)
