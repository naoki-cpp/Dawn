---
id      : ADR-0038
title   : Station Inventory — SQLite as the durable authority, lazy-loaded in memory
status  : accepted
date    : 2026-07-08
updated : 2026-08-09
deciders: [human, ai-agent]
related : ADR-0034（Economy Foundations）, ADR-0017（snapshot/public Event archive）, ADR-0049（Sector recovery authority）, docs/process/roadmap.md §12
---

# ADR-0038 — Station Inventory SQLite Backing

> **ADR-0049 amendment (2026-08-07):** 下記本文はADR-0038が2026-07-08に選んだ
> SQLite-backed lazy storageの背景・実装判断を履歴として保持する。ただし次のauthority/
> recovery clausesはADR-0049によりforward-amendされた。
>
> - 「Station inventoryのdurable authorityをSQLiteに置く」はsuperseded。現在のexact Sector
>   authorityはADR-0049 recovery journal/checkpointのStation aggregate deltaである。
> - SQLiteを先に同期更新し、その後のpublic Event appendとの間にnarrow inconsistency windowを
>   許容する§帰結は撤回。authoritative transition durable -> local live apply -> required idempotent
>   Station projectionの順で、projection failure後はfail-stop/catch-upする。
> - SQLite/node-local DBという**製品・read-model選択は維持可能**。#277では
>   `SectorRepository`のconnectionを`AdmissionRepository`、`IdentityRepository`、
>   `StationInventoryRepository`のexplicit viewへ分け、`SimulationNode`のinterior-mutability
>   cacheは削除した。repository shapeはrecovery authorityを変更しない。
> - public `PackagedShipBuilt` / `ShipDisassembled` / `ShipAssembled` replayをStation exact reducerに
>   戻さない。Station authorityはRecoveryDeltaであり、public Eventはfact/projection inputである。
> - Station projectionはStation-changing transitionのdedupに加えてglobal contiguous
>   `projection_applied_through`を持つ。非Station transitionもno-opとしてwatermarkを進め、
>   replica promotion pointと同じauthoritative journal coordinateでfreshnessを証明する。
>
> 以下の旧本文中「SQLiteがauthority」「snapshot+event tail」「不整合windowを許容」は**歴史的な
> 原決定の記録**であり、現在のnormative recovery behaviorではない。

> **#277 / Station production projection amendment (2026-08-22):** `repositories.rs` now owns the
> node-local SQLite schema. Fresh admission reservations durably consume IDs and
> persist allocator watermarks before `Welcome`; existing protocol rows and
> materialized snapshot IDs raise those watermarks on reopen. Station rows are
> read through `StationInventoryRepository`, while projection transitions are
> deduplicated and advanced through a contiguous global journal cursor. The
> production projection API. The shared runtime now feeds each committed
> RecoveryDelta into it after local live apply, passing the complete journal
> range and ordered Station mutations. Command preparation uses only a
> frame-local touched-key overlay; the full Station aggregate is never copied
> into `SimulationNode`, `NodeState`, or every checkpoint.

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

  > **訂正（2026-07-28）**: この「後方互換フィールドとしてのみ残す」判断は誤りだった。
  > postcard は自己記述形式ではないため、フィールドを削った旧形式のスナップショットは
  > 読み込み自体が `DeserializeUnexpectedEnd` で失敗する（ADR-0017 §6）。つまり
  > `restore_from` の移行分岐（`migrate_from_snapshot`）は**到達不能**であり、それを
  > 覆っていたテストもスナップショットをメモリ上で直接組み立てていたため `load` 経路を
  > 一度も通っていなかった。フィールド・移行メソッド・当該テストはいずれも削除済み。
  > 本 ADR の実質的な決定（SQLite が耐久性の権威、メモリは有界キャッシュ）は変わらない。
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

> **Current interpretation after ADR-0049:** 上記の`apply_event` removal自体は維持するが、
> 理由は「SQLiteが独立authorityだから」ではなく、public Event replayをStation exact reducerに
> しないためである。live/recovery Station mutationはauthoritative RecoveryDeltaから行い、
> SQLite/repository projectionはstable transition identityで冪等にcatch upする。

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
  M-9（`architecture-review/server-pending.md`）で既に許容している「crash-onlyで
  narrowな不整合ウィンドウ」と同じクラスのリスクとして許容し、今回は
  解決しない。
- `dawn-sector`に`rusqlite`（bundled feature）を追加する。FBD-002
  （dawn-coreへの外部依存禁止）は`dawn-sector`には適用されない。
- 実装は `docs/process/roadmap.md` §12 9B補足「Station inventory の保存戦略」
  を参照。

> **Current consequence after ADR-0049:** 上記narrow inconsistency windowの許容は撤回済み。
> #271/#272/#277のmigration後はjournal-first authoritative transition + idempotent projectionで
> recoveryする。`rusqlite`/bounded lazy accessという製品判断は引き続き利用可能である。
