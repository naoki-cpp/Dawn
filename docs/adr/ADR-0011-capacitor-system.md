---
id      : ADR-0011
title   : サイクルベース Capacitor システムとクライアント側シミュレーション
status  : accepted
date    : 2026-06-08
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0006（Fitting/Combat）, ADR-0008（VelocityChanged / 派生状態の設計）
---

# ADR-0011 — サイクルベース Capacitor システムとクライアント側シミュレーション

## 背景

Phase 6 で Active モジュール（武器・アフターバーナー等）の運用コストを表現する
Capacitor（cap, EVE Online でいう「エネルギー」）システムを導入する必要が生じた。

検討すべき論点は2つある。

```
論点1: cap の消費モデルをどう設計するか
       （毎 Tick 少しずつ削るか、サイクル開始時にまとめて消費するか）

論点2: cap の現在値をクライアント（Godot）にどう伝えるか
       （サーバーが毎 Tick 送るか、クライアントが自前で計算するか）
```

---

## 決定1: cap はサイクル開始時に消費する（per-tick drain ではない）

### 検討した設計案

```
案A（却下）: cap_cost_per_tick による継続的ドレイン
  - 毎 Tick、稼働中モジュールの数だけ cap を少しずつ減らす
  - 実装は単純だが、EVE Online の「サイクル」概念と乖離する
  - 「あと何 Tick で次の消費が来るか」が表現できない
    → クライアント表示や UI 演出（次サイクルのカウントダウン）と相性が悪い

案B（採用）: cap_cost_per_cycle + cycle_time_ticks によるサイクル消費
  - モジュールは cycle_time_ticks 分の「サイクル」を繰り返す
  - サイクル開始時（cycle_remaining == 0）に cap_cost_per_cycle を一括消費
  - 消費できなければモジュールを強制 OFF（ModuleDeactivated を発行）
  - サイクル中（cycle_remaining > 0）は何も消費せず、カウントダウンのみ
```

### 採用理由

- EVE Online の実際の挙動（武器のリロード/サイクルタイム）に忠実であり、
  ゲームバランス調整（`data/modules.toml` の `cycle_time_ticks` 変更）が直感的になる。
- 「次の消費まであと何 Tick か」が `cycle_remaining` として明示的な状態になり、
  クライアント側シミュレーション（決定2）の基盤になる。
- cap 不足による強制 OFF が「サイクル境界で判定される」という分かりやすい規則になる。

### 実装

```rust
// FittedSlot に追加
pub struct FittedSlot {
    // ...
    pub cycle_remaining: u64, // 0 = 次 Tick で新サイクルを開始できる
}

// CapacitorSystem::run() の各 Tick の処理（dawn-ecs/src/systems/capacitor.rs）
// 1. recharge: cap = (cap + cap_recharge_per_tick).min(cap_max)
// 2. 各 Active モジュールについて:
//      cycle_remaining == 0 の場合:
//        cap >= cap_cost_per_cycle なら消費して cycle_remaining = cycle_time_ticks
//        不足なら強制 OFF（ModuleDeactivated 発行）
//      cycle_remaining > 0 の場合:
//        デクリメントするのみ（消費なし）
```

`cycle_remaining` は ECS 上の一時状態であり、再起動時には `0`（即座に新サイクル開始可能）
にリセットされる（`from_snapshot()`）。これは Position 同様、Replay によって
正しいサイクルの位相に自然収束するため INV-002 違反ではない
（cap 自体が cap_max からの再計算で復元される派生状態であることと同じ理由）。

---

## 決定2: cap の値はイベント化せず、クライアント側でシミュレーションする

### 検討した設計案

```
案A（却下）: サーバーが毎 Tick CapUpdate メッセージを WebSocket で送信する
  - 問題1: cap は「派生状態」である（ADR-0008 の Position と同じ位置づけ）。
           毎 Tick 変化する値を逐一イベント化／配信するのは
           「位置を毎 Tick 記録する ShipMoved」の二の舞であり、
           Event Sourcing の精神（INV-MOVE）に反する。
  - 問題2: 通信量が「Tick数 × 船数」に比例して増加し、
           プレイヤー数のスケールに対して線形に悪化する。
  - 問題3: cap は決定論的に計算可能な値であり、
           サーバーが計算結果を送り続ける必然性がない。

案B（採用）: クライアントがサーバーと同一のロジックを再現し、自前で cap を計算する
  - サーバーは初期パラメータ（cap_max, cap_recharge_per_tick）を
    InitialState で一度だけ送る。
  - サーバーはモジュール定義（cap_cost_per_cycle, cycle_time_ticks）を
    PlayerFitting で一度だけ送る。
  - クライアントは ModuleActivated / ModuleDeactivated イベント（既存の認可された
    イベント）を受信した時点を起点として、_simulate_cap() でサーバーと同じ
    回復・消費ロジックをローカル実行する。
  - サーバー発の ModuleDeactivated（cap 枯渇による強制 OFF）が、
    クライアントの予測とサーバーの真実がズレた場合の権威的な補正として機能する。
```

### 採用理由

人間レビュアーの指摘により、案Aは「位置を毎 Tick 配信する」のと
構造的に同じ問題（継続的な派生状態のブロードキャスト）を持つことが判明した。
これは ADR-0008 で `ShipMoved` を廃止し `VelocityChanged` のみを
イベント化した判断と完全に対称的である。

```
位置（Position）: VelocityChanged を起点にクライアントが線形外挿で再現する
cap             : ModuleActivated/Deactivated を起点にクライアントが
                  recharge/consume ロジックで再現する

→ どちらも「変化点だけをイベント化し、連続値はクライアントが計算する」
  という同一の設計パターンに従う。
```

### 実装

- `client/scripts/main.gd`
  - `_simulate_cap(ticks: int)`: `CapacitorSystem::run()` のロジックをミラーする
    純粋関数的シミュレーション（recharge → サイクル判定 → 消費）。
  - `_handle_velocity_changed()` 内で Tick 経過分だけ `_simulate_cap()` を呼ぶ
    （VelocityChanged は毎 Tick 飛んでくるため、自然な同期点になる）。
  - `cap_forced_off` フラグ: プレイヤー操作によらない `ModuleDeactivated` を
    検出し、HUD 上で "CAP!" として cap 切れによる強制停止を視覚化する。

---

## この ADR が確立する設計規則

```
規則1: cap のような「連続的に変化する派生状態」を新たにイベント化したり、
       Tick ごとに配信するメッセージとして設計してはならない。
       → INV-MOVE / ADR-0008 の精神を継承する。

規則2: クライアントへの状態同期は「変化のきっかけとなる離散イベント」
       （ModuleActivated, ModuleDeactivated, VelocityChanged 等）の配信と、
       それらを起点としたクライアント側シミュレーションの組み合わせで行う。

規則3: モジュールの運用コストはサイクルベース（cap_cost_per_cycle +
       cycle_time_ticks）で表現する。per-tick drain 方式を新たに追加しない。
```

将来、新たに「連続的に変化する値」（例: 装甲の自己修復、シールドの
リチャージなど）を実装する場合は、本 ADR のパターン（離散イベント +
クライアント側シミュレーション）を踏襲すること。

---

## 影響を受けるコンポーネント・イベント

```
新規コンポーネント: CapacitorComp { current: f32 }
変更コンポーネント: ShipStatsComp（cap_max, cap_recharge_per_tick 追加）
                    FittedSlot（cycle_remaining 追加）
変更モジュール定義: ModuleDefinition（cap_cost_per_cycle, cycle_time_ticks 追加）
新規システム      : CapacitorSystem（dawn-ecs/src/systems/capacitor.rs）
利用イベント      : ModuleActivated, ModuleDeactivated（新規イベントは追加していない）
```

cap の値そのものに対応するイベントは存在しない。これは意図的な設計であり、
INV-002（Replay による完全な状態復元）は「cap_max からの再計算」によって
満たされる（Position が `velocity` の積分によって満たされるのと同型）。
