---
scope    : 存在する全イベントと全コマンドの完全仕様。「何が起きうるか」の唯一の真実
audience : AI Agent / Human Developer
update   : イベント / コマンドを追加・変更するたびに必ず更新する
related  : entity-model.md, tick-model.md, CLAUDE.md §7
---

# Event Catalog

## 1. このカタログの使い方

### コードとの同期ルール

`dawn-core/src/events.rs` / `dawn-core/src/commands.rs` の定義と
このカタログは**常に一致していなければならない**。
イベント / コマンドを追加・変更した場合は、コードとカタログを同一 PR で更新すること。

### イベント追加の手順

```
1. このカタログに新しいイベントを追記する
2. dawn-core/src/events.rs に型を追加する
3. 対応する Command が必要なら dawn-core/src/commands.rs にも追加する
4. 単体テストを events.rs 内に書く
5. PR 説明に「変更したイベント一覧」を記載する
```

### 後方互換性ルール

**プレリリース段階（現在）:** 外部ユーザーのイベントログが存在しないため、
フィールド削除・型変更・イベント削除などの破壊的変更を直接行ってよい。

**リリース以降:**
```
許可: 新しいフィールドを Option<T> として追加する
禁止: 既存フィールドを削除する
禁止: 既存フィールドの型を変更する
禁止: 既存フィールドの名前を変更する
禁止: イベント名を変更する（代わりに V2 を新設する）
```

リリース後に破壊的変更が必要な場合は [Upcaster の手順](#6-upcasterカタログ) に従うこと。
→ 詳細は CLAUDE.md §7 参照。

---

## 2. イベント設計の原則

### Command と Event の違い

| | Command | Event |
|---|---|---|
| 意味 | 変更の**要求** | 変更が起きた**事実** |
| 拒否 | される可能性がある | されない（既に起きた） |
| 保存 | しない | Append-only で永続化 |
| ファイル | `commands.rs` | `events.rs` |

Command と Event を同じ型・同じ enum で表現してはならない（INV-006）。

### 全イベントが持つ共通フィールド

全イベントは必ず `tick: Tick` を持つ。
`tick` を省略したイベントは INV-005 違反として拒否する。

### Optional フィールドの方針

- 最初に定義するフィールドは全て必須（`Option` にしない）
- 後から追加するフィールドは全て `Option<T>` とする
- 最初から `Option` にすることは禁止（意図のない省略を許すため）

---

## 3. イベント一覧

### 3.1 Ship ライフサイクル

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `ShipSpawned` | Ship が世界に出現した | `SimulationNode::spawn_ship()` | ✅ 実装済み |
| `ShipDespawned` | Ship が世界から消えた（手動） | `SimulationNode` | 型定義のみ（発行箇所なし・Replay 対応あり） |
| `ShipDestroyed` | Ship が戦闘で破壊された | `CombatSystem` | ✅ 実装済み |

### 3.2 Movement

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `VelocityChanged` | Ship の速度が変化した | `MovementSystem::run()` | ✅ 実装済み（ADR-0008） |

### 3.3 Fitting

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `ShipFitted` | Ship の装備スロットが変更された | `SimulationNode::fit_module()` | ✅ 実装済み |
| `ModuleActivated` | Active モジュールがオンになった | `SimulationNode::activate_module_owned()` | ✅ 実装済み |
| `ModuleDeactivated` | Active モジュールがオフになった（手動 or cap 枯渇による強制 OFF） | `SimulationNode::deactivate_module_owned()` / `CapacitorSystem` | ✅ 実装済み（cap 枯渇による強制 OFF は ADR-0011 参照） |

### 3.4 Lock-on

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `TargetLocked` | ロックオンが完了した | `LockSystem::run()` | ✅ 実装済み |
| `LockLost` | ロックが消失した | `LockSystem::run()` | ✅ 実装済み |

### 3.5 Combat

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `WeaponFired` | 武器が発射された | `CombatSystem::run()` | ✅ 実装済み |
| `DamageTaken` | Ship がダメージを受けた | `CombatSystem::run()` | ✅ 実装済み |

### 3.6 Sector Transit（ADR-0014）

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `SectorTransitRequested` | Sector Transit が提案された（所有権は from のまま） | `SimulationNode::propose_transit()` | ✅ 実装済み |
| `SectorTransitCompleted` | Sector Transit が完了した（所有権が to に移った） | `SimulationNode::export_transit()` / `import_transit()`（from / to 双方が自ログに Append） | ✅ 実装済み |
| `SectorTransitAborted` | Transit が中断された（所有権は from に残る） | （宛先ノード障害時・未配線） | 型定義のみ |

バリデーション段階の拒否はイベントではなく `CommandRejected` の返却で
表現する（INV-006）。`SectorTransitRejected` というイベントは定義しない。
`propose_transit` は Ship 不在 / 既に Transit 中の場合 `Err` を返し、
イベントを発行しない。

`TransitCommand { ship_id, to }` が対応する Command（dawn-core/src/commands.rs）。
Transit Proposal（`TransitOp::Request` / `Commit`）は Raft Log を経由して
コミットされ、各ノードが Tick Step 7.5（`apply_committed_raft_entries`）で
ECS に適用したうえで上記イベントを自分の EventStore に Append する。

### 3.7 Jump Gate Navigation（ADR-0009・実装中）

| イベント名 | 説明 | 発行者 | ステータス |
|---|---|---|---|
| `JumpGateUsed` | Ship がジャンプゲートを使って別 Sector に移動した | `SectorSimulatorActor`（Step 7.5、destination ノード） | ✅ 実装済み（Raft パイプライン） |
| `StarSystemChanged` | Ship が別の星系に移動した（`JumpGateUsed` と同時） | `SectorSimulatorActor`（Step 7.5、destination ノード） | ✅ 実装済み（Raft パイプライン） |

`JumpCommand { ship_id, gate_id }` が対応する Command。
`TransitCommand` と同じ Raft Log 経路（ADR-0014）でコミットする。
`TransitOp::Request`/`Commit` は `gate_id: Option<JumpGateId>` を持ち、
Step 7.5 で destination ノードが `SectorTransitCompleted` に加えて
`JumpGateUsed` を Append し、`from`/`to` の `StarSystemId` が異なる場合は
`StarSystemChanged` も Append する（`SimulationNode::append_jump_events`）。

静的トポロジー（3 星系・4 ジャンプゲート）は `dawn-simulation/src/star_map.rs`
に定義する。`ws_server.rs` / `main.rs` / Godot クライアントへの配線は未実装
（ADR-0009 実装チェックリスト参照）。

### 3.8 System（将来予約）

| イベント名 | 説明 | ステータス |
|---|---|---|
| `TickStarted` | Tick の開始 | 未実装 |
| `TickCompleted` | Tick の完了 | 未実装 |

---

## 4. コマンド一覧

コマンドは `dawn-core/src/commands.rs` で定義される。
クライアントからサーバーへは `ClientCommand` enum（`dawn-actor`）でラップして送信する。

| コマンド名 | 説明 | 対応イベント | ステータス |
|---|---|---|---|
| `MoveCommand` | 推力方向を指定する | — | ✅ 実装済み |
| `LockOnCommand` | ロックオン開始を要求する | `TargetLocked` | ✅ 実装済み |
| `FitModuleCommand` | モジュールを装備する | `ShipFitted` | ✅ 実装済み |
| `ActivateModuleCommand` | Active モジュールをオンにする | `ModuleActivated` | ✅ 実装済み |
| `DeactivateModuleCommand` | Active モジュールをオフにする | `ModuleDeactivated` | ✅ 実装済み |
| `AttackCommand` | 攻撃対象を指定する | `WeaponFired` | ✅ 型定義・WsServer JSON パーサー実装済み（Phase 5）|
| `StopCommand` | 加速度を用いて速度をゼロに減速する | — | ✅ 実装済み |
| `TransitCommand` | Sector Transit を要求する（Raft 経由・ADR-0014） | `SectorTransitRequested` / `Completed` | ✅ 実装済み |

---

## 5. イベント詳細仕様

### `ShipSpawned`

**説明:** Ship が Sector 内に生成された。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 生成された Ship の一意な識別子 |
| `sector_id` | `SectorId` | ✓ | 生成先の Sector |
| `initial_position` | `Position` | ✓ | 生成時の座標 |
| `ship_type_id` | `ShipTypeId` | ✓ | 船種 ID（`ShipTypeDefinition` レジストリで解決） |
| `tick` | `Tick` | ✓ | 生成された Tick |

**不変条件:** `ship_id` は世界全体で一意であり、再利用されない（INV-004）。
`ship_type_id` を含めることで Replay 時に正確な base_stats が復元できる（INV-002）。

---

### `VelocityChanged`

**説明:** Ship の速度が変化した。`MovementSystem` が物理計算を行い、前 Tick から速度が変わった場合のみ発行する。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id`  | `ShipId`  | ✓ | 速度が変わった Ship |
| `velocity` | `Velocity` | ✓ | 変化後の速度ベクトル（units/tick） |
| `tick`     | `Tick`    | ✓ | 速度が確定した Tick |

**不変条件:** `velocity` は前 Tick と異なる値でなければ発行しない（変化なしはイベントを出さない）。

**Replay:** `VelocityChanged` を時系列に適用し、各 Tick で `position += velocity` を計算する。
物理シミュレーションは不要。`position += velocity` は純粋な算術である。

**設計根拠:** 位置は派生状態であり権威的イベントに含めない。
物理入力（推力）もコマンドであり権威的イベントに含めない（ADR-0008）。

---

### `ShipDespawned`

**説明:** Ship が世界から永続的に取り除かれた（手動削除）。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 消滅した Ship |
| `tick` | `Tick` | ✓ | 消滅した Tick |

---

### `ShipFitted`

**説明:** Ship の装備スロットが変更された。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 装備を変更した Ship |
| `fitting` | `FittingSnapshot` | ✓ | 変更後の全スロットのスナップショット（モジュール ID リスト） |
| `tick` | `Tick` | ✓ | 装備変更が確定した Tick |

**設計メモ:** `stats` フィールドは持たない。Replay 時は `FittingSnapshot` から
`apply_fitting()` で再計算するため（INV-002 準拠）。

---

### `TargetLocked`

**説明:** `LockSystem` のカウントダウンが完了し、ロックオンが確立した。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | ロックした Ship |
| `target_id` | `ShipId` | ✓ | ロックされた Ship |
| `tick` | `Tick` | ✓ | ロックが完了した Tick |

**Replay:** `LockComp` の該当エントリを `Locked` 状態に更新する。

---

### `LockLost`

**説明:** ロックが消失した。ターゲットが撃沈または射程外になった場合に発行する。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `locker_id` | `ShipId` | ✓ | ロックを失った Ship |
| `target_id` | `ShipId` | ✓ | ロック対象だった Ship |
| `tick` | `Tick` | ✓ | ロックが消失した Tick |

**Replay:** `LockComp` から該当エントリを削除する。

---

### `WeaponFired`

**説明:** 武器が発射され、かつ命中した。ミス（命中率チェック失敗）の場合はイベントを発行しない。
ダメージは同 Tick の `DamageTaken` で確認できる。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `attacker_id` | `ShipId` | ✓ | 発射した Ship |
| `target_id` | `ShipId` | ✓ | 攻撃対象の Ship |
| `damage` | `f32` | ✓ | 実際に与えるダメージ量（基礎ダメージ × ランダム倍率 0.49〜1.49、1%確率で 3.0） |
| `tick` | `Tick` | ✓ | 発射した Tick |

**発行条件（ADR-0012）:**
1. ターゲットが `LockComp` で `Locked` 状態である
2. Capacitor サイクルが開始された Tick である（`fire_triggers` に含まれる）
3. 命中率チェックを通過した（`rand() < hit_chance`）

命中率 = `0.5 ^ ((angular / (tracking × sig))² + (max(0, dist − optimal) / falloff)²)`

**Replay:** ECS 状態を変更しない（発射ログのみ）。`damage` フィールドに実際の値が記録されているため、
Replay 時は乱数を再計算しない。

---

### `DamageTaken`

**説明:** Ship がダメージを受け、HP が変化した。
HP は Shield → Armor → Hull の順に消費される。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | ダメージを受けた Ship |
| `damage` | `f32` | ✓ | 受けたダメージ量（適用前） |
| `current_shield` | `f32` | ✓ | ダメージ後のシールド残量 |
| `current_armor` | `f32` | ✓ | ダメージ後のアーマー残量 |
| `current_hull` | `f32` | ✓ | ダメージ後のハル残量 |
| `tick` | `Tick` | ✓ | ダメージを受けた Tick |

**設計メモ:** 3 フィールドを含めることで Replay 時に `HullComp` を正確に復元できる（INV-002 準拠）。

---

### `ModuleActivated`

**説明:** Active モジュールがオンになった。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id`   | `ShipId`  | ✓ | 操作した Ship |
| `module_id` | `ModuleId` | ✓ | 対象モジュール |
| `slot`      | `SlotKind` | ✓ | 装備スロット種別 |
| `tick`      | `Tick`    | ✓ | 活性化した Tick |

**設計メモ:** `is_active: true` という状態変化ではなく「オンにした」という事実として表現する。
Replay 時は `FittedSlot.is_active = true` にセットし、`apply_fitting()` を再実行する。

---

### `ModuleDeactivated`

**説明:** Active モジュールがオフになった。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id`   | `ShipId`  | ✓ | 操作した Ship |
| `module_id` | `ModuleId` | ✓ | 対象モジュール |
| `slot`      | `SlotKind` | ✓ | 装備スロット種別 |
| `tick`      | `Tick`    | ✓ | 非活性化した Tick |

**設計メモ:** `ModuleActivated` の対。Replay 時は `FittedSlot.is_active = false` にセットする。

---

### `ShipDestroyed`

**説明:** Ship が戦闘で HP ゼロになり破壊された。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 破壊された Ship |
| `killer_id` | `ShipId` | ✓ | 最後の一撃を与えた Ship |
| `tick` | `Tick` | ✓ | 破壊された Tick |

**Replay:** `ship_id` に対応する Entity を ECS と `ship_index` から削除する。

---

### `SectorTransitRequested`

**説明:** Sector Transit が Raft でコミットされた。所有権は `SectorTransitCompleted` まで `from` に残る（ADR-0014）。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Transit する Ship |
| `from` | `SectorId` | ✓ | 現在の所有 Sector |
| `to` | `SectorId` | ✓ | 宛先 Sector |
| `tick` | `Tick` | ✓ | コミットが適用された Tick |

**Replay:** `TransitComp` を `InTransit { to }` に更新する。

---

### `SectorTransitCompleted`

**説明:** Sector Transit が完了し、所有権が `from` から `to` に移った。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Transit した Ship |
| `from` | `SectorId` | ✓ | 元の所有 Sector |
| `to` | `SectorId` | ✓ | 新しい所有 Sector |
| `entry_pos` | `Position` | ✓ | 宛先 Sector での入場座標 |
| `velocity` | `Velocity` | ✓ | 入場時の速度（INV-002: Replay で完全復元するため必須） |
| `tick` | `Tick` | ✓ | 完了した Tick |

**Replay:** from ノードでは Ship を ECS から削除、to ノードでは `entry_pos` / `velocity` で Ship を追加する。

---

### `SectorTransitAborted`

**説明:** コミット済み Transit が中断された。所有権は `from` に残る。
バリデーション段階の拒否は `CommandRejected` で表現し、本イベントは発行しない（INV-006）。

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Transit を中断した Ship |
| `from` | `SectorId` | ✓ | 所有 Sector（変わらない） |
| `to` | `SectorId` | ✓ | 中断された宛先 Sector |
| `tick` | `Tick` | ✓ | 中断が確定した Tick |

**ステータス:** 型定義のみ（宛先ノード障害時の発行は未配線）。

---

## 6. Upcasterカタログ

破壊的変更があった場合にのみここに記録する。

現時点での破壊的変更: **なし**

### Upcaster の実装手順（将来のための記録）

```
1. 旧イベントを Deprecated としてマークする（削除しない）
2. 新イベントを別名（V2）で定義する
3. impl Upcaster for 旧イベント { fn upcast(self) -> 新イベント } を実装する
4. Replay パスで Upcaster を通す
5. このカタログに変更履歴を記録する
6. 新 ADR を作成する
```
