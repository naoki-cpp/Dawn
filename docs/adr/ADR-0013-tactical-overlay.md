---
id      : ADR-0013
title   : タクティカルオーバーレイ（射程リング）
status  : accepted
date    : 2026-06-09
deciders: [human, ai-agent]
related : ADR-0006（フィッティング）, ADR-0012（タレット追跡）
---

# ADR-0013 — タクティカルオーバーレイ（射程リング）

## 背景

現在のクライアントには武器の射程を視覚的に確認する手段がない。  
プレイヤーは「最適射程はどこか」「フォールオフ境界はどこか」を
数値から推測するしかなく、ポジション判断が困難である。

EVE Online のタクティカルカメラオーバーレイは以下を表示する:

- **最適射程リング** — この距離以内なら命中率に射程ペナルティなし
- **フォールオフリング** — 最適 + フォールオフ距離。ここで命中率 50%
- その他（速度ベクトル、方位マーカーなど）

今回は戦闘判断に直結する射程リング 2 本を実装する。

---

## 決定事項

### 1. 表示するリング

| リング | 半径 | 色 | 意味 |
|---|---|---|---|
| 最適射程 | `weapon_range` | 緑 (rgba 0.2, 0.9, 0.2, 0.7) | この距離以内で命中率最大 |
| フォールオフ | `weapon_range + weapon_falloff` | 橙 (rgba 0.9, 0.5, 0.1, 0.5) | ここで hit_chance = 0.5 |

武器を装備していない場合はリングを表示しない（weapon_range = 0）。

### 2. トグル

`Tab` キーで表示 / 非表示を切り替える。デフォルト: **表示**。

### 3. レンダリング方式

Godot の `ImmediateMesh` を用い、毎フレーム XZ 平面上にポリラインで描画する。  
リングは水平面上の円（Y = プレイヤー船の Y 座標）。  
セグメント数 = 128（十分に滑らかに見える最小値）。

`MeshInstance3D` を `world_space = true` で配置し、
プレイヤー船 Node3D の位置に追従させる。

### 4. 射程データの取得方法

サーバーが送信する `PlayerFitting` JSON の各モジュールエントリに
`stat_delta` フィールドを追加する。

```json
{
  "type": "PlayerFitting",
  "modules": [
    {
      "slot": "High",
      "index": 0,
      "module_id": 1,
      "name": "Small Railgun I",
      "is_active": true,
      "is_active_module": true,
      "cap_cost_per_cycle": 60.0,
      "cycle_time_ticks": 10,
      "stat_delta": {
        "weapon_damage_add": 25.0,
        "weapon_range_add": 1500.0,
        "falloff_range_add": 1000.0,
        "tracking_speed_add": 0.035
      }
    }
  ]
}
```

クライアントは `is_active == true` かつ `kind == Weapon` のモジュールを集計:

```
weapon_range   = Σ weapon_range_add   (アクティブ Weapon モジュール)
weapon_falloff = Σ falloff_range_add  (アクティブ Weapon モジュール)
```

`ModuleActivated` / `ModuleDeactivated` 受信時に再集計して
リング半径をリアルタイム更新する。

### 5. 実装範囲

| 対象 | 変更内容 |
|---|---|
| `dawn-simulation/src/node.rs` | `build_player_fitting_json`: `stat_delta` フィールドを追加 |
| `client/scripts/main.gd` | `_on_player_fitting`: `weapon_range` / `weapon_falloff` を計算して保持 |
| `client/scripts/main.gd` | `_on_module_activated/deactivated`: 射程を再集計 |
| `client/scripts/main.gd` | Tab キートグル、`TacticalOverlay` ノード制御 |
| `client/scripts/tactical_overlay.gd` | 新規スクリプト: ImmediateMesh によるリング描画 |
| `client/scenes/tactical_overlay.tscn` | 新規シーン |

---

## 影響

**ポジティブ**
- 最適射程とフォールオフ境界が一目でわかり、
  「距離 1200u でオービットすれば命中率は？」が直感的に理解できる。
- ADR-0012 のトラッキング式と合わせて、
  近距離オービット vs 遠距離スナイプの判断材料が揃う。

**ネガティブ / リスク**
- `stat_delta` を PlayerFitting に含めると JSON が大きくなる。
  ただし接続時の 1 回のみ送信するため問題にならない。
- モジュール変更時に射程表示が一瞬ずれる可能性。
  サーバーの `ModuleActivated` を待ってから再集計するため
  最大 1 Tick（100ms）の遅延が生じるが許容範囲。

---

## 却下した代替案

### A. 専用 PlayerStats メッセージ

`weapon_range` / `weapon_falloff` をサーバーで計算して別メッセージで送る案。  
フィッティング変更のたびに追加イベントが必要になり、
イベントスキーマが複雑になる。`PlayerFitting` に `stat_delta` を含める方が
「1 メッセージで装備情報を完結させる」設計として整合する。

### B. 2D スクリーンオーバーレイ（Camera overlay）

カメラ投影を使って 2D に描画する案。  
視点が変わると射程リングが歪むため、距離感が直感的でない。  
3D リングは視点に関わらず正確な距離を示す。

### C. カメラ距離に合わせたスケール調整

リングをカメラから一定のスクリーン幅に固定する案（ミニマップ的表示）。  
これも距離感の直感的表現を損なう。3D 空間に正確な半径で置くことが本質。

---

*最終更新: 2026-06-09*
*対応 ADR: ADR-0006, ADR-0012*
