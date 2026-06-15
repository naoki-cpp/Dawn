---
id      : ADR-0022
title   : イントラセクター Warp — WarpCommand / WarpComp（align/warping 2 フェーズ）
status  : accepted
date    : 2026-06-15
deciders: [human, ai-agent]
related : ADR-0008（Ship Movement Events / INV-MOVE）, ADR-0009（Jump Gate Navigation）, ADR-0015（Approach / 半自動操船）, ADR-0016 §5（戦闘の深み）, docs/reference/eve-reference.md §7.4.1, CLAUDE.md §6（Tick Model）, CLAUDE.md §1（Scope）
---

# ADR-0022 — イントラセクター Warp

## 背景

EVE Online の戦闘が成立する根本前提は **Warp（高速離脱）と、それを止める Tackle** の
非対称性である（eve-reference §7.4.1）。dawn には現状この高速離脱が無く、セクター内の
移動はスラスター（`MoveCommand` / `ApproachCommand`・ADR-0015）のみ。結果として:

- ゲートまでスラスターで延々飛ぶしかなく、移動が単調。
- 逃走が「ただ離れる」だけになり、Tackle（次 ADR-0023）が成立する土台が無い。

EVE の Warp は「150km 以上離れた warp 可能オブジェクト（天体・ゲート・ブックマーク等）へ
高速移動する」もの。dawn でも**同一セクター内の Warp** を導入し、

> 「ゲートまで Warp → Jump」という EVE の移動ループを成立させ、
> かつ「整列中に Tackle を入れられたら逃げられない」という戦術的緊張の土台を作る。

> 設計の中心的な問い（docs/game-design.md）:
> 「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
> → Warp は「いつ・どこへ離脱を試みるか」「整列の隙を突かれないか」という
>   コミットを伴う判断を生む。Yes。

本 ADR は **Warp 単体**を対象とする。Tackle（Warp 阻害）は次の ADR-0023 で扱う。

### lore 上の位置づけ（docs/lore/technology.md・glossary.md）

dawn の lore は移動を **Fold（折畳）** で統一している:

- **Fold Transit（折畳航法）** = **Sector 間**移動。Fold Drive が深度差異廊道（**Fold Lane**）へ
  整合（alignment）して目的地を幾何学的に近づける。= 現状の **Sector Transit / Jump Gate（ADR-0009/0014）**。
- **本 ADR の intra-Sector "Warp"** = lore 上は **短距離 Fold** — 同一 Sector 内の**局所的な深度差異**へ
  整合して遠方アンカー（Fold Lane 端 = Jump Gate）へ寄せる。Fold Lane（安定廊道）を要さず Trace Fuel も軽微。
  プレイヤーはこれを口語で **「ワープ（warp）」** と呼ぶ（用語はコード/UI でも Warp を用いる・分かりやすさ優先）。
- **整合（alignment）フェーズ** = lore の「整合中、船は動かない」そのもの。中断可能＝ **Tackle 窓**。

> **Tackle は lore で命名済み**: **Fold Disruptor**（technology.md 電子戦システム）。「Fold Drive を妨害された
> 船はジャンプで逃げられない」。Fold Drive 起動の阻害なので、**intra-Sector Warp と inter-Sector Fold Transit の
> 両方を一括で塞ぐ** — ADR-0023（Tackle）が `can_propose_warp` と `can_propose_jump/transit` の双方を
> 拒否する設計と完全に一致する。glossary に「Warp（短距離 Fold の俗称）」項を追加する。

---

## 決定

### 1. Warp は「align → warping」の 2 フェーズ持続モードである

`WarpCommand` を受理すると `WarpComp` を付与し、以下のフェーズを進む:

```
WarpCommand { ship_id, gate_id }
  → WarpComp { gate_id, phase: Align { remaining: ALIGN_TICKS } } を付与
  ── 毎 Tick process_warp() がフェーズを進める ──
  [Align]    remaining を 1 ずつ減らす。まだ warp に入っていない。
             → Move/Stop で中断可。Tackle（ADR-0023）はここでのみ阻害できる。
  [Warping]  remaining == 0 で遷移。velocity = unit(gate - pos) * WARP_SPEED。
             → コミット済み。Move/Stop は無効（InTransit と同じ拒否）。
             → 到着半径内に入ったら velocity = ZERO・WarpComp 除去で終了。
```

この 2 フェーズ分離が本 ADR の核である。**中断・Tackle 阻害は align フェーズにのみ作用し、
warping に入ったら終点までライドする**（EVE: warp 突入後は tackle 不可・自分でも止まれない）。

| フェーズ | Move/Stop | Tackle（ADR-0023） |
|---|---|---|
| Align（`ALIGN_TICKS`） | 中断可 → WarpComp 除去 | 有効（warp 突入を阻止 = コミット確定） |
| Warping | **無効**（終点までライド） | 無効（手遅れ） |

### 2. Warp 対象は自セクター内の Jump Gate のみ（slice 1）

dawn に現存する静的オブジェクトは Jump Gate だけ（天体・ステーション・ブックマークは未実装）。
よって slice 1 の対象はゲートに限定する。これで「ゲートまで Warp → Jump」ループが成立する。

- ゲートが自セクターに属さない → コマンド拒否。
- 到着点 = ゲートの `activation_radius` 内（着いたら即 Jump 可能）。

将来（別 ADR）: warp 可能対象の拡張（フリート員 / ブックマーク / 天体）。

### 3. 最小 Warp 距離（EVE の 150km 相当）

対象までの距離が `MIN_WARP_DISTANCE` 未満なら `WarpCommand` を拒否する（近すぎる →
`ApproachCommand` を使え）。Warp が「近距離テレポート」に堕ちるのを防ぐ。

### 4. 新規型定義

```rust
// dawn-core/src/commands.rs
pub struct WarpCommand {
    pub ship_id: ShipId,
    pub gate_id: JumpGateId,   // slice 1: 対象は Jump Gate のみ
}

// dawn-ecs/src/components/movement.rs
/// Persistent two-phase warp state (ADR-0022). Transient steering state like
/// ThrustComp / ApproachComp — NOT persisted in ShipSnapshot, never its own event.
pub enum WarpPhase {
    Align { remaining: u64 },  // spin-up; interruptible; tackle window (ADR-0023)
    Warping,                   // committed; movement-controlled by process_warp
}
pub struct WarpComp {
    pub gate_id: JumpGateId,
    pub phase  : WarpPhase,
}
```

### 5. 新イベント型は作らない（ADR-0008 / INV-MOVE 準拠）

Warp の移動は **既存の `VelocityChanged` で記録する**。`process_warp` が warping 中の船の
velocity を warp 速度ベクトルに設定し、Movement と同じく velocity 変化時に `VelocityChanged`
を発行する。Replay は `position += velocity` の純粋算術で warp 軌道を完全再現できる（INV-MOVE）。

- `WarpComp` は `ThrustComp` / `ApproachComp` と同じ派生操船状態であり、`ShipSnapshot` に
  **含めない**。warp 途中でのクラッシュ復旧は WarpComp を失うが、最後に記録された
  `VelocityChanged` の速度で漂流し、Movement のクランプで sublight 速度へ自然減速する（安全・自己回復）。
- **event-catalog.md の変更は無し**（新イベント型を作らないため）。

> `WarpStarted/WarpFinished`（ADR-0008 の例示・game-design.md §385）は採らない。
> 理由は ADR-0015 が `ApproachStarted/Stopped` を却下したのと同一（状態の記述であって
> 「事実」ではない・INV-006 / 設計違反パターン 8）。移動は `VelocityChanged` で記録済み。

### 6. Tick 処理順序への追加（CLAUDE.md §6）

`process_warp()` を **Approach（Step 2.5）の後・Movement（Step 3）の前** の Step 2.6 として追加する。

```
2.   コマンドキュー処理（WarpCommand → WarpComp 付与 / Move・Stop で align のみ解除）
2.5  Approach System（process_approach）
2.6  Warp System（process_warp）                ← 新規
       Align:   remaining を減らす。0 で Warping へ遷移し warp velocity を設定。
       Warping: 到着判定。残距離 ≤ 1 tick 分なら到着点へ着地→停止・WarpComp 除去。
                さもなくば warp velocity 維持。VelocityChanged を発行。
3.   Movement System（warping 中の船はスキップ = process_warp が所有）
4.   Capacitor System
...
```

Movement System は `Option<&WarpComp>` を参照し、**phase == Warping の船をスキップ**する
（位置・velocity・クランプ・イベント発行を `process_warp` が一貫して所有するため）。
Align フェーズの船は通常どおり Movement が処理する（sublight 移動を続けてよい）。

`max_speed` クランプは sublight 推力上限であり warp には適用しない。warping 船を Movement が
スキップすることでクランプを回避する（クランプ条件に warp 特例を足すのと等価だが結合が小さい）。

### 7. 解除・拒否条件

`WarpComp` を除去する:
- align 中に同じ船へ `MoveCommand` / `StopCommand` が発行された（中断）。
- warping が到着半径に達した（完了）。
- 対象ゲートが消えた等で到着不能（安全側で停止）。

`can_propose_warp(ship_id, gate_id)` が false を返す（コマンド拒否・INV-006 の Validation 段階）:
- 船が存在しない / 既に `TransitState::InTransit` / 既に `WarpComp` を持つ。
- ゲートが自セクターに無い。
- 距離 < `MIN_WARP_DISTANCE`。
- （ADR-0023 で追加）tackled である。

warping 中（コミット済み）は `MoveCommand` / `StopCommand` / 二重 warp を拒否する
（`TransitState::InTransit` と同じ扱い）。

### 8. 対象の選択・送信はクライアント側

Approach（ADR-0015）と同じ UX 系統。プレイヤーがクリックで選択したゲートを対象に、
専用キー（例: `W`）で `WarpCommand` を送る（JSON に `gate_id`）。サーバーは所有権
（`ship_owners`）を確認してから `can_propose_warp` 検証 → `WarpComp` 付与。

### 9. チューニング定数（data/ または定数）

| 定数 | 役割 | 初期値（暫定） |
|---|---|---|
| `ALIGN_TICKS` | align フェーズ長（= Tackle 窓） | 30 tick |
| `WARP_SPEED` | warp 中の速度（>> max_speed） | 5000 u/tick |
| `MIN_WARP_DISTANCE` | warp 可能な最小距離（150km 相当） | 3000 u |
| 到着半径 | warping 終了距離 | gate.activation_radius |

初期値はプレイテストで調整する（ship_types.toml と同様にデータ化可能だが、slice 1 は定数で可）。

---

## 影響

| 対象 | 変更 |
|---|---|
| `dawn-core` | `WarpCommand` を追加（commands.rs）+ test |
| `dawn-ecs` | `WarpComp` / `WarpPhase` を追加（components/movement.rs）。Movement System が warping 船をスキップ |
| `dawn-ecs` | `process_warp` 本体（systems/warp.rs 新規）+ tests |
| `dawn-simulation` | `SimulationNode::can_propose_warp` / `apply_warp_command_owned` / process_warp 配線（Step 2.6）、Move/Stop での align 解除、warping 中の操作拒否 |
| `dawn-actor` | `ClientCommand::Warp` を追加 |
| `dawn-simulation`（ws/cluster） | JSON パース・コマンドディスパッチ |
| クライアント | ゲート選択 + W キー送信、warp 状態 HUD（任意） |
| CLAUDE.md | §1 スコープに「WarpComp / WarpCommand」、§6 Tick 順序に Step 2.6 を追記（人間承認のうえ） |
| docs | tick-model.md §3 に Step 2.6、roadmap.md（戦闘の深み着手）を更新 |

イベントスキーマ（event-catalog.md）の変更は**なし**（新イベントを作らない）。

---

## 実装チェックリスト

- [ ] `WarpCommand` を dawn-core に追加（+ test）
- [ ] `WarpComp` / `WarpPhase` を dawn-ecs に追加
- [ ] `process_warp()`（systems/warp.rs）を実装（align countdown / warping 着地・到着停止）（+ tests）
- [ ] Movement System が warping 船をスキップ（+ test）
- [ ] `can_propose_warp`（距離・セクター・InTransit・重複検証）を node.rs に追加（+ test）
- [ ] `apply_warp_command_owned`（所有権チェック付き）を追加（+ test）
- [ ] tick の Step 2.6 として process_warp を配線
- [ ] `apply_move_command` / `apply_stop_command` で align フェーズの WarpComp を解除（warping は不可）（+ test）
- [ ] warping 中の Move/Stop/二重 warp 拒否（+ test）
- [ ] `ClientCommand::Warp` を dawn-actor に追加 + ws_server パース（+ test）
- [ ] run_cluster_server / run_phase4_server でディスパッチ
- [ ] クライアント: ゲート選択 + W キー（+ 任意の warp HUD）
- [ ] CLAUDE.md §1/§6 の更新（人間承認のうえ）

---

## 却下した代替案

- **`WarpStarted` / `WarpEnded` 専用イベント**（ADR-0008 例示・game-design.md §385）:
  状態の記述であり「事実」ではない（INV-006 / 設計違反パターン 8）。warp の移動は
  `VelocityChanged` で記録され replay 可能（INV-MOVE）。ADR-0015 が `ApproachStarted/Stopped`
  を却下したのと同一理由で却下。
- **warp を位置直接更新（velocity を経由しない）で実装**: replay は `VelocityChanged` の
  積分でしか位置を再現しない（INV-MOVE）ため、velocity を経由しない位置変更は replay で失われる。
  これを救うには専用イベントが要り、上記理由で不採用。よって warp も velocity 経由で表現する。
- **align フェーズを省く（即 warp）**: Tackle（ADR-0023）が作用する窓が消え、戦術的緊張が
  生まれない。align = Tackle 窓が本機能の眼目なので必須。
- **warp 中も中断可能にする**: EVE の「warp 突入後はコミット」を壊し、Tackle の意味
  （align で捕まえる skill）を薄める。warping は committed とする。
- **対象を任意座標 / 敵船に開放**: EVE でも直接敵船 warp は不可（ブックマーク / フリート要）。
  slice 1 は静的オブジェクト（ゲート）に限定し、拡張は別 ADR。
