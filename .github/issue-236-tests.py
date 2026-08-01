from pathlib import Path
import re


def read(path):
    return Path(path).read_text()


def write(path, text):
    Path(path).write_text(text)


def exact(path, old, new, expected=1):
    text = read(path)
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected}, found {count}: {old[:120]!r}")
    write(path, text.replace(old, new))


def regex(path, pattern, replacement, expected=1):
    text = read(path)
    new, count = re.subn(pattern, replacement, text, flags=re.S)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected} regex matches, found {count}: {pattern[:100]!r}")
    write(path, new)


# pipeline.rs test module
path = "crates/dawn-sector/src/transit/pipeline.rs"
text = read(path)
text = text.replace(
    "use dawn_core::events::{SectorTransitCompleted, SectorTransitRequested, TransitShipState};",
    "use dawn_core::events::{SectorTransitCompleted, SectorTransitRequested};",
)
text = text.replace(
    "source.complete_outgoing_transit(\n            &proposal.handoff,",
    "source.complete_outgoing_transit(\n            proposal.handoff.ship_id,",
)
text = text.replace(
    '''        assert!(apply_ack(
            &mut sector_b,
            &return_ack.ship,
            return_ack.from,
            return_ack.to,
            return_ack.entry_pos_abs,
            return_ack.request_tick,
        ));
''',
    '''        assert!(apply_ack(
            &mut sector_b,
            return_ack.ship_id,
            return_ack.from,
            return_ack.to,
            return_ack.request_tick,
        ));
''',
)
text = text.replace(
    '''        assert!(!apply_ack(
            &mut sector_a,
            &delayed_outbound_ack.ship,
            delayed_outbound_ack.from,
            delayed_outbound_ack.to,
            delayed_outbound_ack.entry_pos_abs,
            delayed_outbound_ack.request_tick,
        ));
''',
    '''        assert!(!apply_ack(
            &mut sector_a,
            delayed_outbound_ack.ship_id,
            delayed_outbound_ack.from,
            delayed_outbound_ack.to,
            delayed_outbound_ack.request_tick,
        ));
''',
)
helper_pattern = r"    fn test_ship\(ship_id: ShipId\) -> ShipSnapshot \{.*?    fn transit_state\(\) -> TransitShipState \{.*?\n    \}\n"
helper_replacement = '''    fn test_handoff(ship_id: ShipId) -> TransitHandoffState {
        TransitHandoffState {
            ship_id,
            ship_type_id: ShipTypeId(1),
            velocity: Velocity::ZERO,
            current_shield: 100.0,
            current_armor: 100.0,
            current_hull: 100.0,
            is_destroyed: false,
            capacitor: Some(100.0),
            fitting: dawn_core::fitting::FittingSnapshot::empty(),
            inventory: std::collections::BTreeMap::new(),
        }
    }
'''
text, count = re.subn(helper_pattern, helper_replacement, text, flags=re.S)
if count != 1:
    raise SystemExit(f"{path}: expected one old test helper pair, found {count}")
text = text.replace("test_ship(", "test_handoff(")
old_completed = '''        DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            ship_id,
            from,
            to,
            request_tick: Tick(request_tick),
            entry_pos: AbsolutePosition::ORIGIN,
            velocity: Velocity::ZERO,
            tick: Tick(event_tick),
            ship_state: transit_state(),
        })
'''
new_completed = '''        DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            handoff: test_handoff(ship_id),
            from,
            to,
            request_tick: Tick(request_tick),
            entry_pos: AbsolutePosition::ORIGIN,
            tick: Tick(event_tick),
        })
'''
if text.count(old_completed) != 1:
    raise SystemExit(f"{path}: expected one completed helper body")
text = text.replace(old_completed, new_completed)
write(path, text)


# transit/tests.rs protocol and end-to-end tests
path = "crates/dawn-sector/src/transit/tests.rs"
text = read(path)
helper_pattern = r"fn sample_ship\(\) -> ShipSnapshot \{.*?\n\}\n\n"
helper_replacement = '''fn sample_handoff() -> TransitHandoffState {
    TransitHandoffState {
        ship_id: ShipId::new(NodeId(0), 7),
        ship_type_id: ShipTypeId(1),
        velocity: Velocity::new(4.0, 5.0, 6.0),
        current_shield: 10.0,
        current_armor: 20.0,
        current_hull: 30.0,
        is_destroyed: false,
        capacitor: Some(50.0),
        fitting: FittingSnapshot::empty(),
        inventory: std::collections::BTreeMap::new(),
    }
}

'''
text, count = re.subn(helper_pattern, helper_replacement, text, flags=re.S)
if count != 1:
    raise SystemExit(f"{path}: expected one sample ShipSnapshot helper, found {count}")
text = text.replace("sample_ship()", "sample_handoff()")
text = text.replace("TransitOp::Commit {\n        ship:", "TransitOp::Commit {\n        handoff:")
text = text.replace("TransitOp::Commit {\n            ship:", "TransitOp::Commit {\n            handoff:")
text = text.replace("TransitOp::Commit { ship, .. }", "TransitOp::Commit { handoff, .. }")
text = text.replace("TransitOp::Commit {\n            ship,", "TransitOp::Commit {\n            handoff,")
text = text.replace("assert_eq!(ship.ship_id, ship_id);", "assert_eq!(handoff.ship_id, ship_id);")
text = text.replace(
    '''            ship.tackled_by.is_empty(),
            "Sector-local tackle state must not cross the boundary on retry"
''',
    '''            handoff.ship_id == ship_id,
            "retry handoff must preserve the canonical Ship identity"
''',
)
old_ack = '''    let ack = TransitOp::Ack {
        ship: Box::new(sample_handoff()),
        from: SectorId(0),
        to: SectorId(1),
        entry_pos_abs: AbsolutePosition::new(500.0, 0.0, 0.0),
        request_tick: Tick(12),
    };
'''
new_ack = '''    let ack = TransitOp::Ack {
        ship_id: sample_handoff().ship_id,
        from: SectorId(0),
        to: SectorId(1),
        request_tick: Tick(12),
    };
'''
if text.count(old_ack) != 1:
    raise SystemExit(f"{path}: expected one Ack round-trip fixture")
text = text.replace(old_ack, new_ack)
write(path, text)


# node/transit.rs unit and replay tests
path = "crates/dawn-sector/src/node/transit.rs"
text = read(path)
text = text.replace("complete_outgoing_transit(&snapshot,", "complete_outgoing_transit(snapshot.ship_id,")
text = text.replace(
    "complete_outgoing_transit(\n            &snapshot,",
    "complete_outgoing_transit(\n            snapshot.ship_id,",
)
text = text.replace("complete_outgoing_transit(&exported,", "complete_outgoing_transit(exported.ship_id,")
text = text.replace(
    "DomainEvent::SectorTransitCompleted(e) => {\n                assert_eq!(e.ship_id, ship_id);",
    "DomainEvent::SectorTransitCompleted(e) => {\n                assert_eq!(e.handoff.ship_id, ship_id);",
)
helper_old = '''    fn sample_transit_ship_state() -> dawn_core::events::TransitShipState {
        dawn_core::events::TransitShipState {
            ship_type_id: ShipTypeId(1),
            current_shield: 80.0,
            current_armor: 90.0,
            current_hull: 100.0,
            is_destroyed: false,
            capacitor: Some(40.0),
            fitting: dawn_core::fitting::FittingSnapshot::empty(),
            inventory: std::collections::BTreeMap::new(),
        }
    }
'''
helper_new = '''    fn sample_transit_handoff(
        ship_id: ShipId,
        velocity: Velocity,
    ) -> TransitHandoffState {
        TransitHandoffState {
            ship_id,
            ship_type_id: ShipTypeId(1),
            velocity,
            current_shield: 80.0,
            current_armor: 90.0,
            current_hull: 100.0,
            is_destroyed: false,
            capacitor: Some(40.0),
            fitting: dawn_core::fitting::FittingSnapshot::empty(),
            inventory: std::collections::BTreeMap::new(),
        }
    }
'''
if text.count(helper_old) != 1:
    raise SystemExit(f"{path}: expected one old TransitShipState helper")
text = text.replace(helper_old, helper_new)
fixture_pattern = re.compile(
    r"dawn_core::events::SectorTransitCompleted \{\n"
    r"\s*ship_id,\n"
    r"(?P<middle>\s*from:.*?\s*entry_pos:.*?\n)"
    r"\s*velocity: (?P<velocity>[^,\n]+(?:\([^\n]*\))?),\n"
    r"\s*tick: (?P<tick>[^,\n]+),\n"
    r"\s*ship_state: sample_transit_ship_state\(\),\n"
    r"\s*\}",
    re.S,
)

def replace_fixture(match):
    middle = match.group("middle")
    velocity = match.group("velocity")
    tick = match.group("tick")
    return (
        "dawn_core::events::SectorTransitCompleted {\n"
        f"                handoff: sample_transit_handoff(ship_id, {velocity}),\n"
        f"{middle}"
        f"                tick: {tick},\n"
        "            }"
    )
text, count = fixture_pattern.subn(replace_fixture, text)
if count != 3:
    raise SystemExit(f"{path}: expected three old completed fixtures, found {count}")
write(path, text)
