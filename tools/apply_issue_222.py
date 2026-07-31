from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text)


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def sub_once(text: str, pattern: str, replacement: str, label: str) -> str:
    text, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return text


# dawn-wire: required tagged targets and direct projection.
path = "crates/dawn-wire/src/client_command.rs"
s = read(path)
s = replace_once(
    s,
    '''/// A `{"Gate": N}` or `{"Body": N}` warp destination, as sent by
/// `WarpCommand`'s current wire format (externally tagged: the variant name
/// is the JSON object's only key).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub enum WarpTargetWire {
    Gate(u32),
    Body(u32),
}
''',
    '''/// A `{"Ship": N}` or `{"Gate": N}` navigation target for Approach,
/// Orbit, and KeepAtRange (externally tagged: the variant name is the JSON
/// object's only key).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub enum NavigationTargetWire {
    Ship(u64),
    Gate(u32),
}

impl From<NavigationTargetWire> for ApproachTarget {
    fn from(target: NavigationTargetWire) -> Self {
        match target {
            NavigationTargetWire::Ship(ship) => {
                ApproachTarget::Ship(ShipId(EntityId::from_raw(ship)))
            }
            NavigationTargetWire::Gate(gate) => {
                ApproachTarget::Gate(dawn_core::JumpGateId(gate))
            }
        }
    }
}

/// A `{"Gate": N}` or `{"Body": N}` warp destination, as sent by
/// `WarpCommand` (externally tagged: the variant name is the JSON object's
/// only key).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub enum WarpTargetWire {
    Gate(u32),
    Body(u32),
}

impl From<WarpTargetWire> for dawn_core::WarpTarget {
    fn from(target: WarpTargetWire) -> Self {
        match target {
            WarpTargetWire::Gate(gate) => Self::Gate(dawn_core::JumpGateId(gate)),
            WarpTargetWire::Body(body) => Self::Body(dawn_core::CelestialBodyId(body)),
        }
    }
}
''',
    "wire target enums",
)
s = replace_once(
    s,
    '''/// wire protocol (see [`crate::EventWire`] for the server -> client half). It
/// intentionally mirrors the wire format exactly, including the two
/// backward-compatible quirks below -- it does not enforce the "exactly one
/// of these two fields" business rules those quirks involve; that
/// validation still happens in [`client_command_from_wire`].
///
/// - `WarpCommand` accepts either `target` (current) or `gate_id` (legacy);
///   `target` wins if both are present.
/// - `ApproachCommand` / `OrbitCommand` / `KeepAtRangeCommand` select their
///   target with either `gate_id` (a Jump Gate) or `target_id` (a Ship);
///   `gate_id` wins if both are present.
''',
    '''/// wire protocol (see [`crate::EventWire`] for the server -> client half).
/// Navigation commands use required tagged target enums, so invalid states
/// such as a missing target or simultaneous Ship/Gate targets cannot be
/// represented after successful decoding.
''',
    "wire documentation",
)
s = replace_once(
    s,
    '''    ApproachCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
    },
    WarpCommand {
        target: Option<WarpTargetWire>,
        /// Legacy form: `{"gate_id": N}` instead of `{"target": {"Gate": N}}`.
        gate_id: Option<u32>,
    },
    OrbitCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        radius: Option<f64>,
    },
    KeepAtRangeCommand {
        gate_id: Option<u32>,
        target_id: Option<u64>,
        range: Option<f64>,
    },
''',
    '''    ApproachCommand {
        target: NavigationTargetWire,
    },
    WarpCommand {
        target: WarpTargetWire,
    },
    OrbitCommand {
        target: NavigationTargetWire,
        radius: Option<f64>,
    },
    KeepAtRangeCommand {
        target: NavigationTargetWire,
        range: Option<f64>,
    },
''',
    "wire variants",
)
s = sub_once(
    s,
    r'''fn approach_target_from_gate_or_ship\(.*?\n}\n\n(?=/// Convert an already-decoded)''',
    "",
    "legacy target helper",
)
s = sub_once(
    s,
    r'''        ClientCommandWire::ApproachCommand \{ gate_id, target_id \} => \{.*?        ClientCommandWire::KeepAtRangeCommand \{\n            gate_id,\n            target_id,\n            range,\n        \} => \{.*?\n        }\n(?=        ClientCommandWire::FitModuleCommand)''',
    '''        ClientCommandWire::ApproachCommand { target } => {
            Some(ClientCommand::Approach(ApproachCommand {
                target: target.into(),
            }))
        }
        ClientCommandWire::WarpCommand { target } => {
            Some(ClientCommand::Warp(dawn_core::WarpCommand {
                target: target.into(),
            }))
        }
        ClientCommandWire::OrbitCommand { target, radius } => {
            if radius.is_some_and(|r| !r.is_finite()) {
                return None;
            }
            Some(ClientCommand::Orbit(dawn_core::OrbitCommand {
                target: target.into(),
                radius,
            }))
        }
        ClientCommandWire::KeepAtRangeCommand { target, range } => {
            if range.is_some_and(|r| !r.is_finite()) {
                return None;
            }
            Some(ClientCommand::KeepAtRange(dawn_core::KeepAtRangeCommand {
                target: target.into(),
                range,
            }))
        }
''',
    "wire conversion arms",
)
s = s.replace(
    'r#"{"OrbitCommand":{"gate_id":2,"radius":1e+400}}"#',
    'r#"{"OrbitCommand":{"target":{"Gate":2},"radius":1e+400}}"#',
)
s = s.replace(
    'r#"{"KeepAtRangeCommand":{"gate_id":2,"range":1e+400}}"#',
    'r#"{"KeepAtRangeCommand":{"target":{"Gate":2},"range":1e+400}}"#',
)
s = sub_once(
    s,
    r'''    #\[test\]\n    fn warp_command_json_is_parsed_into_client_command_warp\(\) \{.*?\n    }\n\n(?=    #\[test\]\n    fn dock_command_json)''',
    '''    #[test]
    fn warp_command_json_is_parsed_into_client_command_warp() {
        let gate = r#"{"WarpCommand":{"target":{"Gate":2}}}"#;
        let gate_cmd = command_from_json(gate).expect("must parse");
        match gate_cmd {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }

        let body = r#"{"WarpCommand":{"target":{"Body":1}}}"#;
        let body_cmd = command_from_json(body).expect("must parse");
        match body_cmd {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(
                    c.target,
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(1))
                );
            }
            other => panic!("expected Warp, got {other:?}"),
        }
    }

    #[test]
    fn legacy_warp_gate_id_shape_fails_to_parse() {
        let legacy = r#"{"WarpCommand":{"gate_id":2}}"#;
        assert!(serde_json::from_str::<ClientCommandWire>(legacy).is_err());
    }

''',
    "warp tests",
)
s = sub_once(
    s,
    r'''    #\[test\]\n    fn orbit_command_json_with_target_id_is_parsed_into_client_command_orbit\(\) \{.*?\n    }\n\n    #\[test\]\n    fn orbit_command_json_with_gate_id_and_no_radius_is_parsed\(\) \{.*?\n    }\n\n    #\[test\]\n    fn keep_at_range_command_json_is_parsed_into_client_command_keep_at_range\(\) \{.*?\n    }\n''',
    '''    #[test]
    fn approach_command_json_uses_a_required_tagged_target() {
        let ship = r#"{"ApproachCommand":{"target":{"Ship":2}}}"#;
        let ship_cmd = command_from_json(ship).expect("must parse");
        match ship_cmd {
            dawn_core::ClientCommand::Approach(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
            }
            other => panic!("expected Approach, got {other:?}"),
        }

        let gate = r#"{"ApproachCommand":{"target":{"Gate":4}}}"#;
        let gate_cmd = command_from_json(gate).expect("must parse");
        match gate_cmd {
            dawn_core::ClientCommand::Approach(c) => {
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(4)));
            }
            other => panic!("expected Approach, got {other:?}"),
        }
    }

    #[test]
    fn legacy_navigation_target_fields_fail_to_parse() {
        let approach = r#"{"ApproachCommand":{"target_id":2}}"#;
        let orbit = r#"{"OrbitCommand":{"gate_id":4}}"#;
        let keep = r#"{"KeepAtRangeCommand":{"target_id":2}}"#;
        assert!(serde_json::from_str::<ClientCommandWire>(approach).is_err());
        assert!(serde_json::from_str::<ClientCommandWire>(orbit).is_err());
        assert!(serde_json::from_str::<ClientCommandWire>(keep).is_err());
    }

    #[test]
    fn navigation_commands_without_a_target_fail_to_parse() {
        let approach = r#"{"ApproachCommand":{}}"#;
        let orbit = r#"{"OrbitCommand":{"radius":3000.0}}"#;
        let keep = r#"{"KeepAtRangeCommand":{"range":5000.0}}"#;
        let warp = r#"{"WarpCommand":{}}"#;
        assert!(serde_json::from_str::<ClientCommandWire>(approach).is_err());
        assert!(serde_json::from_str::<ClientCommandWire>(orbit).is_err());
        assert!(serde_json::from_str::<ClientCommandWire>(keep).is_err());
        assert!(serde_json::from_str::<ClientCommandWire>(warp).is_err());
    }

    #[test]
    fn orbit_command_json_with_ship_target_is_parsed_into_client_command_orbit() {
        let line = r#"{"OrbitCommand":{"target":{"Ship":2},"radius":3000.0}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.radius, Some(3000.0));
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_gate_target_and_no_radius_is_parsed() {
        let line = r#"{"OrbitCommand":{"target":{"Gate":4}}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(4)));
                assert_eq!(c.radius, None);
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn keep_at_range_command_json_is_parsed_into_client_command_keep_at_range() {
        let line = r#"{"KeepAtRangeCommand":{"target":{"Ship":2},"range":5000.0}}"#;
        let cmd = command_from_json(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::KeepAtRange(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.range, Some(5000.0));
            }
            other => panic!("expected KeepAtRange, got {other:?}"),
        }
    }
''',
    "navigation tests",
)
write(path, s)

# Godot GDExtension command builders use the same tagged targets.
path = "crates/dawn-client-gdext/src/client_command_gd.rs"
s = read(path)
s = replace_once(s, "    WarpTargetWire,\n", "    NavigationTargetWire, WarpTargetWire,\n", "gdext import")
s = replace_once(
    s,
    '''/// Commands whose wire shape carries sentinel values (e.g. `<= 0.0` meaning
/// "server default", ADR-0031) or an exclusive-selection field pair (e.g.
/// `gate_id` xor `target_id`) get a dedicated method, since that logic is
/// domain semantics, not just field copying. Everything else -- a flat
''',
    '''/// Commands whose wire shape carries sentinel values (e.g. `<= 0.0` meaning
/// "server default", ADR-0031) or a tagged navigation target get a dedicated
/// method, since that logic is domain semantics, not just field copying.
/// Everything else -- a flat
''',
    "gdext docs",
)
s = s.replace("no sentinel/exclusive-selection semantics", "no sentinel/tagged-target semantics")
replacements = {
'''ClientCommandWire::ApproachCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
        }''': '''ClientCommandWire::ApproachCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
        }''',
'''ClientCommandWire::ApproachCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
        }''': '''ClientCommandWire::ApproachCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
        }''',
'''ClientCommandWire::WarpCommand {
            target: Some(WarpTargetWire::Gate(gate_id as u32)),
            gate_id: None,
        }''': '''ClientCommandWire::WarpCommand {
            target: WarpTargetWire::Gate(gate_id as u32),
        }''',
'''ClientCommandWire::WarpCommand {
            target: Some(WarpTargetWire::Body(body_id as u32)),
            gate_id: None,
        }''': '''ClientCommandWire::WarpCommand {
            target: WarpTargetWire::Body(body_id as u32),
        }''',
'''ClientCommandWire::OrbitCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            radius: positive_or_none(range_m),
        }''': '''ClientCommandWire::OrbitCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
            radius: positive_or_none(range_m),
        }''',
'''ClientCommandWire::OrbitCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            radius: positive_or_none(range_m),
        }''': '''ClientCommandWire::OrbitCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
            radius: positive_or_none(range_m),
        }''',
'''ClientCommandWire::KeepAtRangeCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            range: positive_or_none(range_m),
        }''': '''ClientCommandWire::KeepAtRangeCommand {
            target: NavigationTargetWire::Ship(target_id as u64),
            range: positive_or_none(range_m),
        }''',
'''ClientCommandWire::KeepAtRangeCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            range: positive_or_none(range_m),
        }''': '''ClientCommandWire::KeepAtRangeCommand {
            target: NavigationTargetWire::Gate(gate_id as u32),
            range: positive_or_none(range_m),
        }''',
}
for old, new in replacements.items():
    s = replace_once(s, old, new, "gdext command constructor")
write(path, s)

# Public export.
path = "crates/dawn-wire/src/lib.rs"
s = read(path)
s = replace_once(
    s,
    '''    client_command_from_wire, client_command_wire_json_schema, ClientCommandWire, PosWire, VelWire,
    WarpTargetWire,
''',
    '''    client_command_from_wire, client_command_wire_json_schema, ClientCommandWire,
    NavigationTargetWire, PosWire, VelWire, WarpTargetWire,
''',
    "wire export",
)
write(path, s)

# GdUnit command-shape assertions.
path = "client/test/client_command_gd_test.gd"
s = read(path)
s = replace_once(
    s,
    '''func test_orbit_command_omits_radius_when_not_positive() -> void:
''',
    '''func test_approach_command_wraps_the_ship_id_in_the_target_tag() -> void:
\tvar bytes: PackedByteArray = _cmd.approach_command(7)
\tvar d: Dictionary = _decoder.decode(bytes)
\tassert_str(d["type"]).is_equal("ApproachCommand")
\tvar target: Dictionary = d["target"]
\tassert_int(int(target["Ship"])).is_equal(7)


func test_approach_gate_command_wraps_the_gate_id_in_the_target_tag() -> void:
\tvar bytes: PackedByteArray = _cmd.approach_gate_command(4)
\tvar d: Dictionary = _decoder.decode(bytes)
\tvar target: Dictionary = d["target"]
\tassert_int(int(target["Gate"])).is_equal(4)


func test_orbit_command_omits_radius_when_not_positive() -> void:
''',
    "gdunit approach tests",
)
s = replace_once(
    s,
    '''\tassert_str(d["type"]).is_equal("OrbitCommand")
\tassert_int(int(d["target_id"])).is_equal(7)
\tassert_object(d["radius"]).is_null()
''',
    '''\tassert_str(d["type"]).is_equal("OrbitCommand")
\tvar target: Dictionary = d["target"]
\tassert_int(int(target["Ship"])).is_equal(7)
\tassert_object(d["radius"]).is_null()
''',
    "gdunit orbit target",
)
s = replace_once(
    s,
    '''func test_keep_at_range_gate_command_uses_gate_id_not_target_id() -> void:
\tvar bytes: PackedByteArray = _cmd.keep_at_range_gate_command(4, 1000.0)
\tvar d: Dictionary = _decoder.decode(bytes)
\tassert_str(d["type"]).is_equal("KeepAtRangeCommand")
\tassert_int(int(d["gate_id"])).is_equal(4)
\tassert_object(d["target_id"]).is_null()
\tassert_float(d["range"]).is_equal_approx(1000.0, 0.0001)
''',
    '''func test_keep_at_range_gate_command_uses_a_tagged_gate_target() -> void:
\tvar bytes: PackedByteArray = _cmd.keep_at_range_gate_command(4, 1000.0)
\tvar d: Dictionary = _decoder.decode(bytes)
\tassert_str(d["type"]).is_equal("KeepAtRangeCommand")
\tvar target: Dictionary = d["target"]
\tassert_int(int(target["Gate"])).is_equal(4)
\tassert_float(d["range"]).is_equal_approx(1000.0, 0.0001)
''',
    "gdunit keep target",
)
write(path, s)

# Protocol prose: remove compatibility precedence and document required tags.
path = "docs/architecture/wire-protocol.md"
s = read(path)
s = sub_once(
    s,
    r'''`ClientCommandWire` mirrors the wire format exactly, including two\nbackward-compatible quirks it does not itself resolve \(that validation\nhappens in `client_command_from_wire\(\)`\):\n\n- `WarpCommand` accepts a legacy .*?if neither is present\.\n''',
    '''Navigation commands use one required tagged target representation:

- `ApproachCommand`, `OrbitCommand`, and `KeepAtRangeCommand` carry
  `target: {"Ship": N}` or `target: {"Gate": N}`.
- `WarpCommand` carries `target: {"Gate": N}` or `target: {"Body": N}`.

After successful wire decoding, exactly one target is present. The legacy
`gate_id`/`target_id` field pairs and Warp's legacy `gate_id` fallback are no
longer part of the protocol.
''',
    "wire protocol legacy section",
)
write(path, s)

print("Issue #222 transformations applied")
