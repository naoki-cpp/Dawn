---
id      : ADR-0038
title   : Station Inventory — SQLite as the durable authority, lazy-loaded in memory
status  : accepted
date    : 2026-07-08
deciders: [human, ai-agent]
related : ADR-0034（Economy Foundations §2 Market/データベースの境界 — このADRが宿題として残した storage seam を実装する）, ADR-0017（Event Store 2層ログ・snapshot+tail replay の元設計）, docs/process/roadmap.md §12（Phase 9B 補足「Station inventory の保存戦略」）
---

# ADR-0038 — Station Inventory SQLite Backing

## 背景

`SimulationNode.station_inventories: BTreeMap<(PlayerId, StationId), BTreeMap<ItemId, u64>>`
は全プレイヤー分の Station inventory を起動から終了までメモリに常駐させ続け、
`StateSnapshot`（`persistence/snapshot.rs`）にもその全体を毎回まるごと
シリアライズしている。プレイヤー人口が増えるほどこれは際限なく肥大化する
——ADR-0034 §2 が「全プレイヤー分を常時常駐させるのではなく、lazy load /
write-back cache として扱う余地を残す」と明記しつつ、9B の MVP としては
意図的に単純化していた部分（"最終形ではなく、storage seam を切った上で
後から DB-backed 実装へ差し替えられる前提の一時的な単純化"）そのものである。

`station_inventory_storage()`/`station_inventory_storage_mut()`（`node/station.rs`）
という seam は ADR-0034 の時点で既に切ってあり、呼び出し側は生の `BTreeMap` を
直接知らない。今回はこの seam の**内側だけ**を SQLite バックエンドへ差し替える。

## 決定

- Station inventory の**永続化の権威を SQLite に置く**。`credit_station_item`/
  `try_debit_station_item` は呼ばれるたびに SQLite へ同期的に書き込む。
- メモリ上には**直近に触れた `(player, station)` だけの有界キャッシュ**を持つ
  （容量超過分は追い出す。追い出しは常に安全——追い出す時点で既に SQLite へ
  同期書き込み済みだからflush-before-evictが不要）。
- `StateSnapshot.station_inventories` フィールドは**今後書かれなくなる**
  （古い形式のスナップショットを読むための後方互換フィールドとしてのみ残す）。
  Sector の再起動時に全プレイヤー分をメモリへ丸ごと読み込むコストが消える
  ——これが今回のユーザー報告「全アイテムをロードしておく必要があるのはまずい」
  への直接の対処。
- `apply_event` のリプレイ側（`PackagedShipBuilt`/`ShipDisassembled`/
  `ShipAssembled` の3アーム）は Station inventory への `credit`/`try_debit`
  呼び出しを**取り除く**。理由は次節。

## ADR-0034 との関係（矛盾ではなく実装）

ADR-0034 §2 は「Tick/commandのたびにSQLを直接叩く権威モデルは採らない」と
明記しているが、これは「**毎回のドッキング/読み取りでDB往復するモデル**」
（Marketのような高頻度アクセスを想定した拒否）を指す。本ADRの設計は:

- **読み取り**はキャッシュ優先（miss時のみ1回DBを読み、以降はキャッシュ）。
  `can_use_station` 等のドッキング判定自体は Station inventory に一切触れない
  （`docked_players`マップのみで完結）ため、入港のたびにDB往復は発生しない。
- **書き込み**は Build/Disassemble/Assemble/TransferToStation という、
  プレイヤーが能動的に押す低頻度の経済アクションでのみ発生する。
  「command validationはメモリ上のinventoryを使う」というADR-0034の原則は
  維持している——SQLiteへの同期書き込みは、そのメモリ上の変更に対する
  耐久化の副作用であって、判定自体をDB越しにするわけではない。

つまり本ADRはADR-0034 §2が残した「storage seam を切った上で後から
DB-backed 実装へ差し替える」という宿題の実装であり、その拒否した代替案
（高頻度アクセスでのDB権威化）とは別のものである。

## 正しさの要点: リプレイとの二重適用を避ける

現状、`apply_event` の `PackagedShipBuilt`/`ShipDisassembled`/`ShipAssembled`
アームは `credit_station_item`/`try_debit_station_item` を自分でも呼ぶ。
これは「スナップショットの `station_inventories` は `snapshot.log_index`
時点で切り取られており、それより後のイベントだけが再生される」という前提の下で
正しく機能していた（他の状態と同じ snapshot+tail replay パターン、ADR-0017）。

SQLite が権威になり、ライブコマンド実行時に同期的に書き込まれるようになると、
SQLite は**クラッシュ直前まで**の状態を既に持っている。この状態のまま
`apply_event` のリプレイが同じ `credit`/`try_debit` を再実行すると、
**二重適用**になる。

対処: 上記3アームから Station inventory への `credit`/`try_debit` 呼び出しを
削除する（ship の挿入/削除・tickの更新など、そのアームの他の効果はそのまま
残す）。ライブ側のコマンドハンドラ（`station.rs`/`inventory.rs`）は元々
イベント発行前に自分で `credit`/`try_debit` を呼んでいるため、ライブパスの
挙動は変わらない——リプレイ側の**冗長になった**再構築だけを取り除く。

## 却下した案

- **SQLiteを単なるlazy-loadキャッシュにし、真実の情報源はevent replayのまま**:
  素直だが、「起動時に全員分をメモリに載せる」問題そのものは解決しない
  （snapshotの`station_inventories`が残り続ける限り、そこに全員分が集まる）。
  今回のユーザー報告の直接原因を解消しないため却下。
- **書き込みを非同期write-behindキューにする**: Station操作はtickごとではなく
  プレイヤーが能動的に送る低頻度コマンドであり、既に`FileEventStore::append()`
  がtick終端で同期flushしているのと同程度の負荷でしかない。非同期化の複雑さに
  見合わないため却下（同期書き込みを採用）。

## 帰結

- クラッシュのタイミングによっては、SQLiteへの書き込みとイベントログへの
  追記の間に**狭い不整合ウィンドウ**が残る（例: Station inventoryは加算済み
  だが`ShipDisassembled`はログに記録されずshipが残ったまま）。これは
  M-9（`architecture-review-server.md`）で既に許容している「crash-onlyで
  narrowな不整合ウィンドウ」と同じクラスのリスクとして許容し、今回は
  解決しない。
- `dawn-sector`に`rusqlite`（bundled feature）を追加する。FBD-002
  （dawn-coreへの外部依存禁止）は`dawn-sector`には適用されない。
- 実装は `docs/process/roadmap.md` §12 9B補足「Station inventory の保存戦略」
  を参照。
