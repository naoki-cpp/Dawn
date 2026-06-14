---
id      : ADR-0004
title   : クライアント技術選択（Godot 4 + godot-rust）
status  : accepted
date    : 2026-06-04
deciders: [human, ai-agent]
---

# ADR-0004 — クライアント技術選択（Godot 4 + godot-rust）

## コンテキスト

EVE Online ライクな 3D 宇宙ゲームのクライアントを構築するにあたり、
以下の制約のもとで技術選択を行った。

```
要件:
  - EVE Online レベルの宇宙グラフィックス（宇宙船・ネビュラ・エフェクト）
  - AI エージェントによる継続開発
  - Rust で書かれたサーバー（dawn-core）との統合
  - Client-Side Prediction + Reconciliation の実装

禁止:
  - グラフィックス品質の妥協
  - AI が開発困難な複雑なコードベース
```

---

## 検討した選択肢

### A: Bevy（Rust 製ゲームエンジン）

**メリット:**
- サーバーと同一言語（Rust）
- `dawn-core` の型を直接 import できる
- Cargo Workspace に追加するだけで統合完了

**デメリット:**
- グラフィックスエコシステムが発展途上
  - パーティクルエフェクトが弱い
  - WGSL シェーダーの既存資産が少ない
  - ビジュアルエディタなし（全てコードで配置）
- バージョンごとの破壊的変更が多い（AI の生成コードが古くなりやすい）
- UI / HUD 構築が苦手

**宇宙ゲーム固有の問題:**  
EVE レベルの宇宙エフェクト（ネビュラ・ワープエフェクト・爆発）を
Bevy で実現するには、現時点では大量のカスタム実装が必要。

### B: Godot 4 + GDScript（+ 将来 godot-rust）

**メリット:**
- 3D ゲーム開発の実績が豊富（宇宙ゲームの事例多数）
- パーティクル・シェーダー・エフェクトが out-of-the-box で利用可能
- GDScript は Python ライクで AI の生成精度が高い
- Godot 4.x は API が安定しており、AI の生成コードが陳腐化しにくい
- ビジュアルエディタでアセット・シーン配置が可能
- godot-rust（GDExtension）で Rust コードを統合できる

**デメリット:**
- GDScript と Rust の 2 言語管理が必要
- サーバーとの型変換レイヤーが必要（proto 経由）
- GDExtension のセットアップに初期コストがある

### C: Three.js / Babylon.js（Web）

**デメリット:**
- 数万エンティティの 3D 描画で WebGL が限界に達する
- 型共有が完全に不可能
- 却下

### D: Unreal Engine

**デメリット:**
- 複雑性・ライセンス・規模が過剰
- 却下

---

## 決定

**Godot 4 + GDScript をベースとし、Phase 11 以降で godot-rust（GDExtension）を統合する**

---

## 根拠

### グラフィックス品質の優先

EVE Online レベルの宇宙表現には、成熟したパーティクル・シェーダーシステムが必要。
Bevy は技術的には可能だが現時点では工数が過大。
Godot 4 はこの要件を out-of-the-box で満たす。

### AI 開発効率

GDScript は Python ライクな構文を持ち、AI の生成精度が高い。
Bevy の ECS API（複雑な型パラメータ・lifetime・頻繁な破壊的変更）は
AI 生成コードの品質を下げるリスクがある。

ゲームロジックの大部分（シーン管理・UI・エフェクト）は GDScript で書き、
性能が必要な処理（Client-Side Prediction・型変換）は Rust（GDExtension）で書く。
この分離により、AI は得意な言語で各層を担当できる。

### 役割の明確な分離

```
Godot GDScript : 見た目・演出・UI（AI が主に書く）
godot-rust     : ロジック・型変換・予測補正（Rust で保証）
Dawn Server    : シミュレーションの真実（Rust で保証）
```

Bevy では「見た目」と「ロジック」が同一言語・同一フレームワーク内に混在し、
責務の分離が曖昧になりやすい。

---

## 統合アーキテクチャ

```
┌──────────────────────────────────────────────┐
│  Godot 4 クライアント                          │
│                                              │
│  GDScript 層（AI が主に書く）                  │
│  ├── シーン管理・カメラ制御                    │
│  ├── UI / HUD / マーカー表示                  │
│  ├── パーティクルエフェクト（爆発・エンジン）    │
│  └── サーバーイベント → 描画への反映            │
│                                              │
│  GDExtension 層（godot-rust / Phase 11〜）    │
│  ├── use dawn_core::{ShipId, Position, ...}  │
│  ├── Client-Side Prediction                  │
│  ├── Reconciliation                          │
│  └── gRPC クライアント実装                    │
└──────────────────────────────────────────────┘
                  ↕ gRPC（Phase 4〜）
┌──────────────────────────────────────────────┐
│  Dawn サーバー（Rust）                         │
│  dawn-core / dawn-ecs / dawn-event-store      │
└──────────────────────────────────────────────┘
```

## 段階的移行計画

| フェーズ | クライアント実装 | 型共有方法 |
|---|---|---|
| Phase 7-Client | Godot + GDScript のみ | proto 変換 |
| Phase 8–10 | GDScript でゲーム機能追加 | proto 変換 |
| Phase 11 | GDExtension（godot-rust）導入 | dawn-core を直接 import |

---

## 影響

### リポジトリ構成

```
dawn/        ← Cargo Workspace（サーバー・変更なし）
client/      ← Godot 4 プロジェクト（新規追加）
  project.godot
  scenes/
  scripts/   ← GDScript
  assets/
  gdextension/ ← Phase 11 以降（godot-rust）
```

### AI 開発ルールへの追加

```
- client/scripts/ の GDScript は Godot 4.x の API に準拠する
- GDScript から直接 gRPC を呼ばない（専用の connection.gd に集約）
- dawn-core の型への直接アクセスは gdextension/ 内の Rust コードのみ
```

---

## 今後の再評価トリガー

- Bevy のエコシステムが成熟し、グラフィックス品質が Godot に追いついた場合
- GDExtension の godot-rust バインディングが安定性の問題を起こした場合
- AI エージェントが GDScript より Bevy ECS を高品質に生成できると判明した場合
