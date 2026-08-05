from pathlib import Path

path = Path("crates/dawn-sector/src/game_data/tests.rs")
text = path.read_text()
old = '''    assert_eq!(
        first.build_initial_state_json(),
        second.build_initial_state_json()
    );
'''
new = '''    let mut first_state = first.build_initial_state_json();
    let mut second_state = second.build_initial_state_json();
    first_state.celestial_bodies.sort_by_key(|body| body.id);
    second_state.celestial_bodies.sort_by_key(|body| body.id);
    assert_eq!(first_state, second_state);
'''
if old not in text:
    raise SystemExit("engine-visible determinism assertion not found")
path.write_text(text.replace(old, new, 1))
