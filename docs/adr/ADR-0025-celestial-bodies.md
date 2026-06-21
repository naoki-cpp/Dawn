---
id      : ADR-0025
title   : 天体（恒星・惑星）— ワープ対象・空の太陽方向
status  : accepted
date    : 2026-06-18
deciders: [human, ai-agent]
related : ADR-0022（Warp）, ADR-0009（Star System Navigation）,
          ADR-0008（Ship Movement Events）, ADR-0016 §5（戦闘の深み）
---

# ADR-0025 — 天体（恒星・惑星）: ワープ対象・空の太陽方向

## 背景

各星系にはナビゲーション対象としてジャンプゲートしか存在しない。
星系に存在感を持たせ、イントラセクター内の移動目的地を増やすために、
恒星と惑星を静的な天体として追加する。

プレイヤーができるようにしたいこと：

1. 恒星がプレイヤーの移動に伴い常に正しい方向から見えること（宇宙背景に太陽ディスクを描画）
2. ゲートと同じ W キーのメカニクスで惑星・恒星にワープできること

## 決定

### 1. 新型定義（dawn-core）

```rust
pub struct CelestialBodyId(pub u32);

pub enum CelestialBodyKind { Star, Planet }

pub struct CelestialBodyDef {
    pub id           : CelestialBodyId,
    pub sector       : SectorId,
    pub kind         : CelestialBodyKind,
    pub name         : String,
    pub position     : Position,
    /// 論理半径（ユニット）。ワープ到着距離 = radius × 1.5。
    pub radius       : f32,
    /// 黒体スペクトル型 [0=O型/青 … 1=M型/赤]。惑星では無視。
    pub spectral_type: f32,
}

/// ワープターゲット。ゲートまたは天体を指定できる（ADR-0022 の拡張）。
pub enum WarpTarget {
    Gate(JumpGateId),
    Body(CelestialBodyId),
}
```

`WarpCommand.target: WarpTarget` が旧 `gate_id: JumpGateId` を置き換える。  
`WarpComp.target: WarpTarget` が旧 `WarpComp.gate_id` を置き換える。

### 2. 静的マップデータ（dawn-sector/galaxy.rs + data/galaxy*.toml）

`celestial_bodies_in_sector(sector_id)` がそのセクターの天体一覧を返す。  
初期トポロジー（Alpha は恒星1 + 惑星2、Beta/Gamma は恒星1 + 惑星1）：

スケール：**1 unit = 1,000 km → 1 AU ≈ 150,000 units**（ゲート位置 490,000 units ≈ 3.3 AU = 小惑星帯外縁）

> **[Superseded]** (2026-06-21) 当初は 1 unit = 10,000 km（1 AU ≈ 15,000 units）だったが、
> ワープ速度（5,000 units/tick）に対して軌道間の距離が近すぎ、移動が瞬間的に感じられた。
> 天体・ゲートの**位置**のみを 10 倍したスケールに変更（半径は変更なし。ゲームプレイ上
> 天体は元々実寸より誇張されたサイズなので、位置スケールの変更でむしろ実際の
> 恒星-軌道比に近づいた）。AU 換算比は変わらないため下表の軌道（AU）表記は不変。
>
> **[Superseded 改訂]** (2026-06-21・後) 上記「1 unit = 1,000 km」は誤り。戦闘データ
> （[data/modules.toml](../../data/modules.toml)「20,000 units = 20 km」）と整合する正準単位は
> **`1 unit = 1 m`**（EVE 準拠）。よってスケールは「1 unit = 1 m、星系内距離は**ゲーム的圧縮距離**
> （真の AU ではない・f32 が成立する ≤10⁶〜10⁷ units に収める）」とする。
> 下表の位置値・「AU」表記は**実天文スケールではなく圧縮スケールの目安**であり、移動感は
> Warp の所要時間（[ADR-0022](ADR-0022-intra-sector-warp.md) 媒介変数改訂）で調整する。
> 真の AU を 1 星系に置く大座標化は [ADR-0028](ADR-0028-large-world-coordinates.md)（Deferred）。
> ※ 具体的な圧縮値への data/galaxy*.toml 再調整（8abbe3f の 10× 見直しを含む）は別途実施。
>
> **[改訂]** (2026-06-21・さらに後) 「広く感じる」には**実際に遠くに置く**必要がある（移動目標は
> 視覚位置を実位置から切り離せない＝恒星ドリフトと同じ問題になる）。ただしその距離は f32 安全圏
> （~10⁷ units）で足り、i64 は不要。よって**過圧縮（±50,000）を解凍**し「広いスケール」に：
> `UNITS_PER_AU = 200,000`（惑星 ~10⁵ units）・ゲート ±600,000・`DEFAULT_HALF = 700,000`・
> `WARP_SPEED = 10,000`。狙いは「サブライト移動中に天体の方位がほとんど変わらない＝遠い」と
> 感じさせること。サブライトは天体近傍ローカル、天体間はワープ（EVE 流）。
>
> **[改訂]** (2026-06-21・後) 天体の `position` は **AU で記述**し、読込時に
> `UNITS_PER_AU = 30,000`（`crates/dawn-sector/src/galaxy.rs`）で units へ換算する方式にした
> （設計者が AU で軌道を考えられる）。ゲート `position` と天体 `radius` は units のまま
> （ゲートは Sector 縁のインフラ、radius は誇張視覚値）。上表の units 値は当時のスナップショットで、
> 現行の権威値は data/galaxy*.toml の AU 記述を参照。

| 星系   | 天体               | 軌道              | 位置                     | 半径   | スペクトル型 |
|--------|------------------|-------------------|--------------------------|--------|------------|
| Alpha  | Helios（G型恒星） | —                 | (0, 0, 0)                | 15 000 | 0.60       |
| Alpha  | Forge（惑星）     | 地球軌道（1.0 AU）| (150 000, 0, 0)          | 3 500  | —          |
| Alpha  | Meridian（惑星）  | 1.8 AU            | (207 000, 0, 174 000)    | 4 000  | —          |
| Beta   | Aegis（A型恒星）  | —                 | (0, 0, 0)                | 12 000 | 0.30       |
| Beta   | Haven（惑星）     | 火星軌道（1.52 AU）| (−216 000, 0, 72 000)   | 4 500  | —          |
| Gamma  | Crimson（K型恒星）| —                 | (0, 0, 0)                | 18 000 | 0.85       |
| Gamma  | Bastion（惑星）   | 金星軌道（0.72 AU）| (100 000, 0, −40 000)   | 3 000  | —          |

`CelestialBodyId` は全セクターにわたってグローバルに一意（Alpha: 0, 1, 6、Beta: 2-3、Gamma: 4-5）。
`CelestialBodyDef.sector` が所属 Sector を明示するため、天体 ID の割り当て規約に依存しない。

### 3. 天体へのワープ（dawn-simulation）

`can_propose_warp` と `apply_warp_command` が `WarpTarget` を受け取る。

- `Gate` ターゲット：従来どおり `activation_radius × 0.75` で停止。
- `Body` ターゲット：`body.radius × 1.5` の地点で停止（`BODY_WARP_ARRIVAL_FACTOR = 1.5`）。

`auto_jump` は `Gate` ターゲット専用。`Body` ターゲットでは無視する。

### 4. 空シェーダーの太陽方向（space_sky.gdshader）

以下の uniform を追加する：

```glsl
uniform vec3  sun_direction;  // 恒星への正規化ワールド方向ベクトル
uniform float sun_active;     // 0.0 = 無効、1.0 = 有効
uniform vec3  sun_color;      // スペクトル型に対応した色
```

`main.gd` の `_process()` で毎フレーム  
`normalize(star_server_pos - ship_server_pos)` を計算し、Godot 座標系（Z 反転）に変換してシェーダーに渡す。  
シェーダーはその方向に太陽ディスク・コロナ・グローを描画する。

### 5. Godot クライアント（main.gd）

- `CELESTIAL_BODIES` 定数がサーバー側のトポロジーをミラーする。
  > **[Superseded]** この `CELESTIAL_BODIES`（および `JUMP_GATES`/`STAR_SYSTEM_NAMES`）の
  > クライアント定数によるミラー方式は撤廃された。現在は `InitialState` メッセージが
  > `systems`/`jump_gates`/`celestial_bodies` を都度サーバーから送信し、`main.gd` は
  > それを `_gates`/`_bodies`/`_system_names` に取り込む（ハードコードなし）。
  > 詳細は `crates/dawn-sector/src/node/serialization.rs` の `initial_state_json()` と
  > `client/scripts/main.gd` の `_ingest_star_map()` を参照。本 ADR の元の決定事項
  > （ワープ機構・WarpCommand 等）はこの変更の影響を受けない。
- 接続時・星系変更時に天体ノード（MeshInstance3D）をワールド座標に配置し、星系遷移時に再生成する。
- 恒星：小さい発光スフィア（視覚半径 ≈ `radius × WORLD_SCALE × 0.05`）。スペクトル型に合わせたブルーム発光。
  > **[Superseded]**（2026-06-21）恒星の実体メッシュ（MeshInstance3D）は削除した。
  > 空シェーダー（`space_sky.gdshader`）が `sun_direction` ベースで恒星のディスク・コロナ・
  > グローを描画する一方、恒星の実体メッシュは有限距離の3Dオブジェクトとして配置されていたため、
  > 視点移動に伴う視差のつき方が両者で食い違い、角度によって恒星の見た目がズレる問題があった。
  > 恒星は方向ベースの空シェーダー描画のみとし、実体メッシュ（クリック選択・W キーでのワープ対象）
  > は撤廃した。惑星は実体メッシュを維持し、引き続きワープ対象として選択できる。
  > `sun_direction`/`sun_color` の計算（`main.gd` の `_update_sun_direction()`）はこの変更の
  > 影響を受けない。詳細は `client/scripts/navigation_marker_renderer.gd` の
  > `spawn_body_markers()` を参照。
- 惑星：サーフェスマテリアルのスフィア（視覚半径 ≈ `radius × WORLD_SCALE × 0.08`）。
- 天体をクリックするとワープターゲットとして選択。W キーで WarpCommand を送信。
  > **[Superseded]**（2026-06-21）上記の通り恒星はクリック選択・ワープ対象から外れた。
  > 惑星は変更なし。

### 6. ワイヤフォーマット

```json
{"type":"WarpCommand","ship_id":1,"target":{"Gate":2}}
{"type":"WarpCommand","ship_id":1,"target":{"Body":1}}
```

旧形式 `{"gate_id":N}` も後方互換として受け付ける（`protocol.rs` の `parse_client_command`）。

## 影響

- 既存の `WarpCommand { gate_id }` 呼び出し箇所はすべて `WarpCommand { target: WarpTarget::Gate(id) }` に変更が必要。
- Bot AI は引き続きジャンプゲートをターゲットにする（変更なし）。
- 新規イベントなし。ワープ移動は既存の `VelocityChanged` で記録される（ADR-0008）。
- `InitialState` の JSON に `celestial_bodies` 配列を追加（クライアントとの整合性確保）。

## 実装チェックリスト

- [x] ADR 承認
- [x] `CelestialBodyId / Kind / Def`、`WarpTarget` を dawn-core に追加
- [x] `CelestialBodyDef.sector` で天体の Sector 帰属を明示
- [x] `WarpCommand.target: WarpTarget` を dawn-core に追加
- [x] `WarpComp.target: WarpTarget` を dawn-ecs に追加
- [x] `Galaxy::bodies_in_sector()` を dawn-sector/galaxy.rs に追加
- [x] `SimulationNode.celestial_bodies` + ワープロジック更新を node.rs に追加
- [x] `InitialState` に `celestial_bodies` 配列を含める
- [x] ws_server.rs で WarpCommand の `target` フィールドをパース（旧 `gate_id` も受理）
- [x] space_sky.gdshader に太陽ディスク・コロナ・グローを追加
- [x] main.gd に天体ノード・ワープ・sun_direction 更新を追加
- [x] `cargo test --workspace` グリーン
- [x] テスト: 天体へのワープが `radius × 1.5` 以内で完了する
- [x] テスト: 別セクターの天体へのワープが拒否される
