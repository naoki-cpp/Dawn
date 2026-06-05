---
id      : ADR-0008
title   : 船の移動をイベントログにどう記録するか
status  : proposed
date    : 2026-06-05
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0006（Fitting/Combat）
---

# ADR-0008 — 船の移動をイベントログにどう記録するか

## 背景

現在の実装では、船が移動するたびに毎 Tick `ShipMoved` イベントを記録している。

```rust
pub struct ShipMoved {
    pub ship_id : ShipId,
    pub from    : Position,
    pub to      : Position,
    pub tick    : Tick,
}
```

しかし `ShipMoved` は「物理計算の結果（導出値）」であり、
「何が起きたか（原因・事実）」ではない。

```
原因: プレイヤーが推力方向を指定した
結果: 毎 Tick、位置が物理法則に従って変化する

現在はこの「結果」を記録している。
```

**問題点:**
- 10,000 ships × 100 ticks = 1,000,000 events（ほぼ全てが導出可能な値）
- 推力が変わらない限り、位置は数式で完全に求まる
- Event Log の本来の役割（**因果の追跡**）から外れている

---

## 選択肢

### Option A: ThrustApplied のみ記録（原因だけ記録）

```rust
/// 推力方向が変わったときのみ発行する。
/// 推力を持つ全ての Ship（NPC を含む）が対象。
pub struct ThrustApplied {
    pub ship_id   : ShipId,
    pub direction : Velocity,  // 正規化済み単位ベクトル。ZERO = 推力なし
    pub tick      : Tick,
}
```

`ShipMoved` は廃止。位置は Replay 時にシミュレーションで再計算する。

```
ログのサイズ: 推力が変わった回数のみ（大幅に削減）

Replay:
  ThrustApplied の履歴 + 初期 Position + 物理ルール → 任意 Tick の位置
  ※ 物理ルール（加速度計算）が決定論的であることが前提

クライアント（Godot）:
  位置は別途サーバーから通知が必要（ShipMoved に相当する projection を送信）
  ただしこの projection はログには記録しない
```

**メリット:**
- Event Log が純粋な「原因の記録」になる
- ログサイズが劇的に削減される
- 「いつ誰が推力を変えたか」という因果が明確に追跡できる

**デメリット:**
- Replay に物理シミュレーションが必要
- 物理ルール（MovementSystem）を変更すると、過去ログとの整合性が崩れる
  → Upcaster の代わりに「物理バージョン」の管理が必要になる
- Godot クライアント向けに別途 position ストリームを維持する必要がある

---

### Option B: Hybrid（ThrustApplied = canonical、ShipMoved = projection）

```
永続化する Event Log（唯一の真実）:
  ThrustApplied { ship_id, direction, tick }  ← 原因のみ

クライアント向けメッセージ（非永続、Projection）:
  ShipMoved { ship_id, to, tick }  ← 物理計算結果、ログに書かない
```

Projection はサーバーが毎 Tick 計算してクライアントに送信するが、
EventStore には記録しない。

```
INV-002（Event Replay で完全復元）の扱い:
  ログから再構築できるのは「推力の履歴」のみ。
  正確な位置の復元には物理シミュレーションが必要。
  → INV-002 の「完全再現」の定義を「物理シミュレーションを経た再現」と解釈する。
```

**メリット:**
- Event Log は薄く・純粋
- クライアントは引き続き位置更新を受け取れる（UI は変わらない）
- 分散ノード間の同期は ThrustApplied の伝播だけでよい

**デメリット:**
- EventStore の「イベント」と「プロジェクション」を概念として明確に分離する必要がある
- 物理ルール変更時の互換性問題は Option A と同様

---

### Option C: 現状維持（ShipMoved を記録し続ける）

```
変更なし。毎 Tick 全移動船の ShipMoved を記録する。
```

**メリット:**
- 実装が最も簡単
- Replay が単純（シミュレーション不要）
- 物理ルールを変えても過去ログに影響しない

**デメリット:**
- 「導出可能な値」を大量に記録し続ける
- 因果追跡の観点から原則（CLAUDE.md §1）と乖離している
- ログサイズが大きい（Phase 8 のスケールで問題になる可能性）

---

## トレードオフ整理

| 観点 | A（ThrustApplied のみ） | B（Hybrid） | C（現状維持） |
|---|---|---|---|
| ログの純粋性 | ✅ 原因のみ | ✅ 原因のみ | ❌ 導出値を含む |
| ログサイズ | ✅ 最小 | ✅ 最小 | ❌ 大きい |
| Replay の簡便さ | ❌ シミュレーション必要 | ❌ 同左 | ✅ 直接適用 |
| 物理ルール変更耐性 | ❌ 互換性問題あり | ❌ 同左 | ✅ 影響なし |
| クライアント実装 | △ projection 別管理 | ✅ 変更なし | ✅ 変更なし |
| 因果追跡の明確さ | ✅ 明確 | ✅ 明確 | ❌ 結果のみ |

---

## 未解決の設計問題（Option A / B 共通）

**物理ルールのバージョン管理:**

推力計算ロジックが変わると、過去の `ThrustApplied` イベントから位置を再計算したとき
結果が変わってしまう。これは Event Sourcing における「Upcaster 問題の物理版」である。

対処案:
1. 物理ルールを不変にする（変更禁止）
2. 物理バージョンを Event に含める
3. `ShipMoved` は廃止せず「チェックポイント」として定期的に記録する

---

## 決定

**未決定。** 以下の観点で判断を求める:

1. Phase 8 のスケール（1 Sector 5,000 ships）でログサイズが問題になるか
2. 物理ルールは将来変更する可能性があるか
3. INV-002 の「完全再現」にシミュレーション実行を含めるか

---

## 参照

- ADR-0001: Event Sourcing 基本原則
- CLAUDE.md §1: 「Event が唯一の真実。State は派生物に過ぎない」
- CLAUDE.md §2 INV-002: State は Event の Replay で完全再現できなければならない
- docs/tick-model.md: Tick 処理順序と MovementSystem
