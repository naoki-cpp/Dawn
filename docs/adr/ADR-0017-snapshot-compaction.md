---
id      : ADR-0017
title   : イベントログのスナップショット圧縮と2層ログ（INV-002 改訂）
status  : accepted
date    : 2026-06-14
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0014（Raft / failover）, AI_DEVELOPMENT_GUIDE.md「Architecture Invariants」, docs/architecture/forbidden-changes.md, docs/reference/eve-reference.md §11.2
---

# ADR-0017 — スナップショット圧縮と2層ログ

> **ステータス注記**: 本 ADR は CLAUDE.md（権威ある運用規約）の INV-002 改訂 / FBD-001 注記追加を伴う。
> 人間の承認を得て CLAUDE.md §2・§10 に適用済み（2026-06-14）。
> なお圧縮・コールドアーカイブの**コード実装は未着手**（下記チェックリスト参照）。
> 本 ADR は方針確定であり、実装は別タスクで段階的に行う。

## 背景

eve-reference.md §11.2 で指摘した潜在的な設計矛盾を解消する。

現状、3 つの規則が長寿命シャードでは**同時に成立しない**:

- **FBD-001**: `EventStore` に `update / delete / truncate / rewrite` を持たせない。
- **INV-001**: イベントログは append-only。
- **INV-002**: State はイベントログの replay で**完全に**再現できる。

スナップショット基盤は既に存在する（[crates/dawn-simulation/src/snapshot.rs](../../crates/dawn-simulation/src/snapshot.rs)）。
`take_snapshot()` が `log_index = event_store.len()` の時点の ECS 状態を保存し、
`restore_from()` がスナップショット + それ以降のイベント replay で復元する。

しかし現在のスナップショットは設計上**最適化に過ぎない**。snapshot.rs のコメントが明言する通り:

> A snapshot is a performance optimisation — if it is lost or corrupt,
> the full state can always be recovered by replaying from log index 0.

つまり「index 0 からの完全 replay」が常にフォールバックとして要求される。
`EventStore` には圧縮手段が無く（`append / iter_from / len` のみ）、ログは**無限に成長**する。
帰結:

1. **replay 時間が無限に増大**する（再起動・障害復旧のたびに全履歴を再生）。
2. **ノード障害時の failover が非現実的**になる（ADR-0014 のノード引き継ぎが創世記 replay を要求）。
3. ディスク使用量が単調増加する。

「長寿命シャード」「数万エンティティの単一世界」を名乗る前に塞ぐべき**正しさ・運用性の穴**である。

---

## 決定

### 1. 2層ログモデルを導入する

イベントの保存先を 2 つに分ける。**どちらも何も破壊しない**（append-only の精神を全体で維持）。

| 層 | 性質 | 用途 |
|---|---|---|
| **ホットログ** | 検証済みスナップショットの背後を**圧縮**して有界に保つ | クラッシュ復旧 / ノード移行（ADR-0014 failover）/ 通常 replay |
| **コールドアーカイブ** | append-only・永久・圧縮・**経路外** | 完全な因果追跡 / タイムトラベルデバッグ / 監査（ADR-0001 の価値） |

「圧縮」とは、ホットログから**古いイベントを破壊すること**ではない。
検証済みスナップショットが覆う**前方区間をコールドアーカイブへ移送し、ホットログ側から切り離す**ことである。
イベントそのものはコールドアーカイブに永久に残る。

### 2. INV-002 を改訂する（CLAUDE.md §2）

```
INV-002（改訂案・2026-06-14 明確化を反映）:

スナップショットは権威ある永続チェックポイントである（単なる最適化ではない）。
状態は (最新の検証済みスナップショット) + (それ以降のホットログのイベントの末尾 catch-up)
から再構築する。運用で要る replay は「末尾 catch-up」のみ。

  - 派生・transient 状態（位置・capacitor・lock カウントダウン等）はスナップショットに
    永続化する。毎 Tick の純粋関数でありイベントに記録しない（位置と同じ扱い）。
    「イベントのみから」再構築できる必要はない。
  - スナップショットは検証可能でなければならない:
        ① snapshot → restore → snapshot がバイト一致（round-trip）
        ② snapshot + 末尾 Tick の再実行 == その時点の live 状態
  - 創世記（index 0）からの再構築は経路外（監査・災害復旧のみ）。イベント適用で権威ある状態を
    組み直し、transient 派生状態は sim を前進させて再計算する。通常運用・failover の依存先ではない。
```

旧 INV-002（「ログ index 0 のみから完全再構築」）はこの改訂で置き換える。
前方区間の移送は「過去の改変」ではなく「同じ過去のより簡潔な表現」である（FBD-001 の精神を侵さない）。

#### 2.1 なぜ replay が要るか（2026-06-14 明確化）

「genesis からの bit-equivalent replay」を検証しようとした際、capacitor.current（spawn 時付与・毎 Tick
recharge・イベント無し）が純粋なイベント replay で再構築できないことが判明した。これは穴ではなく、
**当初 INV-002 の "ログから完全再現" が dawn の運用実態より強すぎた**ことを示す。replay/ログの実需:

| 用途 | 必要な機構 | genesis replay 要否 |
|---|---|---|
| クラッシュ再起動 / failover（ADR-0014） | スナップショット + 末尾 catch-up | 不要（末尾のみ） |
| Sector Transit（ADR-0014） | ShipSnapshot を Raft で転送 | 不要（replay ですらない） |
| ノード間収束（Gossip/CRDT・将来） | 受信イベントを live 状態に apply | 不要（apply であって再構築ではない） |
| タイムトラベル / 監査（ADR-0001 の価値） | イベント履歴 + 任意時点の再構築 | 経路外（デバッグ時のみ） |

→ 運用ホットパスで genesis 完全 replay を要求する経路は無い。よって capacitor 等の transient 派生状態は
**スナップショットが持てば十分**で、イベントから再構築できる必要はない（位置と同じ）。ログは捨てない
（履歴・監査・ノード間伝播・スナップショット生成元として残す）。

### 3. FBD-001 は維持する（trait は append-only のまま）

圧縮を `EventStore` trait のメソッドにはしない。
`truncate(from_index)` のような**任意区間の切り捨て**は引き続き禁止（FBD-001）— それは履歴改変を許す。

圧縮は trait の外側の**運用プロセス（セグメント単位の移送）**として実装する:

```
圧縮手順（原子的・冪等）:
  1. スナップショット S を取得し、検証する（round-trip バイト一致 / snapshot + 末尾再実行 == live）。
  2. S を durable 化する（fsync）。
  3. S が覆う前方区間をコールドアーカイブへ追記する（append-only / 圧縮）。
  4. アーカイブ追記が durable になって初めて、
     ホットログを「スナップショット index から始まる新ファイル」として書き直し、
     旧ファイルと原子的に swap する（in-place 編集はしない）。
  5. いずれかの段階でクラッシュしても、旧ホットログがそのまま残るので安全
     （何も失われない。再試行で前進する）。
```

セグメント内のイベントは**決して書き換えない**。古いセグメントを丸ごと移送・切り離すだけ
（Kafka log compaction / LSM の考え方）。これにより FBD-001 はコード上も文言上も**そのまま維持**される。
CLAUDE.md §10 には「圧縮はセグメント移送として ADR-0017 が規定する。trait メソッド化は禁止のまま」と注記する。

### 4. failover はスナップショット + 末尾 replay を前提にする（ADR-0014 連携）

ノード障害で別ノードが Sector を引き継ぐとき、**創世記 replay を行ってはならない**。
最新スナップショット + ホットログ末尾 replay で復元する（EVE の Brain-in-a-Box 相当）。
これによりスナップショットは「ディスク節約」ではなく **failover の必須前提**になる。

### 5. （旧 D の格納）Transit コンセンサスは単一 Raft を意図的に維持する

本 ADR の失敗時復旧は Raft（ADR-0014）に隣接するため、撤回した「マルチ Raft」案の結論をここに 1 段落で残す。

- **単一 Raft グループは意図的な設計**である。ホットパスは Sector 内戦闘であり Raft に触れない。
  Raft に触れるのは低頻度の境界越え transit のみで、単一グループのスループットで長期間十分。
- **マルチグループ Raft（境界ごと）はメンテナンス不能な複雑さ**（クロスグループ 2PC・複数リーダー選出）
  のため**採用しない**。seam も事前構築しない（YAGNI）。
- 唯一の単純な備えは**バッチ提案**（fleet jump = N 隻を 1 エントリに束ねる。consensus モデルを変えない）。
  ただし**今は実装せず**、fleet-jump レイテンシが実測で問題になってから入れる。
- 既知の限界として明記: もし transit consensus が実測ボトルネックになったら、脱出路は
  境界ごとのマルチグループ Raft（Spanner 型）という**既知だが大きい**再設計である。事前構築はしない。

### 6. スナップショット形式はバイナリとバージョンロックされる（2026-07-28 追記）

`StateSnapshot` は postcard で保存される。**postcard は自己記述形式ではない** —
構造体のフィールドは位置で読まれ、名前もタグも持たない。したがって:

- **`#[serde(default)]` はフィールド追加の後方互換性を与えない。** JSON のような
  自己記述形式なら「キーが無いのでデフォルト値」と解釈できるが、postcard に「無い」を
  表す手段は無く、短いバッファは `DeserializeUnexpectedEnd` で失敗する。
  実測で確認済み（`snapshot.rs` の `a_snapshot_written_with_fewer_fields_fails_to_load`
  がこの挙動を固定している）。
- 過去に `station_inventories` / `docked_ships` / `docked_players` /
  `ShipSnapshot.anchor` / `.inventory` などへ付与されていた `#[serde(default)]` と
  「古いスナップショットも読める」というコメントは**事実に反していた**。それらの
  フィールド追加はいずれも形式破壊であり、旧形式のスナップショットは読み込み自体が
  失敗する。2026-07-28 に属性とコメントを削除した。
- **結論**: スナップショット形式はバイナリとバージョンロックされる。プレリリース段階の
  ため許容し、形式を変えたら運用者はスナップショットを再生成する。将来スナップショットを
  跨いだ互換性が必要になったら、明示的な形式バージョンヘッダを導入する（今は YAGNI）。

どのフィールドが永続化されるかは**両側から強制**する:
`SimulationNode::take_snapshot` はノードを網羅的に分解し、
`SimulationNode::apply_snapshot` は `StateSnapshot` を網羅的に分解する（いずれも `..` 無し）。
どちらかにフィールドが増えるとコンパイルが通らず、永続化するかどうかの判断を強制される。
この強制が無かったために `player_id_counter` が復元されず、再起動後に
`next_player_id` が既存プレイヤーと衝突する `PlayerId` を払い出していた（同日修正）。

---

## 影響

| 対象 | 変更 |
|---|---|
| CLAUDE.md §2 INV-002 | 「index 0 から完全再現」→「検証済みスナップショット + 末尾」に改訂（**要人間承認**） |
| CLAUDE.md §10 FBD-001 | 文言維持。「圧縮はセグメント移送として ADR-0017 が規定／trait メソッド化は禁止のまま」を注記 |
| crates/dawn-event-store | セグメント化 + コールドアーカイブ追記 + 原子的 swap（trait シグネチャは不変） |
| crates/dawn-simulation/snapshot.rs | コメントの「always recover by replaying from log index 0」を改訂後 INV-002 に合わせて更新 |
| ADR-0001 | 「再評価トリガー: ログが膨大化した場合」を本 ADR が引き取る（supersede ではなく拡張） |
| ADR-0014 | failover をスナップショット + 末尾 replay 前提に明確化（別途追記） |

イベントスキーマの変更は**なし**。スナップショットのバイナリ形式の変更も**なし**（プレリリースのため必要なら可）。

---

## トレードオフ（正直に）

- **ホットノード上での創世記 replay / 全履歴監査を失う**。これらはコールドアーカイブからの
  オフライン replay に移る（遅い・経路外）。タイムトラベルデバッグは可能だが「即時」ではなくなる。
- **スナップショット検証コスト**: round-trip（snapshot→restore→snapshot バイト一致）と
  snapshot+末尾再実行==live の検証は高価。毎スナップショットではなく、圧縮時 / 定期スケジュールでのみ実施する。
- **圧縮は状態を持つ運用プロセス**であり固有の障害モードを持つ（途中クラッシュ）。
  原子的（write-new-then-swap）かつ冪等にすることで「何も失わない」を保証する（決定 §3-5）。
- **コールドアーカイブの保管コスト**が新たに発生する（圧縮済み・経路外だが管理対象が増える）。

---

## 実装チェックリスト

- [x] 本 ADR を人間が承認する（status: proposed → accepted・2026-06-14）
- [x] CLAUDE.md §2 INV-002 を改訂（人間承認のうえ適用・2026-06-14）
- [x] CLAUDE.md §10 FBD-001 に圧縮の注記を追加（人間承認のうえ適用・2026-06-14）
- [x] スナップショット検証テスト: ① round-trip（snapshot→restore→snapshot バイト一致）② snapshot + 末尾 Tick == live（cap/hull 含む。INV-002 改訂の検証）— take_snapshot を正準ソートし 2 テスト追加（2026-06-14）
- [x] ホットログのセグメント化 + コールドアーカイブ書き出し（append-only）+ 原子的 swap の実装とテスト — `FileEventStore::compact`（base_index をファイルヘッダに埋め、rename 一発で原子的）+ 4 テスト（2026-06-14）
- [x] 圧縮の自動トリガ（ノードのスナップショット周期 → `compact()` 呼び出し）のオーケストレーション — `SimulationNode::checkpoint()`（snapshot→save→compact、save 先行でクラッシュ安全）+ `CheckpointScheduler`（論理 Tick 周期で駆動 / `checkpoint.rs`）+ Phase 3 デモへ実配線 + 3 テスト（2026-06-14）
- [x] failover が創世記 replay を要求しないことのテスト（ADR-0014 連携）— 圧縮後 reopen + restore で実証（2026-06-14）
- [x] snapshot.rs のドキュメントコメントを改訂後 INV-002 に更新（2026-06-14）
- [x] docs/architecture/event-catalog.md / docs/architecture/architecture.md に2層ログを反映（event-catalog §2 / architecture §5-C に ADR-0017 参照付きで記載済み）

---

## 却下した代替案

- **現状維持（スナップショット＝最適化のみ・圧縮しない）**: ログ無限成長・failover 非現実的・
  §11.2 の矛盾がそのまま残る。却下。
- **`EventStore` に `truncate()` を生やす**: 任意区間の切り捨てを許し、履歴改変の扉を開く。
  FBD-001 の精神を侵す。却下（圧縮はセグメント移送で代替）。
- **スナップショットのみ保持し全イベントを破棄（アーカイブ無し）**: ADR-0001 の核である
  因果追跡・タイムトラベルを破壊する。却下（コールドアーカイブで両立させる）。
- **マルチグループ Raft で transit をスケールさせる**: メンテナンス不能な複雑さ。却下（決定 §5）。
