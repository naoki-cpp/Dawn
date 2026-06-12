---
id      : ADR-0009
title   : 星系間ナビゲーション — StarSystem / JumpGate 設計
status  : accepted
date    : 2026-06-06
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0003（Local-First）, ADR-0014（Raft Consensus）, CLAUDE.md §5（Entity Ownership）
---

> **着手時の補足（2026-06-12・Phase 7 完了後）**
>
> 本 ADR は Phase 7（ADR-0014: Raft Log によるリーダー選出 / Sector Transit）の
> 完成を前提として `deferred` から `accepted` に変更し、実装を開始する。
>
> §5「実装方針」で「Raft 導入後の設計スケッチ」としていた部分を確定する:
>
> - `JumpCommand` は `TransitCommand` と同じ経路（バリデーション →
>   `RaftActor::propose` → Raft Log コミット → Tick Step 7.5
>   `apply_committed_raft_entries`）で Sector 変更を行う。
> - `TransitOp::Request` / `Commit` のペイロードに「ゲート経由か否か」を
>   表す情報（`Option<JumpGateId>`）を含め、コミット適用時に
>   `SectorTransitCompleted` に加えて `JumpGateUsed` を Append する。
>   （`JumpGateUsed` は `SectorTransitCompleted` を置き換えるものではなく、
>   「どう移動したか」を記録する追加イベント）
> - `to_sector` が別 `StarSystemId` に属する場合のみ `StarSystemChanged` も
>   同 Tick で Append する。
> - スコープ制約の「追加しない: Raft 経由の排他制御（Phase 7 以降）」は
>   Phase 7 完了により解消された — Raft 経由が前提になる。

# ADR-0009 — 星系間ナビゲーション

## 背景

現在のシミュレーションは 1 Sector 固定で運用している。
プレイヤーから「複数星系へ移動したい」というフィードバックがあり、
星系間ナビゲーション（Jump Gate による Star System 跨ぎ移動）の追加を検討する。

---

## 決定

### 1. 概念モデル

```
StarSystem（星系）
  └── Sector（宙域）× 1..N
        └── JumpGate（ジャンプゲート）× 0..N
              └── 宛先: 別 Sector（同一または別星系）
```

- **StarSystem** はメタレイヤー。Ship は常に特定の Sector に存在し、Sector が StarSystem に帰属する。
- **JumpGate** は Sector 内の固定座標に配置されたオブジェクト。Ship が一定距離内に近づくと
  ジャンプコマンドが有効になる。
- Sector 間の遷移は **既存の Entity Ownership ルール**（CLAUDE.md §5）を継承する。
  Ship の所有権は常に 1 つの Sector が保持し、Transit 中は元 Sector が保有し続ける。

### 2. 新規型定義（dawn-core）

```rust
// dawn-core/src/star_system.rs

/// 星系の識別子。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StarSystemId(pub u32);

/// 星系の静的定義（マップデータ）。
pub struct StarSystemDef {
    pub id     : StarSystemId,
    pub name   : &'static str,
    pub sectors: Vec<SectorId>,
}

/// ジャンプゲートの識別子。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JumpGateId(pub u32);

/// ジャンプゲートの静的定義。
pub struct JumpGateDef {
    pub id           : JumpGateId,
    pub from_sector  : SectorId,
    pub position     : Position,      // Sector 内座標
    pub to_sector    : SectorId,      // 宛先 Sector（別星系でも可）
    pub activation_radius: f32,       // この距離内でジャンプ有効
}
```

### 3. 新規イベント（dawn-core/src/events.rs）

イベントソーシング原則（INV-001〜INV-002）に従い、
位置ではなく「何が起きたか」を記録する。

```rust
/// Ship がジャンプゲートを通過し、別 Sector に移動した。
///
/// 生成タイミング: JumpGate 範囲内で JumpCommand が受理された後。
pub struct JumpGateUsed {
    pub ship_id    : ShipId,
    pub gate_id    : JumpGateId,
    pub from_sector: SectorId,
    pub to_sector  : SectorId,
    pub entry_pos  : Position,   // 宛先 Sector の出現座標
    pub tick       : Tick,
}

/// Ship が別の星系に移動した（StarSystem レイヤーでの変化通知）。
///
/// JumpGateUsed と同時に発行される（宛先が別星系の場合のみ）。
pub struct StarSystemChanged {
    pub ship_id    : ShipId,
    pub from_system: StarSystemId,
    pub to_system  : StarSystemId,
    pub tick       : Tick,
}
```

### 4. 新規コマンド（dawn-core/src/commands.rs）

```rust
/// ジャンプゲートを使って別 Sector に移動する。
///
/// 拒否条件:
///   - Ship が gate の activation_radius 外にいる
///   - Ship が Transit 中（CLAUDE.md §5）
///   - Ship が存在しない
pub struct JumpCommand {
    pub ship_id: ShipId,
    pub gate_id: JumpGateId,
}
```

### 5. 実装方針

#### 実装タイミング：Phase 7 以降

Sector Transit の整合性保証（Raft）が完成してから実装する。
単一プロセスの簡易実装は行わない（Phase 7 で書き直しが発生するため）。

```
JumpCommand 受信
  → Ship が gate_activation_radius 内にいるか検証
  → Ship を from_sector の ECS から削除
  → Ship を to_sector の ECS に追加（entry_pos に配置）
  → JumpGateUsed イベントを EventStore に Append
  → 宛先が別星系なら StarSystemChanged も Append
  → Godot へブロードキャスト
```

上記フローは Raft 導入後の設計スケッチである。
Sector 間遷移は Raft 経由で排他制御し（INV-003）、
Transit 中の状態管理もその時点で詳細化する。

### 6. 星系マップ（静的定義）

初期実装として 3 星系・各 1 Sector のトポロジーを用意する。

```
Alpha System ←→ Beta System ←→ Gamma System
   (SectorId 0)       (SectorId 1)       (SectorId 2)

Jump Gates:
  Gate 0: Sector 0 → Sector 1（Alpha → Beta）
  Gate 1: Sector 1 → Sector 0（Beta → Alpha）
  Gate 2: Sector 1 → Sector 2（Beta → Gamma）
  Gate 3: Sector 2 → Sector 1（Gamma → Beta）
```

各ゲートは Sector の端部（外縁部）に配置する。

### 7. Godot 側の対応

`JumpGateUsed` イベントを受信したら:
- `ship_id` の Ship を現在位置から `entry_pos` へ瞬間移動
- カメラがプレイヤー船を追従している場合、新しい Sector の背景/環境に切り替え
- HUD に「別星系に移動した」表示

新メッセージタイプを ws_server.rs に追加（`EventJson` enum への variant 追加）。

---

## 却下した代替案

### 案A: ワープトンネル（連続移動）

Ship がゲートに近づくと「ワープトンネルモード」になり、
物理的な移動アニメーションで移動する。

**却下理由**: ワープ中の Ship の物理状態をどう扱うか（速度・位置の連続性）が
Event Sourcing と相性が悪い。「移動アニメーション」はクライアント側の表現であり、
サーバーは「瞬時に Sector が変わった」として記録する。

### 案B: 惑星・天体を Entity として追加

星・惑星・小惑星帯を ECS の Entity として追加する。

**却下理由**: Ship のみが Entity というスコープ制約（CLAUDE.md §1）に違反する。
天体は静的な環境データ（マップデータ）として扱い、ECS に含めない。

---

## スコープ制約

この ADR で追加するのは以下に限定する。

```
追加する:
  - StarSystemId, JumpGateId 型
  - JumpGateDef, StarSystemDef（静的マップデータ）
  - JumpCommand コマンド
  - JumpGateUsed, StarSystemChanged イベント
  - dawn-simulation 内の Jump 処理ロジック
  - ws_server.rs への JumpGateUsed ブロードキャスト
  - Godot 側の JumpGateUsed 受信 + Ship 瞬間移動

追加しない:
  - 惑星・天体エンティティ（Case B 却下済み）
  - 市場・ステーション（FBD-008）
  - 採掘コンテンツ（FBD-009）
  - Raft 経由の排他制御（Phase 7 以降）
```

---

## 実装チェックリスト

### dawn-core

- [x] `src/star_system.rs` 追加（`StarSystemId`, `JumpGateId`, `StarSystemDef`, `JumpGateDef`）
- [x] `src/events.rs` に `JumpGateUsed`, `StarSystemChanged` 追加
- [x] `src/commands.rs` に `JumpCommand` 追加
- [x] `src/lib.rs` に re-export 追加
- [x] 各型に単体テスト追加

### dawn-simulation

- [x] `src/star_map.rs` 追加（初期 3 星系トポロジーの静的定義）
- [x] `src/node.rs` に `jump_gates: HashMap<JumpGateId, JumpGateDef>` フィールド追加
- [x] `src/node.rs` に `can_propose_jump` / `append_jump_events` 実装
  - Ship が gate_activation_radius 内にいるか確認（`can_propose_jump`）
  - Sector 遷移は既存の `TransitOp` Raft パイプライン経由（`gate_id: Option<JumpGateId>` を追加）
  - `JumpGateUsed` イベント Append（`append_jump_events`、Step 7.5）
  - 別星系なら `StarSystemChanged` イベント Append（`append_jump_events`）
- [x] `src/ws_server.rs` に `JumpGateUsed`, `StarSystemChanged` の EventJson 追加
- [x] `src/ws_server.rs` に `JumpCommand` の JSON パーサー追加
- [x] `src/main.rs` に `ClientCommand::Jump` のマッチアーム追加
  （`--serve` は単一 Sector ノードのため Raft なしの遷移は行わず無視する。
  FBD-006 参照。フル経路は `MultiNodeCluster` の統合テストで検証済み）
- [x] 統合テスト: ジャンプ後に Ship が宛先 Sector に存在すること（`committed_jump_moves_ship_to_gates_destination_sector`）

### dawn-actor

- [x] `ClientCommand` に `Jump(JumpCommand)` variant 追加

### Godot（client/）

- [ ] `connection.gd` に `jump_gate_used` シグナル追加
- [ ] `main.gd` の `_on_jump_gate_used` で Ship 位置を `entry_pos` に更新
- [ ] `main.gd` のジャンプコマンド送信（ゲート近接時に UI を表示し確定）

### docs

- [ ] `docs/event-catalog.md` に `JumpGateUsed`, `StarSystemChanged` を追記
- [ ] `docs/roadmap.md` を更新

---

## 参照

- CLAUDE.md §5: Entity Ownership Rules（SectorTransit 設計の原則を継承）
- CLAUDE.md §1: 現在のスコープ
- ADR-0001: Event Sourcing（JumpGateUsed はイベント、位置は派生状態）
- ADR-0003: Local-First（Raft なしで単一プロセスで実装する）
- docs/tick-model.md: Tick 順序（Jump 処理は Combat の後に行う）
