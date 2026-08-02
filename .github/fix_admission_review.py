from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one match, found {count}: {old[:100]!r}")
    p.write_text(text.replace(old, new, 1))


def insert_before(path: str, marker: str, addition: str) -> None:
    replace_once(path, marker, addition + marker)


# ── Composite fresh-admission commit event ───────────────────────────────────
replace_once(
    "crates/dawn-core/src/events.rs",
    """    ClientAdmissionIdentityReserved(ClientAdmissionIdentityReserved),
}""",
    """    ClientAdmissionIdentityReserved(ClientAdmissionIdentityReserved),

    /// A fresh client admission committed its complete starter state.
    /// Replay materializes the Ship, fitting/cargo projection, ownership, and
    /// idempotent Station-inventory grant from this single durable fact.
    ClientAdmissionCommitted(ClientAdmissionCommitted),
}""",
)
replace_once(
    "crates/dawn-core/src/events.rs",
    """            Self::ClientAdmissionIdentityReserved(e) => e.ship_id,
        }""",
    """            Self::ClientAdmissionIdentityReserved(e) => e.ship_id,
            Self::ClientAdmissionCommitted(e) => e.ship_id,
        }""",
)
replace_once(
    "crates/dawn-core/src/events.rs",
    """            Self::ClientAdmissionIdentityReserved(e) => e.tick,
        }""",
    """            Self::ClientAdmissionIdentityReserved(e) => e.tick,
            Self::ClientAdmissionCommitted(e) => e.tick,
        }""",
)
insert_before(
    "crates/dawn-core/src/events.rs",
    "// ── VelocityChanged",
    """/// Atomic, replay-complete starter state for one successful fresh admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientAdmissionCommitted {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub sector_id: SectorId,
    pub initial_position: AbsolutePosition,
    pub ship_type_id: ShipTypeId,
    pub fitting: FittingSnapshot,
    pub inventory: Vec<ItemId>,
    pub starter_station_id: StationId,
    pub starter_item_id: ItemId,
    pub starter_item_count: u64,
    pub tick: Tick,
}

""",
)

# Replace the provisional-admission implementation as one coherent unit.
Path("crates/dawn-sector/src/node/admission_provisional.rs").write_text(r'''//! Non-durable fresh-admission preview and atomic commit materialization.
//!
//! Begin records only an identity watermark. Commit appends one replay-complete
//! `ClientAdmissionCommitted` event and applies its Station grant through an
//! idempotent SQLite ledger, so every crash boundary converges to the same state.

use dawn_core::{
    events::{ClientAdmissionCommitted, ClientAdmissionIdentityReserved},
    fitting::ActivationMode,
    DomainEvent, ItemId, PlayerId, Position, ShipId, SlotKind, StationId, Velocity,
};
use dawn_ecs::components::{FittedSlot, FittingComp, InventoryComp, IsNpcComp};
use dawn_event_store::store::EventStore;

use super::{HandoffPayload, MissingObserverShip, SimulationNode};

impl<S: EventStore> SimulationNode<S> {
    /// Reserve identities without materializing a durable Ship. The watermark
    /// is appended before any handshake frame can expose either ID.
    pub(crate) fn reserve_fresh_admission_identity(&mut self) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;
        let inserted = self.pending_fresh_admissions.insert(ship_id);
        debug_assert!(inserted, "fresh admission ShipId reservation must be unique");
        self.event_store
            .append(DomainEvent::ClientAdmissionIdentityReserved(
                ClientAdmissionIdentityReserved {
                    player_id,
                    ship_id,
                    tick: self.current_tick,
                },
            ));
        (player_id, ship_id)
    }

    /// Build a fresh handoff from a temporary in-memory Ship and remove it
    /// before returning. Snapshots therefore never capture an uncommitted Ship.
    pub(crate) fn build_fresh_admission_handoff(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
        aoi_cell_size: f64,
    ) -> Result<HandoffPayload, MissingObserverShip> {
        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        let handoff = self.build_handoff_payload(ship_id, aoi_cell_size);
        self.remove_ship(ship_id);
        handoff
    }

    /// Commit a fresh admission as one replay-complete event plus an idempotent
    /// Station-inventory grant. The reservation remains held until the event is
    /// durably appended; a crash after append is repaired by replay/reconcile.
    pub(crate) fn commit_reserved_fresh_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
    ) -> bool {
        if !self.pending_fresh_admissions.contains(&ship_id)
            || self.ships.index.contains_key(&ship_id)
        {
            return false;
        }

        self.materialize_admission_player_ship(player_id, ship_id, spawn_position);
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return false;
        };
        let fitting = self
            .world
            .get::<FittingComp>(entity)
            .map(|fitting| fitting.to_snapshot())
            .unwrap_or_else(dawn_core::FittingSnapshot::empty);
        let inventory = self
            .world
            .get::<InventoryComp>(entity)
            .map(|inventory| inventory.items.clone())
            .map(|items| {
                items
                    .into_iter()
                    .flat_map(|(item_id, count)| std::iter::repeat_n(item_id, count as usize))
                    .collect()
            })
            .unwrap_or_default();
        let event = ClientAdmissionCommitted {
            player_id,
            ship_id,
            sector_id: self.sector_id,
            initial_position: spawn_position.into(),
            ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
            fitting,
            inventory,
            starter_station_id: StationId(0),
            starter_item_id: ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            starter_item_count: 1,
            tick: self.current_tick,
        };

        self.event_store
            .append(DomainEvent::ClientAdmissionCommitted(event.clone()));
        self.pending_fresh_admissions.remove(&ship_id);
        self.ensure_client_admission_grant(&event);
        true
    }

    /// Replay one atomic fresh-admission commit. This method is idempotent for
    /// both ECS state and the Station grant ledger.
    pub(super) fn replay_client_admission_commit(&mut self, event: &ClientAdmissionCommitted) {
        if !self.ships.index.contains_key(&event.ship_id) {
            self.insert_to_world(event.ship_id, Position::ORIGIN, Velocity::ZERO);
            self.set_spawn_anchor_abs(event.ship_id, event.initial_position);
            self.materialize_ship_stats(
                event.ship_id,
                event.ship_type_id,
                dawn_ecs::components::ShipStatsComp::PLAYER,
            );
            if let Some(&entity) = self.ships.index.get(&event.ship_id) {
                let _ = self.world.remove_one::<IsNpcComp>(entity);
                self.seed_player_inventory(entity);
                let fitting = FittingComp::from_snapshot(&event.fitting, &self.module_registry);
                let _ = self.world.insert_one(entity, fitting);
                let items = event.inventory.iter().copied().fold(
                    std::collections::BTreeMap::new(),
                    |mut items, item_id| {
                        *items.entry(item_id).or_default() += 1;
                        items
                    },
                );
                let _ = self.world.insert_one(entity, InventoryComp { items });
                self.reapply_fitting(event.ship_id);
            }
        }
        self.ships
            .active_ship
            .insert(event.player_id, event.ship_id);
        self.ships.owners.insert(event.ship_id, event.player_id);
        self.player_id_counter = self.player_id_counter.max(event.player_id.0 + 1);
        self.id_counter = self.id_counter.max(event.ship_id.0.counter() + 1);
        self.ensure_client_admission_grant(event);
    }

    /// Release only the live capacity reservation. The consumed IDs remain in
    /// the append-only watermark event and are never reused.
    pub(crate) fn abort_reserved_fresh_admission(&mut self, ship_id: ShipId) {
        self.pending_fresh_admissions.remove(&ship_id);
        debug_assert!(
            !self.ships.index.contains_key(&ship_id),
            "fresh admission preview must not survive begin"
        );
    }

    /// True when the requested resume would overwrite a different established
    /// Player/Ship relationship. The same exact identity may reconnect and
    /// replace its old runtime session.
    pub(crate) fn resume_admission_identity_conflicts(
        &self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        self.ships
            .owners
            .get(&ship_id)
            .is_some_and(|owner| *owner != player_id)
            || self
                .ships
                .active_ship
                .get(&player_id)
                .is_some_and(|active_ship| *active_ship != ship_id)
    }

    /// Acquire both the Ship and Player sides of the in-flight resume lock.
    pub(crate) fn reserve_resume_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        if !self.ships.index.contains_key(&ship_id)
            || self.pending_resume_admissions.contains_key(&ship_id)
            || self
                .pending_resume_admissions
                .values()
                .any(|pending_player| *pending_player == player_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            return false;
        }
        self.pending_resume_admissions.insert(ship_id, player_id);
        true
    }

    pub(crate) fn release_resume_admission(&mut self, player_id: PlayerId, ship_id: ShipId) {
        if self.pending_resume_admissions.get(&ship_id) == Some(&player_id) {
            self.pending_resume_admissions.remove(&ship_id);
        }
    }

    /// Compare-and-set the exact identity captured at begin. Ownership may be
    /// absent after restart or already equal during a reconnect, but it may not
    /// have changed to a different Player/Ship while the socket was in flight.
    pub(crate) fn commit_reserved_resume_admission(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> bool {
        if self.pending_resume_admissions.get(&ship_id) != Some(&player_id)
            || self.resume_admission_identity_conflicts(player_id, ship_id)
        {
            self.release_resume_admission(player_id, ship_id);
            return false;
        }
        let committed = self.resume_player_ship(ship_id, player_id);
        self.release_resume_admission(player_id, ship_id);
        committed
    }

    fn materialize_admission_player_ship(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        spawn_position: Position,
    ) {
        self.insert_to_world(ship_id, spawn_position, Velocity::ZERO);
        self.set_spawn_anchor(ship_id, spawn_position);
        self.materialize_ship_stats(
            ship_id,
            crate::ship_types::SHIP_TYPE_MAGPIE,
            dawn_ecs::components::ShipStatsComp::PLAYER,
        );

        if let Some(&entity) = self.ships.index.get(&ship_id) {
            let _ = self.world.remove_one::<IsNpcComp>(entity);
        }
        self.ships.active_ship.insert(player_id, ship_id);
        self.ships.owners.insert(ship_id, player_id);

        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.seed_player_inventory(entity);
        }

        for (slot, module_id) in [
            (SlotKind::High, crate::modules::MODULE_RAILGUN_SMALL),
            (SlotKind::Mid, crate::modules::MODULE_AFTERBURNER),
            (SlotKind::Mid, crate::modules::MODULE_FOLD_DISRUPTOR),
        ] {
            self.fit_admission_module_in_memory(ship_id, slot, module_id);
        }
        self.reapply_fitting(ship_id);
    }

    fn fit_admission_module_in_memory(
        &mut self,
        ship_id: ShipId,
        slot: SlotKind,
        module_id: dawn_core::ModuleId,
    ) {
        let Some(def) = self.module_registry.get(&module_id).cloned() else {
            return;
        };
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let is_active = matches!(def.activation_mode, ActivationMode::Passive);
        if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
            fitting.slot_mut(slot).push(FittedSlot {
                def,
                is_active,
                cycle_remaining: 0,
                target_ship_id: None,
            });
        }
    }
}
''')

# Replay the composite event directly.
replace_once(
    "crates/dawn-sector/src/node/apply_event.rs",
    """            DomainEvent::ClientAdmissionIdentityReserved(e) => {
                self.player_id_counter = self.player_id_counter.max(e.player_id.0 + 1);
                self.id_counter = self.id_counter.max(e.ship_id.0.counter() + 1);
            }

            DomainEvent::ShipSpawned(e) => {""",
    """            DomainEvent::ClientAdmissionIdentityReserved(e) => {
                self.player_id_counter = self.player_id_counter.max(e.player_id.0 + 1);
                self.id_counter = self.id_counter.max(e.ship_id.0.counter() + 1);
            }

            DomainEvent::ClientAdmissionCommitted(e) => {
                self.replay_client_admission_commit(e);
            }

            DomainEvent::ShipSpawned(e) => {""",
)

# SQLite idempotency ledger and transactional grant.
replace_once(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    """        conn.execute(
            \"CREATE TABLE IF NOT EXISTS station_inventory (
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL,
                PRIMARY KEY (player_id, station_id, item_type, module_id, ship_type_id)
            )\",
            [],
        )?;
        Ok(Self { conn })""",
    """        conn.execute(
            \"CREATE TABLE IF NOT EXISTS station_inventory (
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL,
                PRIMARY KEY (player_id, station_id, item_type, module_id, ship_type_id)
            )\",
            [],
        )?;
        conn.execute(
            \"CREATE TABLE IF NOT EXISTS client_admission_grants (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL
            )\",
            [],
        )?;
        Ok(Self { conn })""",
)
insert_before(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "    /// Subtract `count`",
    """    /// Apply a starter grant exactly once, keyed by the committed ShipId.
    /// The ledger marker and inventory upsert share one SQLite transaction.
    pub(super) fn ensure_client_admission_grant(
        &mut self,
        ship_id: dawn_core::ShipId,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> rusqlite::Result<bool> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            \"INSERT OR IGNORE INTO client_admission_grants
             (ship_id, player_id, station_id, item_type, module_id, ship_type_id, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)\",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                station_id.0 as i64,
                item_type,
                module_id,
                ship_type_id,
                count as i64,
            ],
        )?;
        if inserted == 1 {
            tx.execute(
                \"INSERT INTO station_inventory
                 (player_id, station_id, item_type, module_id, ship_type_id, count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (player_id, station_id, item_type, module_id, ship_type_id)
                 DO UPDATE SET count = count + excluded.count\",
                params![
                    player_id.0 as i64,
                    station_id.0 as i64,
                    item_type,
                    module_id,
                    ship_type_id,
                    count as i64,
                ],
            )?;
        }
        tx.commit()?;
        Ok(inserted == 1)
    }

""",
)
insert_before(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "    #[test]\n    fn try_debit_rejects_a_missing_stack()",
    """    #[test]
    fn client_admission_grant_is_idempotent() {
        let mut db = StationInventoryDb::open_in_memory().unwrap();
        let ship_id = dawn_core::ShipId::new(dawn_core::NodeId(2), 7);
        let item = ItemId::PackagedShip(ShipTypeId(7));
        assert!(db
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                StationId(7),
                item,
                1,
            )
            .unwrap());
        assert!(!db
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                StationId(7),
                item,
                1,
            )
            .unwrap());
        assert_eq!(db.get_all(PlayerId(1), StationId(7)).get(&item), Some(&1));
    }

""",
)

# SimulationNode grant application and startup reconciliation.
replace_once(
    "crates/dawn-sector/src/node/station_inventory.rs",
    "use dawn_core::{ItemId, PlayerId, StationId};",
    "use dawn_core::{events::ClientAdmissionCommitted, DomainEvent, ItemId, PlayerId, StationId};",
)
insert_before(
    "crates/dawn-sector/src/node/station_inventory.rs",
    "    /// Add `count` of `item_id`",
    """    pub(super) fn ensure_client_admission_grant(
        &mut self,
        event: &ClientAdmissionCommitted,
    ) {
        self.station_inventory_db
            .ensure_client_admission_grant(
                event.ship_id,
                event.player_id,
                event.starter_station_id,
                event.starter_item_id,
                event.starter_item_count,
            )
            .expect(\"client admission Station grant transaction\");
        let inventory = self
            .station_inventory_db
            .get_all(event.player_id, event.starter_station_id);
        self.station_inventory_cache.get_mut().insert(
            event.player_id,
            event.starter_station_id,
            inventory,
        );
    }

    pub(super) fn reconcile_client_admission_grants(&mut self) -> rusqlite::Result<()> {
        let grants: Vec<ClientAdmissionCommitted> = self
            .event_store
            .iter_from(0)
            .filter_map(|record| match &record.event {
                DomainEvent::ClientAdmissionCommitted(event) => Some(event.clone()),
                _ => None,
            })
            .collect();
        for event in grants {
            self.station_inventory_db.ensure_client_admission_grant(
                event.ship_id,
                event.player_id,
                event.starter_station_id,
                event.starter_item_id,
                event.starter_item_count,
            )?;
            let inventory = self
                .station_inventory_db
                .get_all(event.player_id, event.starter_station_id);
            self.station_inventory_cache.get_mut().insert(
                event.player_id,
                event.starter_station_id,
                inventory,
            );
        }
        Ok(())
    }

""",
)
replace_once(
    "crates/dawn-sector/src/node/mod.rs",
    """        self.station_inventory_db = station_inventory_db::StationInventoryDb::open(path)?;
        self.station_inventory_cache
            .replace(station_inventory::StationInventoryCache::new());
        Ok(())""",
    """        self.station_inventory_db = station_inventory_db::StationInventoryDb::open(path)?;
        self.station_inventory_cache
            .replace(station_inventory::StationInventoryCache::new());
        self.reconcile_client_admission_grants()?;
        Ok(())""",
)

# Resume conflict is distinct from an in-flight lock collision.
replace_once(
    "crates/dawn-sector/src/client_admission.rs",
    """    ResumeAlreadyPending {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// A freshly-created observer""",
    """    ResumeAlreadyPending {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// The requested pair would overwrite a different established identity.
    ResumeIdentityConflict {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// A freshly-created observer""",
)
replace_once(
    "crates/dawn-sector/src/client_admission.rs",
    """            Self::ResumeAlreadyPending { player_id, ship_id } => write!(
                f,
                \"resume refused for {player_id}: ship #{} already has an in-flight resume\",
                ship_id.raw()
            ),
            Self::MissingObserver(error)""",
    """            Self::ResumeAlreadyPending { player_id, ship_id } => write!(
                f,
                \"resume refused for {player_id}: ship #{} or player already has an in-flight resume\",
                ship_id.raw()
            ),
            Self::ResumeIdentityConflict { player_id, ship_id } => write!(
                f,
                \"resume refused for {player_id}: ship #{} conflicts with established ownership\",
                ship_id.raw()
            ),
            Self::MissingObserver(error)""",
)
replace_once(
    "crates/dawn-sector/src/client_admission.rs",
    """                if !self.reserve_resume_admission(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeAlreadyPending {
                        player_id,
                        ship_id,
                    });
                }""",
    """                if self.resume_admission_identity_conflicts(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeIdentityConflict {
                        player_id,
                        ship_id,
                    });
                }
                if !self.reserve_resume_admission(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeAlreadyPending {
                        player_id,
                        ship_id,
                    });
                }""",
)
insert_before(
    "crates/dawn-sector/src/client_admission.rs",
    "    #[test]\n    fn missing_resume_never_falls_back_to_fresh_spawn()",
    """    #[test]
    fn one_player_cannot_hold_two_concurrent_resume_attempts() {
        let mut node = node();
        let player_id = PlayerId(12);
        let first_ship = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let second_ship = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id,
                    ship_id: first_ship,
                },
                AOI_CELL_SIZE,
            )
            .expect(\"first resume obtains the Player lock\");
        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id,
                    ship_id: second_ship,
                },
                AOI_CELL_SIZE,
            )
            .expect_err(\"same Player cannot concurrently resume another Ship\"),
            ClientAdmissionRefusal::ResumeAlreadyPending {
                player_id,
                ship_id: second_ship,
            }
        );
        first.abort(&mut node);
    }

    #[test]
    fn established_owner_cannot_be_overwritten_by_another_player() {
        let mut node = node();
        let owner = PlayerId(12);
        let attacker = PlayerId(13);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.begin_client_admission(
            ClientAdmissionIntent::Resume {
                player_id: owner,
                ship_id,
            },
            AOI_CELL_SIZE,
        )
        .expect(\"owner resume\")
        .commit(&mut node)
        .expect(\"owner commit\");

        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: attacker,
                    ship_id,
                },
                AOI_CELL_SIZE,
            )
            .expect_err(\"different Player cannot take an owned Ship\"),
            ClientAdmissionRefusal::ResumeIdentityConflict {
                player_id: attacker,
                ship_id,
            }
        );
    }

    #[test]
    fn exact_resume_identity_may_reconnect() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.begin_client_admission(
            ClientAdmissionIntent::Resume { player_id, ship_id },
            AOI_CELL_SIZE,
        )
        .expect(\"first resume\")
        .commit(&mut node)
        .expect(\"first commit\");
        let reconnect = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect(\"same identity reconnects and replaces its runtime session\");
        reconnect.abort(&mut node);
    }

""",
)

# New refusal logging in all adapters.
for path, old, new in [
    (
        "crates/dawn-sector-node/src/client_admission.rs",
        """                Err(ClientAdmissionRefusal::FreshAtPopulationCap) => {""",
        """                Err(ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. }) => {
                    eprintln!(
                        \"[Node] resume refused from {}: ship #{} conflicts with established ownership\",
                        request.peer_addr,
                        ship_id.raw()
                    );
                    continue;
                }
                Err(ClientAdmissionRefusal::FreshAtPopulationCap) => {""",
    ),
    (
        "crates/dawn-simulation/src/serve/single.rs",
        """        ClientAdmissionRefusal::MissingObserver(error) => {""",
        """        ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. } => {
            eprintln!(
                \"[Server] resume from {addr} refused: ship #{} conflicts with established ownership\",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {""",
    ),
    (
        "crates/dawn-simulation/src/serve/cluster.rs",
        """        ClientAdmissionRefusal::MissingObserver(error) => {""",
        """        ClientAdmissionRefusal::ResumeIdentityConflict { ship_id, .. } => {
            eprintln!(
                \"[Server] clustered resume from {addr} refused: ship #{} conflicts with established ownership\",
                ship_id.raw()
            );
        }
        ClientAdmissionRefusal::MissingObserver(error) => {""",
    ),
]:
    replace_once(path, old, new)

# Exact identity reconnect replaces any old runtime session rather than leaving
# two command sources alive.
replace_once(
    "crates/dawn-sector-node/src/runtime.rs",
    """        seed_runtime_session(&mut self.aoi_frame, node, &sess);
        self.sessions.push(sess);""",
    """        self.sessions.retain(|existing| {
            existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
        });
        self.aoi_frame
            .retain_players(|player_id| player_id != sess.player_id);
        seed_runtime_session(&mut self.aoi_frame, node, &sess);
        self.sessions.push(sess);""",
)
replace_once(
    "crates/dawn-simulation/src/serve/single.rs",
    """            aoi_delivery.seed_single_player(&node, sess.player_id, sess.ship_id);

            if duel_mode""",
    """            sessions.retain(|existing| {
                existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
            });
            aoi_delivery.seed_single_player(&node, sess.player_id, sess.ship_id);

            if duel_mode""",
)
replace_once(
    "crates/dawn-simulation/src/serve/cluster.rs",
    """            aoi_delivery.seed_cluster_player(&nodes, sector, sess.player_id, sess.ship_id);
            sessions.push(sess);""",
    """            sessions.retain(|existing| {
                existing.player_id != sess.player_id && existing.ship_id != sess.ship_id
            });
            aoi_delivery.seed_cluster_player(&nodes, sector, sess.player_id, sess.ship_id);
            sessions.push(sess);""",
)
replace_once(
    "crates/dawn-simulation/src/serve/cluster.rs",
    """            Ok(committed) => {
                player_sector.insert(committed.player_id, sector);
                ship_player.insert(committed.ship_id, committed.player_id);
                Some((value, committed))
            }""",
    """            Ok(committed) => {
                player_sector.retain(|player_id, _| *player_id != committed.player_id);
                ship_player.retain(|ship_id, player_id| {
                    *ship_id != committed.ship_id && *player_id != committed.player_id
                });
                player_sector.insert(committed.player_id, sector);
                ship_player.insert(committed.ship_id, committed.player_id);
                Some((value, committed))
            }""",
)

# Wire projection: the composite event is Sector-internal; AoI publishes the
# newly materialized Ship from authoritative state.
replace_once(
    "crates/dawn-wire/src/server_event.rs",
    """        DomainEvent::ClientAdmissionIdentityReserved(_) => return None,
    })""",
    """        DomainEvent::ClientAdmissionIdentityReserved(_) => return None,
        DomainEvent::ClientAdmissionCommitted(_) => return None,
    })""",
)

# Crash-window replay test: one composite event rebuilds complete state and the
# SQLite grant remains exactly once across repeated reconciliation.
insert_before(
    "crates/dawn-sector/tests/client_admission_replay.rs",
    "#[test]\nfn missing_resume_still_refuses_without_creating_replayable_state()",
    r'''#[test]
fn committed_fresh_admission_replays_complete_state_and_grants_starter_once() {
    let galaxy = Arc::new(Galaxy::demo());
    let catalog = repository_catalog();
    let mut node = SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        Arc::clone(&galaxy),
    );
    for definition in catalog.modules() {
        node.register_module(definition.clone());
    }
    for definition in catalog.ship_types() {
        node.register_ship_type(definition.clone());
    }
    let pre_commit_snapshot = node.take_snapshot();
    let attempt = node
        .begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission");
    let player_id = attempt.player_id();
    let ship_id = attempt.ship_id();
    attempt.commit(&mut node).expect("fresh commit");

    let records = node.event_store().all_records();
    assert_eq!(records.len(), 2);
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(dawn_core::DomainEvent::ClientAdmissionCommitted(event))
            if event.player_id == player_id
                && event.ship_id == ship_id
                && event.fitting.high.len() == 1
                && event.fitting.mid.len() == 2
    ));
    assert!(!records.iter().any(|record| matches!(
        record.event,
        dawn_core::DomainEvent::ShipSpawned(_) | dawn_core::DomainEvent::ShipFitted(_)
    )));

    let mut replay_store = InMemoryEventStore::new();
    for record in records {
        replay_store.append(record.event.clone());
    }
    let mut restored = SimulationNode::restore_from(
        replay_store,
        &pre_commit_snapshot,
        galaxy,
        catalog.modules(),
        catalog.ship_types(),
    );
    assert_eq!(restored.ship_count(), 1);
    assert!(restored.ship_absolute_pos(ship_id).is_some());

    let directory = tempfile::tempdir().unwrap();
    let db_path = directory.path().join("station.sqlite");
    let db_path = db_path.to_str().unwrap();
    restored.open_station_inventory_db(db_path).unwrap();
    let starter = dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(1));
    assert_eq!(
        restored.station_item_count(player_id, dawn_core::StationId(0), starter),
        1
    );
    restored.open_station_inventory_db(db_path).unwrap();
    assert_eq!(
        restored.station_item_count(player_id, dawn_core::StationId(0), starter),
        1
    );
}

''',
)

# Real socket disconnect for the production adapter, not an injected Err only.
replace_once(
    "crates/dawn-sector-node/Cargo.toml",
    "toml             = { version = \"1.1\", features = [\"parse\"] }\n",
    """toml             = { version = \"1.1\", features = [\"parse\"] }

[dev-dependencies]
futures-util      = \"0.3\"
tokio-tungstenite = \"0.30\"
""",
)
insert_before(
    "crates/dawn-sector-node/src/client_admission.rs",
    "    #[test]\n    fn production_adapter_failed_resume_keeps_pre_existing_ship()",
    r'''    #[tokio::test]
    async fn production_adapter_aborts_after_real_socket_disconnect() {
        use futures_util::SinkExt;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
            socket
                .send(Message::Binary(
                    dawn_wire::ClientMessage::Hello(dawn_wire::HelloMessage { resume: None })
                        .encode()
                        .into(),
                ))
                .await
                .unwrap();
            socket.get_mut().shutdown().await.unwrap();
        });
        let (stream, peer_addr) = listener.accept().await.unwrap();
        let request = ws_server::WsServer::accept_handshake_request(stream, peer_addr)
            .await
            .unwrap();
        client.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut node = test_node();
        let mut attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh attempt");
        let player_id = attempt.player_id();
        let ship_id = attempt.ship_id();
        let payload = attempt.take_handoff_payload();
        let result = request
            .complete(
                player_id,
                ship_id,
                payload.initial_state,
                payload.player_loadout,
            )
            .await
            .map_err(|error| error.to_string());
        assert!(result.is_err(), "closed socket must fail the awaited handoff");
        assert!(finish_admission(&mut node, attempt, result).is_none());
        assert_eq!(node.ship_count(), 0);
    }

''',
)

# Documentation aligns with the durable watermark, atomic composite commit,
# idempotent grant recovery, and reconnect replacement semantics.
Path("docs/architecture/client-admission.md").write_text(r'''---
scope    : Client connection admission lifecycle across production and simulation runtimes.
audience : AI Agent / Human Developer
update   : When handshake, resume, ownership, or session-promotion behavior changes.
related  : ownership.md, ADR-0007, ADR-0014
---

# Client Admission

## Single lifecycle owner

`dawn-sector::client_admission` owns authoritative begin/commit/abort behavior.
Production, single-Sector simulation, and clustered simulation are socket
adapters: they pass intent, await the handoff, resolve the attempt on the Sector
thread, and publish only a committed session.

## State machine

```text
Hello -> begin -> await Welcome/InitialState/PlayerLoadout
                    | success -> commit -> publish/replace session
                    | failure -> abort  -> drop socket
```

## Fresh admission

Begin appends `ClientAdmissionIdentityReserved`, permanently consuming the
`PlayerId`/`ShipId`, then uses a temporary in-memory Ship to construct the
observer-scoped handoff. The preview is removed before begin returns. Therefore
an in-flight attempt has one durable allocation-watermark event but no durable
Ship, fitting, ownership, AoI, or Station inventory.

Commit materializes the starter state and appends exactly one
`ClientAdmissionCommitted` event containing the Ship creation, fitting/cargo
snapshot, ownership identity, and starter Station grant description. The
Station grant is applied through a SQLite ledger keyed by `ShipId`; the ledger
marker and inventory upsert share one SQLite transaction. If the process dies
after the event append but before the SQLite write, snapshot+tail replay and
`open_station_inventory_db` reconciliation apply the missing grant exactly once.
No checkpoint can cover a partially-returned commit because commit runs
synchronously on the owning Sector thread.

Abort releases only the live capacity reservation. The watermark remains and
IDs are never reused (INV-004).

## Resume admission

Resume names an exact `(PlayerId, ShipId)` and never falls back to fresh spawn.
Begin reserves both sides of the identity: no other in-flight attempt may use
the same Ship or Player. Existing ownership is compare-and-set compatible only
when absent after restoration or already equal to the exact reconnect identity;
a different owner or a different active Ship is refused.

Ownership changes only after every handshake frame has been await-sent. Abort
releases the reservation without touching the pre-existing Ship. A successful
reconnect for the same exact identity replaces any older runtime session and
its routing/AoI publication, so only one command source remains live.

## Cluster routing

Fresh admission starts in Sector 0. Resume locates the exact authoritative
Sector and carries that index through asynchronous completion. `player_sector`
and `ship_player` are replaced only after commit and are kept one-to-one with
the published session. Admission cannot move a Ship between Sectors or bypass
the ADR-0014 Transit pipeline.
''')
replace_once(
    "docs/architecture/event-catalog.md",
    """| `ClientAdmissionIdentityReserved` | Fresh admission durably consumed a `PlayerId`/`ShipId` pair without materializing a Ship; Replay advances allocation watermarks only | `SimulationNode::reserve_fresh_admission_identity()` | ✅ implemented |
| `ShipDespawned`""",
    """| `ClientAdmissionIdentityReserved` | Fresh admission durably consumed a `PlayerId`/`ShipId` pair without materializing a Ship; Replay advances allocation watermarks only | `SimulationNode::reserve_fresh_admission_identity()` | ✅ implemented |
| `ClientAdmissionCommitted` | Atomic fresh-admission starter state: Ship, fitting/cargo snapshot, ownership identity, and idempotent Station grant description; Replay restores all of them from one event | `SimulationNode::commit_reserved_fresh_admission()` | ✅ implemented |
| `ShipDespawned`""",
)

# Remove this one-shot patch script from the resulting source commit.
Path(".github/fix_admission_review.py").unlink()
