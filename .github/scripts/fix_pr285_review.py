from pathlib import Path

ROOT = Path('.')

# 1) Keep the production constructor API present under cargo test, and make
# test conveniences explicit wrappers around that API.
node_path = ROOT / 'crates/dawn-sector/src/node/mod.rs'
node = node_path.read_text()
start = node.index('// -- Constructors ------------------------------------------------------------')
end = node.index('    fn finish_restore', start)
finish_end = node.index('    // -- Population backstop', end)
new_block = '''// -- Constructors ------------------------------------------------------------

impl SimulationNode<InMemoryEventStore> {
    /// Create a node backed by an in-memory event store.
    pub fn new(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        Self::with_catalog_and_store(
            node_id,
            sector_id,
            bounds,
            galaxy,
            catalog,
            InMemoryEventStore::new(),
        )
    }

    /// Test fixture constructor using the complete validated repository catalog.
    #[cfg(test)]
    pub(crate) fn new_test(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
    ) -> Self {
        Self::new(
            node_id,
            sector_id,
            bounds,
            galaxy,
            crate::game_data::test_catalog_arc(),
        )
    }
}

impl<S: EventStore> SimulationNode<S> {
    /// Create a node with a caller-supplied event store and validated catalog.
    pub fn with_store(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
        store: S,
    ) -> Self {
        Self::with_catalog_and_store(node_id, sector_id, bounds, galaxy, catalog, store)
    }

    /// Test fixture constructor using the complete validated repository catalog.
    #[cfg(test)]
    pub(crate) fn with_test_store(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        store: S,
    ) -> Self {
        Self::with_store(
            node_id,
            sector_id,
            bounds,
            galaxy,
            crate::game_data::test_catalog_arc(),
            store,
        )
    }

    fn with_catalog_and_store(
        node_id: NodeId,
        sector_id: SectorId,
        bounds: SectorBounds,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
        store: S,
    ) -> Self {
        let sector_map = SectorMap::from_galaxy(sector_id, Arc::clone(&galaxy));
        let anchor_table = crate::anchor::AnchorTable::from_galaxy(&galaxy);

        Self {
            node_id,
            sector_id,
            bounds,
            world: SimWorld::new(sector_id),
            event_store: store,
            current_tick: Tick::ZERO,
            id_counter: 0,
            ships: ShipRegistry::new(),
            module_registry: catalog.module_index(),
            ship_type_registry: catalog.ship_type_index(),
            base_stats: HashMap::new(),
            player_id_counter: 0,
            pending_fresh_admissions: HashSet::new(),
            pending_resume_admissions: HashMap::new(),
            pending_bot_lock_commands: Vec::new(),
            sector_map,
            anchor_table,
            population_cap: POPULATION_CAP,
            station_inventory_db: station_inventory_db::StationInventoryDb::open_in_memory()
                .expect("in-memory sqlite connection never fails to open"),
            station_inventory_cache: std::cell::RefCell::new(
                station_inventory::StationInventoryCache::new(),
            ),
            docked_ships: BTreeMap::new(),
            docked_players: BTreeMap::new(),
            pending_auto_jumps: Vec::new(),
            completed_warps: Vec::new(),
            completed_incoming_transits: Vec::new(),
        }
    }

    /// Restore a node from a snapshot plus its event tail using the exact
    /// validated catalog selected by the runtime.
    pub fn restore_from(
        store: S,
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        catalog: Arc<GameDataCatalog>,
    ) -> Self {
        let node = Self::with_catalog_and_store(
            snapshot.node_id,
            snapshot.sector_id,
            snapshot.bounds,
            galaxy,
            catalog,
            store,
        );
        Self::finish_restore(node, snapshot)
    }

    /// Test restore fixture. Overrides are folded into a complete catalog and
    /// validated before the authoritative engine is constructed.
    #[cfg(test)]
    pub(crate) fn restore_from_test(
        store: S,
        snapshot: &StateSnapshot,
        galaxy: Arc<crate::galaxy::Galaxy>,
        modules: &[ModuleDefinition],
        ship_types: &[ShipTypeDefinition],
    ) -> Self {
        Self::restore_from(
            store,
            snapshot,
            galaxy,
            crate::game_data::test_catalog_with_overrides(modules, ship_types),
        )
    }

'''
finish_block = node[end:finish_end]
node_path.write_text(node[:start] + new_block + finish_block + node[finish_end:])

# 2) Add explicit validated test-catalog builders.
game_data_path = ROOT / 'crates/dawn-sector/src/game_data/mod.rs'
game_data = game_data_path.read_text()
needle = '''#[cfg(test)]
pub(crate) fn test_catalog() -> &'static GameDataCatalog {
    static TEST_CATALOG: OnceLock<GameDataCatalog> = OnceLock::new();
    TEST_CATALOG.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        GameDataCatalog::load_from_paths(
            root.join(PRODUCTION_MODULES_PATH),
            root.join(PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog")
    })
}
'''
addition = needle + '''
#[cfg(test)]
pub(crate) fn test_catalog_arc() -> Arc<GameDataCatalog> {
    Arc::new(test_catalog().clone())
}

#[cfg(test)]
pub(crate) fn test_catalog_with_overrides(
    module_overrides: &[ModuleDefinition],
    ship_type_overrides: &[ShipTypeDefinition],
) -> Arc<GameDataCatalog> {
    let mut modules = test_catalog().modules().to_vec();
    for definition in module_overrides {
        match modules.iter_mut().find(|item| item.id == definition.id) {
            Some(existing) => *existing = definition.clone(),
            None => modules.push(definition.clone()),
        }
    }

    let mut ship_types = test_catalog().ship_types().to_vec();
    for definition in ship_type_overrides {
        match ship_types.iter_mut().find(|item| item.id == definition.id) {
            Some(existing) => *existing = definition.clone(),
            None => ship_types.push(definition.clone()),
        }
    }

    Arc::new(
        GameDataCatalog::from_definitions(modules, ship_types)
            .expect("test catalog overrides must remain complete and valid"),
    )
}
'''
if needle not in game_data:
    raise SystemExit('test_catalog block not found')
game_data_path.write_text(game_data.replace(needle, addition, 1))

# 3) Move dawn-sector crate-unit tests to explicit fixture methods.
sector_src = ROOT / 'crates/dawn-sector/src'
for path in sector_src.rglob('*.rs'):
    if path in {node_path, sector_src / 'lib.rs', sector_src / 'game_data/tests.rs'}:
        continue
    text = path.read_text()
    text = text.replace('SimulationNode::new(', 'SimulationNode::new_test(')
    text = text.replace('SimulationNode::with_store(', 'SimulationNode::with_test_store(')
    text = text.replace('SimulationNode::restore_from(', 'SimulationNode::restore_from_test(')
    path.write_text(text)

node = node_path.read_text()
marker = '    // -- Population backstop'
head, tail = node.split(marker, 1)
tail = tail.replace('SimulationNode::new(', 'SimulationNode::new_test(')
tail = tail.replace('SimulationNode::with_store(', 'SimulationNode::with_test_store(')
tail = tail.replace('SimulationNode::restore_from(', 'SimulationNode::restore_from_test(')
node_path.write_text(head + marker + tail)

# 4) Load serve dependencies once per composition root and share their Arcs.
serve_mod = ROOT / 'crates/dawn-simulation/src/serve/mod.rs'
text = serve_mod.read_text()
function_start = text.index('pub(crate) fn build_serve_node(')
function_end = text.index('// ── Integration tests', function_start)
new_functions = '''pub(crate) fn load_serve_dependencies(
) -> (std::sync::Arc<Galaxy>, std::sync::Arc<GameDataCatalog>) {
    let galaxy = std::sync::Arc::new(
        Galaxy::load_from_file(PRODUCTION_GALAXY_PATH)
            .unwrap_or_else(|error| panic!("failed to load production galaxy map: {error}")),
    );
    let catalog = std::sync::Arc::new(
        GameDataCatalog::load_runtime()
            .unwrap_or_else(|error| panic!("failed to load required game-data catalog: {error}")),
    );
    (galaxy, catalog)
}

/// Build a `SimulationNode` from dependencies owned by the serve composition root.
pub(crate) fn build_serve_node(
    id: NodeId,
    sector: SectorId,
    bounds: SectorBounds,
    pop_cap: usize,
    galaxy: std::sync::Arc<Galaxy>,
    catalog: std::sync::Arc<GameDataCatalog>,
) -> SimulationNode {
    let mut node = SimulationNode::new(id, sector, bounds, galaxy, catalog);
    node.set_population_cap(pop_cap);
    node
}

'''
serve_mod.write_text(text[:function_start] + new_functions + text[function_end:])

single = ROOT / 'crates/dawn-simulation/src/serve/single.rs'
text = single.read_text()
text = text.replace(
    '    build_serve_node, market::MarketRuntime, AoiDelivery, DuelMetrics, AOI_CELL_SIZE, P4_TICK_MS,',
    '    build_serve_node, load_serve_dependencies, market::MarketRuntime, AoiDelivery, DuelMetrics,\n    AOI_CELL_SIZE, P4_TICK_MS,',
    1,
)
text = text.replace(
    '    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);\n    let mut node = build_serve_node(NodeId(0), SectorId(0), bounds, pop_cap);',
    '    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);\n    let (galaxy, catalog) = load_serve_dependencies();\n    let mut node = build_serve_node(\n        NodeId(0),\n        SectorId(0),\n        bounds,\n        pop_cap,\n        galaxy,\n        catalog,\n    );',
    1,
)
single.write_text(text)

cluster = ROOT / 'crates/dawn-simulation/src/serve/cluster.rs'
text = cluster.read_text()
text = text.replace(
    '    build_serve_node, market::MarketRuntime, runtime, AoiDelivery, AOI_CELL_SIZE, P4_TICK_MS,',
    '    build_serve_node, load_serve_dependencies, market::MarketRuntime, runtime, AoiDelivery,\n    AOI_CELL_SIZE, P4_TICK_MS,',
    1,
)
old_nodes = '''    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let mut nodes: Vec<SimulationNode> = ids
        .iter()
        .map(|&id| build_serve_node(id, SectorId(id.0), bounds, pop_cap))
        .collect();
'''
new_nodes = '''    let bounds = SectorBounds::centered(SectorBounds::DEFAULT_HALF);
    let (galaxy, catalog) = load_serve_dependencies();
    let mut nodes: Vec<SimulationNode> = ids
        .iter()
        .map(|&id| {
            build_serve_node(
                id,
                SectorId(id.0),
                bounds,
                pop_cap,
                std::sync::Arc::clone(&galaxy),
                std::sync::Arc::clone(&catalog),
            )
        })
        .collect();
'''
if old_nodes not in text:
    raise SystemExit('cluster node construction block not found')
cluster.write_text(text.replace(old_nodes, new_nodes, 1))

# 5) Extend determinism coverage through engine-visible behavior.
tests_path = ROOT / 'crates/dawn-sector/src/game_data/tests.rs'
tests = tests_path.read_text()
engine_test = r'''

#[test]
fn definition_order_does_not_change_engine_visible_initial_state() {
    let baseline = test_catalog();
    let mut modules = baseline.modules().to_vec();
    let mut ship_types = baseline.ship_types().to_vec();
    modules.reverse();
    ship_types.reverse();

    let reordered = std::sync::Arc::new(
        GameDataCatalog::from_definitions(modules, ship_types)
            .expect("reordered definitions remain a valid complete catalog"),
    );
    let baseline = std::sync::Arc::new(baseline.clone());
    let galaxy = std::sync::Arc::new(crate::galaxy::Galaxy::demo());
    let bounds = dawn_core::SectorBounds::centered(dawn_core::SectorBounds::DEFAULT_HALF);

    let mut first = crate::node::SimulationNode::new(
        dawn_core::NodeId(0),
        dawn_core::SectorId(0),
        bounds,
        std::sync::Arc::clone(&galaxy),
        baseline,
    );
    let mut second = crate::node::SimulationNode::new(
        dawn_core::NodeId(0),
        dawn_core::SectorId(0),
        bounds,
        galaxy,
        reordered,
    );

    let first_ship = first.spawn_ship(
        crate::ship_types::SHIP_TYPE_MAGPIE,
        dawn_core::Position::ORIGIN,
        dawn_core::Velocity::ZERO,
    );
    let second_ship = second.spawn_ship(
        crate::ship_types::SHIP_TYPE_MAGPIE,
        dawn_core::Position::ORIGIN,
        dawn_core::Velocity::ZERO,
    );
    assert_eq!(first_ship, second_ship);

    let first_fitted = first.fit_module(dawn_core::FitModuleCommand {
        ship_id: first_ship,
        slot: dawn_core::SlotKind::High,
        module_id: crate::modules::MODULE_RAILGUN_SMALL,
    });
    let second_fitted = second.fit_module(dawn_core::FitModuleCommand {
        ship_id: second_ship,
        slot: dawn_core::SlotKind::High,
        module_id: crate::modules::MODULE_RAILGUN_SMALL,
    });
    assert!(first_fitted && second_fitted);

    assert_eq!(
        first.build_initial_state_json(),
        second.build_initial_state_json()
    );
}
'''
if 'fn definition_order_does_not_change_engine_visible_initial_state()' not in tests:
    tests_path.write_text(tests + engine_test)

# 6) Keep architecture text accurate.
doc_path = ROOT / 'docs/architecture/game-data-catalog.md'
doc = doc_path.read_text()
doc = doc.replace(
    'Inside `dawn-sector`, crate-unit tests use one crate-private complete validated fixture;\n'
    'they do not mutate definitions after constructing the node.',
    'Inside `dawn-sector`, crate-unit tests use crate-private fixture wrappers that\n'
    'delegate to the same catalog-requiring constructors as production. Focused\n'
    'restore overrides are folded into a complete catalog and revalidated before\n'
    'the node is constructed; definitions are never mutated after construction.',
)
doc = doc.replace(
    'Tests reverse the complete input vectors and verify that iteration\n'
    'order and lookup results remain identical.',
    'Tests reverse the complete input vectors and verify that iteration order,\n'
    'lookup results, and engine-visible initial-state behavior remain identical.',
)
doc_path.write_text(doc)

node = node_path.read_text()
for forbidden in [
    '#[cfg(not(test))]\n    pub fn new',
    '#[cfg(not(test))]\n    pub fn with_store',
    '#[cfg(not(test))]\n    pub fn restore_from',
    'Arc::make_mut(&mut node.module_registry)',
    'Arc::make_mut(&mut node.ship_type_registry)',
]:
    if forbidden in node:
        raise SystemExit(f'forbidden pattern remains: {forbidden}')

print('PR #285 review fixes applied')
