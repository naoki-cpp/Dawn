---
id      : ADR-0015
title   : アプローチ（半自動操船） — ApproachCommand / ApproachComp
status  : accepted
date    : 2026-06-13
deciders: [human, ai-agent]
related : ADR-0008（Ship Movement Events）, CLAUDE.md §6（Tick Model）, CLAUDE.md §1（Scope）
---

# ADR-0015 — アプローチ（半自動操船）

## 背景

現在の操船はダブルクリックによる `MoveCommand`（推力方向の一回指定）のみである。
ターゲットが移動すると、プレイヤーは追従するために何度も推力方向を取り直す必要がある。

EVE Online の "Approach"（対象に自動で接近し続ける）に相当する半自動操船を追加し、
「どの相手に詰めるか / 距離を取るか」という戦術判断にプレイヤーが集中できるようにする。

> 設計の中心的な問い（docs/game-design.md）:
> 「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
> → アプローチは「接近 / 離脱の判断」を低操作コストで実行可能にし、
>   ロックオン・射程管理・モジュール操作といった判断に注意を割けるようにする。Yes。

---

## 決定

### 1. アプローチは「持続的な操船モード」である

一度 `ApproachCommand` を受理すると、対象が存在する限り **毎 Tick** 推力方向を
対象の最新位置へ向け直す。Bot AI（`process_bots`）が既に行っている操船ロジックと
同一の振る舞いを、プレイヤーに開放するものである。

```
ApproachCommand { ship_id, target_id }
  → ApproachComp { target } を ship に付与
  → 毎 Tick process_approach() が対象方向へ thrust を更新
  → 一定距離（ARRIVAL_RADIUS）まで詰めたら is_braking = true で停止
```

### 2. 新規型定義

```rust
// dawn-core/src/commands.rs
pub struct ApproachCommand {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
}

// dawn-ecs/src/components/movement.rs
/// Persistent "approach" steering target. While present, the node's
/// process_approach() step recomputes ThrustComp toward the target each tick.
pub struct ApproachComp {
    pub target: ShipId,
}
```

### 3. Tick 処理順序への追加（CLAUDE.md §6）

`process_approach()` を **Movement System（Step 3）の直前** に実行する新しい
Step 2.5 として追加する。対象の最新位置に基づいて thrust を更新してから
Movement で位置を積分するため、追従の遅延を 1 Tick 減らせる。

```
2.  コマンドキュー処理（ApproachCommand → ApproachComp 付与 / Move・Stop で解除）
2.5 Approach System（process_approach）        ← 新規。Movement の前
3.  Movement System
4.  Capacitor System
...
```

### 4. アプローチの解除条件

以下のいずれかで `ApproachComp` を除去する:

- 同じ船に対して `MoveCommand`（手動推力）が発行された
- `StopCommand` が発行された
- 対象 Ship が ECS から消えた（破壊・Transit などで `ship_index` に無い）
  - この場合は安全のため `is_braking = true`（惰性で飛び続けない）
- 自船が `TransitState::InTransit` に入った（移動コマンド全般が拒否されるため自然に無効）

### 5. イベントは追加しない（ADR-0008 / INV-MOVE 準拠）

アプローチは「コマンド（意図）」であり「事実（イベント）」ではない。
毎 Tick の thrust 更新により速度が変化すれば、既存の `VelocityChanged` が
Movement System から自然に発行される。**Approach 専用のイベントは作らない。**

- `ApproachComp` は `ThrustComp` と同じく派生的な操船状態であり、
  `ShipSnapshot`（スナップショット）には**含めない**。
  リプレイは `VelocityChanged` の列から完全に再現でき、アプローチ状態の
  永続化は不要（INV-002 を損なわない）。

### 6. 対象の選択はクライアント側で行う

ロックオンとは独立に、プレイヤーが**クリックで選択した船**を対象にする。
クライアントは選択中の `target_id` を保持し、A キーで `ApproachCommand` を送る。
サーバーは所有権（`ship_owners`）を確認してから `ApproachComp` を付与する。

---

## 影響

| 対象 | 変更 |
|---|---|
| `dawn-core` | `ApproachCommand` を追加（commands.rs） |
| `dawn-ecs` | `ApproachComp` を追加（components/movement.rs） |
| `dawn-simulation` | `SimulationNode::process_approach` / `apply_approach_command_owned`、tick への Step 2.5 組み込み、Move/Stop での解除 |
| `dawn-actor` | `ClientCommand::Approach` を追加 |
| `dawn-simulation`（ws/cluster） | JSON パース・コマンドディスパッチ |
| クライアント | クリック選択 + A キー送信、HUD 表示 |
| CLAUDE.md | §1 スコープに「ApproachComp / ApproachCommand」、§6 Tick 順序に Step 2.5 を追記（人間承認のうえ） |

イベントスキーマ（event-catalog.md）の変更は**なし**（新イベントを作らないため）。

---

## 実装チェックリスト

- [x] `ApproachCommand` を dawn-core に追加（+ test）
- [x] `ApproachComp` を dawn-ecs に追加
- [x] `process_approach()` を node.rs に実装し、tick の Step 2.5 として呼ぶ
- [x] `apply_move_command` / `apply_stop_command` で ApproachComp を解除
- [x] `apply_approach_command_owned`（所有権チェック付き）を追加（+ test）
- [x] 対象消失時に解除 + ブレーキする（+ test）
- [x] `ClientCommand::Approach` を dawn-actor に追加 + ws_server パース（+ test）
- [x] run_cluster_server / run_phase4_server でディスパッチ
- [x] クライアント: クリック選択 + A キー + HUD 表示
- [x] CLAUDE.md §1/§6 の更新（人間承認済み・2026-06-13）

---

## 却下した代替案

- **専用イベント `ApproachStarted/Stopped` を作る**: 状態の記述であり「事実」ではない
  （INV-006 / よくある設計違反パターン8）。thrust の結果は VelocityChanged で
  既に記録されるため冗長。却下。
- **ApproachComp をスナップショットに含める**: 操船意図は派生状態であり、
  リプレイ再現に不要。ThrustComp を含めていないのと同じ理由で却下。
- **対象をロックオン対象に固定する**: アプローチと攻撃対象は別判断であるべき
  （例: 攻撃しながら別の相手から離脱）。クリック選択で独立させる。
