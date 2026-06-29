---
id      : ADR-0010
title   : 船の喪失と再出撃 — 脱出ポッド / 拠点帰還 / 新艦派遣
status  : deferred
date    : 2026-06-06
deciders: [human, ai-agent]
related : ADR-0006（Combat）, ADR-0009（StarSystem）, game-design.md §4
---

# ADR-0010 — 船の喪失と再出撃

## ステータスについて

**deferred（延期）**: Phase 6 プレイテストにはデュエルモード（片方が死んだら終了）を採用し、
本 ADR の実装は行わない。脱出ポッド・再出撃の設計はステーション実装後（Phase 7 以降）に実施する。
プレイテスト中の「死亡後の継続プレイ」問題はデュエル形式で回避する。

---

## 背景

プレイテスト（playtest-guide.md）の前提条件として、
船が破壊されたプレイヤーが継続してプレイできる仕組みが必要である。

当初「リスポーン」として設計したが、宇宙ゲームとして不自然であると判断した。

> 「リスポーンという概念がおかしいのではなく、フレーミングがおかしい。
>  宇宙では船はステーションもしくは母船で作られる。」

本 ADR では「脱出ポッド → 拠点帰還 → 新艦派遣」という
3 フェーズ設計を採用し、ゲームデザイン上の意味を持たせる。

---

## 決定

### 概念モデル

```
[戦闘中]
    ↓ ShipDestroyed
[脱出ポッドフェーズ]
    プレイヤーはポッドとして操作継続（低速・非武装・高被弾リスク）
    他プレイヤーはポッドを撃墜できる（「ポッドキル」= さらなる損失）
    ↓ ポッドが拠点座標に到達 or 拠点へのワープコマンド発行
[拠点帰還フェーズ]
    プレイヤーは拠点にいる（操作不能・待機中）
    一定時間後に新しい船が用意される（「整備時間」）
    ↓ 新艦準備完了
[再出撃フェーズ]
    新しい ShipId で拠点座標から出撃
    ShipDispatched イベントを発行
```

### Phase 6 実装（プレイテスト用・簡略版）

フルの3フェーズは将来実装とし、プレイテストのために最小限の動作を先行実装する。

```
Phase 6 簡略版:
  [ShipDestroyed]
      ↓ 即座にポッド移行（ポッドの操作は実装しない）
  [待機カウントダウン: DISPATCH_DELAY_TICKS]
      ↓
  [ShipDispatched: 拠点座標に新艦出現]

  ※ ポッドフェーズは「演出上の待機時間」として扱う
  ※ ポッドキルは Phase 6 では実装しない
```

### イベント設計

```rust
// dawn-core/src/events.rs に追加

/// プレイヤーが脱出ポッドに移行した。
/// 発行タイミング: ShipDestroyed の直後（プレイヤー船の場合のみ）。
pub struct EscapePodEjected {
    pub player_id      : PlayerId,
    pub pod_position   : Position,   // 破壊された船の最後の位置
    pub base_position  : Position,   // 帰還先の拠点座標
    pub dispatch_at_tick: Tick,      // 新艦出撃予定 Tick
    pub tick           : Tick,
}

/// 拠点から新しい船が出撃した。
/// 発行タイミング: dispatch_at_tick に達したとき。
pub struct ShipDispatched {
    pub player_id      : PlayerId,
    pub new_ship_id    : ShipId,
    pub ship_type_id   : ShipTypeId,
    pub position       : Position,   // 拠点座標
    pub tick           : Tick,
}
```

`EscapePodEjected` と `ShipDispatched` は DomainEvent であり EventStore に追記される。
State は Event から完全に Replay 可能（INV-002）。

### 将来実装（フルフェーズ版）

```
ポッドフェーズ（Phase 7 以降）:
  - 脱出ポッドを別の ShipId として ECS に登録する
  - 低速（max_speed: 100）・非武装・Frigate より低い HP
  - プレイヤーはポッドを操作して拠点に向かう
  - 他プレイヤーがポッドをロックして撃墜できる（ポッドキル）
  - ポッドキルされると DISPATCH_DELAY_TICKS が延長 or 装備ペナルティ追加

拠点フェーズ（ステーション実装後・ADR-0009 以降）:
  - 拠点をゲームオブジェクトとして定義（位置・キャパシティ）
  - 拠点ごとに在庫船種が異なる設計にできる
  - 拠点を占領するゲームメカニクスへの伏線

ペナルティ設計（Production 設定）:
  - 装備リセット（デフォルトフィッティングのみで出撃）
  - 延長カウントダウン（DISPATCH_DELAY_TICKS を長くする）
  - 出撃できる船種の制限（拠点在庫）
```

### コマンド設計

```rust
// dawn-core/src/commands.rs に追加（将来フルフェーズ版）

/// 脱出ポッドを操縦して拠点に向かう。
/// Phase 6 では未実装（自動帰還）。
pub struct PodWarpToBaseCommand {
    pub player_id: PlayerId,
}
```

---

## Godot 側の対応

```
EscapePodEjected 受信:
  - プレイヤー船のメッシュを消去（または爆発エフェクト）
  - 「ポッドに乗っています。拠点まで XX 秒...」HUD 表示
  - カメラをポッド位置に固定（Phase 6 では操作不能）
  - dispatch_at_tick からカウントダウンを表示する

ShipDispatched 受信:
  - connection.gd: ship_id を new_ship_id に更新、dispatched シグナル発行
  - main.gd: 新しい Ship ノードを拠点座標に生成、カメラ追従を再開
  - HUD: 「新しい船で出撃しました」表示
```

### セッションメッセージ

`ShipDispatched` は DomainEvent であり全クライアントにブロードキャストされる。
ただし「これがあなたの新しい ship_id です」という通知は
`Welcome` と同様に該当クライアントのみへのセッションメッセージとして送る。

```json
{ "type": "ShipDispatched", "new_ship_id": N }
```

---

## パラメータ

```toml
# data/server_settings.toml（将来追加）に外部化予定
DISPATCH_DELAY_TICKS = 100   # 10秒（テスト用）
# DISPATCH_DELAY_TICKS = 300 # 30秒（本番想定）
```

---

## 却下した代替案

### 案A: 即時リスポーン（固定座標に瞬間移動）

**却下理由:** 宇宙ゲームとして不自然。「死が怖い」という緊張感がなくなる。
game-design.md §2「行動に重みがある（Loss Aversion）」に反する。

### 案C: 死亡=完全退場（次セッションまで再参加不可）

**却下理由:** プレイテストに不向き。最初に死んだプレイヤーが長時間傍観することになる。
playtest-guide.md §2「プレイテスト前の前提条件」に反する。

---

## 実装チェックリスト（Phase 6 簡略版）

### dawn-core

- [ ] `events.rs` に `EscapePodEjected` 追加
- [ ] `events.rs` に `ShipDispatched` 追加
- [ ] `lib.rs` に re-export 追加
- [ ] 各イベントの単体テスト

### dawn-simulation

- [ ] `node.rs`: ShipDestroyed 時に `EscapePodEjected` を EventStore に Append
- [ ] `node.rs`: `pending_dispatches: Vec<PendingDispatch>` フィールド追加
- [ ] `node.rs`: `process_dispatches()` メソッド — 期限到達で `ShipDispatched` Append + 新艦 Spawn
- [ ] `node.rs`: ship_owners / player_ships の死亡時クリーンアップ
- [ ] `ws_server.rs`: `ShipDispatched` の EventJson 追加
- [ ] `ws_server.rs`: `PlayerSession::send_ship_dispatched(new_ship_id)` 追加
- [ ] `main.rs`: `process_dispatches()` 結果を受けてクライアントに通知

### Godot（client/）

- [ ] `connection.gd`: `EscapePodEjected` 受信 → `pod_ejected` シグナル
- [ ] `connection.gd`: セッションメッセージ `ShipDispatched` 受信 → `ship_dispatched` シグナル
- [ ] `main.gd`: `_on_pod_ejected` — 船メッシュ消去・HUD カウントダウン表示
- [ ] `main.gd`: `_on_ship_dispatched` — 新船生成・`_player_ship_id` 更新・HUD リセット

### docs

- [ ] `docs/architecture/event-catalog.md` に `EscapePodEjected` / `ShipDispatched` を追記
- [ ] `docs/design/game-design.md` §4「リスポーン」を「船の喪失と再出撃」に改訂
- [ ] `docs/process/playtest-guide.md` の「前提条件」を本 ADR を参照するよう更新
- [ ] `docs/process/roadmap.md` に Phase 6 Cycle 5 として記録

---

## 参照

- game-design.md §2: Loss Aversion（行動に重みがある）
- game-design.md §4: リスポーン設計案（本 ADR で更新）
- playtest-guide.md §2: プレイテスト前提条件
- ADR-0006: Combat システム（ShipDestroyed イベント）
- ADR-0009: 星系間ナビゲーション（拠点フェーズとの将来統合）
- AI_DEVELOPMENT_GUIDE.md「Project North Star」: スコープ（ポッドは Ship の一種として実装可能）
