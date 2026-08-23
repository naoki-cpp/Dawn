//! Lock system — ロックオン進行 / 完了 / 消失 / NPC 自動ロック。
//!
//! # 処理順序（CLAUDE.md §6 — Movement の後、Combat の前に呼ぶこと）
//!
//! 各 Tick で次を行う:
//!   1. 既存ロックエントリを処理する
//!      a. Locking: remaining_ticks をデクリメント。0 → Locked → TargetLocked イベント
//!      b. ターゲットが存在しない → LockLost イベント → エントリ削除
//!   2. 外部から渡された `LockOnCommand` を処理する（プレイヤー操作）
//!   3. NPC 自動ロック: スロット空き + 武器あり → 射程内最近傍に自動ロック開始
//!
//! # Contract
//! - 純粋計算: I/O なし、グローバル状態なし。
//! - 返り値の events を durable transition journal に含めるのは呼び出し元の責務。

use crate::{
    components::{
        IsNpcComp, LockComp, LockEntry, LockState, PositionComp, ShipIdComp, ShipStatsComp,
    },
    SimWorld,
};
use dawn_core::{
    events::{LockLost, TargetLocked},
    DomainEvent, LockOnCommand, Position, ShipId, Tick,
};
use std::collections::HashSet;

/// Lock System の実行結果。
#[derive(Debug)]
pub struct LockResult {
    pub events: Vec<DomainEvent>,
}

// Internal snapshot type. `entity` is captured so write-back is O(1) per ship
// instead of a full-world scan (ADR-0019: server compute stays O(n)).
struct ShipSnap {
    entity: hecs::Entity,
    ship_id: ShipId,
    pos: Position,
    stats: ShipStatsComp,
    lock_comp: LockComp,
    is_npc: bool, // true = NPC（自動ロック有効）/ false = プレイヤー
}

/// 1 Tick 分のロック処理を実行する。
///
/// `pending_commands`: その Tick に届いた `LockOnCommand`（プレイヤー操作）。
pub fn run(world: &mut SimWorld, tick: Tick, pending_commands: &[LockOnCommand]) -> LockResult {
    // ── 1. 全 Ship をスナップショット ────────────────────────────────────────

    let mut ships: Vec<ShipSnap> = world
        .query::<(
            hecs::Entity,
            &ShipIdComp,
            &PositionComp,
            &ShipStatsComp,
            &LockComp,
            Option<&IsNpcComp>,
        )>()
        .iter()
        .map(|(entity, id, pos, stats, lock, npc)| ShipSnap {
            entity,
            ship_id: id.0,
            pos: pos.0,
            stats: *stats,
            lock_comp: lock.clone(),
            is_npc: npc.is_some(),
        })
        .collect();

    let alive: HashSet<ShipId> = ships.iter().map(|s| s.ship_id).collect();
    let mut events: Vec<DomainEvent> = Vec::new();

    // ── 2. 各 Ship のロック状態を更新 ────────────────────────────────────────

    for i in 0..ships.len() {
        let max_locks = ships[i].stats.max_locks;
        let lock_time = ships[i].stats.lock_time;
        let self_id = ships[i].ship_id;

        // 既存エントリを処理（消滅チェック + カウントダウン）
        let mut new_entries: Vec<LockEntry> = Vec::new();
        for entry in ships[i].lock_comp.entries.drain(..) {
            if !alive.contains(&entry.target_id) {
                // ターゲット消滅 → LockLost
                events.push(DomainEvent::LockLost(LockLost {
                    locker_id: self_id,
                    target_id: entry.target_id,
                    tick,
                }));
                continue;
            }
            match entry.state {
                LockState::Locking { remaining_ticks } if remaining_ticks <= 1 => {
                    // ロック完了
                    events.push(DomainEvent::TargetLocked(TargetLocked {
                        locker_id: self_id,
                        target_id: entry.target_id,
                        tick,
                    }));
                    new_entries.push(LockEntry {
                        target_id: entry.target_id,
                        state: LockState::Locked,
                    });
                }
                LockState::Locking { remaining_ticks } => {
                    new_entries.push(LockEntry {
                        target_id: entry.target_id,
                        state: LockState::Locking {
                            remaining_ticks: remaining_ticks - 1,
                        },
                    });
                }
                LockState::Locked => {
                    new_entries.push(entry);
                }
            }
        }
        // drain 後の確定エントリをセット（has_target / has_capacity の判定に使う）
        ships[i].lock_comp.entries = new_entries;

        // プレイヤーコマンドを適用
        for cmd in pending_commands {
            if cmd.ship_id != self_id {
                continue;
            }
            if cmd.target_id == self_id {
                continue;
            }
            if ships[i].lock_comp.has_target(cmd.target_id) {
                continue;
            }
            if !ships[i].lock_comp.has_capacity(max_locks) {
                continue;
            }
            if !alive.contains(&cmd.target_id) {
                continue;
            }
            // lock_time を initial remaining とする（lock_time Tick 後に Locked になる）
            // remaining = lock_time - 1 で開始し、毎 Tick デクリメント、0 → Locked
            let initial = lock_time.saturating_sub(1);
            ships[i].lock_comp.entries.push(LockEntry {
                target_id: cmd.target_id,
                state: if initial == 0 {
                    LockState::Locked // lock_time=1 なら即 Locked
                } else {
                    LockState::Locking {
                        remaining_ticks: initial,
                    }
                },
            });
        }

        // NPC 自動ロック（NPC のみ + 武器あり + スロット空き）
        // プレイヤー船は手動 LockOnCommand でのみロックする
        if ships[i].is_npc
            && ships[i].stats.weapon_damage > 0.0
            && ships[i].lock_comp.has_capacity(max_locks)
        {
            let origin = ships[i].pos;
            let range = ships[i].stats.weapon_range;
            let nearest = ships
                .iter()
                .filter(|s| s.ship_id != self_id)
                .filter(|s| {
                    let dx = s.pos.x - origin.x;
                    let dy = s.pos.y - origin.y;
                    let dz = s.pos.z - origin.z;
                    (dx * dx + dy * dy + dz * dz).sqrt() <= f64::from(range)
                })
                .min_by(|a, b| {
                    let dist = |s: &&ShipSnap| -> f64 {
                        let dx = s.pos.x - origin.x;
                        let dy = s.pos.y - origin.y;
                        let dz = s.pos.z - origin.z;
                        (dx * dx + dy * dy + dz * dz).sqrt()
                    };
                    dist(a)
                        .partial_cmp(&dist(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|s| s.ship_id);

            if let Some(target) = nearest {
                if !ships[i].lock_comp.has_target(target) {
                    let initial = lock_time.saturating_sub(1);
                    ships[i].lock_comp.entries.push(LockEntry {
                        target_id: target,
                        state: if initial == 0 {
                            LockState::Locked
                        } else {
                            LockState::Locking {
                                remaining_ticks: initial,
                            }
                        },
                    });
                }
            }
        }
    }

    // ── 3. Write back to ECS (O(1) per ship via captured entity handle) ──────

    for s in ships {
        if let Some(mut lock) = world.get_mut::<LockComp>(s.entity) {
            *lock = s.lock_comp;
        }
    }

    LockResult { events }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{components::ShipStatsComp, SimWorld};
    use dawn_core::{NodeId, Position, SectorId, ShipId, Tick, Velocity};

    fn ship_id(n: u64) -> ShipId {
        ShipId::new(NodeId(0), n)
    }

    fn armed_stats() -> ShipStatsComp {
        ShipStatsComp {
            weapon_damage: 10.0,
            weapon_range: 2_000.0,
            lock_time: 3,
            max_locks: 1,
            ..ShipStatsComp::NPC
        }
    }

    fn setup(pos_a: Position, pos_b: Position, stats: ShipStatsComp) -> SimWorld {
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), pos_a, Velocity::ZERO);
        let eb = world.spawn_ship(ship_id(2), pos_b, Velocity::ZERO);
        world.set_ship_stats(ea, stats);
        world.set_ship_stats(eb, stats);
        world
    }

    #[test]
    fn npc_auto_lock_starts_when_target_is_in_range() {
        let mut world = setup(
            Position::ORIGIN,
            Position::new(500.0, 0.0, 0.0),
            armed_stats(),
        );
        run(&mut world, Tick(1), &[]);
        let any_locking = world
            .query::<&LockComp>()
            .iter()
            .any(|l| !l.entries.is_empty());
        assert!(any_locking, "locking entry created when target is in range");
    }

    #[test]
    fn no_auto_lock_when_target_is_out_of_range() {
        let mut world = setup(
            Position::ORIGIN,
            Position::new(10_000.0, 0.0, 0.0),
            armed_stats(),
        );
        run(&mut world, Tick(1), &[]);
        let any_locking = world
            .query::<&LockComp>()
            .iter()
            .any(|l| !l.entries.is_empty());
        assert!(!any_locking, "no auto-lock when target is out of range");
    }

    #[test]
    fn target_locked_event_emitted_after_lock_time_ticks() {
        let mut world = setup(
            Position::ORIGIN,
            Position::new(500.0, 0.0, 0.0),
            armed_stats(),
        );
        run(&mut world, Tick(1), &[]);
        run(&mut world, Tick(2), &[]);
        let r3 = run(&mut world, Tick(3), &[]);
        let count = r3
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::TargetLocked(_)))
            .count();
        assert!(count >= 1, "TargetLocked emitted after lock_time=3 ticks");
    }

    #[test]
    fn lock_lost_emitted_when_target_is_removed_from_world() {
        let mut world = setup(
            Position::ORIGIN,
            Position::new(500.0, 0.0, 0.0),
            armed_stats(),
        );
        run(&mut world, Tick(1), &[]);
        run(&mut world, Tick(2), &[]);
        run(&mut world, Tick(3), &[]); // Locked になる

        // Remove target from ECS.
        let entity = world.find_entity(ship_id(2)).unwrap();
        world.despawn_ship(entity);

        let r = run(&mut world, Tick(4), &[]);
        let lost = r
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::LockLost(_)))
            .count();
        assert!(lost >= 1, "LockLost emitted when target is despawned");
    }

    #[test]
    fn player_lock_on_command_initiates_lock() {
        // weapon_damage=0 → NPC 自動ロックしない
        let unarmed = ShipStatsComp {
            weapon_damage: 0.0,
            lock_time: 2,
            max_locks: 1,
            ..ShipStatsComp::NPC
        };
        let mut world = setup(Position::ORIGIN, Position::new(500.0, 0.0, 0.0), unarmed);
        let cmd = LockOnCommand {
            ship_id: ship_id(1),
            target_id: ship_id(2),
        };
        run(&mut world, Tick(1), &[cmd]);
        let lock_started = world
            .query::<(&ShipIdComp, &LockComp)>()
            .iter()
            .any(|(id, l)| id.0 == ship_id(1) && !l.entries.is_empty());
        assert!(lock_started, "lock initiated by LockOnCommand");
    }

    #[test]
    fn max_locks_limits_simultaneous_lock_count() {
        // max_locks=1 で2コマンド送っても1つしかロックされない
        let unarmed = ShipStatsComp {
            weapon_damage: 0.0,
            lock_time: 2,
            max_locks: 1,
            ..ShipStatsComp::NPC
        };
        let mut world = SimWorld::new(SectorId(0));
        let ea = world.spawn_ship(ship_id(1), Position::ORIGIN, Velocity::ZERO);
        world.spawn_ship(ship_id(2), Position::new(100.0, 0.0, 0.0), Velocity::ZERO);
        world.spawn_ship(ship_id(3), Position::new(200.0, 0.0, 0.0), Velocity::ZERO);
        world.set_ship_stats(ea, unarmed);

        let cmds = vec![
            LockOnCommand {
                ship_id: ship_id(1),
                target_id: ship_id(2),
            },
            LockOnCommand {
                ship_id: ship_id(1),
                target_id: ship_id(3),
            },
        ];
        run(&mut world, Tick(1), &cmds);

        let lock_count = world
            .query::<(&ShipIdComp, &LockComp)>()
            .iter()
            .find(|(id, _)| id.0 == ship_id(1))
            .map(|(_, l)| l.entries.len())
            .unwrap_or(0);
        assert_eq!(
            lock_count, 1,
            "max_locks=1 allows only one simultaneous lock"
        );
    }
}
