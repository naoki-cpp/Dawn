use dawn_core::{
    CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId,
    Position, SectorId, StarSystemDef, StarSystemId,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct StarMapFile {
    #[serde(default)] star_systems    : Vec<StarSystemEntry>,
    #[serde(default)] jump_gates      : Vec<JumpGateEntry>,
    #[serde(default)] celestial_bodies: Vec<CelestialBodyEntry>,
}

#[derive(Deserialize)]
struct StarSystemEntry {
    id     : u32,
    name   : String,
    sectors: Vec<u8>,
}

#[derive(Deserialize)]
struct JumpGateEntry {
    id               : u32,
    from_sector      : u8,
    to_sector        : u8,
    position         : [f32; 3],
    activation_radius: f32,
}

#[derive(Deserialize)]
struct CelestialBodyEntry {
    id           : u32,
    kind         : String,
    name         : String,
    position     : [f32; 3],
    radius       : f32,
    #[serde(default)] spectral_type: f32,
}

fn parse_body_kind(s: &str) -> CelestialBodyKind {
    match s { "Star" => CelestialBodyKind::Star, _ => CelestialBodyKind::Planet }
}

fn entry_to_system(e: StarSystemEntry) -> StarSystemDef {
    StarSystemDef {
        id     : StarSystemId(e.id),
        name   : e.name,
        sectors: e.sectors.into_iter().map(SectorId).collect(),
    }
}

fn entry_to_gate(e: JumpGateEntry) -> JumpGateDef {
    JumpGateDef {
        id               : JumpGateId(e.id),
        from_sector      : SectorId(e.from_sector),
        to_sector        : SectorId(e.to_sector),
        position         : Position::new(e.position[0], e.position[1], e.position[2]),
        activation_radius: e.activation_radius,
    }
}

fn entry_to_body(e: CelestialBodyEntry) -> CelestialBodyDef {
    CelestialBodyDef {
        id           : CelestialBodyId(e.id),
        kind         : parse_body_kind(&e.kind),
        name         : e.name,
        position     : Position::new(e.position[0], e.position[1], e.position[2]),
        radius       : e.radius,
        spectral_type: e.spectral_type,
    }
}

/// Load the star map from a TOML file. Falls back to `fallback` if the file
/// is absent or cannot be parsed.
pub fn load_star_map(path: &str, fallback: dawn_sector::star_map::StarMap) -> dawn_sector::star_map::StarMap {
    let content = match std::fs::read_to_string(path) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[DataLoader] '{}' not found ({}), using built-in star map.", path, e);
            return fallback;
        }
    };

    match toml::from_str::<StarMapFile>(&content) {
        Ok(f) => {
            let systems = f.star_systems.into_iter().map(entry_to_system).collect::<Vec<_>>();
            let gates   = f.jump_gates.into_iter().map(entry_to_gate).collect::<Vec<_>>();
            let bodies  = f.celestial_bodies.into_iter().map(entry_to_body).collect::<Vec<_>>();
            println!("[DataLoader] loaded star map from '{}': {} systems, {} gates, {} bodies.",
                path, systems.len(), gates.len(), bodies.len());
            dawn_sector::star_map::StarMap::new(systems, gates, bodies)
        }
        Err(e) => {
            eprintln!("[DataLoader] parse error in '{}': {}, using built-in star map.", path, e);
            fallback
        }
    }
}
