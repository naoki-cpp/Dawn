---
id      : ADR-0034
title   : Economy Foundations — Item Generalization, Packaged Ships, Scrap Metal, and the Market/DB Boundary
status  : accepted
date    : 2026-07-02
deciders: [human, ai-agent]
related : ADR-0016（FBD-008 撤廃・段階的拡張方針§5）, ADR-0032（InventoryComp / Fit・Unfit の初出、本ADRが一般化する）, ADR-0003（Local-first development）, docs/process/roadmap.md §12（Phase 9）/ §9（継続的システム）, CONTEXT.md
---

# ADR-0034 — Economy Foundations

## 背景

roadmap.md §12（Phase 9 — Resource + Economy Context）は方向性のみで、具体的な
資源シンク・データモデル・Market の置き場所が未確定だった。`/grill-with-docs`
（`/grilling` + `/domain-modeling`）で人間と対話しながら以下を1つずつ決定した。
ADR-0016 §5「各項目は個別ADRを起票して着手する」に従い、本ADRでその決定を記録する。

現状の `InventoryComp`（ADR-0032）は `items: Vec<ModuleId>` で Module 専用に
型が固定されている。今回 Item の種類を増やす（Packaged Ship・Scrap Metal）に
あたり、この型を一般化する必要がある。

## 決定

### 1. Item の一般化

`Item` を「プレイヤーが所有し、インベントリに保管し、Station操作を通じて
消費/生産できるもの」の上位概念として定義する（CONTEXT.md へ反映済み）。
Module・Packaged Ship・Scrap Metal はすべて Item である。

```rust
enum ItemId {
    Module(ModuleId),
    PackagedShip(ShipTypeId),
    ScrapMetal,
}

// InventoryComp.items: Vec<ModuleId> を置き換える
pub struct InventoryComp {
    pub items: BTreeMap<ItemId, u64>,  // 種類ごとの数量スタック
}
```

`Vec<ModuleId>`（1エントリ=1個）から `BTreeMap<ItemId, u64>`（種類ごとの数量）
へ変える。Scrap Metal はバルクな数量資源であり「1エントリ=1個」方式では
表現できないため。`HashMap` ではなく `BTreeMap` にするのは、決定論的シミュレー
ション（INV-005・Replay再現性）で反復順序を安定させるため。

`ShipTypeId` と `ModuleId` は構造的には統合しない（船体はスロットレイアウト・
HPを持ち、モジュールはスタットデルタを持つ——フィールド形状が全く異なる）。
統合するのは経済上の扱い（Item として同じ棚に乗る）だけで、既存レジストリ
（`module_registry`/`ship_type_registry`）はそのまま別に保つ。

### 2. Packaged Ship / Station / Assemble・Disassemble

Ship は「Packaged Ship」（Item形態・格納可能・搭乗不可）と「Ship」（搭乗可能な
エンティティ）の2形態を持つ。**Station** で相互変換する:

- **Assemble**: Packaged Ship → Ship。Packaged Ship が未艤装であることが条件
  （艤装情報はPackaged Ship側に持たせない——既存の `fit_module_owned` の延長で
  Assemble後に艤装する）。
- **Disassemble**: Ship → Packaged Ship。Ship が**無傷**（Shield/Armor/Hull
  フル）かつ**未艤装**であることが条件。無傷を要求するのは、Disassemble→
  Assembleの往復で無料修理ができてしまうと既存の Local Repair モジュール
  （ADR-0033）の存在価値を壊すため。未艤装を要求するのは、艤装済みモジュールを
  Packaged Ship 側に持たせる新データ構造を避け、既存の Fit/Unfit 経路
  （ADR-0032）だけで足りるようにするため。

Station は初期段階では **NPC提供のみ**（プレイヤー建造不可）。プレイヤーが
建造できる構造物（Smart Assembly相当のアクセス制御述語含む）は
roadmap.md §12 の 9C で別途扱う。Assemble/Disassemble 自体は**無料**で
何度でも往復可能（新規建造ではなく状態変換のため）。

また、Station は「その場で操作を許可する地点」だけでは足りず、**Packaged Ship
と Scrap Metal を置く最小インベントリ（保管先）**を持つ。そうしないと、
Disassemble の生成物、Packaged Ship 建造の入力資源、建造の生成物の置き場所が
曖昧になるため。MVP では `PlayerId` 単位の Station inventory を持てば十分で、
最初から汎用倉庫や複雑な権限モデルまでは要らない。

さらに、Station 操作は単なる距離条件ではなく、**明示的な Dock/Undock 状態**を
通して許可する。つまり「半径内にいるのでそのまま使える」ではなく、
半径内で `DockCommand` が受理されて `docked_at = station_id` になっている間だけ
Assemble/Disassemble/建造が可能、という形にする。

実装順としては、**Station の最小実装と利用可否判定を先に置き、その内側で
Station inventory を置き、その上で Assemble/Disassemble と Packaged Ship 建造を
順に有効化する**。建造コスト（Scrap Metal 消費）は「どこでも押せるコマンド」
ではなく、Station 利用条件と Station inventory の上に乗るべきだからである。

Station inventory の**永続化境界**は Market と同じにはしない。Market は
「遅くてよい別ドメイン」なので SQL をそのまま権威にできるが、Station 操作は
`Dock` / `BuildPackagedShip` / `Disassemble` / 将来の `Assemble` のように
Sector の command validation と同じホットパスに乗る。そのため、**Tick/command
のたびに SQL を直接叩く権威モデルは採らない**。

代わりに、Station inventory は将来的に **「実行中の権威状態はメモリ、
耐久保存と容量対策は DB/スナップショット」** の二層構成へ進める。すなわち:

- command validation と event 生成は Sector メモリ上の inventory を使う
- durable state は snapshot / DB に保存する
- 全プレイヤー分を常時常駐させるのではなく、dock 中または最近使った player /
  station を lazy load / write-back cache として扱う余地を残す

MVP の 9B では `PlayerId` 単位の in-memory `Station inventory` を採用してよい。
2026-07-09 amendment: 実装は `(PlayerId, StationId)` 単位へ更新し、在庫は
station-local に分離した。dock 中の station context を通して参照する。
ただしこれは**最終形ではなく、storage seam を切った上で後から DB-backed 実装へ
差し替えられる前提の一時的な単純化**である。

### 3. Scrap Metal（資源シンク）

Packaged Ship を**新規に建造する**ときに消費する生資源を `Scrap Metal` と
命名する。当初「船の移動（Warp/Jump）で燃料を消費する」モデルを検討したが
却下し（§却下した代替案）、「建造コスト」モデルを採用した。

Scrap Metal は `ShipDestroyed` イベント発生時に**撃破者へ即座に加算**される
（新しい Wreck エンティティは作らない）。既存の戦闘という能動的行為がそのまま
資源獲得の導線になり、ADR-0016 §5 / eve-reference §7.4.3 の「受動採取
（AFK放置）は禁止・能動的な複数ステップの導線にする」という制約と、Non-Goals
の「no AFK mining」に整合する。

建造コストが発生するのは Packaged Ship の**新規建造時のみ**（一度きり）。
Assemble/Disassemble のループ自体には追加コストがかからない。

### 4. Market / データベースの境界

将来の Market（プレイヤー間取引）を見据え、SQL 等のデータベースをどこに
導入するかの境界を決めた。

- **Market の内部（注文帳・マッチング・価格履歴）は完全に独立した新規クレート
  （例: `dawn-market`）が SQL（SQLiteが第一候補・ADR-0003 Local-first と
  相性が良い）を**それ自身の権威**として持つ。Tickの決定論制約
  （INV-001/002/005）には縛られない、遅延許容の別ドメインとして切り離す。
- Item の実体増減（プレイヤーが所有するShip/PlayerのInventoryComp書き換え）
  だけは Sector 側の権威状態である。Marketはその操作を直接実行せず、
  `SettlementIntent`をtransactional outboxへ保存する。`dawn-simulation`のadapterだけが
  intentを**片側だけの独立した3つのCommand**へ変換して橋渡しする:
  - `RemoveItemCommand`（出品/List時、売り手のSectorへ）
  - `ReturnItemCommand`（キャンセル時、売り手のSectorへ）
  - `CreditItemCommand`（成立/Settle時、買い手のSectorへ）

  すべてのCommandはstableな`SettlementId`を持つ。Sectorは適用済みIDをcheckpointと
  `ShipFitted`イベント再生へ記録するため、ACK喪失後の再配送は在庫を二重に増減させない。対象Sectorが使えない場合は
  outboxをPendingのまま再試行し、拒否時はCurrency refund・Item compensationまたは
  Terminal状態をMarket側に記録する。

  「Aから消してBへ移す」という単一のTransferではなく、常に片側1SectorだけへComamndを
  発行する設計にすることで、売り手/買い手が別Sectorに所属していても
  Transit/Raft合意（INV-003）が一切不要になる（エンティティの所有権移転
  ではなく、単なるInventory内訳の増減のため）。

Market は**Station に dock 中のプレイヤーだけが操作できる**。Market の注文帳は
サーバー上で存続するが、閲覧・発注・Cancel の wire request は、プレイヤーの
所有 Sector が保持する `docked_players` に現在の Station context がある場合だけ
受理する。宇宙空間のクライアントがwireを直接送っても同じ境界で拒否する。
MVP では複数 Station 間で同一の Market 注文帳を共有する。Station ごとの注文帳や
遠隔注文管理は、Station/Market の地理的分離を決める別タスクで扱う。

### 5. Currency（通貨）は Market 側の台帳、Item ではない

Currency は `ItemId` に含めず、**`dawn-market` の SQL 台帳に `PlayerId` 単位の
残高**として持つ。Sector 側の `InventoryComp` には一切現れない。

理由: 価格情報（bid/ask の指値注文帳）はすでに Market が SQL で権威として
持っている。取引に使う Currency の残高までSector側 Item として二重に持たせると、
「本当の残高はどちらか」という権威の分裂を生む。Currency は Market ドメイン
だけで完結させるのが自然。

副次的な帰結: **Currency は Ship が撃破されても失われない**（EVE の ISK
ウォレットと同じ扱い）。Scrap Metal / Module / Packaged Ship は Ship が物理的に
運ぶ Item（`InventoryComp`）なので `ShipDestroyed` で失われうるが、Currency は
Player 単位の Market 台帳なので対象外——これは意図的な非対称性である。

決済時の扱いも非対称になる: Item の受け渡しは §4 の片側Command
（`RemoveItemCommand`/`ReturnItemCommand`/`CreditItemCommand`）で Sector へ
橋渡しするが、Currency の受け渡しは **Market内部のSQLトランザクションだけで完結**
し、Sector へは一切Commandを発行しない（買い手のCurrency残高を減らし、
売り手のCurrency残高を増やすだけ）。

### 6. 価格決定: 指値マッチング（板・オーダーブック）

Market は固定価格やアルゴリズム式（AMM/Bonding curve）で価格を決めない。
**買い手・売り手それぞれが指値（bid/ask）を出し、Market が交差した時点で
約定させる**、通常の板（オーダーブック）方式を採用する。

理由: CONTEXT.mdの設計原則「意図的なプレイヤー判断を増やすか」に沿う——
指値を出す/受け入れるという判断自体がプレイヤーの意思決定になる。アルゴリズム式
だと Market 自身が価格を決めてしまい、プレイヤーの判断が「買うか買わないか」
だけに縮小する。需要が多ければ bid が自然に吊り上がるので、追加の価格式は不要。

## 却下した代替案

- **資源シンクを船の燃料消費（Warp/Jump時）にする**: eve-reference §7.4.3の
  Frontier燃料経済を参考に最初に提案したが、ユーザーの意図は「建造コスト」
  であり却下。建造コストの方が「実損のある危険な宇宙」（ADR-0016柱④）に
  直結する。
- **ShipTypeId/ModuleId を1つのRust型に構造的統合する**: フィールド形状が
  大きく異なり、統合すると大きなリファクタが必要な割に経済的利益が薄い。
  `ItemId` enumによる語彙レベルの統一で十分。
- **Disassemble時に艤装ごとPackaged Shipへ保存する**: 艤装情報を持つ新しい
  データ構造が必要になる。既存のUnfit経路を使う方が実装コストが低い。
- **損傷した船でもDisassemble可能にする**: Disassemble→Assembleの往復による
  無料修理という抜け穴を生み、Local Repairモジュール（ADR-0033）を無意味に
  する。フルHP必須のバリデーションで防ぐ。
- **Scrap MetalをWreck（残骸）に残し、拾いに行く必要がある形にする**: 「拾う
  リスク」という駆け引きは魅力的だが、新しいエンティティ型・所有権・AoI配信
  が必要になり、§9Aの最小スコープを超える。即時加算をMVPとし、Wreck方式は
  将来の拡張候補として残す。
- **プレイヤーが最初からStationを建造できるようにする**: 構造物のアクセス
  制御（Smart Assembly相当）はそれ自体が別の大きな決定空間（§9C）。NPC提供の
  最小Stationだけを先に用意し、資源シンクの動作確認を待たずに済ませる。
- **Station inventory も Market と同様に SQL を即時の権威にする**: 大量入港時の
  メモリ圧迫懸念は正当だが、Station 操作は Sector の command validation と同じ
  ホットパスにあるため、毎回 DB 往復を伴う権威モデルは相性が悪い。永続化は
  DB/スナップショットへ逃がしつつ、実行中の権威状態はメモリに置く二層構成を
  採る。
- **MarketをSector側イベントから構築する「read model」（投影）として設計する**:
  当初はこの形（Marketの状態は全てEvent Logから再構築可能な投影）を提案したが、
  「Marketは遅くていい」という前提のもとで不要な複雑さと判断し、Market自身の
  状態（注文・マッチング）はSQL側を直接の権威とする形に変更した。Sector側の
  権威状態（Item所持）とだけ、片側Command経由で整合性を取る。
- **Currency を `ItemId` の1バリアントとして `InventoryComp` に持たせる**:
  最初に提案したが、Market が既に価格情報（指値注文帳）をSQLで権威として
  持っている以上、取引に使うCurrency残高までSector側Itemとして二重に持たせる
  意味がないと判断し却下。Currencyは`dawn-market`の`PlayerId`単位の台帳に
  一本化する。
- **Marketの価格をアルゴリズム式（AMM/Bonding curve）で決める**: Market自身が
  数式で価格を提示する方式も検討したが、プレイヤーの指値という意思決定を
  奪ってしまうため却下。板（オーダーブック）方式を採用する。

## 実装チェックリスト

- [x] dawn-core: Station 系イベント列を完成させる（`ShipDocked`/`ShipUndocked`/`PackagedShipBuilt`/`ShipDisassembled`/`ShipAssembled` 実装済み、event-catalog.md 追記済み）
- [x] dawn-core: `ItemId` enum（Module/PackagedShip/ScrapMetal。**Currencyは含まない**）
- [x] dawn-ecs: `InventoryComp.items` を `Vec<ModuleId>` → `BTreeMap<ItemId, u64>` へ一般化（ADR-0032 のデータモデルを置き換え）
- [x] dawn-sector: `ShipDestroyed` 発生時に Scrap Metal を撃破者へ加算する経路（MVP は `1 kill = 1 Scrap Metal` の固定値）
- [x] dawn-sector: スナップショット/Transit/PlayerLoadout JSON を `InventoryComp.items: BTreeMap<ItemId, u64>` に追従
- [ ] 「受動採取ではない」ことの再点検項目化（現状は取得経路が `ShipDestroyed` のみなのでコード読解で十分。別経路追加時に自動検証/CI 昇格を検討）
- [x] dawn-sector: Station（NPC提供の最小実装）
- [x] dawn-sector: Dock/Undock + Station 利用可否判定（`can_use` は docked 状態を見る）
- [x] dawn-sector: Station inventory（`PackagedShip` / `ScrapMetal` の最小保管先）
- [x] dawn-sector: Station inventory storage seam（ADR-0038 で SQLite projection として実装済み、`node/repositories/station_inventory.rs` の `StationInventoryRepository`）
- [x] dawn-sector: Assemble コマンド・バリデーション（入力は Station inventory 上の `PackagedShip`、**docked 中のみ**、Assemble 後の艤装は既存 Fit 経路で行う。`node/station_materialization.rs::assemble_ship_owned`）
- [x] dawn-sector: Disassemble コマンド・バリデーション（無傷・未艤装チェック、出力は Station inventory 上の `PackagedShip`、**docked 中のみ**）
- [x] dawn-sector: Packaged Ship 建造（Scrap Metal 消費、入出力とも Station inventory、**docked 中のみ**。MVP コストは `1 Scrap Metal / 1 hull` の固定値）
- [x] 新規クレート `dawn-market` の Dependency DAG 上の位置を確定（2026-07-13、`dawn-protocol`と同じ葉クレート。`dawn-simulation`にのみ組み込み、`dawn-sector-node`への配線は別タスク。詳細は roadmap.md §12 9D-1）
- [x] `dawn-market`: SQLite バックエンドの指値注文帳（bid/ask マッチング、2026-07-13。roadmap.md §12 9D-2）
- [x] `dawn-market`: `PlayerId` 単位の Currency 台帳（Bid時エスクロー・約定時決済・Cancel時払い戻し、2026-07-13。roadmap.md §12 9D-3）
- [x] `dawn-market`: `SettlementIntent` transactional outbox、stable `SettlementId`、SQLite atomic commit、Sector側重複配送防止（2026-08-10。#279）
- [x] client: Dock/Undock + Station操作UI（入港状態の表示と `D` / `U` / `B` / `Y` 操作、Packaged Ship のインベントリ表示、Assemble/Disassemble/建造UI すべて実装済み。Station Inventory のクリック/ドロップ方針と typed request 構築は `dawn-client-core::StationInventoryInteraction`、Godot側は行描画・ヒットテスト・drag geometry、GDExtensionは薄い型変換adapterを担当する）
- [x] client: Market閲覧UI（指値注文の発注・Currency残高表示、2026-07-17。Market専用postcard envelope、single/cluster runtime bridge、Godot `market_surface.gd` の板・Bid/Ask・発注・Cancel・Currency表示を実装。snapshotは最大200件）

#279後のSector側適用は`dawn-simulation::serve::market_settlement`に限定する。
`SimulationNode`はstable IDを受け取り、所有権・数量検証と重複排除を行った後、既存の
`ShipFitted`インベントリスナップショットへ記録する。Marketのorders schemaでは`ship_id`
を必須とし、outbox schemaはsettlement effect/status/compensationを保持する。
pre-release SQLite schemaは非対応で、clean schemaへ作り直す。9D-5のwire envelopeと
`MarketSnapshot`は維持し、`dawn-sector`は引き続き`dawn-market`を知らない。
- [x] CONTEXT.md: `Item`/`Packaged Ship`/`Station`/`Scrap Metal`/`Currency` を追記済み（本セッション中）
- [x] `cargo test --workspace` / `fmt` / `clippy -D warnings` 全緑
