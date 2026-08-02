from pathlib import Path

session_path = Path("crates/dawn-client-core/src/world_session.rs")
text = session_path.read_text()
old = '''    fn simulate_cap(&mut self, ticks: i64, loadout: Option<&mut PlayerLoadoutMsg>) {
        if self.cap_current < 0.0 || self.player_ship_id < 0 {
            return;
        }
        let ticks = u32::try_from(ticks).unwrap_or(0);
'''
new = '''    fn simulate_cap(&mut self, ticks: i64, loadout: Option<&mut PlayerLoadoutMsg>) {
        if self.cap_current < 0.0 || self.player_ship_id < 0 {
            return;
        }
        if loadout.as_ref().is_some_and(|loadout| {
            loadout
                .active_ship_id
                .and_then(|ship_id| i64::try_from(ship_id).ok())
                != Some(self.player_ship_id)
        }) {
            return;
        }
        let ticks = u32::try_from(ticks).unwrap_or(0);
'''
assert text.count(old) == 1, "simulate_cap anchor changed"
session_path.write_text(text.replace(old, new))

client_path = Path("crates/dawn-client-core/src/client_state.rs")
text = client_path.read_text()
marker = '''    #[test]
    fn unknown_active_ship_switch_completes_when_ship_enters() {
'''
test = '''    #[test]
    fn pending_active_ship_switch_pauses_capacitor_until_registration() {
        let (mut session, mut loadout) = setup();
        let mut active_module = module(7, true);
        active_module.cycle_remaining = 0;
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(33),
                modules: vec![active_module],
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 1);
        assert_eq!(session.cap_current(), 100.0);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 1 })
            .unwrap();
        assert_eq!(session.current_tick(), 1);
        assert_eq!(session.cap_current(), 100.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 0);

        session.advance_client_ticks(1, loadout.as_mut());
        assert_eq!(session.current_tick(), 2);
        assert_eq!(session.cap_current(), 100.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 0);

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipEntered {
                ship: ship(33, true),
                connection_ship_id: 1,
            })
            .unwrap();
        assert_eq!(
            effect,
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: true,
            }
        );
        assert_eq!(session.player_ship_id(), 33);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 3 })
            .unwrap();
        assert_eq!(session.cap_current(), 95.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 10);
    }

'''
assert text.count(marker) == 1, "test insertion anchor changed"
client_path.write_text(text.replace(marker, test + marker))
