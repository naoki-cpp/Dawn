//! Market-to-Sector settlement delivery adapter.
//!
//! `dawn-market` emits durable value intents. This module is the only place
//! that translates those intents into Sector-local inventory mutations. Since
//! issue #315, that translation runs through the Sector's own Tick pipeline
//! (`dawn_sector::transition::FrameInput::market_settlements`) rather than a
//! synchronous mutation outside any tick boundary, so a settlement's effect
//! is captured in the same durable journal append as the tick that applies
//! it. This is a two-phase outbox drain:
//!
//! 1. [`MarketSettlement::drain_pending_inputs`] reads pending settlement
//!    intents from the Market DB and translates the ones whose destination
//!    ship is owned/docked in this Sector into `MarketSettlementInput`s for
//!    this tick's `FrameInput`. Intents for ships not owned/docked here are
//!    left pending for a future tick (matching the previous "Unavailable"
//!    semantics — e.g. the ship jumped away or undocked).
//! 2. After the tick commits, [`MarketSettlement::acknowledge_outcomes`]
//!    reports each `MarketSettlementOutcome` back to the Market DB.
//!
//! `MarketSettlement::place`/`cancel` no longer touch Sector state at all:
//! they only place/cancel the order in the Market ledger. Settlement is
//! always deferred to the next tick's drain — never synchronous within the
//! same request.

use dawn_core::{
    CreditItemCommand, ItemId, PlayerId, RemoveItemCommand, ReturnItemCommand, ShipId,
};
use dawn_market::{
    MarketCommand, MarketDb, MarketError, OrderId, OrderSide, SettlementEffect, SettlementId,
    SettlementIntent,
};
use dawn_sector::node::SimulationNode;
use dawn_sector::transition::{MarketSettlementInput, MarketSettlementOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarketSettlementResult {
    Completed(String),
    Rejected(String),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ParsedOrder {
    pub(super) ship_id: ShipId,
    pub(super) item_id: ItemId,
    pub(super) side: OrderSide,
    pub(super) price: u64,
    pub(super) quantity: u64,
}

impl MarketSettlementResult {
    pub(super) fn notice(&self) -> &str {
        match self {
            Self::Completed(notice) | Self::Rejected(notice) => notice,
        }
    }
}

/// One settlement intent that has been translated into Sector input and is
/// waiting on a Tick to admit it, plus enough of its Market identity to
/// acknowledge (or reject) it back into the Market DB once a
/// `MarketSettlementOutcome` comes back.
#[derive(Debug, Clone, Copy)]
pub(super) struct QueuedSettlement {
    settlement_id: SettlementId,
    input: MarketSettlementInput,
}

impl QueuedSettlement {
    pub(super) fn input(&self) -> MarketSettlementInput {
        self.input
    }
}

pub(super) struct MarketSettlement;

impl MarketSettlement {
    pub(super) fn place(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
    ) -> MarketSettlementResult {
        let command = MarketCommand::PlaceOrder {
            player_id,
            ship_id: order.ship_id,
            item_id: order.item_id,
            side: order.side,
            price: order.price,
            quantity: order.quantity,
        };
        match db.execute(command) {
            Ok(_transition) => {
                MarketSettlementResult::Completed("Order placed; settlement pending".to_owned())
            }
            Err(error) => rejected(error),
        }
    }

    pub(super) fn cancel(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
    ) -> MarketSettlementResult {
        match db.execute(MarketCommand::CancelOrder {
            player_id,
            order_id,
        }) {
            Ok(_transition) => {
                MarketSettlementResult::Completed("Order cancelled; settlement pending".to_owned())
            }
            Err(error) => rejected(error),
        }
    }

    /// Drain pending settlement intents from `db`, translating the ones
    /// whose destination ship is owned/docked in `node` into
    /// `MarketSettlementInput`s for this tick's `FrameInput`. Intents for a
    /// ship that isn't owned/docked here are left pending (e.g. it jumped
    /// away or undocked in the meantime) so a later tick — possibly in a
    /// different Sector for a clustered runtime — can pick them up.
    ///
    /// An intent whose `SettlementId` can't be represented as the Tick
    /// pipeline's `u64` is surfaced anyway (rather than silently dropped
    /// forever) so the caller's `acknowledge_outcomes` — or an immediate
    /// direct reject — can clear it out of the queue. Since such an intent
    /// never becomes a valid `MarketSettlementInput`, this function rejects
    /// it in the DB immediately and does not include it in the returned
    /// queue.
    pub(super) fn drain_pending_inputs(
        db: &mut MarketDb,
        node: &SimulationNode,
    ) -> Vec<QueuedSettlement> {
        let pending = match db.pending_settlements() {
            Ok(pending) => pending,
            Err(error) => {
                eprintln!("[Server] Market settlement drain failed: {error}");
                return Vec::new();
            }
        };

        let mut queued = Vec::with_capacity(pending.len());
        for intent in pending {
            let (player_id, ship_id) = intent_identity(intent);
            if !owns_docked_ship(node, player_id, ship_id) {
                continue;
            }
            let Ok(settlement_id) = u64::try_from(intent.id.0).map(|id| (id, id > 0)) else {
                reject_invalid_settlement(db, intent.id);
                continue;
            };
            let (settlement_id, is_valid) = settlement_id;
            if !is_valid {
                reject_invalid_settlement(db, intent.id);
                continue;
            }
            queued.push(QueuedSettlement {
                settlement_id: intent.id,
                input: to_market_settlement_input(intent, settlement_id),
            });
        }
        queued
    }

    /// Report each `MarketSettlementOutcome` from a committed Tick back to
    /// the Market DB: acknowledge the ones that applied, reject the ones
    /// that didn't. A queued settlement without a matching outcome, or a
    /// `db.execute` failure while acknowledging/rejecting, is logged and
    /// skipped rather than treated as fatal — the settlement simply stays
    /// pending and is retried on a later drain.
    pub(super) fn acknowledge_outcomes(
        db: &mut MarketDb,
        queued: &[QueuedSettlement],
        outcomes: &[MarketSettlementOutcome],
    ) {
        for settlement in queued {
            let expected_id = settlement.input.settlement_id();
            let Some(outcome) = outcomes
                .iter()
                .find(|outcome| outcome.settlement_id == expected_id)
            else {
                eprintln!(
                    "[Server] Market settlement {:?} had no matching Tick outcome",
                    settlement.settlement_id
                );
                continue;
            };
            let result = if outcome.applied {
                db.execute(MarketCommand::AcknowledgeSettlement {
                    settlement_id: settlement.settlement_id,
                })
            } else {
                db.execute(MarketCommand::RejectSettlement {
                    settlement_id: settlement.settlement_id,
                    reason: "Sector rejected the inventory settlement".to_owned(),
                })
            };
            if let Err(error) = result {
                eprintln!(
                    "[Server] failed to record Market settlement outcome for {:?}: {error}",
                    settlement.settlement_id
                );
            }
        }
    }
}

fn reject_invalid_settlement(db: &mut MarketDb, settlement_id: SettlementId) {
    if let Err(error) = db.execute(MarketCommand::RejectSettlement {
        settlement_id,
        reason: "invalid settlement identity".to_owned(),
    }) {
        eprintln!("[Server] failed to reject invalid Market settlement {settlement_id:?}: {error}");
    }
}

fn rejected(error: MarketError) -> MarketSettlementResult {
    MarketSettlementResult::Rejected(error.to_string())
}

fn to_market_settlement_input(
    intent: SettlementIntent,
    settlement_id: u64,
) -> MarketSettlementInput {
    match intent.effect {
        SettlementEffect::ReserveAsk {
            player_id,
            ship_id,
            item_id,
            quantity,
            ..
        } => MarketSettlementInput::RemoveItem(RemoveItemCommand {
            player_id,
            ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
        SettlementEffect::ReturnItem {
            player_id,
            ship_id,
            item_id,
            quantity,
        } => MarketSettlementInput::ReturnItem(ReturnItemCommand {
            player_id,
            ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
        SettlementEffect::CreditItem {
            buyer,
            buyer_ship_id,
            item_id,
            quantity,
            ..
        } => MarketSettlementInput::CreditItem(CreditItemCommand {
            player_id: buyer,
            ship_id: buyer_ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
    }
}

fn intent_identity(intent: SettlementIntent) -> (PlayerId, ShipId) {
    match intent.effect {
        SettlementEffect::ReserveAsk {
            player_id, ship_id, ..
        }
        | SettlementEffect::ReturnItem {
            player_id, ship_id, ..
        } => (player_id, ship_id),
        SettlementEffect::CreditItem {
            buyer,
            buyer_ship_id,
            ..
        } => (buyer, buyer_ship_id),
    }
}

fn owns_docked_ship(node: &SimulationNode, player_id: PlayerId, ship_id: ShipId) -> bool {
    node.owns_ship(player_id, ship_id) && node.docked_station(ship_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_frame::{RuntimeFrameHost, RuntimeFramePolicy};
    use dawn_core::{ClientRequest, DomainEvent, NodeId, SectorBounds, SectorId, StationId};
    use dawn_sector::transit::LocalRuntimeConsensus;
    use dawn_storage::InMemoryJournal;

    fn order(ship_id: ShipId, side: OrderSide, quantity: u64) -> ParsedOrder {
        ParsedOrder {
            ship_id,
            item_id: ItemId::ScrapMetal,
            side,
            price: 100,
            quantity,
        }
    }

    fn node_with_docked_ship(player_id: PlayerId) -> (SimulationNode, ShipId) {
        let mut node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );
        let station_position = node
            .station(StationId(0))
            .expect("demo station exists")
            .position;
        let ship_id = node.spawn_player_ship_at_pub(player_id, station_position);
        node.apply_client_request(
            player_id,
            ClientRequest::Dock {
                station: StationId(0),
            },
            &mut Vec::new(),
        )
        .unwrap();
        (node, ship_id)
    }

    /// Number of `item_id` units present in `ship_id`'s cargo, read off the
    /// `ShipFitted` snapshot event that every cargo mutation emits (the one
    /// externally-visible source of truth for cargo contents outside the
    /// `dawn-sector` crate).
    fn cargo_count_from_events(events: &[DomainEvent], ship_id: ShipId, item_id: ItemId) -> usize {
        events
            .iter()
            .rev()
            .find_map(|event| match event {
                DomainEvent::ShipFitted(fitted) if fitted.ship_id == ship_id => Some(
                    fitted
                        .inventory
                        .iter()
                        .filter(|item| **item == item_id)
                        .count(),
                ),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn host_with_docked_ship(
        player_id: PlayerId,
    ) -> (
        RuntimeFrameHost<InMemoryJournal, LocalRuntimeConsensus>,
        ShipId,
    ) {
        let (node, ship_id) = node_with_docked_ship(player_id);
        let host = RuntimeFrameHost::new(
            node,
            InMemoryJournal::new(),
            LocalRuntimeConsensus,
            RuntimeFramePolicy::local_durable(0),
        );
        (host, ship_id)
    }

    #[test]
    fn drain_translates_pending_intents_for_owned_docked_ships() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (mut node, seller_ship) = node_with_docked_ship(seller);
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id: seller,
            ship_id: seller_ship,
            item_id: ItemId::ScrapMetal,
            quantity: 5,
            settlement_id: 1,
        }));

        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));

        let queued = MarketSettlement::drain_pending_inputs(&mut db, &node);
        assert_eq!(queued.len(), 1);
        match queued[0].input() {
            MarketSettlementInput::RemoveItem(cmd) => {
                assert_eq!(cmd.player_id, seller);
                assert_eq!(cmd.ship_id, seller_ship);
                assert_eq!(cmd.quantity, 5);
            }
            other => panic!("expected RemoveItem, got {other:?}"),
        }
    }

    #[test]
    fn drain_skips_intents_for_ships_not_owned_or_docked_here() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (mut node, seller_ship) = node_with_docked_ship(seller);
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id: seller,
            ship_id: seller_ship,
            item_id: ItemId::ScrapMetal,
            quantity: 5,
            settlement_id: 1,
        }));
        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));

        // A different (empty) node has no owned/docked ships at all, so the
        // pending intent above must be left pending rather than translated.
        let other_node = SimulationNode::new(
            NodeId(1),
            SectorId(1),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );

        let queued = MarketSettlement::drain_pending_inputs(&mut db, &other_node);
        assert!(queued.is_empty());
        // Still pending in the DB, not rejected.
        assert_eq!(db.pending_settlements().unwrap().len(), 1);

        // A later drain against the real owning node still finds it.
        let queued = MarketSettlement::drain_pending_inputs(&mut db, &node);
        assert_eq!(queued.len(), 1);
    }

    #[test]
    fn acknowledge_outcomes_acks_applied_and_rejects_unapplied() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (node, seller_ship) = node_with_docked_ship(seller);
        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));
        let queued = MarketSettlement::drain_pending_inputs(&mut db, &node);
        assert_eq!(queued.len(), 1);
        let settlement_id = queued[0].input().settlement_id();

        MarketSettlement::acknowledge_outcomes(
            &mut db,
            &queued,
            &[MarketSettlementOutcome {
                settlement_id,
                applied: false,
            }],
        );

        assert!(db.pending_settlements().unwrap().is_empty());
        // The settlement is already terminal (rejected); re-acknowledging it
        // is a harmless duplicate, not an error -- the Market policy treats
        // non-pending settlement rows as already decided.
        assert!(db
            .execute(MarketCommand::AcknowledgeSettlement {
                settlement_id: queued[0].settlement_id,
            })
            .is_ok());
    }

    #[test]
    fn placing_an_ask_reserves_cargo_only_after_a_committed_tick() {
        // Full round trip: place -> drain -> real RuntimeFrameHost Tick
        // pipeline apply -> acknowledge. This is the durability property
        // issue #315 is about: the settlement's cargo mutation only exists
        // once it has gone through `run_frame_with_output`'s
        // prepare/durable-journal/apply pipeline, i.e. it is captured by the
        // same durable write set as everything else in that tick, never
        // applied synchronously outside of a tick boundary.
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (mut host, seller_ship) = host_with_docked_ship(seller);
        host.with_node_mut(|node| {
            node.credit_item_owned(CreditItemCommand {
                player_id: seller,
                ship_id: seller_ship,
                item_id: ItemId::ScrapMetal,
                quantity: 5,
                settlement_id: 1,
            })
        });
        let _ = host.drain_pending_events();

        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));
        let queued = MarketSettlement::drain_pending_inputs(&mut db, host.node());
        assert_eq!(queued.len(), 1);

        let inputs: Vec<MarketSettlementInput> =
            queued.iter().map(QueuedSettlement::input).collect();

        let mut acknowledged_outcomes = Vec::new();
        let output = host
            .run_frame_with_output(
                dawn_sector::transition::FrameInput {
                    lock_commands: &[],
                    market_settlements: &inputs,
                },
                |_node, tick_result, _events| {
                    acknowledged_outcomes = tick_result.market_settlement_outcomes.clone();
                },
            )
            .expect("Tick with a queued Market settlement must be preparable and durable");

        assert_eq!(
            cargo_count_from_events(&output.events, seller_ship, ItemId::ScrapMetal),
            0,
            "the Ask reservation must remove the item from cargo once the tick commits"
        );

        MarketSettlement::acknowledge_outcomes(&mut db, &queued, &acknowledged_outcomes);
        assert!(db.pending_settlements().unwrap().is_empty());
        assert_eq!(db.open_orders_for(seller).unwrap().len(), 1);
    }

    #[test]
    fn failed_credit_comes_back_as_an_unapplied_outcome_for_a_committed_tick() {
        // Mirrors the previous immediate-apply test's coverage: a credit the
        // Sector can't apply comes back from a real, committed Tick as an
        // `applied: false` outcome rather than being silently dropped or the
        // client being told it completed. Using an invalid (zero)
        // `settlement_id` is the cleanest deterministic way to force
        // `apply_market_settlement_input` to reject without touching cargo
        // at all (`apply_market_item_mutation` rejects `settlement_id == 0`
        // up front). The DB side of turning that outcome into a rejection is
        // already covered by
        // `acknowledge_outcomes_acks_applied_and_rejects_unapplied`.
        let buyer = PlayerId(1);
        let (mut host, buyer_ship) = host_with_docked_ship(buyer);

        let credit_input = MarketSettlementInput::CreditItem(CreditItemCommand {
            player_id: buyer,
            ship_id: buyer_ship,
            item_id: ItemId::ScrapMetal,
            quantity: 2,
            settlement_id: 0,
        });

        let mut outcomes = Vec::new();
        host.run_frame_with_output(
            dawn_sector::transition::FrameInput {
                lock_commands: &[],
                market_settlements: &[credit_input],
            },
            |_node, tick_result, _events| {
                outcomes = tick_result.market_settlement_outcomes.clone();
            },
        )
        .expect("Tick preparation must still succeed even though the settlement is rejected");

        assert_eq!(
            outcomes,
            vec![MarketSettlementOutcome {
                settlement_id: 0,
                applied: false,
            }]
        );
    }
}
