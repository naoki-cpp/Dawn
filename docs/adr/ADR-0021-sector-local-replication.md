---
id      : ADR-0021
title   : Sector-local 複製は単一所有 + 追記ログのゴシップ配布（CRDT / LWW は採らない）
status  : accepted
date    : 2026-06-15
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0002（Actor / ReplicationBus）, ADR-0014（Raft / Sector Transit）, ADR-0017（2層ログ / スナップショット権威）, AI_DEVELOPMENT_GUIDE.md「Event Workflow」（CRDT と Raft の責務分離）, AI_DEVELOPMENT_GUIDE.md「Architecture Invariants」（INV-001/002/004/005）, AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」（Dependency DAG）, docs/architecture/ownership.md（Entity Ownership）
---

# ADR-0021 — Sector-local 複製は単一所有 + 追記ログのゴシップ配布

> **ステータス注記**: 本 ADR は **proposed**（人間承認待ち）。承認されると CLAUDE.md §1 の
> 「CRDT と Raft の責務分離による高スループット同期」の**文言修正**を伴う（CRDT/LWW という機構名を
> 「追記ログのゴシップ配布」に置き換える）。CLAUDE.md の不変条件（INV-001..006）の**改訂は伴わない** —
> むしろ既存不変条件が CRDT 不要を含意していることの明文化である。実装は Phase 8D の前提整理。

## 背景

AI_DEVELOPMENT_GUIDE.md「Event Workflow」は同期戦略を「**CRDT と Raft の責務分離**」と表現し、roadmap 8D-2 は
`dawn-distributed（Gossip + CRDT / LWW-Register）` を新規クレートとして予定している。
8D 着手前に「Sector-local 複製に CRDT（特に LWW-Register）は本当に要るか」を確定する。

CRDT が価値を持つのは「**複数の複製が同じデータを同時並行に書き、後で調整役なしに収束させたい**」場面である
（オフライン編集の和集合、分散カウンタ等）。dawn の権威モデルがこの前提に当てはまるかを検証した。

### 用語: CRDT とは

**CRDT（Conflict-free Replicated Data Type / 競合なし複製データ型）** = 複数ノードが同じデータの複製を持ち、
各自バラバラに（同時・オフラインでも）更新しても、**調整役（リーダー・ロック・合意）なしで自動的に
同じ状態へ収束する**データ構造。鍵は、マージ操作が次を満たすこと:

- **可換**（更新の到着順に依らない）・**結合的**（grouping に依らない）・**冪等**（同じ更新が何度届いても結果不変）

これらを満たすと、どの順序で・何回 更新が届いても必ず同じ状態に収束するため、合意もロックも要らない。
代表例: G-Counter（増えるだけのカウンタ）、G-Set / OR-Set（集合・和集合でマージ）、
**LWW-Register**（単一の値。衝突したら**より新しいタイムスタンプの書き込みが勝つ** = Last-Writer-Wins）、
Sequence CRDT（共同編集テキスト）。roadmap が名指す「LWW-Register」は、1 つの可変値を複数ノードで複製し
衝突を「新しい方が勝つ」で解決する最も単純な CRDT である。

**Raft との対比**（dawn は両者を使い分ける設計 / AI_DEVELOPMENT_GUIDE.md「Event Workflow」）:

| | Raft | CRDT |
|---|---|---|
| 一貫性 | 強一貫（単一の合意ログ） | 結果整合（いずれ収束） |
| 調整 | リーダー + 過半数が必要 | 不要・常に書ける |
| 代償 | 可用性を一部犠牲（過半数割れで停止） | 単一の全順序を諦める |
| 競合時 | 1 つの正規履歴に直列化 | マージで**両者を融合** |

本 ADR の論点は「Sector-local 状態の複製に、この CRDT（特に LWW）のマージが要るか」である。

### 検証: 所有権モデルの下で並行衝突は構造的に発生しない

docs/architecture/ownership.md は「**各 Sector は必ず 1 ノードが所有**」「**各エンティティは必ず 1 Sector に所有される**」と定める。
権威ある状態への並行衝突が起きうる経路を総当たりした:

> **用語: split-brain（スプリットブレイン）** = フェイルオーバーの最中に 2 ノードが同時に「自分が所有者」と
> 思い込み両方が書く危険状態。ネットワーク分断で旧所有者が「死んだ」のか「見えないだけ」か区別できないために
> 起きる。dawn では Raft の過半数ルールが防ぐ: 分断された少数派（旧所有者）は commit に必要な過半数に届かず
> 何も確定できない。多数派側で選ばれた新所有者だけが正規に進み、分断回復時に旧所有者の未確定 tail は破棄される。

| 経路 | 衝突するか | 実際の解決 |
|---|---|---|
| 通常の Sector-local 書き込み | しない。所有ノードが唯一のライター | — |
| エンティティが複数 Sector から同時更新（構造物への多方面ダメージ等） | しない。エンティティは 1 Sector 所有。他所の作用は所有 Sector へ command route | 所有ノードが順に適用 |
| 市場の板・在庫アイテム（将来） | しない。板/アイテムも 1 エンティティ = 1 所有 Sector | command route |
| Failover 中の split-brain（旧所有者が分断中に書く） | **見かけ上ありうるが権威ではない** | **Raft が所有権を裁定**。旧所有者の分断中 tail は再合流時に破棄（divergent tail truncation） |

**結論**: 権威ある状態は論理時刻のどの瞬間も**単一ライター**である。並行衝突は構造的に起きない。
唯一の"衝突"＝所有権移転は **Raft が裁定**し、「**新所有者が丸ごと勝ち、旧 tail を破棄**」で解決する。
これは CRDT のマージ（両者を融合）とは**正反対の意味論**であり、ここで CRDT を使うのはむしろ誤りである
（分断中の不正な書き込みを正規の状態に融合してはならない）。

### 追記ログはそれ自体が収束する（CRDT の自明な下位集合）

権威モデルはイベントソーシング（追記専用ログ / INV-001）。ログは:

- **追記専用・イベント不変**（INV-001）→ 上書き・削除がない
- **(論理 Tick, NodeId) で全順序**（INV-005）→ 受信順に依らず同じ列に整列できる
- **エンティティ ID が一意・再利用不可**（INV-004）→ イベント適用が冪等（重複受信は no-op）

ゆえに「Sector-local 複製」の実体は、**所有ノードの追記ログを他ノードへ配り、論理時刻順に適用する**だけ。
これは grow-only ログ（= 自明な CvRDT）であり、**LWW-Register のような競合解決機構を必要としない**。
「どちらが新しいか」の判定も、LWW のような物理タイムスタンプ（**FBD-003 で禁止**）ではなく
論理 Tick で決まる。結局すべて「ログを順に流す」に帰着する。

---

## 決定

### 1. Sector-local 複製 = 追記ログのゴシップ配布（log shipping）。CRDT/LWW は採らない

```
複製の単位 : 所有ノードの追記専用イベントログ（INV-001）
配布       : ゴシップ（エピデミック配布）で interested ノードへ伝播。スループットのため
             Raft を経由しない（Raft は Sector 越え transit 専用 / ADR-0014）
整列       : 受信側は (論理 Tick, NodeId) で整列して適用（INV-005・決定的）
冪等性     : 同一イベントの重複適用は no-op（INV-004 の一意 ID で識別）
アンチエントロピー: 取りこぼしは `PublicEventIndex` 範囲で再要求し、
`PublicEventTail` が保持範囲外を返した場合は snapshotへ切り替える。
追いつき   : base_index より遅れた複製はスナップショット転送で追いつく（ADR-0017・InstallSnapshot）
```

**競合解決 CRDT（LWW-Register / OR-Set 等）は導入しない。** 単一所有により不要であり、
所有権移転では CRDT マージは誤りだからである（上記検証）。

### 2. 「高スループット同期」の本質は「Raft 迂回の非同期ゴシップ」であって「CRDT」ではない

AI_DEVELOPMENT_GUIDE.md「Event Workflow」の意図（重要な排他は Raft で厳密に、大量の Sector-local 状態は安く速く）は**正しい**。
ただしその"安く速い"経路の実体は **追記ログを Raft に通さず非同期ゴシップで配る**ことであり、
CRDT のマージ演算ではない。文言を機構名（CRDT/LWW）から仕組み（追記ログのゴシップ配布）へ正す。

### 3. dawn-distributed の責務を「ログ配布 + アンチエントロピー + スナップショット転送」に確定

roadmap 8D-2 の `dawn-distributed（Gossip + CRDT / LWW-Register）` を
`dawn-distributed（追記ログのゴシップ配布 + アンチエントロピー + スナップショット転送）` に改める。
CRDT ライブラリ・LWW レジスタは作らない。現行の `ReplicationBus`（In-Memory broadcast / ADR-0002）の
ネットワーク版に相当する。

---

## 唯一の正直な反例と、その扱い

**非権威・ソフトな多重ライター状態**（例: プレゼンス「誰がオンラインか」、メトリクスの緩い集約）は、
強整合が不要で AP 寄りに振りたい。そこでは将来 CRDT が局所的に妥当になりうる。

- だがこれは**コアのゲーム状態ではない**し、現時点で要件がない。
- 必要が生じた時に、その状態に限定して個別 ADR で CRDT を採用すればよい（YAGNI）。
- **基盤の中核（権威ある Sector 状態の複製）に CRDT を最初から焼き込まない。** これが本 ADR の主眼。

---

## 不変条件との関係

| 不変条件 | 関係 |
|---|---|
| INV-001 | 複製単位は追記専用ログ。ゴシップは「配る」だけで上書き・削除をしない |
| INV-002 | 遅れた複製はスナップショット + 末尾 catch-up で追いつく（ADR-0017）。創世記 replay 不要 |
| INV-004 | 一意・再利用不可 ID によりイベント適用が冪等。重複ゴシップ受信が安全 |
| INV-005 | 受信側は論理 Tick + NodeId で整列。物理時刻不使用（FBD-003）。LWW のタイムスタンプ前提と相容れない＝採らない理由の一つ |
| §5 所有権 | 単一所有 = 単一ライター。並行衝突なし。移転は Raft 裁定（新所有者が勝つ・旧 tail 破棄） |
| ADR-0014 | Sector 越え transit のみ Raft。Sector-local はゴシップで Raft 迂回（高スループット） |

---

## 代替案

- **CRDT + LWW-Register を Sector-local に採用（現 roadmap 8D-2 の文言）**: 単一所有の下で競合が起きないため
  競合解決機構が遊ぶ。所有権移転ではマージが誤り。物理時刻前提（LWW）が FBD-003 と衝突。**却下。**
- **Sector-local も Raft に通す**: 厳密だが、大量の Sector-local 書き込みを毎回合意に通すとスループットが死ぬ。
  Raft を transit 専用にした ADR-0014 / 高スループット方針に反する。却下。
- **state-based 全状態ゴシップ（毎回フル状態をマージ）**: 帯域が状態サイズに比例。追記ログの差分配布
  （op-based 相当）の方が安い。却下（差分 + アンチエントロピーを採る）。
- **何もしない（複製しない）**: 単一ノードなら可だが 8D（物理分散）で read-replica / failover が要る。却下。

---

## スコープ外（本 ADR では決めない）

- ゴシップの具体プロトコル（push/pull/push-pull、fanout、周期）— dawn-distributed 実装時に決める。
- ワイヤ形式（postcard 再利用か dawn-proto か）— 別途（dawn-proto の要否は未決・本 ADR とは独立）。
- スナップショット転送の具体プロトコルは本ADRで決めない。現在は
  `CatchUpManager` が retained public tail のギャップを検出し、スナップショット
  fallback後に `ReplicaSet::install_snapshot` から suffix catch-up を再開する。
- 非権威ソフト状態への将来的な局所 CRDT 採用 — 要件が生じた時に個別 ADR。

---

## 実装チェックリスト

- [x] 本 ADR を人間が承認する（proposed → accepted・2026-06-15）
- [x] CLAUDE.md §1 の「CRDT と Raft の責務分離」を「追記ログのゴシップ配布 と Raft の責務分離」へ
      文言修正（人間承認のうえ適用・2026-06-15）。§3/§8/§11 の CRDT 言及も整合のため更新。INV は不変
- [x] roadmap 8D-2 を `dawn-distributed（追記ログのゴシップ配布 + アンチエントロピー + スナップショット転送）`
      に改め、CRDT/LWW を外す（2026-06-15）
- [x] architecture.md §5（将来スコープ）の「CRDT による最終一貫性」を「追記ログのゴシップ配布による
      最終一貫性（単一所有のため競合解決 CRDT は不要）」へ更新（2026-06-15）
- [x] （8D-2b）log index アンチエントロピー（`iter_from` 再利用）+ 重複/overlap/gap 判定テスト
- [x] （8D-2c）TCP gossip 配布（4-byte length prefix + postcard / LAN plaintext）+ 受信テスト
- [x] （8D 後続）retained public tail より遅れた複製が
      `CatchUpManager` のスナップショット fallback を使い、
      `ReplicaSet::install_snapshot` 後に snapshot index から catch-up を再開するテスト
      （`compacted_gap_falls_back_to_snapshot_then_resumes_at_snapshot_index`）

---

*提案: 2026-06-15。同日 accepted（人間承認済み）。CLAUDE.md §1/§3/§8/§11 文言・roadmap 8D-2・
architecture.md §5・README インデックスへ反映済み。*
