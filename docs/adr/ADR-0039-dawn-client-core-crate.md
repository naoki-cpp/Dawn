---
id      : ADR-0039
title   : dawn-client-core — Godot非依存クライアントドメインモデルのRust抽出（Phase 1: Loadout）
status  : accepted
date    : 2026-07-10
deciders: [human, ai-agent]
related : ADR-0004（クライアント技術選定・GDExtension既定路線）, ADR-0007（通信方式・Phase 9以降で再検討と明記）,
          ADR-0032（インベントリ/ランタイム換装の初出）, ADR-0034（ItemId一般化）,
          docs/process/roadmap.md §13（Phase 10・GDExtension導入）
---

# ADR-0039 — dawn-client-core クレート新設（Phase 1: Loadout モジュール）

> Amendment (#339, 2026-08-24): `ModuleRow.kind` now uses the canonical
> `dawn_core::ModuleKind`. The migration-era client mirror and its fictional
> `Unknown` fallback were removed; invalid Godot strings now fail explicitly.

## 背景

`/improve-codebase-architecture` によるクライアント側の deletion test で、
`client/scripts/player_loadout.gd`・`module_row.gd`・`item_row.gd` の3ファイルが
「削除すると複雑さが複数箇所に再出現する」唯一の候補と判定された。具体的には:

- `ModuleRow`/`ItemRow` は typed `PlayerLoadout` projection
  payload（`crates/dawn-sector/src/node/player_loadout_projection.rs`）が
  `serde_json::json!` マクロで非型付きに組み立てるフィールド形状を、GDScript側で
  手書きの `REQUIRED_KEYS` チェックとして**独立に再実装**している。サーバー側の
  フィールドが変わってもクライアント側はコンパイルエラーにならず、実行時に
  `push_error` + 行ドロップとしてしか気づけない。
- `PlayerLoadout.simulate_modules_capacitor_ticks()`（毎フレーム呼ばれる capacitor
  シミュレーション）・`weapon_ranges()`・`effective_range_for_activation()` は
  純粋なドメインロジックだが、GDScript内にあるため GdUnit4（Godotエディタ起動が
  必要）でしかテストできず、`cargo test` の対象にならない。

一方、`connection.gd` も同じ wire payload を JSON Dict として独立に手書きしており
（`crates/dawn-actor/src/protocol/client_command.rs` の `ClientCommandJson` と同様の
関係）、型知識が「サーバーの projection」「クライアントの `*_row.gd`」
「`connection.gd` のコマンド送信」の3箇所に分裂している。

ADR-0004 は GDExtension 導入を既定路線として決定済みで、roadmap.md §13
（Phase 10）に位置づけられているが、Phase 10 は「GDExtension バインディング +
Client-Side Prediction」という大きな一括作業として計画されており、8D最小化方針
（roadmap「巨大基盤の一括建設をしない・薄いスライス」）に反する。本ADRは
Phase 10 を待たず、**GDExtensionなしで着手できる部分**（純粋Rustライブラリとして
の型定義とロジック）を独立した最初のスライスとして切り出す。

## 決定

- 新規クレート `dawn-client-core` を追加する。**`dawn-core` にのみ依存**し、
  Godot（GDExtension含む）・`dawn-sector`・`dawn-actor` への実行時依存は持たない。
- Dependency DAG 上の位置: `dawn-core` の直下（`dawn-ecs`/`dawn-storage` と
  並列）。他のどのクレートにも依存されない葉ノードとして追加する
  （将来 `dawn-client-gdext` が依存する）。
- Phase 1（本ADRのスコープ）は **Loadout モジュールのみ** を移植する:
  - `PlayerLoadoutMsg`（旧 `player_loadout.gd` 本体）
  - `ModuleRow`（旧 `module_row.gd`）
  - `ItemRow`（旧 `item_row.gd`）
  - `capacitor::simulate_modules_capacitor_ticks()` ほか純粋関数群
- 型設計:
  - `ModuleRow.kind` / `ItemRow.item_type` は `String` ではなく enum
    （`#[serde(other)] Unknown` 付き）にする。`_range_family()` 相当の match が
    exhaustive になり、サーバー側で kind が増えたときにコンパイラが気づかせる。
  - `stat_delta` はサーバーが送る11フィールド全てを持つ構造体
    （`#[serde(default)]` 付き）。GDScriptは4フィールドしか読んでいなかったが、
    型共有する以上サーバー側の実際の形状に合わせる。
  - 数値は `f64`（GDScriptの `float` は64bitなので現行の実効精度と一致）。
  - `cycle_remaining`/`forced_reason` はクライアントのみのランタイム状態として
    `#[serde(skip)]`。
  - null許容フィールド（`docked_station_id`/`active_ship_id`等）は `Option<T>`。
    GDScript側の `-1` 番兵は廃止する。
- **型の一致はコンパイル時ではなくテストで保証する。** `dawn-sector`（projection の
  実装先）は `serde_json::json!` マクロで非型付きにJSONを組むままとし、本ADRでは
  変更しない（projection 自体の型付け直しはスコープ外・将来の再評価対象）。
  代わりに `dawn-client-core` の **dev-dependency に `dawn-sector` を追加**し、
  「実サーバーの `build_player_loadout_json()` が出力したJSON文字列を
  `dawn-client-core::PlayerLoadoutMsg` が正しくパースできる」契約テストを書く。
  dev-dependencyは本番ビルドのDAGに影響しないため、`dawn-proto`（過去に「見返りが
  乏しい」と却下）を再燃させずに型ドリフトを検出できる。
- GDScript側（`player_loadout.gd`/`module_row.gd`/`item_row.gd`）は本ADRでは
  **変更しない**。GDExtensionバインディング（後続フェーズ）ができてから
  `main.gd` の呼び出し先を切り替え、その時点で旧GDScriptファイルとGdUnit4テストを
  削除する。

## 却下した案

- **`dawn-proto` のような共有スキーマクレートを新設し、`dawn-sector` 側の
  projection もそこから生成する**: サーバー側の projection は現状
  `serde_json::json!` で十分に機能しており、型付け直しは本ADRのスコープ
  （クライアント側の型共有）と独立した別の改善。一度に両方を変えると差分が
  大きくなり、レビュー・ロールバックの単位が崩れる。将来 projection 側を
  型付けし直す価値が出たら別ADRで扱う。
- **GDExtensionバインディングを本ADRに含める**: interface（Rust側の型・関数
  シグネチャ）が固まる前にFFI境界を書くと、interfaceの手戻りがFFI層にまで
  波及する。まず `cargo test` だけで interface を検証できる形に留め、
  GDExtension化は独立した後続ADRで扱う。
- **WebSocket送受信自体をRustに移す**: `WebSocketPeer`（Godot組み込み）は
  再接続・エラーハンドリングが枯れており、型共有という動機に対して移す
  追加コストが見合わない。移すのはメッセージの構築/パースのみ
  （Phase 3で `dawn-actor` の `ClientCommandJson`/`EventJson` 型を使う予定）。

## 実装チェックリスト

- [x] `crates/dawn-client-core/Cargo.toml` 新設。`dependencies` は `dawn-core`
      + `serde`(derive) のみ。`dev-dependencies` に `dawn-sector` を追加
- [x] ワークスペース `Cargo.toml` の `members` に追加
- [x] `AI_DEVELOPMENT_GUIDE.md`「Crate Boundaries」に1行追加
- [x] `docs/architecture/architecture.md` のクレート表・DAG図に追加
- [x] `CONTEXT.md`「Runtime Boundaries」に1行追加
- [x] `PlayerLoadoutMsg`/`ModuleRow`/`ItemRow`/`OwnedShipRow`/`StatDelta` 型定義
      （`#[derive(Debug, Clone, PartialEq, Deserialize)]`）
- [x] `simulate_modules_capacitor_ticks()`/`weapon_ranges()`/
      `effective_range_for_activation()` の純粋関数移植 + 単体テスト
- [x] 契約テスト: `dawn-sector::build_player_loadout_json()` の出力を
      `PlayerLoadoutMsg` でパースできることを確認するテスト
      （`dawn-client-core` 側、dev-dependency経由）
- [x] `cargo fmt --all -- --check` / `cargo test --workspace` /
      `cargo clippy --workspace -- -D warnings` 全件通過
- [x] GDScript側（`player_loadout.gd`等）は変更なし・GdUnit4は現状維持
- [x] `ModuleKind` mirror / `Unknown` fallback を削除し、`dawn_core::ModuleKind`
      を client-core / GDExtension で直接使用。Godot kind stringsは明示的な
      `Option` parseで検証する。
