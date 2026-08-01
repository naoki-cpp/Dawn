from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: str, old: str, new: str, expected: int = 1) -> None:
    file_path = ROOT / path
    text = file_path.read_text()
    count = text.count(old)
    if count != expected:
        raise RuntimeError(f"{path}: expected {expected} matches, found {count}: {old[:80]!r}")
    file_path.write_text(text.replace(old, new))


def find_matching_paren(text: str, open_index: int) -> int:
    depth = 0
    state = "normal"
    i = open_index
    while i < len(text):
        ch = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if state == "normal":
            if ch == '"':
                state = "string"
            elif ch == "'":
                state = "char"
            elif ch == "/" and nxt == "/":
                state = "line_comment"
                i += 1
            elif ch == "/" and nxt == "*":
                state = "block_comment"
                i += 1
            elif ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
                if depth == 0:
                    return i
        elif state == "string":
            if ch == "\\":
                i += 1
            elif ch == '"':
                state = "normal"
        elif state == "char":
            if ch == "\\":
                i += 1
            elif ch == "'":
                state = "normal"
        elif state == "line_comment":
            if ch == "\n":
                state = "normal"
        elif state == "block_comment":
            if ch == "*" and nxt == "/":
                state = "normal"
                i += 1
        i += 1
    raise RuntimeError("unmatched parenthesis")


def split_top_level_args(text: str) -> list[str]:
    args: list[str] = []
    start = 0
    depth = 0
    state = "normal"
    i = 0
    while i < len(text):
        ch = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if state == "normal":
            if ch == '"':
                state = "string"
            elif ch == "'":
                state = "char"
            elif ch == "/" and nxt == "/":
                state = "line_comment"
                i += 1
            elif ch == "/" and nxt == "*":
                state = "block_comment"
                i += 1
            elif ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
            elif ch == "," and depth == 0:
                args.append(text[start:i].strip())
                start = i + 1
        elif state == "string":
            if ch == "\\":
                i += 1
            elif ch == '"':
                state = "normal"
        elif state == "char":
            if ch == "\\":
                i += 1
            elif ch == "'":
                state = "normal"
        elif state == "line_comment":
            if ch == "\n":
                state = "normal"
        elif state == "block_comment":
            if ch == "*" and nxt == "/":
                state = "normal"
                i += 1
        i += 1
    tail = text[start:].strip()
    if tail:
        args.append(tail)
    return args


def demo_topology_expr(path: Path) -> str:
    relative = path.relative_to(ROOT).as_posix()
    if relative.startswith("crates/dawn-sector/") and relative != "crates/dawn-sector/src/lib.rs":
        return "std::sync::Arc::new(crate::galaxy::Galaxy::demo())"
    return "std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo())"


def rewrite_calls(path: Path, needle: str, expected_args: int, insert_at: int) -> int:
    text = path.read_text()
    pos = 0
    changed = 0
    while True:
        index = text.find(needle, pos)
        if index < 0:
            break
        open_index = index + len(needle) - 1
        close_index = find_matching_paren(text, open_index)
        args = split_top_level_args(text[open_index + 1 : close_index])
        if len(args) == expected_args:
            args.insert(insert_at, demo_topology_expr(path))
            replacement = needle[:-1] + "(" + ", ".join(args) + ")"
            text = text[:index] + replacement + text[close_index + 1 :]
            pos = index + len(replacement)
            changed += 1
        else:
            pos = close_index + 1
    if changed:
        path.write_text(text)
    return changed


# Production serve wiring: load the authoritative topology before construction.
replace_exact(
    "crates/dawn-simulation/src/serve/mod.rs",
    """    let mut node = SimulationNode::new(id, sector, bounds);\n    node.set_population_cap(pop_cap);\n    let star_map = Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)\n        .unwrap_or_else(|e| panic!(\"failed to load production galaxy map: {e}\"));\n    node.set_galaxy(std::sync::Arc::new(star_map));\n""",
    """    let galaxy = std::sync::Arc::new(\n        Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)\n            .unwrap_or_else(|e| panic!(\"failed to load production galaxy map: {e}\")),\n    );\n    let mut node = SimulationNode::new(id, sector, bounds, galaxy);\n    node.set_population_cap(pop_cap);\n""",
)
replace_exact(
    "crates/dawn-simulation/src/serve/mod.rs",
    """        let mut node = SimulationNode::new(id, sector, bounds);\n        node.set_population_cap(pop_cap);\n        node.set_galaxy(std::sync::Arc::new(Galaxy::demo()));\n""",
    """        let mut node = SimulationNode::new(\n            id,\n            sector,\n            bounds,\n            std::sync::Arc::new(Galaxy::demo()),\n        );\n        node.set_population_cap(pop_cap);\n""",
)

# Production Sector Node wiring: one topology value feeds both fresh and restore paths.
replace_exact(
    "crates/dawn-sector-node/src/main.rs",
    """    let catalog = runtime_catalog()\n        .unwrap_or_else(|error| panic!(\"failed to load required game-data catalog: {error}\"));\n\n""",
    """    let catalog = runtime_catalog()\n        .unwrap_or_else(|error| panic!(\"failed to load required game-data catalog: {error}\"));\n    let galaxy = Arc::new(\n        Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)\n            .unwrap_or_else(|e| panic!(\"failed to load production galaxy map: {e}\")),\n    );\n\n""",
)
replace_exact(
    "crates/dawn-sector-node/src/main.rs",
    """                SimulationNode::restore_from(\n                    store,\n                    &snapshot,\n                    catalog.modules(),\n                    catalog.ship_types(),\n                ),\n""",
    """                SimulationNode::restore_from(\n                    store,\n                    &snapshot,\n                    Arc::clone(&galaxy),\n                    catalog.modules(),\n                    catalog.ship_types(),\n                ),\n""",
)
replace_exact(
    "crates/dawn-sector-node/src/main.rs",
    """            let mut node = SimulationNode::with_store(node_id, sector_id, bounds, store);\n""",
    """            let mut node = SimulationNode::with_store(\n                node_id,\n                sector_id,\n                bounds,\n                Arc::clone(&galaxy),\n                store,\n            );\n""",
)
replace_exact(
    "crates/dawn-sector-node/src/main.rs",
    """    node.set_population_cap(cfg.pop_cap);\n    let star_map = Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)\n        .unwrap_or_else(|e| panic!(\"failed to load production galaxy map: {e}\"));\n    node.set_galaxy(Arc::new(star_map));\n""",
    """    node.set_population_cap(cfg.pop_cap);\n""",
)

# Constructor API and restore contract.
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    pub fn new(node_id: NodeId, sector_id: SectorId, bounds: SectorBounds) -> Self {\n        Self::with_store(node_id, sector_id, bounds, InMemoryEventStore::new())\n    }\n""",
    """    pub fn new(\n        node_id: NodeId,\n        sector_id: SectorId,\n        bounds: SectorBounds,\n        galaxy: Arc<crate::galaxy::Galaxy>,\n    ) -> Self {\n        Self::with_store(\n            node_id,\n            sector_id,\n            bounds,\n            galaxy,\n            InMemoryEventStore::new(),\n        )\n    }\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    pub fn with_store(\n        node_id: NodeId,\n        sector_id: SectorId,\n        bounds: SectorBounds,\n        store: S,\n    ) -> Self {\n        let galaxy = Arc::new(crate::galaxy::Galaxy::demo());\n        let sector_map = SectorMap::from_galaxy(sector_id, Arc::clone(&galaxy));\n""",
    """    pub fn with_store(\n        node_id: NodeId,\n        sector_id: SectorId,\n        bounds: SectorBounds,\n        galaxy: Arc<crate::galaxy::Galaxy>,\n        store: S,\n    ) -> Self {\n        let sector_map = SectorMap::from_galaxy(sector_id, Arc::clone(&galaxy));\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    pub fn restore_from(\n        store: S,\n        snapshot: &StateSnapshot,\n        modules: &[ModuleDefinition],\n        ship_types: &[ShipTypeDefinition],\n    ) -> Self {\n""",
    """    pub fn restore_from(\n        store: S,\n        snapshot: &StateSnapshot,\n        galaxy: Arc<crate::galaxy::Galaxy>,\n        modules: &[ModuleDefinition],\n        ship_types: &[ShipTypeDefinition],\n    ) -> Self {\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """        let mut node =\n            Self::with_store(snapshot.node_id, snapshot.sector_id, snapshot.bounds, store);\n""",
    """        let mut node = Self::with_store(\n            snapshot.node_id,\n            snapshot.sector_id,\n            snapshot.bounds,\n            galaxy,\n            store,\n        );\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    /// `modules` and `ship_types` must come from the same\n    /// [`crate::game_data::GameDataCatalog`] used to configure the node, for\n""",
    """    /// `galaxy` is the authoritative topology for the restored process.\n    /// `modules` and `ship_types` must come from the same\n    /// [`crate::game_data::GameDataCatalog`] used to configure the node, for\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    /// `restore_from` default to. Production wiring (`dawn-sector-node`'s\n    /// `build_node`) calls this once after construction, mirroring\n    /// `set_galaxy`'s \"construct generically, configure production specifics\n    /// afterward\" shape. Replaces the cache too, since it would otherwise\n""",
    """    /// `restore_from` default to. Production wiring (`dawn-sector-node`'s\n    /// `build_node`) calls this once after construction because the database\n    /// path is process-local; topology is already fixed by construction.\n    /// Replaces the cache too, since it would otherwise\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """    /// Replace the navigation topology. Rebuilds this Sector's gates, bodies,\n    /// stations, and the shared body-anchor table from the same `Galaxy` value.\n    pub fn set_galaxy(&mut self, galaxy: Arc<crate::galaxy::Galaxy>) {\n        let anchor_table = crate::anchor::AnchorTable::from_galaxy(&galaxy);\n        let sector_map = SectorMap::from_galaxy(self.sector_id, galaxy);\n        self.sector_map = sector_map;\n        self.anchor_table = anchor_table;\n    }\n\n""",
    "",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    "fn set_galaxy_rebuilds_all_sector_projections_and_anchors_from_one_value()",
    "fn construction_builds_all_sector_projections_and_anchors_from_supplied_topology()",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """        let mut node = SimulationNode::new(\n            NodeId(7),\n            sector_id,\n            SectorBounds::centered(SectorBounds::DEFAULT_HALF),\n        );\n        node.set_galaxy(Arc::clone(&galaxy));\n""",
    """        let node = SimulationNode::new(\n            NodeId(7),\n            sector_id,\n            SectorBounds::centered(SectorBounds::DEFAULT_HALF),\n            Arc::clone(&galaxy),\n        );\n""",
)
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    """        assert!(node.anchor_table.abs(dawn_core::AnchorId(0)).is_none());\n    }\n""",
    """        assert!(node.anchor_table.abs(dawn_core::AnchorId(0)).is_none());\n\n        let snapshot = node.take_snapshot();\n        let restored = SimulationNode::restore_from(\n            InMemoryEventStore::new(),\n            &snapshot,\n            Arc::clone(&galaxy),\n            &[],\n            &[],\n        );\n        assert!(Arc::ptr_eq(&restored.sector_map.galaxy, &galaxy));\n        assert_eq!(restored.sector_map.gates, node.sector_map.gates);\n        assert_eq!(restored.sector_map.bodies, node.sector_map.bodies);\n        assert_eq!(restored.sector_map.stations, node.sector_map.stations);\n        for body in &galaxy.bodies {\n            assert_eq!(\n                restored\n                    .anchor_table\n                    .abs(dawn_core::AnchorId::from(body.id)),\n                Some(body.abs_m)\n            );\n        }\n    }\n""",
)

# Public documentation examples.
replace_exact(
    "crates/dawn-sector/src/node/mod.rs",
    "SimulationNode::restore_from(store, &snapshot, &modules, &ship_types)",
    "SimulationNode::restore_from(store, &snapshot, galaxy, &modules, &ship_types)",
)
replace_exact(
    "crates/dawn-sector/src/persistence/snapshot.rs",
    "SimulationNode::restore_from(store, &snapshot, &modules, &ship_types)",
    "SimulationNode::restore_from(store, &snapshot, galaxy, &modules, &ship_types)",
)

# Update every remaining fixture/caller. Production call sites above already
# have the new arity and are skipped by the expected-argument checks.
new_count = 0
with_store_count = 0
restore_count = 0
for path in sorted((ROOT / "crates").rglob("*.rs")):
    new_count += rewrite_calls(path, "SimulationNode::new(", 3, 3)
    with_store_count += rewrite_calls(path, "SimulationNode::with_store(", 4, 3)
    restore_count += rewrite_calls(path, "SimulationNode::restore_from(", 4, 2)

if new_count < 20:
    raise RuntimeError(f"unexpectedly few SimulationNode::new rewrites: {new_count}")
if with_store_count < 3:
    raise RuntimeError(f"unexpectedly few SimulationNode::with_store rewrites: {with_store_count}")
if restore_count < 10:
    raise RuntimeError(f"unexpectedly few SimulationNode::restore_from rewrites: {restore_count}")

print(
    f"rewrote {new_count} new calls, {with_store_count} with_store calls, "
    f"and {restore_count} restore_from calls"
)
