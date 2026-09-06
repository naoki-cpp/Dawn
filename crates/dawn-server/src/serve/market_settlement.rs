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
//! 1. The delivery owner reads one bounded pending settlement page from the
//!    Market DB and passes that same page to every relevant Sector. This
//!    module translates the intents whose destination ship is owned/docked in
//!    this Sector into `MarketSettlementInput`s for this tick's `FrameInput`.
//!    Intents for ships not owned/docked here are left pending for a future
//!    tick (matching the previous "Unavailable" semantics — e.g. the ship
//!    jumped away or undocked).
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
use dawn_sector::transition::{
    MarketSettlementInput, MarketSettlementOutcome, MarketSettlementStatus,
};

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
    /// Owner of the destination ship, so a deferred outcome can be reported
    /// back to the player who placed the order.
    player_id: PlayerId,
    input: MarketSettlementInput,
}

impl QueuedSettlement {
    pub(super) fn input(&self) -> MarketSettlementInput {
        self.input
    }
}

/// What `acknowledge_outcomes` recorded for one committed tick: the
/// settlement identities the Market ledger has now durably decided (so the
/// Sector can retire them from its idempotency guard on the next frame), and
/// the players whose settlement was refused (so the serve loop can tell
/// them, instead of leaving the client on "settlement pending" forever).
#[derive(Debug, Clone, Default)]
pub(super) struct SettlementAcknowledgement {
    pub(super) decided_settlement_ids: Vec<u64>,
    pub(super) rejected_players: Vec<PlayerId>,
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

    /// Translate a delivery-owned pending page into inputs for one Sector.
    ///
    /// The page has already been scanned and its cursor advanced by the
    /// delivery owner. A nonempty page can still produce an empty queue here:
    /// every intent may be unroutable for this Sector, so those intents remain
    /// pending for another Sector or a later cyclic page.
    ///
    /// An intent whose `SettlementId` can't be represented as the Tick
    /// pipeline's `u64` is surfaced anyway (rather than silently dropped
    /// forever) so the caller's `acknowledge_outcomes` — or an immediate
    /// direct reject — can clear it out of the queue. Since such an intent
    /// never becomes a valid `MarketSettlementInput`, this function rejects
    /// it in the DB immediately and does not include it in the returned
    /// queue.
    pub(super) fn queue_pending_inputs(
        db: &mut MarketDb,
        pending: &[SettlementIntent],
        node: &SimulationNode,
    ) -> Vec<QueuedSettlement> {
        let mut queued = Vec::new();
        for &intent in pending {
            let Some(settlement_id) = u64::try_from(intent.id.0).ok().filter(|&id| id > 0) else {
                // Malformed identity: terminal regardless of which Sector
                // owns the ship, so clear it out rather than rescanning it
                // on every tick forever.
                reject_invalid_settlement(db, intent.id);
                continue;
            };
            let (player_id, ship_id) = intent_identity(intent);
            if !owns_docked_ship(node, player_id, ship_id) {
                continue;
            }
            queued.push(QueuedSettlement {
                settlement_id: intent.id,
                player_id,
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
    ) -> SettlementAcknowledgement {
        let mut acknowledgement = SettlementAcknowledgement::default();
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
            let result = match outcome.status {
                MarketSettlementStatus::Applied => {
                    db.execute(MarketCommand::AcknowledgeSettlement {
                        settlement_id: settlement.settlement_id,
                    })
                }
                // Transient: the Sector could not act on it this tick (the
                // ship left, was handed off, or is mid-transit). Leave the
                // outbox row pending so a later drain -- possibly in the
                // Sector that now owns the ship -- retries it. Rejecting
                // here would compensate away a legitimate order.
                MarketSettlementStatus::Unavailable => continue,
                MarketSettlementStatus::Rejected => db.execute(MarketCommand::RejectSettlement {
                    settlement_id: settlement.settlement_id,
                    reason: "Sector rejected the inventory settlement".to_owned(),
                }),
            };
            match result {
                // Only a durably decided settlement may be retired from the
                // Sector's idempotency guard: it will never be redelivered.
                Ok(_) => {
                    acknowledgement.decided_settlement_ids.push(expected_id);
                    if outcome.status == MarketSettlementStatus::Rejected {
                        acknowledgement.rejected_players.push(settlement.player_id);
                    }
                }
                Err(error) => eprintln!(
                    "[Server] failed to record Market settlement outcome for {:?}: {error}",
                    settlement.settlement_id
                ),
            }
        }
        acknowledgement
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
    use dawn_core::{ClientRequest, DomainEvent, NodeId, SectorBounds, SectorId, StationId};
    use dawn_sector::transit::LocalRuntimeConsensus;
    use dawn_server::runtime_frame::{RuntimeFrameHost, RuntimeFramePolicy};
    use dawn_storage::InMemoryJournal;

    /// Settlement identity used only to seed a fixture's starting cargo.
    ///
    /// It must not collide with the ids `MarketDb` allocates (which start at
    /// 1), or the Sector's idempotency guard treats the first real
    /// settlement as already applied and silently skips its mutation.
    const SEED_SETTLEMENT_ID: u64 = 1_000_000;

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
    fn cargo_count_from_events(events: &[DomainEvent], ship_id: ShipId, item_id: ItemId) -> u64 {
        events
            .iter()
            .rev()
            .find_map(|event| match event {
                DomainEvent::ShipFitted(fitted) if fitted.ship_id == ship_id => {
                    Some(fitted.inventory.get(&item_id).copied().unwrap_or(0))
                }
                _ => None,
            })
            .unwrap_or(0)
    }

    fn queue_first_pending_page(db: &mut MarketDb, node: &SimulationNode) -> Vec<QueuedSettlement> {
        let pending = db.pending_settlements_page_after(None).unwrap();
        MarketSettlement::queue_pending_inputs(db, &pending, node)
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

    fn host_with_docked_ship_and_scrap(
        player_id: PlayerId,
        quantity: u64,
    ) -> (
        RuntimeFrameHost<InMemoryJournal, LocalRuntimeConsensus>,
        ShipId,
    ) {
        let (mut node, ship_id) = node_with_docked_ship(player_id);
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity,
            settlement_id: SEED_SETTLEMENT_ID,
        }));
        let _ = node.drain_pending_events();
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
            settlement_id: SEED_SETTLEMENT_ID,
        }));

        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));

        let queued = queue_first_pending_page(&mut db, &node);
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
            settlement_id: SEED_SETTLEMENT_ID,
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

        let queued = queue_first_pending_page(&mut db, &other_node);
        assert!(queued.is_empty());
        // Still pending in the DB, not rejected.
        assert_eq!(db.pending_settlements_page_after(None).unwrap().len(), 1);

        // A later drain against the real owning node still finds it.
        let queued = queue_first_pending_page(&mut db, &node);
        assert_eq!(queued.len(), 1);
    }

    #[test]
    fn acknowledge_outcomes_rejects_a_refused_settlement_and_reports_its_player() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (node, seller_ship) = node_with_docked_ship(seller);
        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));
        let queued = queue_first_pending_page(&mut db, &node);
        assert_eq!(queued.len(), 1);
        let settlement_id = queued[0].input().settlement_id();

        let acknowledgement = MarketSettlement::acknowledge_outcomes(
            &mut db,
            &queued,
            &[MarketSettlementOutcome {
                settlement_id,
                status: MarketSettlementStatus::Rejected,
            }],
        );

        assert!(db.pending_settlements_page_after(None).unwrap().is_empty());
        assert_eq!(
            acknowledgement.decided_settlement_ids,
            vec![settlement_id],
            "a durably decided settlement must be retired from the Sector guard"
        );
        assert_eq!(
            acknowledgement.rejected_players,
            vec![seller],
            "the refused player must be reported so the serve loop can tell them"
        );
    }

    /// Issue #315 follow-up: `Unavailable` (this Sector could not act on the
    /// settlement this tick) must NOT reach the ledger as a rejection.
    /// Collapsing it into a "not applied" boolean compensated away orders
    /// whose ship merely changed Sector between drain and apply.
    #[test]
    fn an_unavailable_settlement_stays_pending_instead_of_being_rejected() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let (node, seller_ship) = node_with_docked_ship(seller);
        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));
        let queued = queue_first_pending_page(&mut db, &node);
        assert_eq!(queued.len(), 1);
        let settlement_id = queued[0].input().settlement_id();

        let acknowledgement = MarketSettlement::acknowledge_outcomes(
            &mut db,
            &queued,
            &[MarketSettlementOutcome {
                settlement_id,
                status: MarketSettlementStatus::Unavailable,
            }],
        );

        assert_eq!(
            db.pending_settlements_page_after(None).unwrap().len(),
            1,
            "a transiently unavailable settlement must stay pending for a later drain"
        );
        assert!(
            acknowledgement.decided_settlement_ids.is_empty(),
            "nothing was decided, so nothing may be retired from the Sector guard"
        );
        assert!(acknowledgement.rejected_players.is_empty());
    }

    /// The exact race the three-state outcome exists for: a ship this Sector
    /// no longer owns (handed off by a committed Transit inside the same
    /// frame, before preparation runs) is transient, not a refusal.
    #[test]
    fn a_settlement_for_a_departed_ship_is_unavailable_not_rejected() {
        let seller = PlayerId(1);
        let (mut host, _seller_ship) = host_with_docked_ship(seller);
        let input = MarketSettlementInput::RemoveItem(dawn_core::RemoveItemCommand {
            player_id: seller,
            ship_id: ShipId::new(NodeId(9), 999),
            item_id: ItemId::ScrapMetal,
            quantity: 1,
            settlement_id: 77,
        });

        let mut outcomes = Vec::new();
        host.run_frame_with_output(
            dawn_sector::transition::FrameInput {
                lock_commands: &[],
                authenticated_requests: &[],
                market_settlements: &[input],
                acknowledged_settlements: &[],
            },
            |_node, tick_result, _events| {
                outcomes = tick_result.market_settlement_outcomes.clone();
            },
        )
        .expect("Tick preparation must succeed");

        assert_eq!(
            outcomes,
            vec![MarketSettlementOutcome {
                settlement_id: 77,
                status: MarketSettlementStatus::Unavailable,
            }],
            "a ship this Sector cannot mutate is transient, not a refusal"
        );
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
        let (mut host, seller_ship) = host_with_docked_ship_and_scrap(seller, 5);

        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 5));
        let queued = queue_first_pending_page(&mut db, host.node());
        assert_eq!(queued.len(), 1);

        let inputs: Vec<MarketSettlementInput> =
            queued.iter().map(QueuedSettlement::input).collect();

        let mut acknowledged_outcomes = Vec::new();
        let output = host
            .run_frame_with_output(
                dawn_sector::transition::FrameInput {
                    lock_commands: &[],
                    authenticated_requests: &[],
                    market_settlements: &inputs,
                    acknowledged_settlements: &[],
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

        let acknowledgement =
            MarketSettlement::acknowledge_outcomes(&mut db, &queued, &acknowledged_outcomes);
        assert!(db.pending_settlements_page_after(None).unwrap().is_empty());
        assert_eq!(db.open_orders_for(seller).unwrap().len(), 1);
        assert_eq!(acknowledgement.decided_settlement_ids.len(), 1);

        // Retiring the acknowledged id on the next frame keeps the guard
        // bounded and must not resurrect the settlement -- the cargo
        // mutation is already durable (issue #315).
        host.run_frame(dawn_sector::transition::FrameInput {
            lock_commands: &[],
            authenticated_requests: &[],
            market_settlements: &[],
            acknowledged_settlements: &acknowledgement.decided_settlement_ids,
        })
        .expect("retiring an acknowledged settlement must commit");
    }

    /// Replaces the pre-#315 `failed_credit_refunds_currency_and_returns_reserved_item`.
    /// A credit the Sector genuinely refuses on its merits (the buyer's ship
    /// holds no room/identity this Sector will accept) must reach the ledger
    /// as a rejection so the Market compensates both sides -- refunding the
    /// buyer's currency and returning the seller's reserved item -- rather
    /// than being dropped. The old version of this test asserted only on a
    /// synthetic `settlement_id: 0` outcome, which short-circuits before any
    /// cargo logic and so proved nothing about compensation.
    #[test]
    fn a_refused_credit_makes_the_market_compensate_both_sides() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let buyer = PlayerId(2);
        let (mut host, seller_ship) = host_with_docked_ship_and_scrap(seller, 2);

        // The seller's Ask reserves cargo through the real drain/apply/ack
        // cycle, one tick's worth at a time.
        MarketSettlement::place(&mut db, seller, order(seller_ship, OrderSide::Ask, 2));
        let events = settle_one_round(&mut db, &mut host);
        assert!(db.pending_settlements_page_after(None).unwrap().is_empty());
        assert!(
            events.iter().any(|event| matches!(
                event,
                DomainEvent::ShipFitted(fitted) if fitted.ship_id == seller_ship
            )),
            "the Ask reservation must actually mutate cargo, not short-circuit"
        );
        assert_eq!(
            cargo_count_from_events(&events, seller_ship, ItemId::ScrapMetal),
            0,
            "the Ask must reserve the seller's cargo first"
        );

        // The buyer funds and matches it. Their ship belongs to no Sector in
        // this process, so the credit is refused rather than merely delayed:
        // `apply_market_item_mutation` rejects an unowned destination.
        db.credit_currency(buyer, 200).unwrap();
        let buyer_ship = ShipId::new(NodeId(0), 4242);
        MarketSettlement::place(&mut db, buyer, order(buyer_ship, OrderSide::Bid, 2));

        // The match's credit targets a ship no Sector here owns, so it is
        // `Unavailable` and correctly stays pending. Decide it once.
        expire_unroutable_settlements(&mut db, host.node());

        // Compensation cascades: rejecting the credit enqueues the refund
        // and the seller's item return, so convergence now takes several
        // ticks where the pre-#315 code looped synchronously.
        let mut events = Vec::new();
        for _ in 0..8 {
            if db.pending_settlements_page_after(None).unwrap().is_empty() {
                break;
            }
            events = settle_one_round(&mut db, &mut host);
        }

        assert!(
            db.pending_settlements_page_after(None).unwrap().is_empty(),
            "the compensation cascade must converge"
        );
        assert_eq!(
            db.currency_balance(buyer).unwrap(),
            200,
            "a refused credit must refund the buyer's currency"
        );
        assert_eq!(
            cargo_count_from_events(&events, seller_ship, ItemId::ScrapMetal),
            2,
            "a refused credit must return the seller's reserved item to cargo"
        );
    }

    /// One tick's worth of the real settlement cycle: drain -> apply through
    /// the durable `run_frame` pipeline -> acknowledge back to the ledger,
    /// retiring last round's decided ids. Returns the frame's events so a
    /// caller can read resulting cargo off the `ShipFitted` snapshot.
    ///
    /// A settlement whose ship this Sector does not own stays `Unavailable`
    /// forever by design, so it is force-rejected here the way an expiry
    /// policy eventually would -- otherwise the loop could not converge.
    fn settle_one_round(
        db: &mut MarketDb,
        host: &mut RuntimeFrameHost<InMemoryJournal, LocalRuntimeConsensus>,
    ) -> Vec<DomainEvent> {
        let queued = queue_first_pending_page(db, host.node());
        let inputs: Vec<MarketSettlementInput> =
            queued.iter().map(QueuedSettlement::input).collect();

        let mut outcomes = Vec::new();
        let output = host
            .run_frame_with_output(
                dawn_sector::transition::FrameInput {
                    lock_commands: &[],
                    authenticated_requests: &[],
                    market_settlements: &inputs,
                    acknowledged_settlements: &[],
                },
                |_node, tick_result, _events| {
                    outcomes = tick_result.market_settlement_outcomes.clone();
                },
            )
            .expect("settlement frame must commit");
        MarketSettlement::acknowledge_outcomes(db, &queued, &outcomes);
        output.events
    }

    /// Reject every still-pending settlement whose destination ship this
    /// Sector does not own, exactly once -- standing in for the expiry
    /// policy that must eventually decide a legitimately `Unavailable` row
    /// so it does not sit in the outbox forever.
    fn expire_unroutable_settlements(db: &mut MarketDb, host_node: &SimulationNode) {
        for intent in db.pending_settlements_page_after(None).unwrap() {
            let (player_id, ship_id) = intent_identity(intent);
            if !owns_docked_ship(host_node, player_id, ship_id) {
                let _ = db.execute(MarketCommand::RejectSettlement {
                    settlement_id: intent.id,
                    reason: "destination ship unavailable".to_owned(),
                });
            }
        }
    }
}
