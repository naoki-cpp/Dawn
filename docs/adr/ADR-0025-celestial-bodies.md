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
初期トポロジー（3星系 × 恒星1 + 惑星1）：

スケール：**1 unit = 10,000 km → 1 AU ≈ 15,000 units**（ゲート位置 49,000 units ≈ 3.3 AU = 小惑星帯外縁）

| 星系   | 天体               | 軌道              | 位置                   | 半径   | スペクトル型 |
|--------|------------------|-------------------|------------------------|--------|------------|
| Alpha  | Helios（G型恒星） | —                 | (0, 0, 0)              | 15 000 | 0.60       |
| Alpha  | Forge（惑星）     | 地球軌道（1.0 AU）| (15 000, 0, 0)         | 3 500  | —          |
| Beta   | Aegis（A型恒星）  | —                 | (0, 0, 0)              | 12 000 | 0.30       |
| Beta   | Haven（惑星）     | 火星軌道（1.52 AU）| (−21 600, 0, 7 200)   | 4 500  | —          |
| Gamma  | Crimson（K型恒星）| —                 | (0, 0, 0)              | 18 000 | 0.85       |
| Gamma  | Bastion（惑星）   | 金星軌道（0.72 AU）| (10 000, 0, −4 000)   | 3 000  | —          |

`CelestialBodyId` は全セクターにわたってグローバルに一意（Alpha: 0-1、Beta: 2-3、Gamma: 4-5）。
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
- 惑星：サーフェスマテリアルのスフィア（視覚半径 ≈ `radius × WORLD_SCALE × 0.08`）。
- 天体をクリックするとワープターゲットとして選択。W キーで WarpCommand を送信。
- HUD に選択中の天体名と `[W] Warp to <名前>` を表示。

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
