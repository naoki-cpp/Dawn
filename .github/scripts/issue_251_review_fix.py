from pathlib import Path
import re


def replace(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"missing replacement anchor in {path}: {old[:100]!r}")
    file.write_text(text.replace(old, new))


# Rust receives both presentation targets explicitly. Scene-tree ownership stays in GDScript.
path = Path("crates/dawn-client-gdext/src/server_message_gd.rs")
text = path.read_text()
signature = "        mut connection_target: Gd<Object>,\n        mut session: Gd<WorldSession>,"
if signature not in text:
    raise SystemExit("server_message_gd dispatch signature anchor missing")
text = text.replace(
    signature,
    "        mut connection_target: Gd<Object>,\n        mut world_target: Gd<Object>,\n        mut session: Gd<WorldSession>,",
    1,
)
start = text.find("        let has_parent = connection_target")
end = text.find("        match &self.message {", start)
if start < 0 or end < 0:
    raise SystemExit("server_message_gd parent-inference block missing")
text = text[:start] + text[end:]
path.write_text(text)

# Connection stores the explicit world presentation target and drops the old event relay.
replace("client/scripts/connection.gd", "signal event_received(outcome: ServerEventOutcome)\n", "")
replace(
    "client/scripts/connection.gd",
    "var _world_session   : WorldSession          = null\nvar _player_loadout  : PlayerLoadout         = null\n",
    "var _world_session   : WorldSession          = null\nvar _player_loadout  : PlayerLoadout         = null\nvar _world_target    : Object                = null\n",
)
replace(
    "client/scripts/connection.gd",
    "func bind_client_state(session: WorldSession, loadout: PlayerLoadout) -> void:\n\t_world_session = session\n\t_player_loadout = loadout\n",
    "func bind_client_state(\n\tsession: WorldSession, loadout: PlayerLoadout, world_target: Object\n) -> void:\n\t_world_session = session\n\t_player_loadout = loadout\n\t_world_target = world_target\n",
)
replace(
    "client/scripts/connection.gd",
    "\t\tif _world_session == null or _player_loadout == null:\n\t\t\tpush_warning(\"[Connection] typed client state is not bound\")\n\t\t\tcontinue\n\t\tif not outcome.dispatch(self, _world_session, _player_loadout, ship_id):\n",
    "\t\tif _world_session == null or _player_loadout == null or _world_target == null:\n\t\t\tpush_warning(\"[Connection] typed client state or world target is not bound\")\n\t\t\tcontinue\n\t\tif not outcome.dispatch(\n\t\t\tself, _world_target, _world_session, _player_loadout, ship_id\n\t\t):\n",
)
replace(
    "client/scripts/connection.gd",
    "\n\nfunc _accept_event(outcome: ServerEventOutcome) -> void:\n\tevent_received.emit(outcome)\n",
    "",
)

# Main explicitly binds itself as scene/presentation owner.
replace(
    "client/scripts/main.gd",
    "\t_connection.bind_client_state(_session, _loadout)\n\t_connection.event_received.connect(_on_event_received)\n",
    "\t_connection.bind_client_state(_session, _loadout, self)\n",
)
replace(
    "client/scripts/main.gd",
    "func _on_event_received(outcome: ServerEventOutcome) -> void:\n\t_sync_session_state()\n\tif not outcome.dispatch(self):\n\t\tpush_warning(\"[World] failed to dispatch typed server event outcome\")\n",
    "",
)

# GdUnit exercises separate connection and world targets.
path = Path("client/test/connection_test.gd")
text = path.read_text()
pattern = re.compile(
    r"\b(outcome|initial|docked)\.dispatch\(([^,\n]+), ([^,\n]+), ([^,\n]+), ([^)]+)\)"
)
text, count = pattern.subn(
    lambda match: (
        f"{match.group(1)}.dispatch({match.group(2)}, {match.group(2)}, "
        f"{match.group(3)}, {match.group(4)}, {match.group(5)})"
    ),
    text,
)
if count < 8:
    raise SystemExit(f"expected at least eight dispatch call updates, found {count}")
old = (
    "\tvar target := EventDispatchTarget.new()\n"
    "\tassert_bool(outcome.dispatch(target, target, state[0], state[1], -1)).is_true()\n"
)
new = (
    "\tvar connection_target := RefCounted.new()\n"
    "\tvar target := EventDispatchTarget.new()\n"
    "\tassert_bool(outcome.dispatch(\n"
    "\t\tconnection_target, target, state[0], state[1], -1\n"
    "\t)).is_true()\n"
)
if old not in text:
    raise SystemExit("explicit world-target GdUnit anchor missing")
text = text.replace(old, new)
text = text.replace(
    "outcome.dispatch(connection, connection, main._session, main._loadout, 11)",
    "outcome.dispatch(connection, main, main._session, main._loadout, 11)",
)
path.write_text(text)

# Remove the obsolete parse-only compatibility class and its registration.
replace("crates/dawn-client-gdext/src/lib.rs", "mod legacy_server_event_outcome;\n", "")
replace(
    "crates/dawn-client-gdext/src/lib.rs",
    "pub use legacy_server_event_outcome::ServerEventOutcome;\n",
    "",
)
Path("crates/dawn-client-gdext/src/legacy_server_event_outcome.rs").unlink()

# Add pure ClientState tests for every acceptance-rule family missing from the first review.
path = Path("crates/dawn-client-core/src/client_state.rs")
text = path.read_text()
old_import = "    use crate::{PositionInput, ShipInput, StationInput, SystemNameInput};"
new_import = """    use crate::{
        ModuleKind, ModuleRow, PositionInput, ShipInput, StatDelta, StationInput,
        SystemNameInput,
    };"""
if old_import not in text:
    raise SystemExit("client_state test import anchor missing")
text = text.replace(old_import, new_import)
helper_anchor = "    fn setup() -> (WorldSessionState, Option<PlayerLoadoutMsg>) {\n"
helper = """    fn module(module_id: u32, active: bool) -> ModuleRow {
        ModuleRow {
            slot: "High".to_owned(),
            index: 0,
            module_id,
            name: "Test module".to_owned(),
            kind: ModuleKind::Weapon,
            is_active: active,
            is_active_module: true,
            cap_cost_per_cycle: 5.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta::ZERO,
            cycle_remaining: 7,
            forced_reason: String::new(),
        }
    }

"""
if helper_anchor not in text:
    raise SystemExit("client_state helper anchor missing")
text = text.replace(helper_anchor, helper + helper_anchor)
tests = """

    #[test]
    fn tick_advances_session_and_capacitor_through_the_shared_loadout() {
        let (mut session, mut loadout) = setup();
        loadout = Some(PlayerLoadoutMsg {
            active_ship_id: Some(1),
            modules: vec![module(7, true)],
            ..PlayerLoadoutMsg::default()
        });

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 1 })
            .unwrap();

        assert_eq!(effect, WorldSessionEffect::TickAdvanced { ticks_elapsed: 1 });
        assert_eq!(session.current_tick(), 1);
        assert_eq!(session.cap_current(), 95.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 10);
    }

    #[test]
    fn module_activation_updates_loadout_state_and_resets_cycle() {
        let (mut session, mut loadout) = setup();
        loadout = Some(PlayerLoadoutMsg {
            modules: vec![module(7, false)],
            ..PlayerLoadoutMsg::default()
        });

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                module_id: 7,
                active: true,
                forced_reason: String::new(),
            })
            .unwrap();

        let row = &loadout.as_ref().unwrap().modules[0];
        assert!(row.is_active);
        assert_eq!(row.cycle_remaining, 0);
    }

    #[test]
    fn system_change_updates_only_the_player_system() {
        let (mut session, mut loadout) = setup();
        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::SystemChanged {
                ship_id: 1,
                to_system: 2,
            })
            .unwrap();

        assert_eq!(
            effect,
            WorldSessionEffect::SystemChanged {
                name: Some("Beta".to_owned())
            }
        );
        assert_eq!(session.current_system_name(), "Beta");
    }

    #[test]
    fn initial_state_resets_old_state_before_registering_new_ships() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::TargetLocked {
                locker_id: 1,
                target_id: 2,
            })
            .unwrap();

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::InitialState {
                navigation: NavigationInput {
                    system_name: "Gamma".to_owned(),
                    ..NavigationInput::default()
                },
                ships: vec![ship(3, true)],
                connection_ship_id: 3,
            })
            .unwrap();

        assert_eq!(session.current_system_name(), "Gamma");
        assert_eq!(session.player_ship_id(), 3);
        assert_eq!(session.player_lock_target(), -1);
        assert_eq!(session.event_count(), 0);
        assert_eq!(session.ship_count(), 1);
    }

    #[test]
    fn ship_spawn_and_destroy_lifecycle_is_reported_by_effects() {
        let (mut session, mut loadout) = setup();
        assert_eq!(
            ClientState::new(&mut session, &mut loadout)
                .apply(ClientFact::ShipSpawned {
                    ship_id: 4,
                    connection_ship_id: 1,
                })
                .unwrap(),
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: false,
            }
        );
        assert!(session.has_ship(4));

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipDestroyed { ship_id: 4 })
            .unwrap();
        let WorldSessionEffect::ShipDestroyed(outcome) = effect else {
            panic!("expected destruction effect");
        };
        assert!(outcome.destroyed);
        assert!(!session.has_ship(4));
    }
"""
if not text.endswith("\n}"):
    raise SystemExit("client_state module terminator missing")
text = text[:-2] + tests + "\n}\n"
path.write_text(text)

# Keep architecture docs aligned with explicit ownership and full deletion.
replace(
    "docs/adr/ADR-0046-world-session-state-ownership.md",
    "world eventは最終`_handle_*` callbackへ直接dispatchし、以前の\n`ServerEventOutcome`生成→signal→再dispatchという二段経路は実行時から削除した。\n旧GDScript型注釈を段階的に外す間だけ、生成されないparse compatibility classを残す。\n",
    "world eventはGDScriptが明示したscene ownerの最終`_handle_*` callbackへ直接dispatchする。\nRust adapterは`get_parent()`などでscene tree構造を推測しない。以前の\n`ServerEventOutcome`生成→signal→再dispatchという二段経路と互換classは削除した。\n",
)
replace(
    "docs/architecture/architecture-review/client.md",
    "| デッドコード | A | `ClientOutcome` mirrorを削除。`ServerEventOutcome`は旧GDScript型注釈用の生成されない互換classのみ |",
    "| デッドコード | A | `ClientOutcome` mirrorと旧`ServerEventOutcome`互換classを削除 |",
)
replace(
    "docs/architecture/architecture-review/client.md",
    "- `ServerMessageOutcome::dispatch`: state commit後に一度だけpresentationを渡す境界",
    "- `ServerMessageOutcome::dispatch`: GDScriptが明示したconnection/world targetへ、state commit後に一度だけpresentationを渡す境界",
)
