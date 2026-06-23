---
id      : ADR-0031
title   : Orbit / Keep at Range — OrbitCommand / KeepAtRangeCommand
status  : accepted
date    : 2026-06-23
deciders: [human, ai-agent]
related : ADR-0015（Approach・同型の持続的操船モード）, ADR-0012（タレット追跡・トランスバーサル速度が命中率に効く根拠）, ADR-0016 §5（戦闘の深みロードマップ）, AI_DEVELOPMENT_GUIDE.md §6（Tick Model）
---

# ADR-0031 — Orbit / Keep at Range

## 背景

ADR-0016 §5 の「戦闘の深み」ロードマップ（Tackle → Signature Resolution →
**Orbit / Keep at Range** → Logistics）の3番目の項目。Tackle（ADR-0024）と
タレット追跡式（ADR-0012）が実装済みの今、操船側に「距離を能動的に管理する」
手段が無いことがボトルネックになっている。現状の操船は:

- `MoveCommand`: 一回限りの推力方向指定。対象が動くと自分で再指定し続ける必要がある。
- `ApproachCommand`（ADR-0015）: 対象に詰め寄って到着半径で停止するだけ。

どちらも「最適射程を保ちながら回り続ける」「最低限の距離だけ離れて逃げる」という
EVE Online の核心的な戦術操船（オービット／キープアットレンジ）を表現できない。
ADR-0012 の命中率式はトランスバーサル速度（角速度）が高いほど被弾しにくくなるため、
**オービットそのものが回避手段として機能する設計**になっている——この武器が
プレイヤーの手に渡っていないのが現状である。

> 設計の中心的な問い（docs/design/game-design.md）:
> 「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
> → 「この距離で回るか、もっと離れるか、詰めるか」は EVE のポジショニング戦の核。
>   ADR-0012 の命中率式と組み合わさって初めて意味を持つ判断を増やす。Yes。

---

## 決定

ADR-0015（Approach）と同型の「持続的な操船モード」として、2つの独立したコマンドを追加する。
Orbit と Keep at Range は異なる操船意図（回る／距離を保つ）なので、1つのコマンドに
フラグで分岐させるのではなく ADR-0015 の前例（Approach 専用コマンド）に倣い、
それぞれ専用の Command/Component を持つ。

### 1. Orbit — 対象の周りを指定半径で周回する

```
OrbitCommand { ship_id, target, radius: Option<f32> }
  → OrbitComp { target, radius } を ship に付与
  → 毎 Tick process_orbit() が「対象から radius だけ離れた円周上、
     接線方向にやや先回りした点」へ thrust を向け直す
  → 半径は指定がなければ既定値 ORBIT_DEFAULT_RADIUS を使う
```

**ステアリング計算**（`process_orbit`、`ApproachComp`/`process_approach` と同じ
「対象点を毎 Tick 再計算して `steer_thrust_toward` に渡す」方式を再利用する）:

```
radial      = ship_pos - target_pos
dist        = |radial|
radial_unit = radial / dist（dist ≈ 0 なら任意の単位ベクトル）
tangent     = normalize(cross(UP, radial_unit))   // UP = (0,1,0) 固定、周回方向を一定にする
target_point = target_pos + radial_unit * radius + tangent * (radius * ORBIT_LEAD_FACTOR)
steer_thrust_toward(ship_pos, target_point)
```

`tangent` への寄与が周回を生み、`radial_unit * radius` への寄与が目標半径への収束を生む。
3D 空間で固定の `UP` 軸を使うのは、EVE 自体もオービットを実質的に2D（観測者から見た平面）
として扱っており、全軸対称な3D周回より「一定方向に回る」決定論的な振る舞いの方が
プレイヤーにとって予測可能なため。

### 2. Keep at Range — 対象から最低指定距離を保つ

```
KeepAtRangeCommand { ship_id, target, range: Option<f32> }
  → KeepAtRangeComp { target, range } を ship に付与
  → 毎 Tick process_keep_at_range() が:
     距離 < range  → 対象から真っ直ぐ離れる方向へ thrust
     距離 >= range → brake_thrust（詰め過ぎない・離れすぎない）
```

Orbit と異なり接線成分を持たない（周回しない）。「この距離より近づきたくない」
という純粋な離脱判断のための機能であり、Orbit（回りながら射程を保つ）とは
プレイヤーが選ぶ意図が異なる。

### 3. 対象とパラメータ

`target` は ADR-0015 の `ApproachTarget`（`Ship | Gate`）をそのまま再利用する
（型を増やさない・既存の `dest_in_ship_frame_abs` 経由の位置解決をそのまま使える）。
`radius`/`range` 省略時の既定値は `ShipStatsComp.weapon_range`
（フィット済み武器の最適射程。武器なしなら定数フォールバック）から導出する —
「射程内で回る／距離を保つ」が最も典型的な使い方であるため、デフォルトのまま
Orbit/Keep at Range を撃てば自然に当てやすい距離になる。

### 4. Tick 処理順序への追加（AI_DEVELOPMENT_GUIDE.md §6）

ADR-0015 の Step 2.5（Approach）の直後、Step 2.6（Warp）の前に追加する。
3つの持続的操船システムはすべて Movement（Step 3）より前に thrust を確定させる
必要があるため、同じ括りに並べる:

```
2.5  Approach System（process_approach）
2.55 Orbit System（process_orbit）              ← 新規
2.56 Keep at Range System（process_keep_at_range）← 新規
2.6  Warp System（process_warp）
3.   Movement System
```

Orbit/Keep at Range と Approach は同時に1つしか持てない（後から発行された方が
前の操船モードのコンポーネントを上書き除去する）。

### 5. 解除条件（ADR-0015 と同型）

以下のいずれかで `OrbitComp` / `KeepAtRangeComp` を除去する:

- 同じ船に `MoveCommand` / `StopCommand` が発行された
- 同じ船に `ApproachCommand` / 別の `OrbitCommand` / `KeepAtRangeCommand` が発行された
  （操船モードは排他 — 同時に2つの持続操船は持たない）
- 対象が ECS から消えた → 安全のためブレーキ
- 自船が `TransitState::InTransit` に入った
- Warp 中（Aligning/Warping）はコマンドを拒否する（Approach と同様、Warp が優先）

### 6. イベントは追加しない（ADR-0008 / ADR-0015 §5 / INV-MOVE 準拠）

Orbit/Keep at Range は Approach と同じ「コマンド（意図）」であり「事実（イベント）」
ではない。毎 Tick の thrust 更新で速度が変化すれば既存の `VelocityChanged` が
Movement System から発行される。専用イベントは作らない。
`OrbitComp`/`KeepAtRangeComp` はスナップショットに含めない（`ApproachComp` と同じ理由）。

---

## 影響

| 対象 | 変更 |
|---|---|
| `dawn-core` | `OrbitCommand` / `KeepAtRangeCommand` を追加（commands.rs） |
| `dawn-ecs` | `OrbitComp` / `KeepAtRangeComp` を追加（components/movement.rs） |
| `dawn-sector` | `node/orbit.rs` 新設。`apply_orbit_command(_owned)` / `apply_keep_at_range_command(_owned)` / `process_orbit` / `process_keep_at_range`。tick への Step 2.55/2.56 組み込み。Move/Stop/Approach/Warp との排他をcommands.rs に追加 |
| `dawn-actor` | `ClientCommand::Orbit` / `ClientCommand::KeepAtRange` を追加 |
| `dawn-simulation`（serve）/ `dawn-sector-node` | JSON パース・コマンドディスパッチ（両方の dispatch 経路） |
| クライアント | O キー（Orbit）/ K キー（Keep at Range）、選択済み対象に対して送信、HUD 表示 |
| `AI_DEVELOPMENT_GUIDE.md` | §6 Tick 順序に Step 2.55/2.56 を追記（人間承認のうえ） |

イベントスキーマ（event-catalog.md）の変更は**なし**（新イベントを作らないため）。

---

## 実装チェックリスト

- [x] `OrbitCommand` / `KeepAtRangeCommand` を dawn-core に追加（+ test）
- [x] `OrbitComp` / `KeepAtRangeComp` を dawn-ecs に追加
- [x] `process_orbit()` / `process_keep_at_range()` を node/orbit.rs に実装し、
      tick の Step 2.55/2.56 として呼ぶ
- [x] `apply_move_command` / `apply_stop_command` / Approach / Warp 開始で
      OrbitComp・KeepAtRangeComp を解除（相互排他）
- [x] `apply_orbit_command_owned` / `apply_keep_at_range_command_owned`
      （所有権チェック付き）を追加（+ test）
- [x] 対象消失時に解除 + ブレーキする（+ test）
- [x] `ClientCommand::Orbit` / `ClientCommand::KeepAtRange` を dawn-actor に追加
      + protocol.rs パース（+ test）
- [x] serve/mod.rs（apply_common_command）と dawn-sector-node/main.rs の
      両方の dispatch 経路に配線
- [x] クライアント: O/K キー + HUD 表示
- [x] AI_DEVELOPMENT_GUIDE.md §6 の更新（人間承認済み・2026-06-23）

---

## 却下した代替案

- **Orbit と Keep at Range を1つの `MaintainDistanceCommand { mode }` に統合する**:
  ADR-0015 が Approach 専用コマンドを選んだ前例に合わせ、操船意図ごとに型を分ける方が
  クライアント側の意図（「回る」か「離れる」か）がコード上も明確になる。却下。
- **真の3D球面オービット（固定 UP 軸を使わない）**: 任意軸での周回は数学的には可能だが、
  プレイヤーから見た周回方向が安定しない（ピッチ次第で回転面が変わる）。EVE 自体も
  視点平面でのオービットとして扱っており、固定 UP 軸の決定論的な振る舞いを優先した。
  将来、視点依存の周回軸が必要になればクライアント側のヒントとして拡張できる。
- **デフォルト半径を固定定数にする**: 武器射程と無関係な固定値だと、デフォルトの
  Orbit/Keep at Range が「当てにくい距離」になりがちで第一印象が悪い。
  `ShipStatsComp.weapon_range` から導出する方が直感的。
- **Tackle 中は Orbit/Keep at Range を禁止する**: Tackle は Warp/Jump のみを拒否する
  （ADR-0024）。タックルされながら距離を取ろうとするのはむしろ典型的な状況であり、
  禁止する理由がない。許可する。
